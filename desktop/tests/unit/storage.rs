use super::*;
use mono_proto_client::adapter::types::updates_state::UpdatesStateDto;
use serde_json::json;

fn message(chat_id: i64, id: i32, text: &str) -> MessageDto {
    serde_json::from_value(json!({
        "chat_id": chat_id, "id": id, "text": text, "date": 10,
        "outgoing": false, "noforwards": false, "supports_streaming": false,
        "replies_count": 0, "forum_topic": false
    }))
    .unwrap()
}

fn text_send() -> TextSend {
    TextSend {
        chat_id: 42,
        text: "persisted message".into(),
        reply_to_msg_id: 0,
        top_msg_id: 0,
        entities_json: None,
        webpage_url: None,
        silent: false,
        no_webpage: false,
        schedule_date: None,
    }
}

fn batch(id: i64, events: Vec<UpdateEventDto>) -> UpdateBatchDto {
    UpdateBatchDto {
        account_generation: 100,
        batch_id: id,
        events,
        cursor_after: Some(UpdatesStateDto {
            pts: id as i32,
            qts: 0,
            seq: id as i32,
            date: 20,
        }),
        channel_pts_after: vec![],
    }
}

#[test]
fn initial_migration_and_legacy_upgrade_preserve_settings() {
    let storage = Storage::open(Path::new(":memory:")).unwrap();
    let version: i64 = storage
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);
    let temporary = DatabaseFile::new();
    let connection = Connection::open(&temporary.0).unwrap();
    connection.execute_batch("CREATE TABLE app_settings(key TEXT PRIMARY KEY NOT NULL,value TEXT NOT NULL); INSERT INTO app_settings VALUES('theme','system'); PRAGMA user_version=1;").unwrap();
    drop(connection);
    let storage = Storage::open(&temporary.0).unwrap();
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='theme'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "system"
    );
}

#[test]
fn outbox_identity_survives_restart_and_ack_is_transactional() {
    let file = DatabaseFile::new();
    let id;
    {
        let storage = Storage::open(&file.0).unwrap();
        storage.configure(123).unwrap();
        id = storage.enqueue(&text_send()).unwrap();
        assert_ne!(id, 0);
        storage.mark_attempted(id).unwrap();
    }
    let mut storage = Storage::open(&file.0).unwrap();
    let pending = storage.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].random_id, id);
    assert!(pending[0].uncertain);
    assert_eq!(pending[0].message.text, "persisted message");
    let ack = batch(
        1,
        vec![UpdateEventDto::OutgoingMessageAcknowledged {
            random_id: id,
            result: OutgoingMessageResult::Sent {
                message: message(42, 1, "persisted message"),
            },
        }],
    );
    assert!(storage.apply_batch(&ack).unwrap());
    assert!(storage.pending().unwrap().is_empty());
    assert_eq!(storage.messages(42, 100).unwrap().len(), 1);
    assert!(!storage.apply_batch(&ack).unwrap());
}

#[test]
fn batch_failure_rolls_back_receipt_projection_and_cursor() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage.connection.execute_batch("CREATE TRIGGER fail_cursor BEFORE INSERT ON cursors BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    let update = batch(
        1,
        vec![UpdateEventDto::NewMessage {
            message: message(42, 1, "hello"),
        }],
    );
    assert!(storage.apply_batch(&update).is_err());
    assert!(storage.messages(42, 100).unwrap().is_empty());
    assert_eq!(
        storage
            .connection
            .query_row("SELECT COUNT(*) FROM update_receipts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT COUNT(*) FROM update_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    storage
        .connection
        .execute_batch("DROP TRIGGER fail_cursor;")
        .unwrap();
    assert!(storage.apply_batch(&update).unwrap());
}

#[test]
fn stale_snapshots_do_not_overwrite_edits_or_resurrect_deletes() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    let original = message(42, 1, "old");
    let mut edited = original.clone();
    edited.text = Some("new".into());
    edited.edit_date = Some(20);
    storage
        .apply_batch(&batch(
            1,
            vec![UpdateEventDto::MessageEdited { message: edited }],
        ))
        .unwrap();
    storage
        .cache_messages(std::slice::from_ref(&original))
        .unwrap();
    assert_eq!(
        storage.messages(42, 100).unwrap()[0].text.as_deref(),
        Some("new")
    );
    storage
        .apply_batch(&batch(
            2,
            vec![UpdateEventDto::MessagesDeleted {
                chat_id: Some(42),
                message_ids: vec![1],
            }],
        ))
        .unwrap();
    storage
        .cache_messages(std::slice::from_ref(&original))
        .unwrap();
    assert!(storage.messages(42, 100).unwrap().is_empty());
    assert!(storage.project_messages(&[original]).unwrap().is_empty());
}

