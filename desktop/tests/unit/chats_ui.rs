use super::*;

fn page() -> ChatsPage {
    ChatsPage::new(Rc::new(I18n::new("en").unwrap()))
}

fn chat(id: i64, title: &str, date: i64) -> ChatDto {
    ChatDto {
        id,
        title: title.into(),
        is_channel: false,
        is_group: false,
        is_forum: false,
        left: false,
        unread_count: 0,
        last_message_preview: None,
        last_message_date: Some(date),
        archived: false,
        muted: false,
        is_contact: false,
        is_bot: false,
        is_verified: false,
        photo_cache_key: None,
        pinned: false,
        read_inbox_max_id: 0,
        read_outbox_max_id: 0,
        peer_status: None,
        peer_status_at: None,
        last_media_thumb_cache_key: None,
        last_message_id: id.unsigned_abs() as i32,
        can_view: true,
        can_send_plain: true,
        can_send_photos: true,
        can_send_polls: true,
        can_forward: true,
        can_delete_others: false,
        emoji_status_document_id: None,
        last_message_outgoing: false,
        mute_override: false,
        unread_mark: false,
        unread_mentions_count: 0,
        unread_reactions_count: 0,
        can_manage_topics: false,
    }
}

#[test]
fn sorts_pinned_then_date_and_keeps_peer_identity() {
    let mut page = page();
    let mut pinned = chat(-1_000_000_000_015, "Pinned", 1);
    pinned.pinned = true;
    page.accept_chats(vec![chat(15, "User", 30), pinned, chat(-15, "Group", 20)]);
    let ids: Vec<_> = page.visible(0).iter().map(|chat| chat.id).collect();
    assert_eq!(ids, [-1_000_000_000_015, 15, -15]);
    page.accept_chats(vec![chat(-15, "Updated", 40)]);
    assert_eq!(page.chats.len(), 3);
    assert_eq!(page.visible(0)[1].title, "Updated");
}

#[test]
fn cursor_uses_network_order_not_display_sort_or_filter_or_updates() {
    let mut page = page();
    page.accept_chats(vec![
        chat(1, "Newest", 300),
        chat(2, "Older", 100),
        chat(3, "Last network row", 200),
    ]);
    page.query = "NEWEST".into();
    assert_eq!(page.visible(0).len(), 1);
    page.chats.insert(3, chat(3, "Live update", 900));
    assert_eq!(
        page.load_more(),
        Some(ChatsOutput::LoadMore {
            offset_date: 200,
            offset_id: 3,
            offset_peer_id: 3,
            folder_id: 0,
        })
    );
    assert_eq!(page.load_more(), None);
}

#[test]
fn cache_is_visible_but_never_advances_network_cursor_or_overwrites_network() {
    let mut page = page();
    page.from_cache = true;
    page.accept_chats(vec![chat(1, "Cached", 100)]);
    assert_eq!(page.visible(0)[0].title, "Cached");
    assert_eq!(page.folders[0].cursor, None);
    assert!(!page.folders[0].initialized);
    page.from_cache = false;
    page.accept_chats(vec![chat(1, "Network", 200)]);
    page.from_cache = true;
    page.accept_chats(vec![chat(1, "Stale cache", 50)]);
    assert_eq!(page.visible(0)[0].title, "Network");
    assert_eq!(page.folders[0].cursor.unwrap().date, 200);
}

#[test]
fn network_updates_never_roll_back_newer_media_preview_or_cache_keys() {
    let mut page = page();
    let mut current = chat(1, "Current", 200);
    current.last_message_id = 20;
    current.last_message_preview = Some("Newest preview".into());
    current.photo_cache_key = Some("avatar-new".into());
    current.last_media_thumb_cache_key = Some("thumb-new".into());
    current.last_message_outgoing = true;
    page.accept_chats(vec![current]);

    let mut stale = chat(1, "Stale", 100);
    stale.last_message_id = 10;
    stale.last_message_preview = Some("Old preview".into());
    stale.photo_cache_key = None;
    stale.last_media_thumb_cache_key = None;
    stale.last_message_outgoing = false;
    page.accept_chats(vec![stale]);

    let merged = &page.chats[&1];
    assert_eq!(merged.last_message_id, 20);
    assert_eq!(merged.last_message_date, Some(200));
    assert_eq!(
        merged.last_message_preview.as_deref(),
        Some("Newest preview")
    );
    assert_eq!(merged.photo_cache_key.as_deref(), Some("avatar-new"));
    assert_eq!(
        merged.last_media_thumb_cache_key.as_deref(),
        Some("thumb-new")
    );
    assert!(merged.last_message_outgoing);
}

