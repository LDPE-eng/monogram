use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::Instant,
};

use mono_proto_client::adapter::{
    runtime::scheduler,
    socket::{ProxyConfig, ProxyKind, TransportMode},
};
use zeroize::Zeroizing;

use crate::{
    client::{RequestGuards, Runtime, WorkerEvent},
    domain::{AuthState, ClientError, ClientState, ConnectionState},
    storage::Storage,
};

#[derive(Clone)]
struct MemorySecrets(std::sync::Arc<Mutex<BTreeMap<String, String>>>);

impl MemorySecrets {
    fn new() -> Self {
        Self(std::sync::Arc::new(Mutex::new(BTreeMap::new())))
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        self.0.lock().unwrap().clone()
    }
}

impl SecretBackend for MemorySecrets {
    fn read(&self, key: &str) -> Result<Option<String>, ClientError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn write(&self, key: &str, value: &str) -> Result<(), ClientError> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), ClientError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Clone)]
struct MemoryConfig(std::sync::Arc<Mutex<StoredConfig>>);

impl MemoryConfig {
    fn new(state: StoredConfig) -> Self {
        Self(std::sync::Arc::new(Mutex::new(state)))
    }

    fn snapshot(&self) -> StoredConfig {
        self.0.lock().unwrap().clone()
    }
}

