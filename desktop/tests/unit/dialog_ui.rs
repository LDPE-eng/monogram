use super::*;

fn message(id: i32, text: &str, edited: Option<i64>) -> MessageDto {
    serde_json::from_value(serde_json::json!({
        "chat_id": 42, "id": id, "sender_id": 1, "text": text, "date": 100,
        "edit_date": edited, "outgoing": true, "media_kind": null, "media_cache_key": null,
        "thumb_cache_key": null, "media_duration": null, "media_width": null, "media_height": null,
        "reply_quote": null, "reply_quote_entities_json": null, "entities_json": null,
        "noforwards": false, "reply_to_msg_id": null, "reply_to_top_id": null,
        "fwd_from": null, "fwd_from_id": null, "fwd_date": null, "via_bot": null,
        "sender_name": null, "sender_emoji_status_document_id": null, "grouped_id": null,
        "file_name": null, "file_size": null, "supports_streaming": false,
        "reactions_json": null, "replies_count": 0, "discussion_peer_id": null,
        "reply_markup_json": null, "forum_topic": false
    }))
    .unwrap()
}

#[test]
fn history_keeps_stable_ids_and_live_updates_over_stale_pages() {
    let mut history = History::default();
    history.insert(message(2, "cached", None), false);
    history.insert(message(2, "live", Some(200)), true);
    history.insert(message(2, "stale", None), false);
    history.insert(message(1, "older", None), false);
    assert_eq!(
        history.messages.keys().copied().collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(history.messages[&2].text.as_deref(), Some("live"));
    history.delete(2);
    history.insert(message(2, "stale resurrection", Some(300)), false);
    assert!(!history.messages.contains_key(&2));
}

#[test]
fn history_rejects_older_edits_and_non_server_ids() {
    let mut history = History::default();
    history.insert(message(1, "newer", Some(300)), false);
    history.insert(message(1, "older", Some(200)), true);
    history.insert(message(0, "not a server id", None), true);
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[&1].text.as_deref(), Some("newer"));
}

