use super::*;
use std::path::Path;

fn offline_resources() -> RuntimeResources {
    use rand::{TryRng, rngs::SysRng};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "client-media-test-{}",
            SysRng.try_next_u64().unwrap()
        ));
    let media_cache = crate::media::cache::MediaCache::open(
        root.join("cache"),
        crate::media::cache::CacheLimits::default(),
    )
    .unwrap();
    let media_staging = root.join("staging");
    std::fs::create_dir_all(&media_staging).unwrap();
    RuntimeResources {
        storage: Storage::open(Path::new(":memory:")).unwrap(),
        session_path: PathBuf::from("unused.session"),
        media_cache,
        media_staging,
    }
}

fn offline_runtime() -> (Runtime, Receiver<WorkerEvent>) {
    let resources = offline_resources();
    let (events, receiver) = mpsc::channel();
    (
        Runtime {
            storage: resources.storage,
            session_path: resources.session_path,
            media_cache: resources.media_cache,
            media_staging: resources.media_staging,
            client: None,
            state: ClientState {
                connection: ConnectionState::Connected,
                auth: AuthState::Authorized { user_id: 1 },
                api_id: Some(1),
            },
            phone: None,
            events,
            next_update: Instant::now(),
            active: Arc::new(AtomicU64::new(0)),
            active_media: Arc::new(AtomicU64::new(0)),
            stopping: Arc::new(AtomicBool::new(false)),
            guards: Arc::new(RequestGuards::default()),
        },
        receiver,
    )
}

fn offline_worker() -> (Worker, Receiver<WorkerEvent>) {
    Worker::spawn(|| Ok(offline_resources())).unwrap()
}

#[test]
fn worker_initializes_handles_commands_and_shuts_down_without_network() {
    let (worker, events) = offline_worker();
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        WorkerEvent::Ready
    ));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        WorkerEvent::State(ClientState {
            connection: ConnectionState::Unconfigured,
            ..
        })
    ));
    worker.submit(7, WorkerCommand::Connect).unwrap();
    loop {
        if let WorkerEvent::Failed { request_id, error } =
            events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert_eq!(request_id, Some(7));
            assert_eq!(error, ClientError::NotConfigured);
            break;
        }
    }
    worker.shutdown().unwrap();
    assert!(
        events
            .try_iter()
            .any(|event| matches!(event, WorkerEvent::Stopped))
    );
    assert!(events.recv().is_err());
}

#[test]
fn worker_reports_initialization_failure() {
    let (worker, events) = Worker::spawn(|| Err(ClientError::Storage)).unwrap();
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        WorkerEvent::StartupFailed
    ));
    drop(worker);
}

#[test]
fn requests_preserve_order_on_single_worker() {
    let (worker, events) = offline_worker();
    for id in 1..=10 {
        worker.submit(id, WorkerCommand::PendingSends).unwrap();
    }
    let mut received = Vec::new();
    while received.len() < 10 {
        if let WorkerEvent::Failed {
            request_id: Some(id),
            ..
        } = events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            received.push(id);
        }
    }
    assert_eq!(received, (1..=10).collect::<Vec<_>>());
    worker.shutdown().unwrap();
}

#[test]
fn cancellation_scope_restores_previous_thread_binding() {
    let previous = request_control::create();
    let original = request_control::bind(previous);
    let active = Arc::new(AtomicU64::new(0));
    let stopping = AtomicBool::new(false);
    {
        let _scope = RequestScope::enter(&active, &stopping).unwrap();
        let current = active.load(Ordering::Acquire);
        assert_ne!(current, previous);
        request_control::cancel(current);
        assert!(request_control::check().is_err());
    }
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert_eq!(request_control::current_id(), previous);
    assert!(request_control::check().is_ok());
    request_control::bind(original);
    request_control::release(previous);
}

#[test]
fn shutdown_prevents_a_new_request_from_starting() {
    let active = Arc::new(AtomicU64::new(0));
    assert!(matches!(
        RequestScope::enter(&active, &AtomicBool::new(true)),
        Err(ClientError::Cancelled)
    ));
    assert_eq!(active.load(Ordering::Acquire), 0);
}

#[test]
fn events_redact_sensitive_payloads() {
    let event = WorkerEvent::Response {
        request_id: 1,
        response: WorkerResponse::QrToken {
            token: b"super-secret-token".to_vec(),
            expires: 123,
        },
    };
    let debug = format!("{event:?}");
    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("123"));
}

