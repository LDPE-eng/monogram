use super::*;

#[test]
fn configuration_is_validated_and_debug_redacts_hash() {
    let hash = "a".repeat(32);
    let config = ApiConfig::new(123, hash.clone()).unwrap();
    assert!(!format!("{config:?}").contains(&hash));
    assert!(ApiConfig::new(0, hash.clone()).is_err());
    assert!(ApiConfig::new(1, "bad".into()).is_err());
    assert!(ApiConfig::new(1, "x".repeat(32)).is_err());
}

#[test]
fn errors_never_expose_server_payloads() {
    use mono_proto_client::MtprotoError;
    let secret = "private text +79000000000 hash=secret";
    let error = ClientError::from(MtprotoError::Message(secret.into()));
    assert_eq!(error, ClientError::Protocol);
    assert!(!format!("{error:?}: {error}").contains(secret));
    assert_eq!(
        ClientError::from(MtprotoError::Message(
            "RPC 420: FLOOD_WAIT_123 sensitive".into()
        )),
        ClientError::FloodWait { seconds: 123 }
    );
    assert_eq!(
        ClientError::from(MtprotoError::Message("request cancelled".into())),
        ClientError::Cancelled
    );
    assert_eq!(
        ClientError::from(MtprotoError::Message(
            "RPC 401: AUTH_KEY_UNREGISTERED".into()
        )),
        ClientError::Unauthorized
    );
    assert_eq!(
        ClientError::from(MtprotoError::PasswordRequired),
        ClientError::PasswordRequired
    );
}

#[test]
fn page_limits_and_text_validation_are_bounded() {
    assert_eq!(page_limit(999), Ok(100));
    assert_eq!(page_limit(1), Ok(1));
    assert!(page_limit(0).is_err());
    let mut message = TextSend {
        chat_id: 1,
        text: "hello".into(),
        reply_to_msg_id: 0,
        top_msg_id: 0,
        entities_json: Some("[]".into()),
        webpage_url: None,
        silent: false,
        no_webpage: false,
        schedule_date: None,
    };
    assert!(message.validate().is_ok());
    message.schedule_date = Some(i32::MAX);
    assert!(message.validate().is_err());
    message.schedule_date = Some(0x7fff_fffe);
    assert!(message.validate().is_ok());
    message.entities_json = Some("{}".into());
    assert!(message.validate().is_err());
}

fn upload(kind: &str) -> MediaUpload {
    MediaUpload {
        path: "source".into(),
        kind: kind.into(),
        mime_type: String::new(),
        file_name: "source".into(),
        caption: String::new(),
        entities_json: None,
        duration: 0,
        width: 0,
        height: 0,
    }
}

#[test]
fn media_upload_album_and_policy_validation_match_protocol_bounds() {
    let mut send = MediaSend {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
        silent: false,
        schedule_date: None,
        items: vec![upload("photo")],
    };
    assert!(send.validate().is_ok());
    send.items = (0..10).map(|_| upload("photo")).collect();
    assert!(send.validate().is_ok());
    send.items.push(upload("photo"));
    assert_eq!(send.validate(), Err(ClientError::InvalidRequest));
    send.items = vec![upload("photo"), upload("video")];
    assert!(send.validate().is_ok());
    send.items = vec![upload("document"), upload("document")];
    assert!(send.validate().is_ok());
    send.items = vec![upload("document"), upload("photo")];
    assert_eq!(send.validate(), Err(ClientError::InvalidRequest));
    send.items = vec![upload("sticker")];
    assert_eq!(send.validate(), Err(ClientError::InvalidRequest));
    let mut policies = AutoDownloadSettings::default();
    policies.roaming.photo.enabled = true;
    policies.roaming.photo.max_bytes = 1024;
    assert!(policies.validate().is_ok());
    assert_eq!(
        policies
            .policy(NetworkClass::Roaming)
            .rule(MediaCategory::Photo),
        Some(policies.roaming.photo)
    );
    policies.wifi.video.max_bytes = 0;
    assert_eq!(policies.validate(), Err(ClientError::InvalidRequest));
    assert_eq!(
        MediaCategory::from_media_kind(Some("gif")),
        MediaCategory::Video
    );
    assert_eq!(
        MediaCategory::from_media_kind(Some("voice")),
        MediaCategory::Document
    );
    assert_eq!(CacheLimit::Unlimited.bytes(), None);
    assert_eq!(CacheLimit::Limited(4096).bytes(), Some(4096));
    assert!(CacheLimit::Unlimited.validate().is_ok());
    assert_eq!(
        CacheLimit::Limited(0).validate(),
        Err(ClientError::InvalidRequest)
    );
    let playback = NetworkPolicy {
        wifi: true,
        mobile: false,
        roaming: true,
    };
    assert!(playback.allows(NetworkClass::Wifi));
    assert!(!playback.allows(NetworkClass::Mobile));
    assert!(playback.allows(NetworkClass::Roaming));
}

