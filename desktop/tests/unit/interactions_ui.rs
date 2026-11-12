use super::*;

#[test]
fn poll_validation_matches_protocol_limits_and_quiz_rules() {
    let answers = vec!["one".into(), "two".into()];
    let spec = poll_spec("Question", &answers, true, true, true, Some(1), None).unwrap();
    assert!(!spec.multiple_choice);
    assert_eq!(spec.correct_answer, Some(1));
    assert!(poll_spec(" ", &answers, true, false, false, None, None).is_none());
    assert!(poll_spec("Question", &answers, true, false, false, Some(0), None).is_none());
    assert!(poll_spec("Question", &answers, true, false, true, None, None).is_none());
    assert!(poll_spec("Question", &answers, true, false, true, Some(2), None).is_none());
    assert!(
        poll_spec(
            "Question",
            &answers,
            true,
            false,
            true,
            Some(0),
            Some(" ".into()),
        )
        .is_some()
    );
    assert!(poll_spec(&"😀".repeat(128), &answers, true, false, false, None, None,).is_none());
    assert!(
        poll_spec(
            "Question",
            &["😀".repeat(51), "ok".into()],
            true,
            false,
            false,
            None,
            None,
        )
        .is_none()
    );
    assert!(
        poll_spec(
            "Question",
            &answers,
            true,
            false,
            true,
            Some(0),
            Some("😀".repeat(101)),
        )
        .is_none()
    );
}

#[test]
fn checklist_validation_matches_protocol_limits() {
    let one = vec!["task".into()];
    assert!(checklist_spec("Title", &one, true, false).is_some());
    assert!(checklist_spec(" ", &one, false, false).is_none());
    assert!(checklist_spec("Title", &[], false, false).is_none());
    assert!(checklist_spec("Title", &[" ".into()], false, false).is_none());
    assert!(checklist_spec(&"😀".repeat(17), &one, false, false).is_none());
    assert!(checklist_spec("Title", &["😀".repeat(33)], false, false).is_none());
    assert!(checklist_spec("Title", &vec!["task".into(); 31], false, false).is_none());
}

#[test]
fn contact_and_location_validation_reject_invalid_protocol_values() {
    assert!(contact_spec("+12345", "First", "Last", "").is_some());
    for phone in ["", "++123", "12", "1234567890123456", "12x"] {
        assert!(contact_spec(phone, "First", "", "").is_none(), "{phone}");
    }
    assert!(contact_spec("+123", " ", "", "").is_none());
    assert!(contact_spec("+123", "First", "", &"x".repeat(65_537)).is_none());

    assert_eq!(
        location_spec("55.75", "37.61", 0, None).map(|spec| (spec.latitude, spec.longitude)),
        Some((55.75, 37.61))
    );
    for period in [-1, 1, 59, 86_401] {
        assert!(location_spec("0", "0", period, None).is_none());
    }
    for period in [0, 60, 86_400, i32::MAX] {
        assert!(location_spec("0", "0", period, None).is_some());
    }
    assert!(location_spec("91", "0", 0, None).is_none());
    assert!(location_spec("0", "181", 0, None).is_none());
    assert!(location_spec("NaN", "0", 0, None).is_none());
    assert!(location_spec("0", "0", 0, Some(90)).is_none());
    assert!(location_spec("0", "0", 60, Some(361)).is_none());
}

#[test]
fn inline_queries_and_dice_accept_only_android_current_values() {
    assert_eq!(
        inline_query("@Example_Bot cats"),
        Some(InlineQuery {
            username: "example_bot".into(),
            query: "cats".into(),
        })
    );
    assert_eq!(inline_query("@Bot").unwrap().query, "");
    for draft in [
        "hello @bot cats",
        "email@example.com",
        "@ab",
        "@1abc",
        "@bot!",
    ] {
        assert!(inline_query(draft).is_none(), "{draft}");
    }
    for emoticon in DICE_EMOTICONS {
        assert_eq!(dice_emoticon(emoticon), Some(emoticon));
    }
    assert_eq!(dice_emoticon("🎤"), None);
}