#[test]
fn same_message_updates_keep_existing_preview_and_image_keys_when_missing() {
    let mut page = page();
    let mut initial = chat(1, "Initial", 150);
    initial.last_message_id = 15;
    initial.last_message_preview = Some("Preview text".into());
    initial.photo_cache_key = Some("avatar".into());
    initial.last_media_thumb_cache_key = Some("thumb".into());
    page.accept_chats(vec![initial]);

    let mut partial = chat(1, "Initial", 150);
    partial.last_message_id = 15;
    partial.last_message_preview = None;
    partial.photo_cache_key = None;
    partial.last_media_thumb_cache_key = None;
    page.accept_chats(vec![partial]);

    let merged = &page.chats[&1];
    assert_eq!(merged.last_message_preview.as_deref(), Some("Preview text"));
    assert_eq!(merged.photo_cache_key.as_deref(), Some("avatar"));
    assert_eq!(merged.last_media_thumb_cache_key.as_deref(), Some("thumb"));
}

#[test]
fn folders_have_independent_cursors_and_empty_page_stops_pagination() {
    let mut page = page();
    page.accept_chats(vec![chat(1, "Main", 100)]);
    page.folder = 1;
    assert_eq!(
        page.load_more(),
        Some(ChatsOutput::LoadMore {
            offset_date: 0,
            offset_id: 0,
            offset_peer_id: 0,
            folder_id: 1,
        })
    );
    let mut archived = chat(-7, "Archived", 20);
    archived.archived = true;
    page.folder = 0;
    page.accept_chats(vec![archived]);
    assert_eq!(page.folders[1].cursor.unwrap().peer_id, -7);
    assert_eq!(page.folders[0].cursor.unwrap().peer_id, 1);
    assert_eq!(page.visible(0).len(), 1);
    assert_eq!(page.visible(1).len(), 1);
    page.folder = 1;
    page.load_more();
    page.accept_chats(Vec::new());
    assert!(page.folders[1].exhausted);
    assert_eq!(page.load_more(), None);
    page.folder = 0;
    assert!(page.load_more().is_some());
}

#[test]
fn saved_messages_title_is_localized_and_searchable() {
    let mut page = page();
    page.current_user = Some(42);
    page.accept_chats(vec![chat(42, "Account name", 100)]);
    page.query = "SAVED messages".into();
    assert_eq!(page.visible(0).len(), 1);
    assert_eq!(page.title(&page.chats[&42]), "Saved Messages");
    page.i18n = Rc::new(I18n::new("ru").unwrap());
    page.query = "ИЗБРАННОЕ".into();
    assert_eq!(page.visible(0).len(), 1);
}

#[test]
fn refresh_replaces_only_requested_folder_and_invalid_peers_are_ignored() {
    let mut page = page();
    let mut archived = chat(3, "Archive", 3);
    archived.archived = true;
    page.accept_chats(vec![chat(1, "Old", 1), archived]);
    page.pending_folder = Some(0);
    page.replacing = true;
    page.accept_chats(vec![chat(2, "New", 2), chat(0, "Invalid", 0)]);
    assert!(!page.chats.contains_key(&1));
    assert!(page.chats.contains_key(&2));
    assert!(page.chats.contains_key(&3));
    assert!(!page.chats.contains_key(&0));
}

#[test]
fn repeated_network_cursor_and_pinned_only_pages_do_not_loop() {
    let mut page = page();
    page.accept_chats(vec![chat(1, "One", 100)]);
    page.load_more();
    page.accept_chats(vec![chat(1, "One", 100)]);
    assert!(page.folders[0].exhausted);
    let mut page = self::page();
    let mut pinned = chat(2, "Pinned", 20);
    pinned.pinned = true;
    page.accept_chats(vec![pinned]);
    assert!(page.folders[0].exhausted);
}