#[test]
fn unscoped_deletes_do_not_delete_channel_messages_with_same_id() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage
        .apply_batch(&batch(
            1,
            vec![UpdateEventDto::MessagesDeleted {
                chat_id: None,
                message_ids: vec![1],
            }],
        ))
        .unwrap();
    storage
        .cache_messages(&[
            message(42, 1, "user"),
            message(-1_000_000_000_001, 1, "channel"),
        ])
        .unwrap();
    assert!(storage.messages(42, 100).unwrap().is_empty());
    assert_eq!(storage.messages(-1_000_000_000_001, 100).unwrap().len(), 1);
}

#[test]
fn reaction_before_history_and_unknown_durable_events_are_preserved() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage
        .apply_batch(&batch(
            1,
            vec![
                UpdateEventDto::MessageReactions {
                    chat_id: 42,
                    message_id: 1,
                    reactions_json: "[]".into(),
                },
                UpdateEventDto::Ignored {
                    kind: "notify_settings:42".into(),
                },
            ],
        ))
        .unwrap();
    storage.cache_messages(&[message(42, 1, "hello")]).unwrap();
    assert_eq!(
        storage.messages(42, 100).unwrap()[0]
            .reactions_json
            .as_deref(),
        Some("[]")
    );
    assert_eq!(
        storage
            .connection
            .query_row("SELECT COUNT(*) FROM update_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn generations_are_independent_and_logout_clears_private_cache() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage.configure(123).unwrap();
    storage.set_user(42).unwrap();
    let first = batch(
        1,
        vec![UpdateEventDto::NewMessage {
            message: message(42, 1, "hello"),
        }],
    );
    storage.apply_batch(&first).unwrap();
    let mut second = first;
    second.account_generation = 101;
    assert!(storage.apply_batch(&second).unwrap());
    storage.enqueue(&text_send()).unwrap();
    storage.clear_account().unwrap();
    assert!(storage.pending().unwrap().is_empty());
    assert!(storage.messages(42, 100).unwrap().is_empty());
    assert_eq!(
        storage.credentials_metadata().unwrap(),
        Some(CredentialsMetadata {
            api_id: 123,
            user_id: None
        })
    );
    let columns: Vec<String> = storage
        .connection
        .prepare("PRAGMA table_info(accounts)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["id", "api_id", "user_id"]);
}

#[test]
fn cursor_never_moves_backwards_and_replay_is_idempotent() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage.apply_batch(&batch(5, vec![])).unwrap();
    storage.apply_batch(&batch(1, vec![])).unwrap();
    assert!(!storage.apply_batch(&batch(5, vec![])).unwrap());
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT pts FROM cursors WHERE generation=100 AND scope=0",
                [],
                |row| row.get::<_, i32>(0)
            )
            .unwrap(),
        5
    );
}

