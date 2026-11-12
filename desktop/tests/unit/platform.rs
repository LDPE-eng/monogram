use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use relm4::gtk::{gio, prelude::*};
use zeroize::Zeroizing;

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "monogram-platform-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn lock(&self) -> PathBuf {
        self.0.join("key.lock")
    }

    fn xdg(&self) -> xdg::BaseDirectories {
        let mut dirs = xdg::BaseDirectories::with_prefix("monogram");
        dirs.config_home = Some(self.0.join("config"));
        dirs.data_home = Some(self.0.join("data"));
        dirs.cache_home = Some(self.0.join("cache"));
        dirs.state_home = Some(self.0.join("state"));
        dirs.runtime_dir = Some(self.0.clone());
        dirs
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct MockBackend {
    value: Mutex<Option<Zeroizing<Vec<u8>>>>,
    writes: AtomicUsize,
    read_error: bool,
    write_error: bool,
    corrupt_write: bool,
}

impl SessionKeyBackend for MockBackend {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError> {
        if self.read_error {
            return Err(PlatformError::SecretServiceLocked);
        }
        Ok(self.value.lock().unwrap().clone())
    }

    fn write(&self, value: &[u8]) -> Result<(), PlatformError> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if self.write_error {
            return Err(PlatformError::SecretServiceUnavailable);
        }
        let mut encoded = Zeroizing::new(value.to_vec());
        if self.corrupt_write {
            encoded[0] = if encoded[0] == b'0' { b'1' } else { b'0' };
        }
        *self.value.lock().unwrap() = Some(encoded);
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
enum SecretFailure {
    #[default]
    None,
    Locked,
    Unavailable,
}

struct TestSecretStore {
    values: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
    failure: SecretFailure,
}

impl TestSecretStore {
    fn new() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
            failure: SecretFailure::None,
        }
    }

    fn locked() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
            failure: SecretFailure::Locked,
        }
    }

    fn unavailable() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
            failure: SecretFailure::Unavailable,
        }
    }
}