#[test]
fn pinned_encrypted_handle_lifecycle_needs_no_network() {
    use rand::{TryRng, rngs::SysRng};
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/encrypted-client-test-{}.session",
        SysRng.try_next_u64().unwrap()
    ));
    assert!(
        proto::create_encrypted_client(
            1,
            "a".repeat(32),
            path.to_string_lossy().into_owned(),
            vec![1; 8]
        )
        .is_err()
    );
    let handle = ClientHandle(
        proto::create_encrypted_client(
            1,
            "a".repeat(32),
            path.to_string_lossy().into_owned(),
            vec![1; 32],
        )
        .unwrap(),
    );
    let id = handle.0;
    assert!(proto::registry::client_exists(id));
    assert!(!proto::is_authorized(id).unwrap());
    drop(handle);
    assert!(!proto::registry::client_exists(id));
    proto::session_store::FileSessionStore::new(path)
        .clear()
        .unwrap();
}

#[test]
fn event_payload_size_stays_bounded() {
    assert!(std::mem::size_of::<WorkerEvent>() <= 256);
    assert!(std::mem::size_of::<WorkerResponse>() <= 192);
}

#[test]
fn guarded_requests_skip_superseded_queries_and_prior_generations() {
    let (release, barrier) = mpsc::channel();
    let (worker, events) = Worker::spawn(move || {
        barrier.recv().unwrap();
        Ok(offline_resources())
    })
    .unwrap();
    let generation = worker.generation();
    worker
        .submit_guarded(1, generation, 10, WorkerCommand::GetFolders)
        .unwrap();
    worker
        .submit_guarded(2, generation, 10, WorkerCommand::GetFolders)
        .unwrap();
    assert_eq!(
        worker.submit_guarded(2, generation, 10, WorkerCommand::GetFolders),
        Err(ClientError::InvalidRequest)
    );
    worker.submit(3, WorkerCommand::Destroy).unwrap();
    assert_ne!(generation, worker.generation());
    assert_eq!(
        worker.submit_guarded(4, generation, 10, WorkerCommand::GetFolders),
        Err(ClientError::Cancelled)
    );
    worker
        .submit_guarded(4, worker.generation(), 10, WorkerCommand::GetFolders)
        .unwrap();
    assert_eq!(
        worker.submit_guarded(
            5,
            worker.generation(),
            10,
            WorkerCommand::MarkDialogUnread {
                chat_id: 42,
                unread: true
            }
        ),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        worker.submit_guarded(5, worker.generation(), 10, WorkerCommand::Connect),
        Err(ClientError::InvalidRequest)
    );
    release.send(()).unwrap();
    let mut failures = Vec::new();
    while failures.len() < 3 {
        if let WorkerEvent::Failed {
            request_id: Some(id),
            error,
        } = events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            failures.push((id, error));
        }
    }
    assert_eq!(
        failures,
        vec![
            (1, ClientError::Cancelled),
            (2, ClientError::Cancelled),
            (4, ClientError::NotConfigured)
        ]
    );
    worker.shutdown().unwrap();
}

#[test]
fn newer_query_ids_invalidate_only_their_scope() {
    let guards = RequestGuards::default();
    let old = guards.register(1, Some(0), Some(10), false).unwrap();
    let other = guards.register(2, Some(0), Some(20), false).unwrap();
    let new = guards.register(3, Some(0), Some(10), false).unwrap();
    assert!(!guards.current(old, 1));
    assert!(guards.current(other, 2));
    assert!(guards.current(new, 3));
}

#[test]
fn selection_and_entity_validation_reject_unsafe_inputs() {
    assert!(features::valid_selection(42, &[1, 2]).is_ok());
    assert_eq!(
        features::valid_selection(42, &[1, 1]),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        features::valid_selection(42, &[1; 101]),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        features::valid_selection(0, &[1]),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        features::valid_entities(Some("{}")),
        Err(ClientError::InvalidRequest)
    );
    assert!(features::valid_entities(Some("[]")).is_ok());
    assert!(features::valid_filter("photo_video").is_ok());
    assert!(features::valid_filter("").is_ok());
    assert_eq!(
        features::valid_filter("unknown"),
        Err(ClientError::UnsupportedContent)
    );
}