struct DatabaseFile(PathBuf);
impl DatabaseFile {
    fn new() -> Self {
        use rand::{TryRng, rngs::SysRng};
        Self(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "target/storage-test-{}.sqlite",
            SysRng.try_next_u64().unwrap()
        )))
    }
}
impl Drop for DatabaseFile {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
use std::path::PathBuf;

fn draft(text: &str, date: i32) -> mono_proto_client::CloudDraft {
    mono_proto_client::CloudDraft {
        chat_id: 42,
        thread_id: 0,
        text: text.into(),
        entities_json: None,
        reply_to_message_id: 0,
        date,
        no_webpage: false,
        has_unsupported_content: false,
    }
}

#[test]
fn draft_revisions_survive_restart_reject_stale_saves_and_preserve_pending() {
    let file = DatabaseFile::new();
    let mut storage = Storage::open(&file.0).unwrap();
    storage.cache_drafts(&[draft("remote", 10)]).unwrap();
    let old = storage.draft_revision(42, 0).unwrap().unwrap();
    let staged = storage
        .stage_draft(&draft("local", 20), old.revision)
        .unwrap()
        .unwrap();
    assert!(
        storage
            .stage_draft(&draft("stale", 20), old.revision)
            .unwrap()
            .is_none()
    );
    storage.cache_drafts(&[draft("stale server", 30)]).unwrap();
    drop(storage);
    let mut storage = Storage::open(&file.0).unwrap();
    let current = storage.draft_revision(42, 0).unwrap().unwrap();
    assert_eq!(current.revision, staged.revision);
    assert_eq!(current.draft.text, "local");
    assert!(current.pending);
    storage.acknowledge_draft(42, 0, old.revision).unwrap();
    assert!(storage.draft_revision(42, 0).unwrap().unwrap().pending);
    storage.acknowledge_draft(42, 0, staged.revision).unwrap();
    storage.cache_drafts(&[draft("stale", 10)]).unwrap();
    assert_eq!(
        storage.draft_revision(42, 0).unwrap().unwrap().draft.text,
        "local"
    );
    let cleared = storage
        .stage_draft(&draft("", 40), staged.revision)
        .unwrap()
        .unwrap();
    assert!(cleared.revision > staged.revision);
    storage.acknowledge_draft(42, 0, cleared.revision).unwrap();
    storage.cache_drafts(&[draft("stale", 30)]).unwrap();
    assert!(
        storage
            .draft_revision(42, 0)
            .unwrap()
            .unwrap()
            .draft
            .text
            .is_empty()
    );
}

#[test]
fn complete_draft_snapshot_clears_absent_remote_but_preserves_pending() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage.cache_drafts(&[draft("remote", 10)]).unwrap();
    let before = storage.draft_revision(42, 0).unwrap().unwrap();
    storage.reconcile_drafts(&[], 20).unwrap();
    let cleared = storage.draft_revision(42, 0).unwrap().unwrap();
    assert!(cleared.draft.text.is_empty());
    assert!(cleared.revision > before.revision);
    storage.reconcile_drafts(&[], 30).unwrap();
    assert_eq!(
        storage.draft_revision(42, 0).unwrap().unwrap().revision,
        cleared.revision
    );
    let staged = storage
        .stage_draft(&draft("pending", 40), cleared.revision)
        .unwrap()
        .unwrap();
    storage.reconcile_drafts(&[], 50).unwrap();
    let current = storage.draft_revision(42, 0).unwrap().unwrap();
    assert_eq!(current.revision, staged.revision);
    assert_eq!(current.draft.text, "pending");
    assert!(current.pending);
}