impl ConfigBackend for MemoryConfig {
    fn load(&self) -> Result<StoredConfig, ClientError> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn save(&self, state: &StoredConfig) -> Result<(), ClientError> {
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}

fn store() -> (ProxyStore, MemorySecrets, MemoryConfig) {
    let secrets = MemorySecrets::new();
    let config = MemoryConfig::new(StoredConfig::default());
    let store = ProxyStore::with_backends(Box::new(secrets.clone()), Box::new(config.clone()));
    (store, secrets, config)
}

fn socks_spec() -> ProxySpec {
    ProxySpec {
        kind: ProxyKind::Socks5,
        host: "127.0.0.1".into(),
        port: 1080,
        username: Some("user".into()),
        password: Some(Zeroizing::new("s3cret".into())),
        mtproto_secret: None,
        fake_tls_domain: None,
    }
}

fn mtproto_spec() -> ProxySpec {
    ProxySpec {
        kind: ProxyKind::Mtproto,
        host: "127.0.0.1".into(),
        port: 443,
        username: None,
        password: None,
        mtproto_secret: Some(Zeroizing::new([7_u8; 16])),
        fake_tls_domain: None,
    }
}

fn plain_proxy(port: u16) -> ProxyConfig {
    ProxyConfig {
        kind: ProxyKind::Socks5,
        host: "127.0.0.1".into(),
        port,
        username: None,
        password: None,
        secret: None,
        fake_tls_domain: None,
    }
}

fn mtproto_proxy() -> ProxyConfig {
    ProxyConfig {
        kind: ProxyKind::Mtproto,
        host: "127.0.0.1".into(),
        port: 443,
        username: None,
        password: None,
        secret: Some([7_u8; 16]),
        fake_tls_domain: None,
    }
}

#[test]
fn proxy_spec_validation_rejects_unsafe_combinations() {
    assert!(validate_spec(&socks_spec()).is_ok());
    assert!(validate_spec(&mtproto_spec()).is_ok());
    assert!(
        validate_spec(&ProxySpec {
            host: String::new(),
            ..socks_spec()
        })
        .is_err()
    );
    assert!(
        validate_spec(&ProxySpec {
            port: 0,
            ..socks_spec()
        })
        .is_err()
    );
    assert!(
        validate_spec(&ProxySpec {
            username: None,
            password: Some(Zeroizing::new("password".into())),
            ..socks_spec()
        })
        .is_err()
    );
    assert!(
        validate_spec(&ProxySpec {
            kind: ProxyKind::Mtproto,
            mtproto_secret: None,
            ..mtproto_spec()
        })
        .is_err()
    );
    assert!(
        validate_spec(&ProxySpec {
            fake_tls_domain: Some("example.com".into()),
            ..socks_spec()
        })
        .is_err()
    );
}

#[test]
fn proxy_links_parse_telegram_formats_and_percent_decoding() {
    let link = format!(
        "tg://proxy?server=1.2.3.4&port=443&secret=dd{}",
        "00".repeat(16)
    );
    let spec = parse_proxy_link(&link).unwrap();
    assert_eq!(spec.kind, ProxyKind::Mtproto);
    assert_eq!(spec.host, "1.2.3.4");
    assert_eq!(spec.port, 443);
    assert_eq!(*spec.mtproto_secret.unwrap(), [0_u8; 16]);

    let fake_tls = format!(
        "tg://proxy?server=example.com&port=443&secret=ee{}6578616d706c652e636f6d",
        "00".repeat(16)
    );
    let spec = parse_proxy_link(&fake_tls).unwrap();
    assert_eq!(spec.fake_tls_domain.as_deref(), Some("example.com"));

    let socks =
        parse_proxy_link("tg://socks?server=127.0.0.1&port=1080&user=alice&pass=s3cret").unwrap();
    assert_eq!(socks.kind, ProxyKind::Socks5);
    assert_eq!(socks.username.as_deref(), Some("alice"));
    assert_eq!(
        socks.password.as_deref().map(|password| password.as_str()),
        Some("s3cret")
    );

    let encoded = parse_proxy_link("tg://socks?server=10%2E0%2E0%2E1&port=1080").unwrap();
    assert_eq!(encoded.host, "10.0.0.1");

    assert!(parse_proxy_link("https://example.com").is_err());
    assert!(parse_proxy_link("tg://proxy?server=x&port=0").is_err());
    assert!(parse_proxy_link("tg://socks?server=x").is_err());
    assert!(parse_proxy_link("tg://proxy?server=x&port=443&secret=zz").is_err());
}

#[test]
fn secret_and_transport_serialization_round_trip() {
    for kind in [
        ProxyKind::Socks5,
        ProxyKind::Http,
        ProxyKind::Https,
        ProxyKind::Mtproto,
    ] {
        assert_eq!(kind_from_name(kind_name(kind.clone())), Some(kind));
    }
    assert_eq!(kind_from_name("unknown"), None);
    for mode in [TransportMode::PaddedIntermediate, TransportMode::Http] {
        assert_eq!(transport_from_name(transport_name(mode)), Some(mode));
    }
    assert_eq!(transport_from_name("unknown"), None);

    let secret = [0xab_u8; 16];
    assert_eq!(
        decode_mtproto_secret_hex(&encode_mtproto_secret(&secret)).unwrap(),
        secret
    );
    assert!(decode_mtproto_secret_hex("00").is_err());
}

#[test]
fn transport_restrictions_block_http_over_mtproto_proxy() {
    assert!(validate_transport_combo(TransportMode::Http, Some(&mtproto_proxy())).is_err());
    assert!(validate_transport_combo(TransportMode::Http, Some(&plain_proxy(1080))).is_ok());
    assert!(validate_transport_combo(TransportMode::Http, None).is_ok());
    assert!(
        validate_transport_combo(TransportMode::PaddedIntermediate, Some(&mtproto_proxy())).is_ok()
    );
}

#[test]
fn notify_update_and_probe_id_validation_are_strict() {
    assert!(validate_notify_update(&NotifyUpdate::reset()).is_ok());
    assert!(
        validate_notify_update(&NotifyUpdate {
            mute_until: -1,
            ..NotifyUpdate::reset()
        })
        .is_err()
    );
    assert!(
        validate_notify_update(&NotifyUpdate {
            sound: "loud".into(),
            ..NotifyUpdate::reset()
        })
        .is_err()
    );
    assert!(
        validate_notify_update(&NotifyUpdate {
            sound: "none".into(),
            ..NotifyUpdate::reset()
        })
        .is_ok()
    );

    assert!(validate_probe_ids(&[1, 2, 3]).is_ok());
    assert!(validate_probe_ids(&[]).is_err());
    assert!(validate_probe_ids(&[1, 1]).is_err());
    assert!(validate_probe_ids(&[0]).is_err());
}

#[test]
fn command_class_separates_reads_writes_and_probes() {
    assert_eq!(
        command_class(&ConnectivityCommand::ListProxies),
        scheduler::RequestClass::InteractiveRead
    );
    assert_eq!(
        command_class(&ConnectivityCommand::AddProxy { spec: socks_spec() }),
        scheduler::RequestClass::InteractiveWrite
    );
    assert_eq!(
        command_class(&ConnectivityCommand::ProbeProxies { ids: vec![1] }),
        scheduler::RequestClass::BackgroundRead
    );
    assert_eq!(
        command_class(&ConnectivityCommand::SetPeerNotify {
            chat_id: 42,
            settings: NotifyUpdate::reset(),
        }),
        scheduler::RequestClass::InteractiveWrite
    );
}

#[test]
fn probe_runner_caps_concurrency_at_three_and_maps_latency_and_errors() {
    let concurrent = std::sync::Arc::new(AtomicUsize::new(0));
    let peak = std::sync::Arc::new(AtomicUsize::new(0));
    let probe = |config: ProxyConfig| -> Result<i64, String> {
        let current = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(current, Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(5));
        concurrent.fetch_sub(1, Ordering::SeqCst);
        if config.port == 0 {
            Err("boom".into())
        } else {
            Ok(7)
        }
    };
    let targets: Vec<_> = (0_u16..7)
        .map(|port| (u64::from(port), plain_proxy(port)))
        .collect();
    let results = run_probes(&targets, &probe);
    assert_eq!(results.len(), 7);
    assert!(peak.load(Ordering::SeqCst) <= 3);
    assert_eq!(
        results
            .iter()
            .filter(|result| result.latency_ms == Some(7))
            .count(),
        6
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| result.error.as_deref() == Some("boom"))
            .count(),
        1
    );
}