#[test]
fn write_requests_use_interactive_write_priority() {
    assert_eq!(
        command_class(&WorkerCommand::RetryText { random_id: 1 }),
        scheduler::RequestClass::InteractiveWrite
    );
    assert_eq!(
        command_class(&WorkerCommand::RefreshUpdates),
        scheduler::RequestClass::BackgroundRead
    );
    assert_eq!(
        command_class(&WorkerCommand::LoadChats),
        scheduler::RequestClass::InteractiveRead
    );
    assert_eq!(
        command_class(&WorkerCommand::RetryMedia { album_id: 1 }),
        scheduler::RequestClass::InteractiveWrite
    );
    assert_eq!(
        command_class(&WorkerCommand::DownloadMessageMedia {
            chat_id: 42,
            message_id: 1,
            kind: crate::domain::MediaDownloadKind::Full,
            mode: crate::domain::MediaDownloadMode::Automatic,
        }),
        scheduler::RequestClass::BackgroundMedia
    );
    assert_eq!(
        command_class(&WorkerCommand::RetrySpecial { random_id: 1 }),
        scheduler::RequestClass::InteractiveWrite
    );
    assert_eq!(
        command_class(&WorkerCommand::Interaction(
            interactions::InteractionCommand::SendDice {
                chat_id: 42,
                emoticon: "🎲".into(),
                reply_to_msg_id: 0,
                top_msg_id: 0,
                persisted_random_id: 1,
                options: SendOptions::default(),
            }
        )),
        scheduler::RequestClass::InteractiveWrite
    );
    assert_eq!(
        command_class(&WorkerCommand::Web(web::WebCommand::Page {
            url: "https://example.com".into(),
            hash: 0,
        })),
        scheduler::RequestClass::InteractiveRead
    );
    assert_eq!(
        command_class(&WorkerCommand::Appearance(
            appearance::AppearanceCommand::GetPreferences
        )),
        scheduler::RequestClass::InteractiveRead
    );
    assert_eq!(
        command_class(&WorkerCommand::Passkey(passkeys::PasskeyCommand::Delete {
            id: "credential".into(),
        })),
        scheduler::RequestClass::InteractiveWrite
    );
}

#[test]
fn media_active_scope_supports_targeted_cancellation() {
    let active = Arc::new(AtomicU64::new(0));
    {
        let _scope = media::ActiveMedia::enter(&active, 42);
        assert_eq!(active.load(Ordering::Acquire), 42);
    }
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert_eq!(media::transfer_id_from_path("/tmp/17.cancel"), Some(17));
    assert_eq!(media::derived_cache_key("display", "remote").len(), 24);
}

#[test]
fn automatic_policy_is_enforced_before_network_download() {
    use crate::domain::{AutoDownloadRule, MediaCategory, MediaDownloadKind, MediaDownloadMode};
    let (mut runtime, _receiver) = offline_runtime();
    let request = |request_id| media::DownloadRequest {
        request_id,
        handle: 0,
        chat_id: 42,
        message_id: 1,
        kind: MediaDownloadKind::Full,
        mode: MediaDownloadMode::Automatic,
        category: MediaCategory::Photo,
        expected_bytes: Some(1024),
        remote_key: "policy-test".into(),
    };
    let result = runtime.download(request(7));
    assert!(matches!(result, Err(ClientError::Forbidden)));
    let mut policies = runtime.storage.auto_download().unwrap();
    policies.wifi.photo = AutoDownloadRule {
        enabled: true,
        max_bytes: 512,
    };
    runtime.storage.set_auto_download(policies).unwrap();
    let result = runtime.download(request(8));
    assert!(matches!(result, Err(ClientError::InvalidRequest)));
    assert_eq!(runtime.media_cache.usage().unwrap().complete_bytes, 0);
}