#[test]
fn entity_ranges_use_utf16_not_bytes_or_scalar_offsets() {
    let text = "a😀b";
    assert!(entities(text, Some(r#"[{"kind":"bold","offset":1,"length":2}]"#)).is_ok());
    assert!(entities(text, Some(r#"[{"kind":"bold","offset":1,"length":1}]"#)).is_err());
    assert!(entities(text, Some(r#"[{"kind":"bold","offset":4,"length":1}]"#)).is_err());
    assert!(entities(text, Some("{}")).is_err());
}

#[test]
fn outgoing_markdown_serializes_protocol_entities_without_losing_unicode() {
    let (text, json) = outgoing_text("😀 **bold**", true).unwrap();
    assert_eq!(text, "😀 bold");
    let json = json.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value[0]["offset"], 3);
    assert_eq!(value[0]["length"], 4);
    assert!(entities(&text, Some(&json)).is_ok());
    assert_eq!(
        outgoing_text(" **literal** ", false).unwrap(),
        (" **literal** ".into(), None)
    );
    assert!(outgoing_text(&"x".repeat(MAX_INPUT_BYTES + 1), false).is_err());
}

#[test]
fn optional_ids_and_schedule_are_checked_without_panics() {
    for value in ["", "0", " 0 "] {
        assert_eq!(optional_number(value), Ok(None));
    }
    assert_eq!(optional_number(" 42 "), Ok(Some(42)));
    for value in ["-1", "2147483647", "2147483648", "tomorrow"] {
        assert!(optional_number(value).is_err());
    }
}

#[test]
fn relm_debug_does_not_log_message_content() {
    let text = "private body never log";
    let input = DialogInput::UpsertMessage(message(1, text, None));
    let output = DialogOutput::EditText {
        chat_id: 42,
        message_id: 1,
        text: text.into(),
        entities_json: None,
    };
    assert!(!format!("{input:?} {output:?}").contains(text));
}

#[test]
fn schedule_validation_rejects_past_overflow_and_too_distant_dates() {
    let now = 1_800_000_000;
    for date in [now - 1, now, now + 59, now + 365 * 86400 + 1] {
        assert!(schedule_timestamp(&date.to_string(), now).is_err());
    }
    for date in [now + 60, now + 365 * 86400] {
        assert_eq!(schedule_timestamp(&date.to_string(), now), Ok(date as i32));
    }
    assert!(schedule_timestamp("2147483647", now).is_err());
    assert!(schedule_timestamp("2147483648", now).is_err());
    assert!(schedule_timestamp("", now).is_err());
}

fn draft_revision(text: &str, revision: i64) -> DraftRevision {
    DraftRevision {
        draft: CloudDraft {
            chat_id: 42,
            thread_id: 0,
            text: text.into(),
            entities_json: None,
            reply_to_message_id: 0,
            date: 100,
            no_webpage: false,
            has_unsupported_content: false,
        },
        revision,
        pending: false,
    }
}

#[test]
fn cloud_draft_revisions_do_not_overwrite_local_edits_or_accept_stale_ack() {
    let mut guard = DraftGuard::default();
    assert!(guard.accept(&draft_revision("remote", 7)));
    assert!(!guard.accept(&draft_revision("stale cache", 6)));
    guard.changed();
    assert!(!guard.accept(&draft_revision("new remote", 8)));
    assert_eq!(guard.server_revision, 8);
    guard.changed();
    guard.acknowledge(1, 9);
    assert!(guard.dirty());
    assert!(!guard.accept(&draft_revision("old save echo", 9)));
    guard.acknowledge(2, 10);
    assert!(!guard.dirty());
    guard.acknowledge(1, 9);
    assert_eq!(guard.server_revision, 10);
    assert!(!guard.dirty());
}

#[test]
fn unsupported_draft_and_real_worker_envelopes_remain_redacted() {
    let mut remote = draft_revision("private cloud body", 11);
    remote.draft.has_unsupported_content = true;
    let mut guard = DraftGuard::default();
    assert!(guard.accept(&remote));
    assert!(guard.unsupported);
    let output = DialogOutput::Command {
        token: 5,
        command: WorkerCommand::SaveCloudDraft {
            draft: remote.draft.clone(),
            expected_revision: remote.revision,
        },
    };
    let input = DialogInput::Response {
        token: 5,
        response: WorkerResponse::Draft(remote),
    };
    assert!(!format!("{input:?} {output:?}").contains("private cloud body"));
}

#[test]
fn thread_context_filters_live_messages_by_real_top_and_reply_ids() {
    let mut dto = message(12, "reply", None);
    assert!(in_context(&dto, Some((42, 0))));
    assert!(!in_context(&dto, Some((43, 0))));
    assert!(!in_context(&dto, Some((42, 7))));
    dto.reply_to_top_id = Some(7);
    assert!(in_context(&dto, Some((42, 7))));
    assert!(!in_context(&dto, Some((42, 8))));
    dto.reply_to_top_id = None;
    dto.reply_to_msg_id = Some(7);
    assert!(in_context(&dto, Some((42, 7))));
    dto.id = 7;
    dto.reply_to_msg_id = None;
    assert!(in_context(&dto, Some((42, 7))));
}

#[test]
fn scheduled_snapshot_changes_invalidate_edit_without_optimistic_success() {
    let original = ScheduledMessageDto {
        message: message(7, "queued", None),
        has_unsupported_content: false,
    };
    let mut changed = original.clone();
    assert!(scheduled_unchanged(&original, &changed));
    changed.message.date += 1;
    assert!(!scheduled_unchanged(&original, &changed));
    changed = original.clone();
    changed.message.text = Some("remote edit".into());
    assert!(!scheduled_unchanged(&original, &changed));
    changed = original.clone();
    changed.has_unsupported_content = true;
    assert!(!scheduled_unchanged(&original, &changed));
}

#[test]
fn album_layout_tracks_groups_without_treating_single_items_as_albums() {
    let mut messages = std::collections::BTreeMap::new();
    for (id, grouped_id) in [(1, Some(100)), (2, None), (3, Some(100)), (4, Some(200))] {
        let mut dto = message(id, "", None);
        dto.grouped_id = grouped_id;
        messages.insert(id, dto);
    }
    let positions = album_positions(&messages);
    assert_eq!(positions.get(&1), Some(&(1, 2)));
    assert_eq!(positions.get(&3), Some(&(2, 2)));
    assert!(!positions.contains_key(&2));
    assert!(!positions.contains_key(&4));
}

#[test]
fn media_summary_reads_webpage_cards_and_privacy_metadata() {
    let mut dto = message(1, "caption", None);
    dto.media_kind = Some("webpage".into());
    dto.media_cache_key = Some("photo:7".into());
    dto.file_name = Some(
        serde_json::json!({"u":"https://example.com","t":"Title","s":"Site","d":"Description"})
            .to_string(),
    );
    let summary = MediaSummary::new(&dto, Some((1, 2))).unwrap();
    assert_eq!(summary.album_position, Some((1, 2)));
    assert_eq!(
        summary.webpage.as_ref().unwrap().title.as_deref(),
        Some("Title")
    );
    assert!(media_action_allowed(
        &summary,
        MediaAction::View,
        MediaStatus::Ready
    ));
    dto.noforwards = true;
    let protected = MediaSummary::new(&dto, None).unwrap();
    for action in [
        MediaAction::Save,
        MediaAction::SaveAs,
        MediaAction::OpenExternal,
        MediaAction::Copy,
    ] {
        assert!(!media_action_allowed(
            &protected,
            action,
            MediaStatus::Ready
        ));
    }
    assert!(media_action_allowed(
        &protected,
        MediaAction::View,
        MediaStatus::Ready
    ));
}

#[test]
fn structured_cards_use_real_dto_payloads_and_validate_geo_navigation() {
    let mut dto = message(1, "", None);
    dto.media_kind = Some("poll".into());
    dto.file_name = Some(
        serde_json::json!({
            "id":"7","creator":1,"r":0,"q":"Question","a":[
                {"t":"A","c":1,"k":1,"v":3,"o":"01"},
                {"t":"B","c":0,"k":0,"v":2,"o":"02"}
            ],"n":5,"z":1,"m":0,"x":0,"o":1,"s":"Solution"
        })
        .to_string(),
    );
    let Some(InteractionCard::Poll(poll)) = parse_interaction(&dto) else {
        panic!("poll card expected");
    };
    assert_eq!(poll.q, "Question");
    assert_eq!(poll.a[0].o, "01");
    assert_eq!(poll.a[0].v, 3);
    assert_eq!(poll.s, "Solution");

    dto.media_kind = Some("todo".into());
    dto.file_name = Some(
        serde_json::json!({"t":"Tasks","a":1,"c":1,"i":[{"id":4,"t":"Task","d":1,"by":7,"at":100}]})
            .to_string(),
    );
    let Some(InteractionCard::Checklist(checklist)) = parse_interaction(&dto) else {
        panic!("checklist expected");
    };
    assert_eq!(checklist.i[0].id, 4);
    assert_eq!(checklist.i[0].d, 1);

    dto.media_kind = Some("venue".into());
    dto.file_name =
        Some(serde_json::json!({"lat":55.75,"long":37.61,"t":"Place","a":"Address"}).to_string());
    let Some(InteractionCard::Geo(mut card)) = parse_interaction(&dto) else {
        panic!("geo card expected");
    };
    let url = geo_url(&card).unwrap();
    assert!(url.contains("mlat=55.75"));
    assert!(url.contains("mlon=37.61"));
    card.lat = Some(91.0);
    assert!(geo_url(&card).is_none());
    card.lat = Some(f64::NAN);
    assert!(geo_url(&card).is_none());
}

#[test]
fn announcement_cards_are_supported_but_read_only_without_file_actions() {
    for (kind, payload) in [
        (
            "giveaway",
            serde_json::json!({"d":"Prize","q":2,"m":12,"x":100}),
        ),
        ("paid_media", serde_json::json!({"x":100,"i":2})),
        (
            "game",
            serde_json::json!({"t":"Game","s":"Description","d":"game"}),
        ),
        (
            "invoice",
            serde_json::json!({"t":"Invoice","s":"Description","d":"100 USD"}),
        ),
        ("story", serde_json::json!({"n":7,"d":"Caption"})),
        ("video_stream", serde_json::json!({"r":1})),
    ] {
        let mut dto = message(1, "", None);
        dto.media_kind = Some(kind.into());
        dto.file_name = Some(payload.to_string());
        let summary = MediaSummary::new(&dto, None).unwrap();
        assert!(summary.supported());
        assert!(matches!(
            summary.interaction,
            Some(InteractionCard::Announcement(_))
        ));
        for action in [MediaAction::Save, MediaAction::SaveAs, MediaAction::Copy] {
            assert!(!media_action_allowed(&summary, action, MediaStatus::Ready));
        }
    }
}

#[test]
fn poll_actions_enforce_closed_quiz_multiple_and_option_rules() {
    let mut poll = PollCard {
        q: "Question".into(),
        a: vec![
            PollAnswer {
                t: "A".into(),
                o: "01".into(),
                ..PollAnswer::default()
            },
            PollAnswer {
                t: "B".into(),
                o: "02".into(),
                ..PollAnswer::default()
            },
        ],
        m: 1,
        ..PollCard::default()
    };
    assert!(poll_vote_allowed(&poll, &[vec![1], vec![2]]));
    assert!(!poll_vote_allowed(&poll, &[vec![1], vec![1]]));
    assert!(!poll_vote_allowed(&poll, &[vec![3]]));
    assert!(!poll_vote_allowed(&poll, &[]));
    poll.m = 0;
    assert!(!poll_vote_allowed(&poll, &[vec![1], vec![2]]));
    poll.a[0].c = 1;
    assert!(poll_retract_allowed(&poll));
    poll.z = 1;
    assert!(!poll_retract_allowed(&poll));
    assert!(!poll_vote_allowed(&poll, &[vec![2]]));
    poll.z = 0;
    poll.x = 1;
    assert!(!poll_retract_allowed(&poll));
    assert!(!poll_vote_allowed(&poll, &[vec![2]]));
}

#[test]
fn reaction_and_keyboard_actions_are_limited_to_supported_protocol_data() {
    let reactions = reaction_summaries(Some(r#"[{"e":"👍","c":2,"me":1},{"d":7,"c":1,"me":0}]"#));
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0].emoticon, "👍");
    assert!(reactions[0].chosen);
    assert_eq!(reactions[1].document_id, 7);

    let rows = keyboard_rows(Some(
        r#"{"k":"inline","rows":[[
            {"x":"Open","t":"url","u":"https://example.com"},
            {"x":"Callback","t":"cb","d":"01ff"},
            {"x":"Copy","t":"copy","c":"text"},
            {"x":"Switch","t":"sw","q":"query","same":true},
            {"x":"Login","t":"login","u":"https://example.com"},
            {"x":"Password","t":"cb","d":"00","pw":true}
        ]]}"#,
    ));
    let actions = rows[0]
        .iter()
        .map(|button| button.action.clone())
        .collect::<Vec<_>>();
    assert_eq!(actions.len(), 4);
    assert!(matches!(actions[0], KeyboardAction::Url(_)));
    assert!(matches!(actions[1], KeyboardAction::Callback(_)));
    assert!(matches!(actions[2], KeyboardAction::Copy(_)));
    assert!(matches!(actions[3], KeyboardAction::SwitchInline(_)));
    let reply = keyboard_rows(Some(
        r#"{"k":"keyboard","rows":[[{"x":"Text","t":"text"},{"x":"Phone","t":"contact"}]]}"#,
    ));
    assert_eq!(reply[0].len(), 1);
    assert!(matches!(reply[0][0].action, KeyboardAction::Text(_)));
}

#[test]
fn inline_and_mention_queries_are_bounded_and_normalized() {
    let query = composer_inline_query("@Example_Bot search text").unwrap();
    assert_eq!(query.username, "example_bot");
    assert_eq!(query.query, "search text");
    assert_eq!(composer_inline_query("@Example_Bot").unwrap().query, "");
    assert!(composer_inline_query("hello @Example_Bot").is_none());
    assert!(composer_inline_query("@1bot query").is_none());
    assert_eq!(
        composer_mention_query("hello @Example_User").as_deref(),
        Some("example_user")
    );
    assert!(composer_mention_query("@bad-user").is_none());
}

#[test]
fn web_external_navigation_accepts_only_supported_schemes() {
    for uri in [
        "https://example.com",
        "tg://resolve?domain=example",
        "mailto:user@example.com",
        "tel:+123456",
        "geo:55.75,37.61",
    ] {
        assert!(external_uri_allowed(uri), "{uri}");
    }
    for uri in [
        "javascript:alert(1)",
        "file:///tmp/private",
        "https://bad host",
    ] {
        assert!(!external_uri_allowed(uri), "{uri}");
    }
}

#[test]
fn telegram_deep_links_use_central_allowlist_and_case_insensitive_schemes() {
    for uri in [
        "TG://RESOLVE?domain=Example_Bot",
        "tg://join?invite=example-hash",
        "HTTPS://example.com",
    ] {
        assert!(external_uri_allowed(uri), "{uri}");
    }
    for uri in [
        "tg://login?code=secret",
        "tg://proxy?server=127.0.0.1&port=443",
        "tg://socks?server=127.0.0.1&port=1080",
        "tg://unknown?value=1",
        "tg://user?id=42",
        "https://t.me/login?code=secret",
        "https://t.me/proxy?server=127.0.0.1&port=443",
        "https://t.me/socks?server=127.0.0.1&port=1080",
    ] {
        assert!(!external_uri_allowed(uri), "{uri}");
    }
}

fn inline_result(id: &str) -> mono_proto_client::InlineBotResultDto {
    mono_proto_client::InlineBotResultDto {
        id: id.into(),
        kind: "article".into(),
        title: Some("Title".into()),
        description: None,
        url: Some("https://example.com".into()),
        document_id: None,
        thumb_cache_key: None,
    }
}

#[test]
fn inline_bot_send_builds_durable_special_and_rejects_invalid_input() {
    let inline = InlineState {
        query_id: 9,
        results: vec![inline_result("a"), inline_result("b")],
        ..InlineState::default()
    };
    let send = inline_bot_send(&inline, 1, 42, 0, 0).unwrap();
    assert_eq!(send.chat_id, 42);
    assert_eq!(send.reply_to_msg_id, 0);
    assert_eq!(send.top_msg_id, 0);
    assert!(!send.silent);
    assert_eq!(send.schedule_date, None);
    assert!(matches!(
        send.content,
        SpecialContent::InlineBotResult {
            query_id: 9,
            result_id
        } if result_id == "b"
    ));

    assert!(inline_bot_send(&inline, 2, 42, 0, 0).is_none());
    assert!(inline_bot_send(&inline, 0, 0, 0, 0).is_none());
    let no_query = InlineState {
        query_id: 0,
        results: vec![inline_result("a")],
        ..InlineState::default()
    };
    assert!(inline_bot_send(&no_query, 0, 42, 0, 0).is_none());
    let empty_id = InlineState {
        query_id: 9,
        results: vec![inline_result("")],
        ..InlineState::default()
    };
    assert!(inline_bot_send(&empty_id, 0, 42, 0, 0).is_none());
}

#[test]
fn instant_media_target_maps_photo_doc_geo_to_message_and_avatar_to_peer() {
    for (key, expected) in [
        ("photo:7", InstantMediaTarget::Message(7)),
        ("doc:8", InstantMediaTarget::Message(8)),
        ("geo:99:10", InstantMediaTarget::Message(99)),
        ("avatar:42", InstantMediaTarget::Peer(42)),
    ] {
        assert_eq!(instant_media_target(key), Some(expected), "{key}");
    }
    for key in [
        "", "photo", "photo:", "photo:0", "video:1", "avatar:x", "geo:x:1",
    ] {
        assert_eq!(instant_media_target(key), None, "{key}");
    }
}

#[test]
fn instant_view_metadata_comes_from_webpage_dto() {
    let mut dto = message(1, "", None);
    dto.media_kind = Some("webpage".into());
    dto.file_name = Some(
        serde_json::json!({"u":"https://example.com/article","t":"Title","iv":true}).to_string(),
    );
    let summary = MediaSummary::new(&dto, None).unwrap();
    assert!(summary.webpage.unwrap().instant_view);
}

#[test]
fn media_actions_follow_real_transfer_state() {
    let mut dto = message(1, "", None);
    dto.media_kind = Some("photo".into());
    dto.media_cache_key = Some("photo:1".into());
    let summary = MediaSummary::new(&dto, None).unwrap();
    assert!(media_action_allowed(
        &summary,
        MediaAction::Download,
        MediaStatus::Missing
    ));
    assert!(media_action_allowed(
        &summary,
        MediaAction::CancelDownload,
        MediaStatus::Loading
    ));
    assert!(media_action_allowed(
        &summary,
        MediaAction::RetryDownload,
        MediaStatus::Failed
    ));
    assert!(!media_action_allowed(
        &summary,
        MediaAction::Save,
        MediaStatus::Loading
    ));
    assert!(media_action_allowed(
        &summary,
        MediaAction::SaveAs,
        MediaStatus::Ready
    ));
    dto.media_kind = Some("video".into());
    dto.supports_streaming = true;
    let streaming = MediaSummary::new(&dto, None).unwrap();
    assert!(media_action_allowed(
        &streaming,
        MediaAction::Play,
        MediaStatus::Missing
    ));
}

#[test]
fn custom_emoji_entities_are_validated_and_deduplicated() {
    let json = r#"[
        {"kind":"custom_emoji","offset":0,"length":2,"url":"7"},
        {"kind":"custom_emoji","offset":2,"length":2,"url":"9"},
        {"kind":"custom_emoji","offset":2,"length":2,"url":"9"},
        {"kind":"custom_emoji","offset":2,"length":2,"url":"0"}
    ]"#;
    assert_eq!(custom_emoji_ids("😀😀", Some(json)), vec![7, 9]);
    assert!(custom_emoji_ids("😀", Some(json)).is_empty());
    assert!(custom_emoji_ids("😀😀", Some("not-json")).is_empty());
}

#[test]
fn animated_sticker_sources_use_bounded_magic_detection() {
    let root = std::env::temp_dir().join(format!(
        "monogram-dialog-sticker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let tgs = root.join("animated.tgs");
    let webm = root.join("animated.data");
    let plain = root.join("plain.webp");
    std::fs::write(&tgs, [0x1f, 0x8b, 0, 0]).unwrap();
    std::fs::write(&webm, [0x1a, 0x45, 0xdf, 0xa3]).unwrap();
    std::fs::write(&plain, b"RIFF").unwrap();
    assert!(matches!(
        sticker_source(&tgs, None),
        Some(StickerSource::Tgs(_))
    ));
    assert_eq!(
        sticker_source(&webm, Some("animated.webm")),
        Some(StickerSource::Webm(webm.clone()))
    );
    assert!(sticker_source(&plain, Some("plain.webp")).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

pub(crate) fn message_body_labels_scenario() {
    let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    render_body(
        &parent,
        "plain **bold**",
        Some(r#"[{"kind":"bold","offset":6,"length":4}]"#),
    );
    let label = parent
        .first_child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
        .expect("message label");
    assert!(
        label
            .css_classes()
            .iter()
            .any(|class| class == "monogram-message-text")
    );
}

fn attachment(path: &str, kind: AttachmentKind) -> SelectedAttachment {
    SelectedAttachment {
        path: path.into(),
        kind,
        file_name: path.rsplit('/').next().unwrap().into(),
        mime_type: String::new(),
    }
}

#[test]
fn attachment_albums_are_bounded_compatible_and_unique() {
    let photo = attachment("/tmp/a.jpg", AttachmentKind::Photo);
    let video = attachment("/tmp/b.mp4", AttachmentKind::Video);
    let album = extend_attachments(std::slice::from_ref(&photo), vec![video]).unwrap();
    assert_eq!(album.len(), 2);
    assert!(extend_attachments(&album, vec![photo]).is_err());
    assert!(extend_attachments(&[], Vec::new()).is_err());
    assert!(
        extend_attachments(
            &album,
            vec![attachment("/tmp/c.bin", AttachmentKind::Document)],
        )
        .is_err()
    );
    let ten = (0..10)
        .map(|index| attachment(&format!("/tmp/{index}.jpg"), AttachmentKind::Photo))
        .collect::<Vec<_>>();
    assert_eq!(extend_attachments(&[], ten.clone()).unwrap().len(), 10);
    assert!(
        extend_attachments(&ten, vec![attachment("/tmp/10.jpg", AttachmentKind::Photo)],).is_err()
    );
}

#[test]
fn album_uploads_preserve_kinds_and_apply_caption_only_once() {
    for attachments in [
        vec![
            attachment("/tmp/a.jpg", AttachmentKind::Photo),
            attachment("/tmp/b.mp4", AttachmentKind::Video),
        ],
        vec![
            attachment("/tmp/a.bin", AttachmentKind::Document),
            attachment("/tmp/b.bin", AttachmentKind::Document),
        ],
    ] {
        let expected = attachments
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>();
        let uploads = media_uploads(&attachments, "caption", Some("[]".into()));
        assert_eq!(
            uploads
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(uploads[0].caption, "caption");
        assert_eq!(uploads[0].entities_json.as_deref(), Some("[]"));
        for item in &uploads[1..] {
            assert!(item.caption.is_empty());
            assert!(item.entities_json.is_none());
        }
        let send = MediaSend {
            chat_id: 42,
            reply_to_msg_id: 0,
            top_msg_id: 0,
            silent: false,
            schedule_date: None,
            items: uploads,
        };
        assert_eq!(send.validate(), Ok(()));
    }
}

#[test]
fn dialog_localization_keys_exist_in_every_language() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "desktop-back",
            "desktop-message",
            "desktop-loading",
            "desktop-cancel",
            "desktop-send",
            "desktop-error-generic",
            "desktop-empty-dialog",
            "desktop-retry",
            "feature-dialog-dialog_peer_list_load_more",
            "feature-dialog-dialog_delete_for_everyone",
            "feature-dialog-dialog_editor_title",
            "feature-dialog-dialog_reply",
            "feature-dialog-dialog_comments_title",
            "feature-dialog-dialog_send_schedule",
            "feature-dialog-dialog_send_silent",
            "feature-dialog-dialog_markdown_editor",
            "feature-dialog-dialog_editing",
            "feature-dialog-dialog_delete_confirm",
            "feature-dialog-dialog_forward_to",
            "feature-dialog-dialog_add_reaction",
            "feature-dialog-dialog_edit",
            "feature-dialog-dialog_delete",
            "feature-dialog-dialog_forward",
            "feature-dialog-dialog_forwarded_message",
            "feature-dialog-dialog_instant_view_unsupported",
            "feature-dialog-dialog_edited",
            "feature-dialog-dialog_outbox_failed",
            "feature-dialog-dialog_outbox_uncertain",
            "feature-dialog-dialog_cant_view",
            "feature-dialog-dialog_cant_send",
            "core-ui-status_network",
            "core-ui-status_outgoing_read",
            "core-ui-status_outgoing_sent",
            "core-ui-status_outgoing_sending",
            "core-ui-media_more_actions",
            "feature-dialog-dialog_discard_draft",
            "feature-dialog-dialog_discard_draft_confirm",
            "feature-dialog-dialog_draft_unsupported",
            "feature-dialog-dialog_outbox_waiting",
            "feature-dialog-dialog_pinned_list",
            "feature-dialog-dialog_scheduled_messages",
            "feature-dialog-dialog_scheduled_empty",
            "feature-dialog-dialog_scheduled_reschedule",
            "feature-dialog-dialog_scheduled_send_now",
            "feature-dialog-dialog_scheduled_send_confirm",
            "feature-dialog-dialog_scheduled_cancel",
            "feature-dialog-dialog_scheduled_cancel_confirm",
            "feature-dialog-dialog_scheduled_edit_changed",
            "feature-dialog-dialog_schedule_invalid",
            "feature-dialog-dialog_attach",
            "feature-dialog-dialog_attach_photo",
            "feature-dialog-dialog_attach_video",
            "feature-dialog-dialog_attach_file",
            "feature-dialog-dialog_attach_remove",
            "feature-dialog-dialog_media_photo",
            "feature-dialog-dialog_media_video",
            "feature-dialog-dialog_media_video_note",
            "feature-dialog-dialog_media_gif",
            "feature-dialog-dialog_media_audio",
            "feature-dialog-dialog_media_voice",
            "feature-dialog-dialog_media_document",
            "feature-dialog-dialog_media_sticker",
            "feature-dialog-dialog_media_link",
            "feature-dialog-dialog_media_poll",
            "feature-dialog-dialog_media_checklist",
            "feature-dialog-dialog_media_contact",
            "feature-dialog-dialog_media_location",
            "feature-dialog-dialog_media_dice",
            "feature-dialog-dialog_media_giveaway",
            "feature-dialog-dialog_media_giveaway_results",
            "feature-dialog-dialog_media_paid_media",
            "feature-dialog-dialog_media_game",
            "feature-dialog-dialog_media_invoice",
            "feature-dialog-dialog_media_story",
            "feature-dialog-dialog_media_rtmp_stream",
            "feature-dialog-dialog_poll_voters_title",
            "feature-dialog-dialog_poll_closed",
            "feature-dialog-dialog_task_done",
            "feature-dialog-dialog_task_open",
            "feature-dialog-dialog_location_open_maps",
            "feature-dialog-dialog_location_live",
            "feature-dialog-dialog_dice_pending",
            "feature-dialog-dialog_media_video_stream",
            "feature-dialog-dialog_media_play",
            "feature-dialog-dialog_media_cancel",
            "feature-dialog-dialog_media_close_viewer",
            "core-ui-media_kind_unknown",
            "core-ui-media_info_name",
            "core-ui-media_info_duration",
            "core-ui-media_info_size",
            "core-ui-media_action_save",
            "core-ui-media_action_save_as",
            "core-ui-media_action_open_externally",
            "core-ui-media_action_copy_image",
            "core-ui-media_action_copy_video",
            "core-ui-media_action_copy_caption",
            "core-ui-media_action_copied_image",
            "core-ui-media_action_copied_video",
            "core-ui-media_action_copy_failed",
            "core-ui-media_info_unknown",
            "core-ui-media_viewer_label",
            "feature-dialog-dialog_instant_view_media_download",
            "feature-dialog-dialog_media_failed",
            "feature-dialog-dialog_media_loading",
            "feature-dialog-dialog_media_uploading",
            "feature-dialog-dialog_media_open_failed",
            "feature-dialog-media_action_saved",
            "feature-dialog-media_action_save_failed",
            "feature-dialog-media_action_no_app",
            "feature-dialog-dialog_poll_updating",
            "feature-dialog-dialog_poll_submit",
            "feature-dialog-dialog_poll_retract",
            "feature-dialog-dialog_poll_close",
            "feature-dialog-dialog_checklist_add_action",
            "feature-dialog-dialog_reaction_users_title",
            "feature-dialog-dialog_read_show_when",
            "feature-dialog-dialog_read_unknown",
            "feature-dialog-dialog_read_receipts_expired",
            "feature-dialog-dialog_read_unread",
            "feature-dialog-dialog_instant_view_open",
            "feature-dialog-dialog_topics_empty",
            "feature-dialog-dialog_topic_hidden",
            "feature-dialog-dialog_topic_show_general",
            "feature-dialog-dialog_topic_hide_general",
            "feature-dialog-dialog_emoji_open",
            "feature-dialog-dialog_panel_stickers",
            "feature-dialog-dialog_saved_gifs",
            "feature-dialog-dialog_saved_gifs_empty",
            "feature-dialog-dialog_sticker_sets_empty",
        ] {
            assert!(i18n.text(key, &[]).is_ok(), "{locale}: {key}");
        }
        assert!(
            i18n.text(
                "core-ui-media_viewer_page_position",
                &[("arg1", "1"), ("arg2", "2")],
            )
            .is_ok(),
            "{locale}: album position"
        );
        assert!(
            i18n.text(
                "core-ui-media_info_dimensions_value",
                &[("arg1", "100"), ("arg2", "200")],
            )
            .is_ok(),
            "{locale}: dimensions"
        );
        assert!(
            i18n.text(
                "feature-dialog-dialog_media_download_progress",
                &[("arg1", "10"), ("arg2", "20")],
            )
            .is_ok(),
            "{locale}: download progress"
        );
        for key in [
            "feature-dialog-dialog_poll_correct_option",
            "feature-dialog-dialog_poll_solution",
            "feature-dialog-dialog_dice_result",
            "feature-dialog-dialog_media_giveaway_months",
            "feature-dialog-dialog_media_giveaway_stars",
            "feature-dialog-dialog_media_giveaway_winners",
            "feature-dialog-dialog_media_paid_media_items",
            "feature-dialog-dialog_media_story_number",
            "feature-dialog-dialog_checklist_progress",
            "feature-dialog-dialog_read_at",
        ] {
            assert!(
                i18n.text(key, &[("arg1", "1"), ("arg2", "2")]).is_ok(),
                "{locale}: {key}"
            );
        }
    }
}

pub(crate) fn wallpaper_dialog_scenario() {
    let page = DialogPage::builder().launch(Rc::new(I18n::new("en").unwrap()));
    let config = WallpaperConfig {
        mode: WallpaperMode::Image,
        path: None,
        telegram: None,
        pattern: false,
        colors: vec![0x102030, 0x405060],
        dim: 0.4,
        blur: true,
        intensity: -20,
        rotation: 30,
    };
    page.emit(DialogInput::SetWallpaper(config));
    let context = gtk::glib::MainContext::default();
    for _ in 0..10 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(page.model().wallpaper.is_visible());
    assert!(page.model().wallpaper.paintable().is_some());
    page.emit(DialogInput::SetWallpaper(WallpaperConfig {
        mode: WallpaperMode::None,
        ..WallpaperConfig::default()
    }));
    for _ in 0..10 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!page.model().wallpaper.is_visible());
    assert!(page.model().wallpaper.paintable().is_none());
}