#[test]
fn special_send_constraints_match_mono_and_checklist_edits_are_optimistic() {
    let poll = mono_proto_client::PollSendSpec {
        question: "Question".into(),
        answers: vec!["One".into(), "Two".into()],
        anonymous: false,
        multiple_choice: false,
        quiz: true,
        correct_answer: Some(1),
        solution: Some("Explanation".into()),
    };
    let mut send = SpecialSend {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
        silent: false,
        schedule_date: None,
        content: SpecialContent::Poll(poll.clone()),
    };
    assert!(send.validate().is_ok());
    let mut invalid = poll.clone();
    invalid.multiple_choice = true;
    send.content = SpecialContent::Poll(invalid);
    assert_eq!(send.validate(), Err(ClientError::InvalidRequest));
    let mut invalid = poll.clone();
    invalid.correct_answer = Some(2);
    send.content = SpecialContent::Poll(invalid);
    assert_eq!(send.validate(), Err(ClientError::InvalidRequest));
    send.content = SpecialContent::Checklist(mono_proto_client::ChecklistSendSpec {
        title: "Tasks".into(),
        items: vec!["First".into(), "Second".into()],
        others_can_append: true,
        others_can_complete: false,
    });
    assert!(send.validate().is_ok());
    let edit = ChecklistTextEdit {
        item_id: Some(1),
        text: "Updated".into(),
        entities_json: Some("[]".into()),
        expected_text: "First".into(),
        expected_entities_json: Some("[]".into()),
    };
    assert!(edit.validate(false).is_ok());
    assert!(edit.clone().validate(true).is_ok());
    let mut invalid_edit = edit;
    invalid_edit.expected_entities_json = Some("{}".into());
    assert_eq!(
        invalid_edit.validate(false),
        Err(ClientError::InvalidRequest)
    );
    assert!(validate_append_checklist(3, &["Third".into()]).is_ok());
    assert_eq!(
        validate_append_checklist(0, &["Third".into()]),
        Err(ClientError::InvalidRequest)
    );
    assert!(validate_checklist_completion(&[1], &[2]).is_ok());
    assert_eq!(
        validate_checklist_completion(&[1], &[1]),
        Err(ClientError::InvalidRequest)
    );
    assert!(validate_poll_vote(&[b"0".to_vec()]).is_ok());
    assert_eq!(validate_poll_vote(&[]), Err(ClientError::InvalidRequest));
    assert_eq!(
        validate_poll_vote(&[b"0".to_vec(), b"0".to_vec()]),
        Err(ClientError::InvalidRequest)
    );
}

#[test]
fn extended_special_content_matches_protocol_constraints() {
    use mono_proto_client::{ContactSendSpec, LocationSendSpec};

    let send = |content| SpecialSend {
        chat_id: 42,
        reply_to_msg_id: 0,
        top_msg_id: 0,
        silent: false,
        schedule_date: None,
        content,
    };

    let contact = ContactSendSpec {
        phone: "+15550123456".into(),
        first_name: "Ada".into(),
        last_name: "Lovelace".into(),
        vcard: String::new(),
    };
    assert!(
        send(SpecialContent::Contact(contact.clone()))
            .validate()
            .is_ok()
    );
    let mut invalid = contact;
    invalid.first_name = " ".into();
    assert_eq!(
        send(SpecialContent::Contact(invalid)).validate(),
        Err(ClientError::InvalidRequest)
    );

    assert!(
        send(SpecialContent::Dice {
            emoticon: "🎯".into()
        })
        .validate()
        .is_ok()
    );
    assert_eq!(
        send(SpecialContent::Dice {
            emoticon: "❌".into()
        })
        .validate(),
        Err(ClientError::InvalidRequest)
    );

    let static_location = LocationSendSpec {
        latitude: 51.5,
        longitude: -0.12,
        live_period: 0,
        heading: None,
    };
    assert!(
        send(SpecialContent::Location(static_location.clone()))
            .validate()
            .is_ok()
    );
    let mut invalid = static_location;
    invalid.heading = Some(90);
    assert_eq!(
        send(SpecialContent::Location(invalid)).validate(),
        Err(ClientError::InvalidRequest)
    );
    let live_location = LocationSendSpec {
        latitude: 51.5,
        longitude: -0.12,
        live_period: 60,
        heading: Some(90),
    };
    assert!(
        send(SpecialContent::Location(live_location.clone()))
            .validate()
            .is_ok()
    );
    let mut scheduled = send(SpecialContent::Location(live_location));
    scheduled.schedule_date = Some(100);
    assert_eq!(scheduled.validate(), Err(ClientError::InvalidRequest));

    assert!(
        send(SpecialContent::InlineBotResult {
            query_id: 7,
            result_id: "result-1".into()
        })
        .validate()
        .is_ok()
    );
    assert_eq!(
        send(SpecialContent::InlineBotResult {
            query_id: 0,
            result_id: "result-1".into()
        })
        .validate(),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        send(SpecialContent::InlineBotResult {
            query_id: 7,
            result_id: String::new()
        })
        .validate(),
        Err(ClientError::InvalidRequest)
    );
}

#[test]
fn secrets_and_preference_documents_are_validated_without_secret_debug_output() {
    let hash = "a".repeat(32);
    let config = ApiConfig::from_secrets(&format!(
        "# comment\nexport API_ID=123\nAPI_HASH='{hash}'\n"
    ))
    .unwrap();
    assert_eq!(config.api_id, 123);
    assert!(!format!("{config:?}").contains(&hash));
    assert!(ApiConfig::from_secrets("API_ID=123").is_err());
    assert!(ApiConfig::from_secrets(&format!("API_ID=1\nAPI_ID=2\nAPI_HASH={hash}")).is_err());
    let document = PreferenceDocument {
        version: APP_PREFERENCES_VERSION,
        payload_json: r#"{"theme":"dark"}"#.into(),
    };
    assert!(document.validate().is_ok());
    assert_eq!(
        PreferenceDocument {
            version: 0,
            ..document.clone()
        }
        .validate(),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        PreferenceDocument {
            payload_json: "not-json".into(),
            ..document
        }
        .validate(),
        Err(ClientError::InvalidRequest)
    );
    assert_eq!(
        PreferenceNamespace::parse("proxy"),
        Some(PreferenceNamespace::Proxy)
    );
    assert_eq!(PreferenceNamespace::parse("unknown"), None);
}
