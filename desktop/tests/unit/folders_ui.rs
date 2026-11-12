use super::*;

fn page() -> FoldersPage {
    FoldersPage::new(Rc::new(I18n::new("en").unwrap()))
}

fn chat(id: i64) -> ChatDto {
    ChatDto {
        id,
        title: format!("Chat {id}"),
        is_channel: false,
        is_group: false,
        is_forum: false,
        left: false,
        unread_count: 0,
        last_message_preview: None,
        last_message_date: None,
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
        last_message_id: 0,
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
fn membership_exclusion_inclusion_rules_types_and_separate_pins() {
    let mut f = empty_folder(2);
    let mut c = chat(42);
    f.pinned_chat_ids = vec![42];
    assert!(!contains(&f, &c));
    f.include_non_contacts = true;
    assert!(contains(&f, &c));
    f.exclude_read = true;
    assert!(!contains(&f, &c));
    c.unread_mark = true;
    assert!(contains(&f, &c));
    c.muted = true;
    f.exclude_muted = true;
    assert!(!contains(&f, &c));
    c.unread_mentions_count = 1;
    assert!(contains(&f, &c));
    c.unread_mentions_count = 0;
    c.unread_reactions_count = 1;
    assert!(contains(&f, &c));
    c.archived = true;
    f.exclude_archived = true;
    assert!(!contains(&f, &c));
    f.chat_ids.push(42);
    assert!(contains(&f, &c));
    f.exclude_chat_ids.push(42);
    assert!(!contains(&f, &c));
}

#[test]
fn all_chats_order_excludes_archive_and_can_move_all_before_rpc() {
    let mut p = page();
    p.accept_folders(vec![
        empty_folder(3),
        empty_folder(-1),
        empty_folder(0),
        empty_folder(2),
        empty_folder(3),
        empty_folder(1),
    ]);
    assert_eq!(
        p.folders.iter().map(|f| f.id).collect::<Vec<_>>(),
        [3, 0, 2]
    );
    let Some(FoldersOutput::Reorder(order)) = p.handle(FoldersInput::Move(1, 2)) else {
        panic!()
    };
    assert_eq!(order, [3, 2, 0]);
    assert_eq!(p.folders.iter().map(|f| f.id).collect::<Vec<_>>(), order);
    assert!(p.busy);
    assert!(p.handle(FoldersInput::Move(0, 1)).is_none());
    p.handle(FoldersInput::Error(p.text("desktop-error-generic")));
    assert!(!p.busy);
    assert!(p.error.is_some());
}

#[test]
fn save_preserves_ordered_pins_and_does_not_fabricate_server_success() {
    let mut p = page();
    let mut f = empty_folder(7);
    f.title = "Work".into();
    f.pinned_chat_ids = vec![99, 2, -18];
    f.chat_ids = vec![5];
    p.accept_folders(vec![f]);
    p.handle(FoldersInput::Edit(7));
    p.handle(FoldersInput::Title("Other".into()));
    let Some(FoldersOutput::Save(saved)) = p.handle(FoldersInput::Save) else {
        panic!()
    };
    assert_eq!(saved.pinned_chat_ids, [99, 2, -18]);
    assert_eq!(saved.chat_ids, [5]);
    assert_eq!(p.folders[1].title, "Work");
    assert!(p.handle(FoldersInput::Delete).is_none());
    p.handle(FoldersInput::Error(p.text("desktop-error-generic")));
    assert_eq!(p.draft.as_ref().unwrap().title, "Other");
    assert!(matches!(
        p.handle(FoldersInput::Delete),
        Some(FoldersOutput::Delete(7))
    ));
    assert_eq!(p.folders.len(), 2);
}

#[test]
fn local_picker_is_bounded_case_insensitive_and_never_adds_unknown_peers() {
    let mut p = page();
    p.accept_folders(Vec::new());
    p.handle(FoldersInput::SetChats((1..=40).map(chat).collect()));
    p.handle(FoldersInput::New);
    assert_eq!(p.picker().len(), 24);
    p.handle(FoldersInput::Query("CHAT 4".into()));
    assert_eq!(p.picker().iter().map(|c| c.id).collect::<Vec<_>>(), [4, 40]);
    p.handle(FoldersInput::Include(500));
    assert!(p.draft.as_ref().unwrap().chat_ids.is_empty());
    p.handle(FoldersInput::Include(4));
    p.handle(FoldersInput::Include(4));
    p.handle(FoldersInput::Exclude(4));
    assert_eq!(p.draft.as_ref().unwrap().chat_ids, [4]);
    assert_eq!(p.draft.as_ref().unwrap().exclude_chat_ids, [4]);
    assert!(!contains(p.draft.as_ref().unwrap(), &chat(4)));
    p.handle(FoldersInput::RemoveExclude(4));
    assert!(contains(p.draft.as_ref().unwrap(), &chat(4)));
}

#[test]
fn blank_title_and_oversized_emoticon_cannot_be_saved() {
    let mut p = page();
    p.accept_folders(Vec::new());
    p.handle(FoldersInput::New);
    assert!(!p.valid());
    p.handle(FoldersInput::Title("Папка".into()));
    p.handle(FoldersInput::Emoticon("😀😀😀😀😀😀😀😀".into()));
    assert!(p.valid());
    p.handle(FoldersInput::Emoticon("😀😀😀😀😀😀😀😀😀".into()));
    assert!(!p.valid());
    assert!(p.handle(FoldersInput::Save).is_none());
    p.handle(FoldersInput::Emoticon(String::new()));
    p.handle(FoldersInput::Title("12345678".into()));
    assert!(p.valid());
    p.handle(FoldersInput::Title("123456789".into()));
    assert!(!p.valid());
    p.handle(FoldersInput::Title("  ".into()));
    assert!(!p.valid());
}

#[test]
fn cache_loading_and_error_preserve_visible_data() {
    let mut p = page();
    assert!(!p.loaded);
    p.handle(FoldersInput::SetFromCache(true));
    p.handle(FoldersInput::SetFolders(vec![empty_folder(2)]));
    p.handle(FoldersInput::SetBusy(true));
    p.handle(FoldersInput::Error(p.text("desktop-error-generic")));
    assert!(p.loaded && p.from_cache && !p.busy);
    assert_eq!(p.folders.len(), 2);
    assert!(p.error.is_some());
    p.handle(FoldersInput::SetFromCache(false));
    p.handle(FoldersInput::SetFolders(vec![]));
    assert_eq!(p.folders[0].id, 0);
    assert!(p.error.is_none());
}

#[test]
fn every_folder_label_exists_in_all_locales() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in FLAGS.into_iter().chain([
            "feature-folders-folders_title",
            "feature-folders-folders_refresh",
            "feature-folders-folders_add",
            "feature-folders-folders_name",
            "feature-folders-folders_emoji",
            "feature-folders-folders_save",
            "feature-folders-folders_delete",
            "feature-folders-folders_loading",
            "core-ui-status_from_cache",
            "feature-folders-folders_empty",
            "feature-folders-folders_all_chats",
            "feature-folders-folders_reorder",
            "feature-folders-folders_search_chats",
            "feature-folders-folders_include_chats",
            "feature-folders-folders_exclude_chats",
            "feature-folders-folders_exclude",
            "desktop-back",
            "desktop-cancel",
            "desktop-error-generic",
        ]) {
            assert!(!i18n.text(key, &[]).unwrap().is_empty(), "{locale}: {key}");
        }
        assert!(
            !i18n
                .text("feature-folders-folders_chat_count", &[("arg1", "2")])
                .unwrap()
                .is_empty()
        );
    }
}