#[test]
fn autoplay_and_preload_decisions_follow_network_and_category() {
    use crate::domain::{
        AutoDownloadRule, MediaCategory, MediaDownloadKind, MediaDownloadMode, NetworkPolicy,
    };
    let (mut runtime, _receiver) = offline_runtime();
    let mut automatic = runtime.storage.auto_download().unwrap();
    automatic.wifi.video = AutoDownloadRule {
        enabled: true,
        max_bytes: 4096,
    };
    runtime.storage.set_auto_download(automatic).unwrap();
    runtime
        .storage
        .set_autoplay(NetworkPolicy {
            wifi: true,
            mobile: false,
            roaming: false,
        })
        .unwrap();
    runtime
        .storage
        .set_preload(NetworkPolicy {
            wifi: true,
            mobile: false,
            roaming: false,
        })
        .unwrap();
    assert_eq!(
        runtime.media_decision(MediaCategory::Video).unwrap(),
        crate::domain::MediaDecision {
            autoplay: true,
            preload: true,
            auto_download: true,
        }
    );
    let photo = runtime.media_decision(MediaCategory::Photo).unwrap();
    assert!(!photo.autoplay);
    assert!(!photo.preload);
    let request = media::DownloadRequest {
        request_id: 9,
        handle: 0,
        chat_id: 42,
        message_id: 1,
        kind: MediaDownloadKind::Full,
        mode: MediaDownloadMode::Autoplay,
        category: MediaCategory::Photo,
        expected_bytes: None,
        remote_key: "autoplay-photo".into(),
    };
    assert!(matches!(
        runtime.download(request),
        Err(ClientError::Forbidden)
    ));
}

#[test]
fn scoped_usage_reconciles_real_cache_and_combined_clear() {
    use crate::{
        domain::{ClearMediaScope, MediaCategory},
        media::cache::{Acquire, CacheKey},
    };
    let (mut runtime, _receiver) = offline_runtime();
    let mut complete = match runtime
        .media_cache
        .acquire(CacheKey::new(1, "photo").unwrap(), Some(100))
        .unwrap()
    {
        Acquire::Download(writer) => writer,
        _ => panic!("new photo must be writable"),
    };
    complete.append(0, &[1; 100]).unwrap();
    drop(complete.finish().unwrap());
    let mut partial = match runtime
        .media_cache
        .acquire(CacheKey::new(1, "video").unwrap(), None)
        .unwrap()
    {
        Acquire::Download(writer) => writer,
        _ => panic!("new video must be writable"),
    };
    partial.append(0, &[2; 50]).unwrap();
    partial.cancel(true).unwrap();
    runtime
        .storage
        .record_media("photo", MediaCategory::Photo, Some(42), 100, true)
        .unwrap();
    runtime
        .storage
        .record_media("video", MediaCategory::Video, Some(42), 50, false)
        .unwrap();
    runtime
        .storage
        .record_media("stale", MediaCategory::Other, None, 999, true)
        .unwrap();
    let usage = runtime.media_usage().unwrap();
    assert_eq!(usage.complete_bytes, 100);
    assert_eq!(usage.partial_bytes, 50);
    assert_eq!(usage.by_category.len(), 2);
    assert_eq!(usage.by_chat.len(), 1);
    assert_eq!(usage.by_chat[0].complete_bytes, 100);
    assert_eq!(usage.by_chat[0].partial_bytes, 50);
    assert!(
        runtime
            .storage
            .media_record_keys(ClearMediaScope::All)
            .unwrap()
            .iter()
            .all(|key| key != "stale")
    );
    runtime
        .execute_media(
            10,
            WorkerCommand::ClearMedia(ClearMediaScope::ChatCategory {
                chat_id: 42,
                category: MediaCategory::Photo,
            }),
        )
        .unwrap();
    let usage = runtime.media_usage().unwrap();
    assert_eq!(usage.complete_bytes, 0);
    assert_eq!(usage.partial_bytes, 50);
    assert_eq!(usage.by_category[0].category, MediaCategory::Video);
}

#[test]
fn cached_range_reads_do_not_change_cache_identity() {
    use crate::media::cache::{Acquire, CacheKey};
    let (runtime, _receiver) = offline_runtime();
    let key = CacheKey::new(1, "stream").unwrap();
    let mut writer = match runtime.media_cache.acquire(key.clone(), Some(6)).unwrap() {
        Acquire::Download(writer) => writer,
        _ => panic!("new stream must be writable"),
    };
    writer.append(0, b"abcdef").unwrap();
    let mut lease = writer.finish().unwrap();
    let (bytes, done) = media::read_cached_range(&mut lease, 2, 2, Some(6)).unwrap();
    assert_eq!(bytes, b"cd");
    assert!(!done);
    let (bytes, done) = media::read_cached_range(&mut lease, 4, 2, Some(6)).unwrap();
    assert_eq!(bytes, b"ef");
    assert!(done);
    assert_eq!(runtime.media_cache.usage().unwrap().complete_bytes, 6);
    drop(lease);
    assert!(matches!(
        runtime.media_cache.acquire(key, Some(6)).unwrap(),
        Acquire::Cached(_)
    ));
}