fn interaction_message(kind: &str, payload: serde_json::Value, outgoing: bool) -> MessageDto {
    serde_json::from_value(serde_json::json!({
        "chat_id": 42, "id": 7, "sender_id": 1, "text": null, "date": 100,
        "edit_date": null, "outgoing": outgoing, "media_kind": kind,
        "media_cache_key": null, "thumb_cache_key": null, "media_duration": null,
        "media_width": null, "media_height": null, "reply_quote": null,
        "reply_quote_entities_json": null, "entities_json": null, "noforwards": false,
        "reply_to_msg_id": null, "reply_to_top_id": null, "fwd_from": null,
        "fwd_from_id": null, "fwd_date": null, "via_bot": null, "sender_name": null,
        "sender_emoji_status_document_id": null, "grouped_id": null,
        "file_name": payload.to_string(), "file_size": null, "supports_streaming": false,
        "reactions_json": null, "replies_count": 0, "discussion_peer_id": null,
        "reply_markup_json": null, "forum_topic": false
    }))
    .unwrap()
}

fn poll_message() -> MessageDto {
    interaction_message(
        "poll",
        serde_json::json!({
            "creator": 1, "r": 0, "q": "Question", "n": 2, "z": 0, "m": 0,
            "x": 0, "o": 1, "s": "", "a": [
                {"t":"One", "c":0, "k":0, "v":1, "o":"01"},
                {"t":"Two", "c":1, "k":0, "v":1, "o":"02"}
            ]
        }),
        true,
    )
}

fn checklist_message(outgoing: bool) -> MessageDto {
    interaction_message(
        "todo",
        serde_json::json!({
            "t":"Title", "a":1, "c":1, "e":null, "i":[
                {"id":1, "t":"First", "d":0, "e":null},
                {"id":3, "t":"Second", "d":1, "e":null}
            ]
        }),
        outgoing,
    )
}

fn poll_model() -> InteractionModel {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    model.handle(InteractionInput::SetMessage(Box::new(poll_message())));
    model
}