impl SecretStore for TestSecretStore {
    fn read(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, PlatformError> {
        match self.failure {
            SecretFailure::Locked => return Err(PlatformError::SecretServiceLocked),
            SecretFailure::Unavailable => return Err(PlatformError::SecretServiceUnavailable),
            SecretFailure::None => {}
        }
        Ok(self.values.lock().unwrap().get(name).cloned())
    }

    fn write(&self, name: &str, secret: &[u8]) -> Result<(), PlatformError> {
        match self.failure {
            SecretFailure::Locked => return Err(PlatformError::SecretServiceLocked),
            SecretFailure::Unavailable => return Err(PlatformError::SecretServiceUnavailable),
            SecretFailure::None => {}
        }
        self.values
            .lock()
            .unwrap()
            .insert(name.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), PlatformError> {
        match self.failure {
            SecretFailure::Locked => return Err(PlatformError::SecretServiceLocked),
            SecretFailure::Unavailable => return Err(PlatformError::SecretServiceUnavailable),
            SecretFailure::None => {}
        }
        self.values.lock().unwrap().remove(name);
        Ok(())
    }
}

#[test]
fn session_key_creates_once_and_never_writes_a_key_to_the_lock_file() {
    let directory = TestDirectory::new();
    let backend = MockBackend::default();
    let first = Zeroizing::new(session_key_with_backend(&backend, &directory.lock()).unwrap());
    let second = Zeroizing::new(session_key_with_backend(&backend, &directory.lock()).unwrap());
    assert!(*first == *second);
    assert_eq!(backend.writes.load(Ordering::Relaxed), 1);
    let value = backend.value.lock().unwrap();
    let encoded = value.as_ref().unwrap();
    assert_eq!(encoded.len(), 64);
    assert!(encoded.iter().all(u8::is_ascii_hexdigit));
    assert_eq!(fs::metadata(directory.lock()).unwrap().len(), 0);
    assert_eq!(
        fs::metadata(directory.lock()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn concurrent_get_or_create_returns_one_key() {
    let directory = TestDirectory::new();
    let backend = Arc::new(MockBackend::default());
    let barrier = Arc::new(Barrier::new(12));
    let workers: Vec<_> = (0..12)
        .map(|_| {
            let backend = Arc::clone(&backend);
            let barrier = Arc::clone(&barrier);
            let lock = directory.lock();
            thread::spawn(move || {
                barrier.wait();
                Zeroizing::new(session_key_with_backend(backend.as_ref(), &lock).unwrap())
            })
        })
        .collect();
    let keys: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(keys.iter().all(|key| **key == *keys[0]));
    assert_eq!(backend.writes.load(Ordering::Relaxed), 1);
}

#[test]
fn existing_invalid_key_is_not_replaced() {
    let directory = TestDirectory::new();
    for invalid in [vec![0; 32], vec![b'x'; 64], vec![b'a'; 63]] {
        let backend = MockBackend {
            value: Mutex::new(Some(Zeroizing::new(invalid))),
            ..Default::default()
        };
        assert!(matches!(
            session_key_with_backend(&backend, &directory.lock()),
            Err(PlatformError::InvalidSessionKey)
        ));
        assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn locked_or_unavailable_service_has_no_fallback() {
    let directory = TestDirectory::new();
    let backend = MockBackend {
        read_error: true,
        ..Default::default()
    };
    assert!(matches!(
        session_key_with_backend(&backend, &directory.lock()),
        Err(PlatformError::SecretServiceLocked)
    ));
    assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
    let backend = MockBackend {
        write_error: true,
        ..Default::default()
    };
    assert!(matches!(
        session_key_with_backend(&backend, &directory.lock()),
        Err(PlatformError::SecretServiceUnavailable)
    ));
    assert!(backend.value.lock().unwrap().is_none());
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn unsuccessful_readback_never_returns_a_generated_key() {
    let directory = TestDirectory::new();
    let backend = MockBackend {
        corrupt_write: true,
        ..Default::default()
    };
    assert!(matches!(
        session_key_with_backend(&backend, &directory.lock()),
        Err(PlatformError::SessionKeyVerificationFailed)
    ));
}

#[test]
fn invalid_lock_path_fails_before_accessing_the_backend() {
    let directory = TestDirectory::new();
    let backend = MockBackend::default();
    assert!(matches!(
        session_key_with_backend(&backend, &directory.0.join("missing/key.lock")),
        Err(PlatformError::Io(_))
    ));
    assert_eq!(backend.writes.load(Ordering::Relaxed), 0);
}

#[test]
fn keyring_errors_do_not_expose_secret_bytes() {
    let error = session::keyring_error(keyring::Error::BadEncoding(b"sensitive-material".to_vec()));
    assert!(matches!(error, PlatformError::InvalidSessionKey));
    assert!(!format!("{error:?}: {error}").contains("sensitive-material"));
}

#[test]
fn api_credentials_roundtrip_and_reject_invalid_values() {
    let store = TestSecretStore::new();
    let hash = "0123456789abcdef0123456789abcdef";
    store_api_credentials_with(&store, 12345, hash).unwrap();
    let loaded = api_credentials_with(&store).unwrap().unwrap();
    assert_eq!(loaded.api_id, 12345);
    assert_eq!(loaded.api_hash.as_str(), hash);
    assert!(store_api_credentials_with(&store, 0, hash).is_err());
    assert!(store_api_credentials_with(&store, 12345, "short").is_err());
    assert!(store_api_credentials_with(&store, 12345, &"g".repeat(32)).is_err());
    assert!(store_api_credentials_with(&store, 12345, &"1".repeat(31)).is_err());
}

#[test]
fn api_credentials_are_independent_entries_and_delete_is_idempotent() {
    let store = TestSecretStore::new();
    assert_eq!(api_credentials_with(&store).unwrap(), None);
    delete_api_credentials_with(&store).unwrap();
    store_api_credentials_with(&store, 42, &"a".repeat(32)).unwrap();
    delete_api_credentials_with(&store).unwrap();
    assert_eq!(api_credentials_with(&store).unwrap(), None);
    delete_api_credentials_with(&store).unwrap();
}

#[test]
fn api_credentials_without_secret_service_never_fall_back() {
    for store in [TestSecretStore::locked(), TestSecretStore::unavailable()] {
        assert!(matches!(
            store_api_credentials_with(&store, 1, &"a".repeat(32)),
            Err(PlatformError::SecretServiceLocked | PlatformError::SecretServiceUnavailable)
        ));
        assert!(matches!(
            api_credentials_with(&store),
            Err(PlatformError::SecretServiceLocked | PlatformError::SecretServiceUnavailable)
        ));
        assert!(store.values.lock().unwrap().is_empty());
    }
}

#[test]
fn proxy_credentials_roundtrip_without_plaintext_password_or_host() {
    let store = TestSecretStore::new();
    let credentials = ProxyCredentials::new(
        "proxy.example.org".into(),
        1080,
        "user".into(),
        "p@ssw0rd".into(),
    );
    store_proxy_credentials_with(&store, &credentials).unwrap();
    let encoded = store.values.lock().unwrap().get("proxy").unwrap().clone();
    let encoded = std::str::from_utf8(&encoded).unwrap();
    assert!(encoded.bytes().all(|byte| byte.is_ascii()));
    assert!(!encoded.contains("p@ssw0rd"));
    assert!(!encoded.contains("proxy.example.org"));
    let loaded = proxy_credentials_with(&store).unwrap().unwrap();
    assert_eq!(loaded, credentials);
}

#[test]
fn proxy_credentials_validate_and_redact_debug_output() {
    assert!(
        store_proxy_credentials_with(
            &TestSecretStore::new(),
            &ProxyCredentials::new(String::new(), 1080, "user".into(), "pw".into()),
        )
        .is_err()
    );
    assert!(
        store_proxy_credentials_with(
            &TestSecretStore::new(),
            &ProxyCredentials::new("host".into(), 0, "user".into(), "pw".into()),
        )
        .is_err()
    );
    let debug = format!(
        "{:?}",
        ProxyCredentials::new("host".into(), 1080, "user".into(), "pw".into())
    );
    assert!(!debug.contains("pw"));
    assert!(
        proxy_credentials_with(&TestSecretStore::new())
            .unwrap()
            .is_none()
    );
    delete_proxy_credentials_with(&TestSecretStore::new()).unwrap();
}

#[test]
fn corrupted_proxy_blob_is_rejected_without_being_returned() {
    let store = TestSecretStore::new();
    store.write("proxy", b"not-base64").unwrap();
    assert!(matches!(
        proxy_credentials_with(&store),
        Err(PlatformError::InvalidProxyCredentials)
    ));
}

#[test]
fn xdg_paths_include_private_state_and_runtime_directories() {
    let directory = TestDirectory::new();
    let paths = AppPaths::from_xdg(&directory.xdg()).unwrap();
    assert_eq!(
        paths.database,
        directory.0.join("data/monogram/monogram.sqlite3")
    );
    assert_eq!(paths.state, directory.0.join("state/monogram"));
    assert_eq!(paths.runtime, directory.0.join("monogram"));
    for path in [
        &paths.config,
        &paths.data,
        &paths.cache,
        &paths.state,
        &paths.runtime,
    ] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

#[test]
fn missing_or_insecure_runtime_is_rejected_without_tmp_fallback() {
    let directory = TestDirectory::new();
    let mut dirs = directory.xdg();
    dirs.runtime_dir = None;
    assert!(AppPaths::from_xdg(&dirs).is_err());
    dirs.runtime_dir = Some(directory.0.clone());
    fs::set_permissions(&directory.0, Permissions::from_mode(0o755)).unwrap();
    assert!(AppPaths::from_xdg(&dirs).is_err());
}

fn target(message_id: Option<i64>) -> NotificationTarget {
    NotificationTarget {
        account_id: "test-account".into(),
        chat_id: 123,
        message_id,
    }
}

#[test]
fn notification_actions_follow_server_capabilities() {
    let unavailable = NotificationCapabilities::default();
    assert!(unavailable.actions_for(&target(Some(9))).is_empty());
    let capabilities =
        NotificationCapabilities::from_names(&["actions".into(), "inline-reply".into()]);
    let actions = capabilities.actions_for(&target(Some(9)));
    assert_eq!(
        actions,
        vec![
            NotificationAction::OpenMessage(target(Some(9))),
            NotificationAction::MarkRead(target(Some(9))),
            NotificationAction::Mute(target(Some(9))),
            NotificationAction::Settings,
        ]
    );
    assert_eq!(
        capabilities.actions_for(&target(None))[0],
        NotificationAction::OpenChat(target(None))
    );
    assert!(!capabilities.supports_inline_reply());
}

#[test]
fn gio_notification_actions_route_without_bus_or_gtk_initialization() {
    let application = gio::Application::new(
        Some("io.github.monogram.platform-test"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    let received = Rc::new(RefCell::new(Vec::new()));
    let output = Rc::clone(&received);
    let _adapter = NotificationAdapter::new(
        &application,
        NotificationCapabilities { actions: true },
        move |action| {
            output.borrow_mut().push(action);
        },
    );
    let action = application.lookup_action("notification").unwrap();
    let expected = NotificationAction::OpenMessage(target(Some(9)));
    action.activate(Some(
        &serde_json::to_string(&expected).unwrap().to_variant(),
    ));
    action.activate(Some(&"invalid-json".to_variant()));
    assert_eq!(*received.borrow(), vec![expected]);
}

#[test]
fn notifications_are_not_sent_from_an_unregistered_application() {
    let application = gio::Application::new(
        Some("io.github.monogram.platform-test-unregistered"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    let adapter =
        NotificationAdapter::new(&application, NotificationCapabilities::default(), |_| {});
    let content = NotificationContent {
        id: "test",
        title: "Monogram",
        body: None,
        target: target(None),
    };
    let labels = NotificationLabels {
        mark_read: "Read",
        mute: "Mute",
        settings: "Settings",
    };
    assert!(matches!(
        adapter.send(&content, &labels),
        Err(PlatformError::ApplicationNotRegistered)
    ));
    assert!(matches!(
        adapter.withdraw("test"),
        Err(PlatformError::ApplicationNotRegistered)
    ));
    assert!(!application.is_registered());
}