#[test]
fn explicit_read_projection_is_monotonic_and_does_not_invent_partial_unread_counts() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    storage.mark_unread(42, true).unwrap();
    storage.mark_read(42, 10).unwrap();
    let state: (i32, Option<i32>) = storage
        .connection
        .query_row(
            "SELECT inbox,unread FROM chat_reads WHERE chat_id=42",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (10, None));
    storage.mark_read(42, 5).unwrap();
    storage.mark_read(42, 0).unwrap();
    let state: (i32, Option<i32>) = storage
        .connection
        .query_row(
            "SELECT inbox,unread FROM chat_reads WHERE chat_id=42",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (10, Some(0)));
    assert!(
        !storage
            .connection
            .query_row(
                "SELECT unread FROM dialog_marks WHERE chat_id=42",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
}

#[test]
fn unsupported_draft_cannot_be_overwritten_or_discarded() {
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    let mut remote = draft("rich", 1);
    remote.has_unsupported_content = true;
    storage.cache_drafts(&[remote]).unwrap();
    let revision = storage.draft_revision(42, 0).unwrap().unwrap().revision;
    assert!(storage.stage_draft(&draft("", 2), revision).is_err());
    assert_eq!(
        storage.draft_revision(42, 0).unwrap().unwrap().draft.text,
        "rich"
    );
}

#[test]
fn scheduled_snapshots_replace_only_requested_chat_and_events_delete_durably() {
    use mono_proto_client::ScheduledMessageDto;
    let mut storage = Storage::open(Path::new(":memory:")).unwrap();
    let scheduled = |chat_id, id| ScheduledMessageDto {
        message: message(chat_id, id, "scheduled"),
        has_unsupported_content: false,
    };
    storage
        .replace_scheduled(42, &[scheduled(42, 1), scheduled(42, 2)])
        .unwrap();
    storage.replace_scheduled(43, &[scheduled(43, 3)]).unwrap();
    storage
        .apply_batch(&batch(
            1,
            vec![UpdateEventDto::ScheduledMessagesDeleted {
                chat_id: 42,
                message_ids: vec![1],
                sent_message_ids: None,
            }],
        ))
        .unwrap();
    assert_eq!(storage.scheduled(42).unwrap().len(), 1);
    storage
        .replace_scheduled(42, &[scheduled(42, 1), scheduled(42, 2)])
        .unwrap();
    assert_eq!(
        storage
            .scheduled(42)
            .unwrap()
            .iter()
            .map(|value| value.message.id)
            .collect::<Vec<_>>(),
        vec![2]
    );
    storage.replace_scheduled(42, &[]).unwrap();
    assert!(storage.scheduled(42).unwrap().is_empty());
    assert_eq!(storage.scheduled(43).unwrap().len(), 1);
    assert!(storage.replace_scheduled(42, &[scheduled(43, 1)]).is_err());
}

#[test]
fn profiles_folder_order_and_unread_marks_roundtrip() {
    use mono_proto_client::{FolderDto, ProfileDto};
    let file = DatabaseFile::new();
    let mut storage = Storage::open(&file.0).unwrap();
    let folder = |id| FolderDto {
        id,
        title: format!("folder {id}"),
        chat_ids: vec![42],
        exclude_chat_ids: vec![43],
        include_contacts: true,
        include_non_contacts: false,
        include_groups: true,
        include_channels: true,
        include_bots: false,
        exclude_muted: false,
        exclude_read: true,
        exclude_archived: true,
        emoticon: "x".into(),
        pinned_chat_ids: vec![42],
    };
    storage.cache_folders(&[folder(3), folder(2)]).unwrap();
    let profile = ProfileDto {
        id: 42,
        kind: "user".into(),
        title: "Title".into(),
        username: Some("name".into()),
        about: Some("about".into()),
        avatar_cache_key: None,
        is_self: false,
        status: Some("offline".into()),
        status_at: Some(20),
        extra_json: Some("{}".into()),
    };
    storage.cache_profile(&profile).unwrap();
    storage.mark_unread(42, true).unwrap();
    drop(storage);
    let mut storage = Storage::open(&file.0).unwrap();
    assert_eq!(
        storage
            .folders()
            .unwrap()
            .iter()
            .map(|value| value.id)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert_eq!(storage.folders().unwrap()[0].pinned_chat_ids, vec![42]);
    assert_eq!(
        storage.profile(42).unwrap().unwrap().extra_json.as_deref(),
        Some("{}")
    );
    storage
        .apply_batch(&batch(
            1,
            vec![UpdateEventDto::Ignored {
                kind: "dialog_unread_mark:42:0".into(),
            }],
        ))
        .unwrap();
    let unread: bool = storage
        .connection
        .query_row(
            "SELECT unread FROM dialog_marks WHERE chat_id=42",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!unread);
    storage.clear_account().unwrap();
    assert!(storage.profile(42).unwrap().is_none());
    assert!(storage.folders().unwrap().is_empty());
}

#[test]
fn version_two_upgrade_preserves_identity_and_outbox() {
    let file = DatabaseFile::new();
    let storage = Storage::open(&file.0).unwrap();
    storage.configure(123).unwrap();
    let id = storage.enqueue(&text_send()).unwrap();
    storage.connection.execute_batch("DROP TABLE app_preferences; DROP TABLE special_outbox; DROP TABLE media_outbox_items; DROP TABLE media_outbox; DROP TABLE media_records; DROP TABLE profiles; DROP TABLE folders; DROP TABLE dialog_marks; DROP TABLE scheduled_deletions; ALTER TABLE drafts DROP COLUMN revision; ALTER TABLE drafts DROP COLUMN pending; PRAGMA user_version=2;").unwrap();
    drop(storage);
    let storage = Storage::open(&file.0).unwrap();
    assert_eq!(storage.credentials_metadata().unwrap().unwrap().api_id, 123);
    assert_eq!(storage.pending().unwrap()[0].random_id, id);
    assert!(storage.drafts().unwrap().is_empty());
}

fn media_send(count: usize) -> crate::domain::MediaSend {
    crate::domain::MediaSend {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
        silent: false,
        schedule_date: None,
        items: (0..count)
            .map(|index| crate::domain::MediaUpload {
                path: format!("retained-source-{index}.jpg"),
                kind: "photo".into(),
                mime_type: "image/jpeg".into(),
                file_name: format!("photo-{index}.jpg"),
                caption: String::new(),
                entities_json: None,
                duration: 0,
                width: 100,
                height: 100,
            })
            .collect(),
    }
}

#[test]
fn media_outbox_ids_retry_state_and_ack_survive_restart() {
    let file = DatabaseFile::new();
    let (album_id, random_ids) = {
        let mut storage = Storage::open(&file.0).unwrap();
        let queued = storage.enqueue_media(&media_send(2)).unwrap();
        storage.mark_media_attempted(queued.0).unwrap();
        queued
    };
    assert_ne!(album_id, 0);
    assert_eq!(random_ids.len(), 2);
    assert_ne!(random_ids[0], random_ids[1]);
    assert!(random_ids.iter().all(|id| *id != 0));
    let mut storage = Storage::open(&file.0).unwrap();
    let pending = storage.pending_media_send(album_id).unwrap().unwrap();
    assert_eq!(pending.random_ids, random_ids);
    assert_eq!(pending.send.items[1].path, "retained-source-1.jpg");
    assert!(pending.uncertain);
    for (index, random_id) in random_ids.iter().copied().enumerate() {
        let acknowledged = batch(
            index as i64 + 1,
            vec![UpdateEventDto::OutgoingMessageAcknowledged {
                random_id,
                result: OutgoingMessageResult::Sent {
                    message: message(42, index as i32 + 1, ""),
                },
            }],
        );
        assert!(storage.apply_batch(&acknowledged).unwrap());
        assert_eq!(
            storage.pending_media_send(album_id).unwrap().is_some(),
            index == 0
        );
        assert!(!storage.apply_batch(&acknowledged).unwrap());
    }
    assert!(storage.pending_media().unwrap().is_empty());
}

#[test]
fn media_settings_scoped_records_and_logout_are_durable() {
    use crate::domain::{
        AutoDownloadRule, AutoDownloadSettings, CacheLimit, ClearMediaScope, MediaCategory,
        NetworkClass, NetworkPolicy,
    };
    let file = DatabaseFile::new();
    let mut storage = Storage::open(&file.0).unwrap();
    let mut policies = AutoDownloadSettings::default();
    policies.roaming.photo = AutoDownloadRule {
        enabled: true,
        max_bytes: 4096,
    };
    storage.set_auto_download(policies).unwrap();
    storage.set_network_class(NetworkClass::Roaming).unwrap();
    storage.set_cache_limit(CacheLimit::Limited(8192)).unwrap();
    storage
        .set_autoplay(NetworkPolicy {
            wifi: true,
            mobile: false,
            roaming: true,
        })
        .unwrap();
    storage
        .set_preload(NetworkPolicy {
            wifi: false,
            mobile: false,
            roaming: true,
        })
        .unwrap();
    storage
        .record_media("photo", MediaCategory::Photo, Some(42), 100, true)
        .unwrap();
    storage
        .record_media("avatar", MediaCategory::Avatar, Some(42), 50, true)
        .unwrap();
    storage.enqueue_media(&media_send(1)).unwrap();
    drop(storage);
    let storage = Storage::open(&file.0).unwrap();
    let settings = storage.media_settings().unwrap();
    assert_eq!(settings.cache_limit, CacheLimit::Limited(8192));
    assert_eq!(settings.network, NetworkClass::Roaming);
    assert_eq!(settings.auto_download.roaming.photo.max_bytes, 4096);
    assert!(settings.auto_download.roaming.photo.enabled);
    assert!(settings.autoplay.wifi);
    assert!(settings.autoplay.roaming);
    assert!(settings.preload.roaming);
    assert!(!settings.preload.wifi);
    assert_eq!(
        storage
            .media_record_keys(ClearMediaScope::Category(MediaCategory::Photo))
            .unwrap(),
        ["photo"]
    );
    assert_eq!(
        storage
            .media_record_keys(ClearMediaScope::ChatCategory {
                chat_id: 42,
                category: MediaCategory::Photo,
            })
            .unwrap(),
        ["photo"]
    );
    assert_eq!(
        storage
            .media_record_keys(ClearMediaScope::All)
            .unwrap()
            .len(),
        2
    );
    storage.set_cache_limit(CacheLimit::Unlimited).unwrap();
    drop(storage);
    let mut storage = Storage::open(&file.0).unwrap();
    assert_eq!(storage.cache_limit().unwrap(), CacheLimit::Unlimited);
    storage.clear_account().unwrap();
    assert!(storage.pending_media().unwrap().is_empty());
    assert!(
        storage
            .media_record_keys(ClearMediaScope::All)
            .unwrap()
            .is_empty()
    );
    assert_eq!(storage.cache_limit().unwrap(), CacheLimit::Unlimited);
}

#[test]
fn version_four_numeric_cache_limit_migrates_to_typed_representation() {
    use crate::domain::CacheLimit;
    let file = DatabaseFile::new();
    let storage = Storage::open(&file.0).unwrap();
    storage.set_cache_limit(CacheLimit::Limited(8192)).unwrap();
    storage
        .connection
        .execute_batch(
            "UPDATE app_settings SET value='8192' WHERE key='media.cache_limit';
             DROP TABLE app_preferences; DROP TABLE special_outbox; PRAGMA user_version=4;",
        )
        .unwrap();
    drop(storage);
    let storage = Storage::open(&file.0).unwrap();
    assert_eq!(storage.cache_limit().unwrap(), CacheLimit::Limited(8192));
    assert_eq!(
        storage
            .connection
            .query_row(
                "SELECT value FROM app_settings WHERE key='media.cache_limit'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "limited:8192"
    );
}

#[test]
fn special_outbox_retry_identity_and_ack_survive_restart() {
    use crate::domain::{SpecialContent, SpecialSend};
    let send = SpecialSend {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
        silent: false,
        schedule_date: None,
        content: SpecialContent::Poll(mono_proto_client::PollSendSpec {
            question: "Question".into(),
            answers: vec!["One".into(), "Two".into()],
            anonymous: true,
            multiple_choice: false,
            quiz: false,
            correct_answer: None,
            solution: None,
        }),
    };
    let file = DatabaseFile::new();
    let random_id = {
        let storage = Storage::open(&file.0).unwrap();
        let random_id = storage.enqueue_special(&send).unwrap();
        storage.mark_special_attempted(random_id).unwrap();
        random_id
    };
    assert_ne!(random_id, 0);
    let mut storage = Storage::open(&file.0).unwrap();
    let pending = storage.pending_special_send(random_id).unwrap().unwrap();
    assert_eq!(pending.random_id, random_id);
    assert!(pending.uncertain);
    let acknowledged = batch(
        1,
        vec![UpdateEventDto::OutgoingMessageAcknowledged {
            random_id,
            result: OutgoingMessageResult::Sent {
                message: message(42, 1, ""),
            },
        }],
    );
    assert!(storage.apply_batch(&acknowledged).unwrap());
    assert!(storage.pending_special().unwrap().is_empty());
    assert!(!storage.apply_batch(&acknowledged).unwrap());
    storage.enqueue_special(&send).unwrap();
    storage.clear_account().unwrap();
    assert!(storage.pending_special().unwrap().is_empty());
}

#[test]
fn special_outbox_migrates_from_v5_and_roundtrips_extended_content() {
    use crate::domain::{SpecialContent, SpecialSend};

    let file = DatabaseFile::new();
    let storage = Storage::open(&file.0).unwrap();
    storage
        .connection
        .execute_batch(
            "DROP TABLE app_preferences; DROP TABLE special_outbox; PRAGMA user_version=5;",
        )
        .unwrap();
    drop(storage);

    let storage = Storage::open(&file.0).unwrap();
    let version: i64 = storage
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);

    let contents = [
        SpecialContent::Contact(mono_proto_client::ContactSendSpec {
            phone: "+15550123".into(),
            first_name: "Ada".into(),
            last_name: "Lovelace".into(),
            vcard: String::new(),
        }),
        SpecialContent::Dice {
            emoticon: "🎲".into(),
        },
        SpecialContent::Location(mono_proto_client::LocationSendSpec {
            latitude: 51.5,
            longitude: -0.12,
            live_period: 60,
            heading: Some(90),
        }),
        SpecialContent::InlineBotResult {
            query_id: 7,
            result_id: "result-1".into(),
        },
    ];

    for content in contents {
        let send = SpecialSend {
            chat_id: 42,
            reply_to_msg_id: 0,
            top_msg_id: 0,
            silent: false,
            schedule_date: None,
            content,
        };
        let payload = encode(&send).unwrap();
        let random_id = storage.enqueue_special(&send).unwrap();
        assert_ne!(random_id, 0);
        storage.mark_special_attempted(random_id).unwrap();
        let pending = storage.pending_special_send(random_id).unwrap().unwrap();
        assert_eq!(pending.random_id, random_id);
        assert!(pending.uncertain);
        assert_eq!(encode(&pending.send).unwrap(), payload);
        assert!(pending.send.validate().is_ok());
    }
}

#[test]
fn versioned_preferences_support_cas_migration_and_survive_logout() {
    use crate::domain::{PreferenceDocument, PreferenceNamespace};
    let file = DatabaseFile::new();
    let mut storage = Storage::open(&file.0).unwrap();
    let initial = PreferenceDocument {
        version: 1,
        payload_json: r#"{"enabled":true}"#.into(),
    };
    assert!(
        storage
            .compare_and_swap_preference(PreferenceNamespace::Notifications, None, &initial)
            .unwrap()
    );
    let updated = PreferenceDocument {
        version: 1,
        payload_json: r#"{"enabled":false}"#.into(),
    };
    assert!(
        storage
            .compare_and_swap_preference(PreferenceNamespace::Notifications, Some(1), &updated,)
            .unwrap()
    );
    assert!(
        !storage
            .compare_and_swap_preference(PreferenceNamespace::Notifications, Some(0), &initial,)
            .unwrap()
    );
    storage
        .set_preference(PreferenceNamespace::Proxy, &initial)
        .unwrap();
    storage.configure(123).unwrap();
    storage.clear_account().unwrap();
    assert_eq!(
        storage
            .preference(PreferenceNamespace::Notifications)
            .unwrap(),
        Some(updated)
    );
    assert_eq!(storage.preferences().unwrap().documents.len(), 2);
    storage
        .connection
        .execute_batch("DROP TABLE app_preferences; PRAGMA user_version=6;")
        .unwrap();
    drop(storage);
    let storage = Storage::open(&file.0).unwrap();
    assert!(storage.preferences().unwrap().documents.is_empty());
    assert_eq!(
        storage
            .connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert!(
        !storage
            .delete_preference(PreferenceNamespace::Proxy)
            .unwrap()
    );
}