#[test]
fn poll_commands_validate_current_dto_and_busy_blocks_duplicates() {
    let mut model = poll_model();
    assert!(
        model
            .handle(InteractionInput::Vote {
                message_id: 7,
                options: vec![vec![1], vec![1]],
            })
            .is_empty()
    );
    assert!(
        model
            .handle(InteractionInput::Vote {
                message_id: 7,
                options: vec![vec![1], vec![2]],
            })
            .is_empty()
    );
    assert!(matches!(
        model.handle(InteractionInput::Vote {
            message_id: 7,
            options: vec![vec![1]],
        })[0],
        WorkerCommand::VotePoll {
            chat_id: 42,
            message_id: 7,
            ..
        }
    ));
    assert!(model.busy);
    assert!(
        model
            .handle(InteractionInput::ClosePoll { message_id: 7 })
            .is_empty()
    );
    model.handle(InteractionInput::SetBusy(false));
    assert!(matches!(
        model.handle(InteractionInput::RetractVote { message_id: 7 })[0],
        WorkerCommand::RetractPollVote { .. }
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(
        model
            .handle(InteractionInput::ClosePoll { message_id: 8 })
            .is_empty()
    );
    assert!(matches!(
        model.handle(InteractionInput::ClosePoll { message_id: 7 })[0],
        WorkerCommand::ClosePoll { .. }
    ));
}

#[test]
fn quiz_and_closed_polls_do_not_emit_invalid_votes() {
    let mut model = poll_model();
    let mut quiz = poll_message();
    quiz.file_name = Some(
        serde_json::json!({
            "creator":0, "r":0, "q":"Quiz", "n":1, "z":0, "m":0, "x":1, "o":0,
            "a":[{"t":"One", "c":1, "k":1, "v":1, "o":"01"}]
        })
        .to_string(),
    );
    model.handle(InteractionInput::SetMessage(Box::new(quiz)));
    assert!(
        model
            .handle(InteractionInput::Vote {
                message_id: 7,
                options: vec![vec![1]]
            })
            .is_empty()
    );
    assert!(
        model
            .handle(InteractionInput::RetractVote { message_id: 7 })
            .is_empty()
    );
    assert!(
        model
            .handle(InteractionInput::ClosePoll { message_id: 7 })
            .is_empty()
    );
    assert!(
        model
            .handle(InteractionInput::LoadVoters {
                message_id: 7,
                offset: None
            })
            .is_empty()
    );
}

#[test]
fn done_response_does_not_fabricate_poll_state() {
    let mut model = poll_model();
    model.handle(InteractionInput::Vote {
        message_id: 7,
        options: vec![vec![1]],
    });
    let before = model.current.as_ref().unwrap().poll.clone();
    model.handle(InteractionInput::Response(WorkerResponse::Done));
    assert_eq!(model.current.as_ref().unwrap().poll, before);
    assert!(!model.busy);
    let mut updated = poll_message();
    updated.edit_date = Some(200);
    updated.file_name = Some(updated.file_name.unwrap().replace("Question", "Updated"));
    model.handle(InteractionInput::SetMessage(Box::new(updated)));
    assert_eq!(
        model.current.as_ref().unwrap().poll.as_ref().unwrap().q,
        "Updated"
    );
}

#[test]
fn checklist_mutations_use_permissions_ids_and_expected_snapshots() {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    model.handle(InteractionInput::SetMessage(Box::new(checklist_message(
        false,
    ))));
    assert!(matches!(
        model.handle(InteractionInput::AppendChecklist {
            message_id: 7,
            titles: vec!["Third".into()]
        })[0],
        WorkerCommand::AppendChecklist { first_id: 4, .. }
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(matches!(
        &model.handle(InteractionInput::ToggleChecklistItem { message_id: 7, item_id: 1 })[0],
        WorkerCommand::ToggleChecklistCompleted { completed, incompleted, .. }
            if completed.as_slice() == [1] && incompleted.is_empty()
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(
        model
            .handle(InteractionInput::EditChecklistTitle {
                message_id: 7,
                text: "New".into()
            })
            .is_empty()
    );
    assert!(
        model
            .handle(InteractionInput::DeleteChecklistItem {
                message_id: 7,
                item_id: 1
            })
            .is_empty()
    );

    model.handle(InteractionInput::SetMessage(Box::new(checklist_message(
        true,
    ))));
    assert!(matches!(
        &model.handle(InteractionInput::EditChecklistItem { message_id: 7, item_id: 1, text: "Changed".into() })[0],
        WorkerCommand::EditChecklistItem { edit: ChecklistTextEdit { expected_text, item_id: Some(1), .. }, .. }
            if expected_text == "First"
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(matches!(
        model.handle(InteractionInput::DeleteChecklistItem {
            message_id: 7,
            item_id: 3
        })[0],
        WorkerCommand::DeleteChecklistItem { .. }
    ));
}

#[test]
fn special_creation_uses_persisted_worker_send_not_local_success() {
    let mut model = InteractionModel::default();
    assert!(
        model
            .handle(InteractionInput::SendChecklist(ChecklistSendSpec {
                title: "Title".into(),
                items: vec!["Task".into()],
                others_can_append: false,
                others_can_complete: false,
            }))
            .is_empty()
    );
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    let outputs = model.handle(InteractionInput::SendPoll(PollSendSpec {
        question: "Question".into(),
        answers: vec!["One".into(), "Two".into()],
        anonymous: true,
        multiple_choice: false,
        quiz: false,
        correct_answer: None,
        solution: None,
    }));
    assert!(matches!(outputs[0], WorkerCommand::SendSpecial(_)));
    assert!(model.current.is_none());
    assert!(model.busy);
}

#[test]
fn identity_sends_require_context_and_reject_invalid_content() {
    let mut model = InteractionModel::default();
    let contact = || ContactSendSpec {
        phone: "+123".into(),
        first_name: "First".into(),
        last_name: String::new(),
        vcard: String::new(),
    };
    assert!(
        model
            .handle(InteractionInput::SendContact(contact()))
            .is_empty()
    );
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    assert!(matches!(
        &model.handle(InteractionInput::SendContact(contact()))[0],
        WorkerCommand::SendSpecial(send) if matches!(&send.content, SpecialContent::Contact(_))
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(
        model
            .handle(InteractionInput::SendDice("🎤".into()))
            .is_empty()
    );
}

fn inline_page(ids: &[&str], next_offset: Option<&str>) -> InlineBotResultsDto {
    InlineBotResultsDto {
        query_id: 77,
        gallery: false,
        next_offset: next_offset.map(str::to_owned),
        cache_time: 60,
        results: ids
            .iter()
            .map(|id| mono_proto_client::InlineBotResultDto {
                id: (*id).into(),
                kind: "article".into(),
                title: Some((*id).into()),
                description: None,
                url: None,
                document_id: None,
                thumb_cache_key: None,
            })
            .collect(),
    }
}

#[test]
fn inline_results_resolve_paginate_and_send_real_ids() {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    assert!(matches!(
        model.handle(InteractionInput::SetInlineDraft("@Example_Bot cats".into()))[0],
        WorkerCommand::Interaction(InteractionCommand::ResolveInlineBot { .. })
    ));
    let resolved = ResolvedPeerDto {
        peer_id: 5,
        username: Some("example_bot".into()),
        title: "Bot".into(),
        is_bot: true,
    };
    assert!(matches!(
        model.handle(InteractionInput::Response(WorkerResponse::Interaction(
            InteractionResponse::ResolvedPeer(resolved),
        )))[0],
        WorkerCommand::Interaction(InteractionCommand::InlineBotResults { bot_id: 5, .. })
    ));
    model.handle(InteractionInput::Response(WorkerResponse::Interaction(
        InteractionResponse::InlineBotResults(inline_page(&["a"], Some("next"))),
    )));
    assert!(matches!(
        &model.handle(InteractionInput::LoadInlineMore)[0],
        WorkerCommand::Interaction(InteractionCommand::InlineBotResults { offset, .. }) if offset == "next"
    ));
    model.handle(InteractionInput::Response(WorkerResponse::Interaction(
        InteractionResponse::InlineBotResults(inline_page(&["a", "b"], None)),
    )));
    let page = model.inline_results.as_ref().unwrap();
    assert_eq!(page.results.len(), 2);
    assert_eq!(page.next_offset, None);

    assert!(
        model
            .handle(InteractionInput::SendInlineResult("missing".into()))
            .is_empty()
    );
    let WorkerCommand::SendSpecial(send) =
        &model.handle(InteractionInput::SendInlineResult("b".into()))[0]
    else {
        panic!("inline result must map to durable SendSpecial");
    };
    assert!(matches!(
        &send.content,
        SpecialContent::InlineBotResult { query_id: 77, result_id } if result_id == "b"
    ));
}

#[test]
fn bot_keyboard_support_is_explicit_and_password_callback_is_not_sent() {
    let mut model = poll_model();
    let mut message = poll_message();
    message.via_bot = Some("example_bot".into());
    message.reply_markup_json = Some(
        serde_json::json!({"rows":[
            [{"t":"cb", "x":"Password", "d":"01", "pw":true}],
            [{"t":"cb", "x":"Callback", "d":"01"}],
            [{"t":"web", "x":"Unsupported", "u":"https://example.com"}],
            [{"t":"url", "x":"Open", "u":"https://example.com"}]
        ]})
        .to_string(),
    );
    model.handle(InteractionInput::SetMessage(Box::new(message)));
    assert!(
        model
            .handle(InteractionInput::PressBotButton {
                message_id: 7,
                row: 0,
                column: 0
            })
            .is_empty()
    );
    assert_eq!(
        model.bot_notice.as_deref(),
        Some("feature-dialog-dialog_bot_callback_password")
    );
    assert!(matches!(
        model.handle(InteractionInput::PressBotButton {
            message_id: 7,
            row: 1,
            column: 0
        })[0],
        WorkerCommand::Interaction(InteractionCommand::BotCallback { .. })
    ));
    model.handle(InteractionInput::SetBusy(false));
    assert!(
        model
            .handle(InteractionInput::PressBotButton {
                message_id: 7,
                row: 2,
                column: 0
            })
            .is_empty()
    );
    assert_eq!(
        model.bot_notice.as_deref(),
        Some("feature-dialog-dialog_unsupported_message")
    );
    assert!(
        model
            .handle(InteractionInput::PressBotButton {
                message_id: 7,
                row: 3,
                column: 0
            })
            .is_empty()
    );
    assert_eq!(model.bot_url.as_deref(), Some("https://example.com"));
}

#[test]
fn topic_hide_waits_for_authoritative_refresh() {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    model.handle(InteractionInput::SetCanManageTopics(true));
    let topic = ForumTopicDto {
        id: 1,
        title: "General".into(),
        icon_color: 0,
        icon_emoji_id: None,
        top_message: 1,
        date: 100,
        unread_count: 0,
        unread_mentions_count: 0,
        unread_reactions_count: 0,
        read_inbox_max_id: 0,
        pinned: false,
        closed: false,
        hidden: false,
        short: false,
        deleted: false,
        last_message_preview: None,
    };
    model.handle(InteractionInput::Response(WorkerResponse::Interaction(
        InteractionResponse::ForumTopics(ForumTopicsPageDto {
            count: 1,
            topics: vec![topic],
        }),
    )));
    assert!(matches!(
        model.handle(InteractionInput::SetGeneralHidden(true))[0],
        WorkerCommand::Interaction(InteractionCommand::SetGeneralTopicHidden { hidden: true, .. })
    ));
    assert!(!model.topics[0].hidden);
    model.handle(InteractionInput::Response(WorkerResponse::Interaction(
        InteractionResponse::Done,
    )));
    assert!(!model.topics[0].hidden);
}

#[test]
fn message_and_chat_transitions_do_not_keep_stale_interaction_state() {
    let mut model = poll_model();
    model.handle(InteractionInput::SetInlineDraft("@bot query".into()));
    model.handle(InteractionInput::SetCanManageTopics(true));
    let mut newer = poll_message();
    newer.id = 8;
    newer.file_name = Some(newer.file_name.unwrap().replace("Question", "Other poll"));
    model.handle(InteractionInput::SetMessage(Box::new(newer)));
    assert_eq!(model.current.as_ref().unwrap().message.id, 8);
    assert_eq!(
        model.current.as_ref().unwrap().poll.as_ref().unwrap().q,
        "Other poll"
    );
    let mut stale = poll_message();
    stale.id = 8;
    stale.edit_date = Some(100);
    let mut current = poll_message();
    current.id = 8;
    current.edit_date = Some(200);
    model.handle(InteractionInput::SetMessage(Box::new(current)));
    model.handle(InteractionInput::SetMessage(Box::new(stale)));
    assert_eq!(model.current.as_ref().unwrap().message.edit_date, Some(200));

    model.handle(InteractionInput::SetContext {
        chat_id: 43,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    assert!(model.current.is_none());
    assert!(model.inline.is_none());
    assert!(model.inline_results.is_none());
    assert!(model.topics.is_empty());
}

#[test]
fn interaction_localization_keys_exist_in_every_language() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "desktop-loading",
            "desktop-message",
            "desktop-error-generic",
            "desktop-contact-first-name",
            "desktop-contact-last-name",
            "desktop-contact-vcard",
            "desktop-coordinates",
            "desktop-topics",
            "feature-dialog-dialog_media_poll",
            "feature-dialog-dialog_poll_question",
            "feature-dialog-dialog_poll_anonymous",
            "feature-dialog-dialog_poll_multiple",
            "feature-dialog-dialog_poll_quiz",
            "feature-dialog-dialog_poll_explanation",
            "feature-dialog-dialog_poll_create",
            "feature-dialog-dialog_poll_closed",
            "feature-dialog-dialog_poll_submit",
            "feature-dialog-dialog_poll_retract",
            "feature-dialog-dialog_poll_close",
            "feature-dialog-dialog_poll_voters_title",
            "feature-dialog-dialog_media_checklist",
            "feature-dialog-dialog_checklist_title",
            "feature-dialog-dialog_checklist_add_hint",
            "feature-dialog-dialog_checklist_allow_append",
            "feature-dialog-dialog_checklist_allow_complete",
            "feature-dialog-dialog_checklist_create",
            "feature-dialog-dialog_checklist_add_action",
            "feature-dialog-dialog_edit",
            "feature-dialog-dialog_delete",
            "feature-dialog-dialog_bot",
            "feature-dialog-dialog_inline_loading",
            "feature-dialog-dialog_inline_empty",
            "feature-dialog-dialog_attach",
            "feature-auth-auth_phone_label",
            "feature-dialog-dialog_media_contact",
            "feature-dialog-dialog_media_dice",
            "feature-dialog-dialog_send",
            "feature-dialog-dialog_location_live",
            "feature-dialog-dialog_location_send_this",
            "feature-dialog-dialog_emoji_open",
            "feature-dialog-dialog_topics_empty",
            "feature-dialog-dialog_seen_row",
            "feature-dialog-dialog_poll_add_option",
            "feature-dialog-dialog_checklist_add",
            "feature-dialog-dialog_peer_list_load_more",
            "feature-dialog-dialog_outbox_waiting",
            "feature-dialog-dialog_panel_stickers",
            "feature-dialog-dialog_panel_emoji",
            "feature-dialog-dialog_saved_gifs",
            "feature-dialog-dialog_saved_gifs_empty",
            "feature-dialog-dialog_sticker_sets_empty",
            "feature-dialog-dialog_pack_empty",
            "feature-dialog-dialog_search",
            "feature-dialog-dialog_refresh",
            "feature-dialog-dialog_topic_collapse_archived",
            "feature-dialog-dialog_topic_archived_general",
            "feature-dialog-dialog_topic_pinned",
            "feature-dialog-dialog_topic_closed",
            "feature-dialog-dialog_topic_hidden",
            "feature-dialog-dialog_topic_hide_general",
            "feature-dialog-dialog_topic_show_general",
            "feature-dialog-dialog_reaction_users_title",
            "feature-dialog-dialog_read_show_when",
            "feature-dialog-dialog_nobody_viewed",
            "feature-dialog-dialog_read_unread",
            "feature-dialog-dialog_read_receipts_expired",
            "feature-dialog-dialog_read_unknown",
            "feature-dialog-dialog_bot_callback_password",
            "feature-dialog-dialog_unsupported_message",
            "feature-dialog-dialog_media_sticker",
            "feature-dialog-dialog_media_gif",
            "feature-dialog-dialog_reply_sticker",
            "feature-dialog-dialog_reply_gif",
            "feature-dialog-dialog_bot_switch_inline",
        ] {
            assert!(i18n.text(key, &[]).is_ok(), "{locale}: {key}");
        }
        for key in [
            "feature-dialog-dialog_poll_option",
            "feature-dialog-dialog_poll_correct_option",
            "feature-dialog-dialog_poll_solution",
            "feature-dialog-dialog_topic_unread",
            "feature-dialog-dialog_read_at",
        ] {
            assert!(i18n.text(key, &[("arg1", "1")]).is_ok(), "{locale}: {key}");
        }
        assert!(
            i18n.text(
                "feature-dialog-dialog_checklist_progress",
                &[("arg1", "1"), ("arg2", "2")],
            )
            .is_ok(),
            "{locale}: checklist progress"
        );
    }
}

pub(crate) fn component_builds_real_gtk_panels_and_updates_without_network() {
    use relm4::{Component, ComponentController};

    let panel = InteractionsPanel::builder()
        .launch(Rc::new(I18n::new("en").unwrap()))
        .detach();
    let context = gtk::glib::MainContext::default();
    let pump = || {
        for _ in 0..4 {
            while context.pending() {
                context.iteration(false);
            }
        }
    };
    assert!(!panel.model().widgets.identity_buttons[0].is_sensitive());
    panel.emit(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    pump();
    assert!(panel.model().widgets.identity_buttons[0].is_sensitive());
    panel.emit(InteractionInput::SetMessage(Box::new(poll_message())));
    pump();
    assert_eq!(
        panel
            .model()
            .widgets
            .poll_current
            .first_child()
            .and_then(|widget| widget.downcast::<gtk::Label>().ok())
            .map(|label| label.text().to_string())
            .as_deref(),
        Some("Question")
    );
    panel.emit(InteractionInput::SetContext {
        chat_id: 43,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });
    pump();
    assert!(panel.model().widgets.identity_buttons[0].is_sensitive());
    assert!(panel.model().widgets.poll_current.first_child().is_none());
}

#[test]
fn relm_messages_do_not_expose_private_content_through_debug() {
    let input = InteractionInput::SendContact(ContactSendSpec {
        phone: "+123".into(),
        first_name: "private name".into(),
        last_name: String::new(),
        vcard: String::new(),
    });
    let output = InteractionOutput(WorkerCommand::SendText(TextSend {
        chat_id: 42,
        text: "private message".into(),
        reply_to_msg_id: 0,
        top_msg_id: 0,
        entities_json: None,
        webpage_url: None,
        silent: false,
        no_webpage: false,
        schedule_date: None,
    }));
    let rendered = format!("{input:?} {output:?}");
    assert!(!rendered.contains("private name"));
    assert!(!rendered.contains("private message"));
}

#[test]
fn durable_special_sends_map_to_send_special_outbox() {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });

    let output = model.handle(InteractionInput::SendPoll(PollSendSpec {
        question: "Question".into(),
        answers: vec!["One".into(), "Two".into()],
        anonymous: false,
        multiple_choice: false,
        quiz: false,
        correct_answer: None,
        solution: None,
    }));
    let WorkerCommand::SendSpecial(send) = &output[0] else {
        panic!("poll must map to durable SendSpecial");
    };
    assert_eq!(send.chat_id, 42);
    assert!(!send.silent);
    assert!(send.schedule_date.is_none());
    assert!(matches!(&send.content, SpecialContent::Poll(spec) if spec.question == "Question"));

    model.handle(InteractionInput::SetBusy(false));
    let output = model.handle(InteractionInput::SendChecklist(ChecklistSendSpec {
        title: "Tasks".into(),
        items: vec!["First".into()],
        others_can_append: true,
        others_can_complete: false,
    }));
    let WorkerCommand::SendSpecial(send) = &output[0] else {
        panic!("checklist must map to durable SendSpecial");
    };
    assert_eq!(send.chat_id, 42);
    assert!(matches!(&send.content, SpecialContent::Checklist(spec) if spec.title == "Tasks"));
}

#[test]
fn identity_sends_map_to_durable_special_send() {
    let mut model = InteractionModel::default();
    model.handle(InteractionInput::SetContext {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
    });

    let output = model.handle(InteractionInput::SendContact(ContactSendSpec {
        phone: "+123".into(),
        first_name: "First".into(),
        last_name: String::new(),
        vcard: String::new(),
    }));
    let WorkerCommand::SendSpecial(send) = &output[0] else {
        panic!("contact must map to durable SendSpecial");
    };
    assert_eq!(send.chat_id, 42);
    assert!(matches!(&send.content, SpecialContent::Contact(spec) if spec.phone == "+123"));

    model.handle(InteractionInput::SetBusy(false));
    let output = model.handle(InteractionInput::SendDice("🎲".into()));
    let WorkerCommand::SendSpecial(send) = &output[0] else {
        panic!("dice must map to durable SendSpecial");
    };
    assert!(matches!(&send.content, SpecialContent::Dice { emoticon } if emoticon == "🎲"));

    model.handle(InteractionInput::SetBusy(false));
    let output = model.handle(InteractionInput::SendLocation(LocationSendSpec {
        latitude: 55.75,
        longitude: 37.61,
        live_period: 0,
        heading: None,
    }));
    let WorkerCommand::SendSpecial(send) = &output[0] else {
        panic!("location must map to durable SendSpecial");
    };
    assert!(matches!(&send.content, SpecialContent::Location(spec) if spec.latitude == 55.75));
}