fn peer(id: i64) -> SearchPeerDto {
    SearchPeerDto {
        peer_id: id,
        title: format!("Peer {id}"),
        username: None,
        kind: "user".into(),
        is_bot: false,
        is_group: false,
        is_channel: false,
    }
}

fn message(chat_id: i64, id: i32) -> MessageDto {
    serde_json::from_value(serde_json::json!({
        "chat_id": chat_id, "id": id, "sender_id": 1, "text": "Result", "date": 100,
        "outgoing": false, "noforwards": false, "supports_streaming": false,
        "replies_count": 0, "forum_topic": false
    }))
    .unwrap()
}

fn results(messages: Vec<MessageDto>, rate: i32, peer_id: i64, id: i32) -> GlobalMessageSearchDto {
    GlobalMessageSearchDto {
        messages,
        next_rate: rate,
        next_peer_id: peer_id,
        next_offset_id: id,
    }
}

#[test]
fn search_generation_rejects_stale_debounce_and_results_after_reset() {
    let mut page = page();
    page.query = "first".into();
    page.invalidate_search();
    let first = page.remote.generation;
    assert_eq!(page.start_search(first).len(), 2);
    assert_eq!(
        page.invalidate_search(),
        Some(ChatsOutput::CancelSearch { generation: first })
    );
    page.query = "second".into();
    let second = page.remote.generation;
    assert!(page.start_search(first).is_empty());
    assert_eq!(page.start_search(second).len(), 2);
    page.accept_contacts(
        first,
        ContactsSearchDto {
            people: vec![peer(1)],
            chats: vec![],
        },
    );
    page.accept_messages(first, results(vec![message(1, 1)], 7, 1, 1));
    assert!(page.remote.people.is_empty());
    assert!(page.remote.messages.is_empty());
    assert!(page.remote.contacts_loading);
    assert!(page.remote.messages_loading);
    page.accept_contacts(
        second,
        ContactsSearchDto {
            people: vec![peer(2)],
            chats: vec![],
        },
    );
    assert_eq!(page.remote.people[0].peer_id, 2);
    page.invalidate_search();
    assert!(page.remote.generation > second);
    assert!(!page.remote.started);
}