#[test]
fn proxy_store_round_trips_metadata_and_keeps_secrets_in_secret_service() {
    let (store, secrets, config) = store();
    let socks = store.add(socks_spec()).unwrap();
    assert_ne!(socks.id, 0);
    assert!(socks.has_secret);

    let serialized = serde_json::to_string(&config.snapshot()).unwrap();
    assert!(!serialized.contains("s3cret"));

    let key = secret_key(socks.id);
    assert_eq!(
        secrets.snapshot().get(&key).map(String::as_str),
        Some("s3cret")
    );

    let config = store.config_for(socks.id).unwrap();
    assert_eq!(config.kind, ProxyKind::Socks5);
    assert_eq!(config.password.as_deref(), Some("s3cret"));

    let active = store.activate(socks.id).unwrap();
    assert!(active.active);
    assert_eq!(store.active().unwrap().unwrap().id, socks.id);
    assert_eq!(
        store.active_config().unwrap().unwrap().password.as_deref(),
        Some("s3cret")
    );

    store.set_direct().unwrap();
    assert!(store.active().unwrap().is_none());

    let mtproto = store.edit(socks.id, mtproto_spec()).unwrap();
    assert_eq!(mtproto.kind, ProxyKind::Mtproto);
    assert!(mtproto.has_secret);
    let mtproto_hex = "07".repeat(16);
    assert_eq!(
        secrets.snapshot().get(&key).map(String::as_str),
        Some(mtproto_hex.as_str())
    );

    let no_secret = ProxySpec {
        kind: ProxyKind::Socks5,
        host: "127.0.0.1".into(),
        port: 1080,
        username: None,
        password: None,
        mtproto_secret: None,
        fake_tls_domain: None,
    };
    let edited = store.edit(socks.id, no_secret).unwrap();
    assert!(!edited.has_secret);
    assert!(!secrets.snapshot().contains_key(&key));

    store.delete(socks.id).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(!secrets.snapshot().contains_key(&key));
}

#[test]
fn transport_mode_is_persisted_and_restored() {
    let (store, _secrets, config) = store();
    assert_eq!(
        store.transport().unwrap(),
        TransportMode::PaddedIntermediate
    );
    store.set_transport(TransportMode::Http).unwrap();
    assert_eq!(store.transport().unwrap(), TransportMode::Http);
    assert_eq!(config.snapshot().transport_mode, "http");
}

#[test]
fn authorized_connectivity_operations_require_a_configured_client() {
    let (mut runtime, _receiver) = offline_runtime();
    let result = runtime.execute_connectivity(ConnectivityCommand::GetDeviceSessions);
    assert!(matches!(result, Err(ClientError::NotConfigured)));
}

fn offline_runtime() -> (Runtime, mpsc::Receiver<WorkerEvent>) {
    use rand::{TryRng, rngs::SysRng};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/connectivity-test-{}",
        SysRng.try_next_u64().unwrap()
    ));
    let media_cache = crate::media::cache::MediaCache::open(
        root.join("cache"),
        crate::media::cache::CacheLimits::default(),
    )
    .unwrap();
    let media_staging = root.join("staging");
    std::fs::create_dir_all(&media_staging).unwrap();
    let (events, receiver) = mpsc::channel();
    (
        Runtime {
            storage: Storage::open(Path::new(":memory:")).unwrap(),
            session_path: PathBuf::from("unused.session"),
            media_cache,
            media_staging,
            client: None,
            state: ClientState {
                connection: ConnectionState::Connected,
                auth: AuthState::Authorized { user_id: 1 },
                api_id: Some(1),
            },
            phone: None,
            events,
            next_update: Instant::now(),
            active: std::sync::Arc::new(AtomicU64::new(0)),
            active_media: std::sync::Arc::new(AtomicU64::new(0)),
            stopping: std::sync::Arc::new(AtomicBool::new(false)),
            guards: std::sync::Arc::new(RequestGuards::default()),
        },
        receiver,
    )
}