#[test]
fn contacts_deduplicate_peer_identity_across_sections_without_touching_cache() {
    let mut page = page();
    page.accept_chats(vec![chat(7, "Cached", 5)]);
    page.query = "peer".into();
    page.start_search(0);
    page.accept_contacts(
        0,
        ContactsSearchDto {
            people: vec![peer(1), peer(1), peer(0)],
            chats: vec![peer(1), peer(-1), peer(-1), peer(-1_000_000_000_001)],
        },
    );
    assert_eq!(
        page.remote
            .people
            .iter()
            .map(|peer| peer.peer_id)
            .collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(
        page.remote
            .chats
            .iter()
            .map(|peer| peer.peer_id)
            .collect::<Vec<_>>(),
        [-1, -1_000_000_000_001]
    );
    assert_eq!(page.chats.len(), 1);
    assert_eq!(page.folders[0].cursor.unwrap().peer_id, 7);
    assert!(page.remote.messages_loading);
    assert!(!page.remote.contacts_loading);
}

#[test]
fn message_paging_preserves_all_server_offsets_and_deduplicates_chat_and_message_pair() {
    let mut page = page();
    page.query = "query".into();
    page.start_search(0);
    assert_eq!(page.search_messages(), None);
    page.accept_messages(
        0,
        results(
            vec![message(1, 9), message(1, 9), message(-1, 9)],
            123,
            -1,
            7,
        ),
    );
    assert_eq!(page.remote.messages.len(), 2);
    assert_eq!(
        page.search_messages(),
        Some(ChatsOutput::SearchMessages {
            generation: 0,
            query: "query".into(),
            offset_rate: 123,
            offset_peer_id: -1,
            offset_id: 7,
            limit: 50,
            folder_id: 0,
        })
    );
    page.accept_messages(
        0,
        results(
            vec![message(1, 9), message(-1, 8), message(0, 3), message(1, 0)],
            124,
            -1,
            6,
        ),
    );
    assert_eq!(page.remote.messages.len(), 3);
    assert_eq!(
        page.remote.cursor,
        SearchCursor {
            rate: 124,
            peer_id: -1,
            message_id: 6
        }
    );
    assert!(page.search_messages().is_some());
    page.accept_messages(0, results(vec![message(-1, 8)], 124, -1, 6));
    assert!(page.remote.exhausted);
    assert_eq!(page.search_messages(), None);
}

#[test]
fn empty_message_page_stops_and_partial_contact_results_stay_visible() {
    let mut page = page();
    page.query = "query".into();
    page.start_search(0);
    page.accept_contacts(
        0,
        ContactsSearchDto {
            people: vec![peer(7)],
            chats: vec![],
        },
    );
    page.accept_messages(0, results(vec![], 0, 0, 0));
    assert_eq!(page.remote.people.len(), 1);
    assert!(page.remote.exhausted);
    assert!(!page.remote.messages_loading);
    assert_eq!(page.search_messages(), None);
}

#[test]
fn archive_scope_is_one_and_custom_filter_search_scope_is_zero() {
    let mut page = page();
    page.query = "query".into();
    page.folder = 1;
    assert!(matches!(
        &page.start_search(0)[1],
        ChatsOutput::SearchMessages { folder_id: 1, .. }
    ));
    page.invalidate_search();
    page.folder = 0;
    page.selected_filter = Some(42);
    let generation = page.remote.generation;
    assert!(matches!(
        &page.start_search(generation)[1],
        ChatsOutput::SearchMessages { folder_id: 0, .. }
    ));
}

#[test]
fn recipient_mode_does_not_search_messages_and_never_emits_a_forward_action() {
    let mut page = page();
    page.query = "recipient".into();
    page.recipient_mode = true;
    assert!(matches!(
        page.start_search(0).as_slice(),
        [ChatsOutput::SearchContacts { .. }]
    ));
    assert_eq!(page.search_messages(), None);
    assert_eq!(
        page.open_peer(42, true),
        Some(ChatsOutput::SelectRecipient(42))
    );
    let mut blocked = chat(7, "Read only", 1);
    blocked.can_forward = false;
    page.chats.insert(7, blocked);
    assert_eq!(page.open_peer(7, false), None);
    page.chats.insert(8, chat(8, "Recipient", 1));
    assert_eq!(
        page.open_peer(8, false),
        Some(ChatsOutput::SelectRecipient(8))
    );
    page.recipient_mode = false;
    assert_eq!(page.open_peer(8, false), Some(ChatsOutput::OpenChat(8)));
}

#[test]
fn search_keys_exist_in_every_supported_locale_and_debug_redacts_queries() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "feature-chats-chats_search_people",
            "feature-chats-chats_search_global_chats",
            "feature-chats-chats_search_messages",
            "feature-chats-chats_search_retry",
            "feature-chats-chats_folder_manage",
            "feature-chats-chats_folder_empty",
        ] {
            assert!(!i18n.text(key, &[]).unwrap().is_empty(), "{locale}: {key}");
        }
    }
    let output = ChatsOutput::SearchMessages {
        generation: 3,
        query: "private query".into(),
        offset_rate: 0,
        offset_peer_id: 0,
        offset_id: 0,
        limit: 50,
        folder_id: 0,
    };
    assert!(!format!("{output:?}").contains("private query"));
}

#[test]
fn debug_messages_never_expose_chat_content_or_search_query() {
    let mut private = chat(42, "Private title", 100);
    private.last_message_preview = Some("Private body".into());
    assert_eq!(
        format!("{:?}", ChatsInput::SetChats(vec![private.clone()])),
        "SetChats"
    );
    assert_eq!(
        format!("{:?}", ChatsInput::UpdateChat(private)),
        "UpdateChat"
    );
    assert_eq!(
        format!("{:?}", ChatsInput::Filter("Private query".into())),
        "Filter"
    );
}
