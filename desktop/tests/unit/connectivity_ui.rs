use super::*;

use std::time::{SystemTime, UNIX_EPOCH};

fn page() -> ConnectivityPage {
    ConnectivityPage::new(Rc::new(I18n::new("en").unwrap()))
}

fn record(id: i64) -> ProxyRecord {
    ProxyRecord {
        id,
        kind: ProxyKind::Socks5,
        host: "proxy.example".into(),
        port: 1080,
        username: String::new(),
        has_password: false,
        has_secret: false,
        active: false,
        checked: false,
        latency_ms: None,
    }
}

fn device(hash: &str, name: &str) -> DeviceRecord {
    DeviceRecord {
        hash: hash.into(),
        name: name.into(),
        is_current: name == "This",
        last_active: None,
    }
}

fn exception(chat_id: i64) -> NotificationException {
    NotificationException {
        chat_id,
        title: Some(format!("Chat {chat_id}")),
        muted: false,
    }
}

fn temp_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "monogram-connectivity-{}-{nanos}-{name}",
        std::process::id()
    ))
}

#[test]
fn proxy_links_parse_mtproto_socks_and_plain_urls() {
    let mtproto = parse_proxy_link("tg://proxy?server=example.com&port=443&secret=aabbcc").unwrap();
    assert_eq!(mtproto.kind, ProxyKind::Mtproto);
    assert_eq!(mtproto.host, "example.com");
    assert_eq!(mtproto.port, 443);
    assert_eq!(mtproto.secret, "aabbcc");

    let socks =
        parse_proxy_link("tg://socks?server=127.0.0.1&port=1080&user=alice&pass=secret").unwrap();
    assert_eq!(socks.kind, ProxyKind::Socks5);
    assert_eq!(socks.username, "alice");
    assert_eq!(socks.password, "secret");

    let url = parse_proxy_link("socks5://user:pw@host.example:1081").unwrap();
    assert_eq!(url.kind, ProxyKind::Socks5);
    assert_eq!(url.host, "host.example");
    assert_eq!(url.port, 1081);
    assert_eq!(url.username, "user");
    assert_eq!(url.password, "pw");

    let https = parse_proxy_link("https://host.example:8443").unwrap();
    assert_eq!(https.kind, ProxyKind::Https);
    assert_eq!(https.host, "host.example");
    assert_eq!(https.port, 8443);
    assert_eq!(https.username, "");

    assert!(parse_proxy_link("not a proxy link").is_none());
}

#[test]
fn proxy_draft_requires_host_port_and_type_specific_secrets() {
    let mut draft = ProxyDraft::new();
    assert!(!draft.valid());
    draft.host = "proxy.example".into();
    draft.set_port("1080".into());
    assert!(draft.valid());
    draft.username = "user".into();
    assert!(!draft.valid());
    draft.password = "pw".into();
    assert!(draft.valid());

    let mtproto_index = PROXY_KINDS
        .iter()
        .position(|kind| *kind == ProxyKind::Mtproto)
        .unwrap();
    draft.set_kind(mtproto_index);
    assert!(!draft.valid());
    draft.secret = "aabb".into();
    assert!(draft.valid());
    draft.secret = "xyz".into();
    assert!(!draft.valid());

    draft.set_kind(0);
    draft.set_port("0".into());
    assert!(!draft.valid());
    draft.set_port("70000".into());
    assert!(!draft.valid());
}

#[test]
fn proxy_editor_round_trips_through_worker_responses_without_fabricating_success() {
    let mut page = page();
    page.handle(ConnectivityInput::ProxyNew);
    assert!(page.draft.is_some());
    page.handle(ConnectivityInput::ProxyHost("proxy.example".into()));
    page.handle(ConnectivityInput::ProxyPort("1080".into()));
    let outputs = page.handle(ConnectivityInput::ProxySave);
    let ConnectivityOutput::ProxySave(submission) = &outputs[0] else {
        panic!()
    };
    assert_eq!(submission.id, 0);
    assert_eq!(submission.host, "proxy.example");
    assert_eq!(submission.port, 1080);
    assert!(page.busy);
    assert!(page.handle(ConnectivityInput::ProxyNew).is_empty());

    page.handle(ConnectivityInput::SetProxies(vec![record(7)]));
    assert!(!page.busy);
    page.handle(ConnectivityInput::ProxyEdit(7));
    assert_eq!(page.draft.as_ref().unwrap().host, "proxy.example");
    let outputs = page.handle(ConnectivityInput::ProxySave);
    let ConnectivityOutput::ProxySave(submission) = &outputs[0] else {
        panic!()
    };
    assert_eq!(submission.id, 7);
}

#[test]
fn proxy_delete_requires_confirmation_and_activates_the_selected_record() {
    let mut first = page();
    first.handle(ConnectivityInput::SetProxies(vec![record(1), record(2)]));
    assert!(matches!(
        first.handle(ConnectivityInput::ProxyActivate(1))[0],
        ConnectivityOutput::ProxyActivate(1)
    ));
    assert!(first.busy);
    first.handle(ConnectivityInput::SetProxies(vec![record(1), record(2)]));

    first.handle(ConnectivityInput::ProxyEdit(2));
    first.handle(ConnectivityInput::ProxyDeleteCurrent);
    assert_eq!(first.confirming_delete, Some(2));
    first.handle(ConnectivityInput::ProxyDeleteCancel);
    assert_eq!(first.confirming_delete, None);
    first.handle(ConnectivityInput::ProxyDeleteCurrent);
    assert!(matches!(
        first.handle(ConnectivityInput::ProxyDeleteConfirm)[0],
        ConnectivityOutput::ProxyDelete(2)
    ));

    let mut second = page();
    second.handle(ConnectivityInput::SetProxies(vec![record(1)]));
    assert!(
        second
            .handle(ConnectivityInput::ProxyDeleteRequest(9))
            .is_empty()
    );
    assert!(
        second
            .handle(ConnectivityInput::ProxyDeleteConfirm)
            .is_empty()
    );
}

#[test]
fn proxy_probe_updates_only_the_matching_record() {
    let mut page = page();
    page.handle(ConnectivityInput::SetProxies(vec![record(1), record(2)]));
    page.handle(ConnectivityInput::SetProxyProbe(ProxyProbe {
        id: 1,
        ok: true,
        latency_ms: Some(42),
    }));
    assert_eq!(page.proxies[0].latency_ms, Some(42));
    assert!(page.proxies[0].checked);
    assert!(!page.proxies[1].checked);
    page.handle(ConnectivityInput::SetProxyProbe(ProxyProbe {
        id: 1,
        ok: false,
        latency_ms: None,
    }));
    assert!(page.proxies[0].checked);
    assert_eq!(page.proxies[0].latency_ms, None);
}

#[test]
fn notifications_follow_worker_round_trips_and_never_duplicate() {
    let mut page = page();
    let outputs = page.handle(ConnectivityInput::NotificationPrivate(false));
    let ConnectivityOutput::NotificationSave(settings) = &outputs[0] else {
        panic!()
    };
    assert!(!settings.private);
    assert!(page.busy);
    assert!(
        page.handle(ConnectivityInput::NotificationGroups(false))
            .is_empty()
    );

    page.handle(ConnectivityInput::SetNotificationSettings(
        NotificationSettings {
            private: false,
            ..NotificationSettings::default()
        },
    ));
    assert!(!page.busy);
    assert!(
        page.handle(ConnectivityInput::NotificationPrivate(false))
            .is_empty()
    );
    assert!(matches!(
        page.handle(ConnectivityInput::NotificationReset)[0],
        ConnectivityOutput::NotificationReset
    ));
    assert!(page.busy);

    page.handle(ConnectivityInput::SetNotificationExceptions(vec![
        exception(42),
    ]));
    assert!(matches!(
        page.handle(ConnectivityInput::NotificationMute { chat_id: 42 })[0],
        ConnectivityOutput::NotificationMute { chat_id: 42 }
    ));
    assert!(page.busy);
    page.handle(ConnectivityInput::SetNotificationExceptions(vec![
        exception(42),
    ]));
    assert!(matches!(
        page.handle(ConnectivityInput::NotificationUnmute { chat_id: 42 })[0],
        ConnectivityOutput::NotificationUnmute { chat_id: 42 }
    ));
}

#[test]
fn local_policy_round_trips_through_xdg_json_file() {
    let path = temp_file("local-policy.json");
    let mut page = page();
    page.handle(ConnectivityInput::SetLocalPolicyPath(path.clone()));
    assert!(page.local_policy.enabled);
    assert!(page.local_policy.preview);

    let outputs = page.handle(ConnectivityInput::NotificationEnabled(false));
    assert!(matches!(
        outputs[0],
        ConnectivityOutput::LocalPolicyChanged(LocalNotificationPolicy { enabled: false, .. })
    ));
    assert!(!LocalNotificationPolicy::load(&path).enabled);
    assert!(LocalNotificationPolicy::load(&path).preview);

    let outputs = page.handle(ConnectivityInput::NotificationPreview(false));
    assert!(matches!(
        outputs[0],
        ConnectivityOutput::LocalPolicyChanged(LocalNotificationPolicy { preview: false, .. })
    ));
    assert_eq!(
        LocalNotificationPolicy::load(&path),
        LocalNotificationPolicy {
            enabled: false,
            preview: false
        }
    );
    let _ = fs::remove_file(&path);
}

#[test]
fn devices_revoke_requires_confirmation_and_qr_requests_emit_commands() {
    let mut page = page();
    page.handle(ConnectivityInput::SetDevices(vec![
        device("a", "Other"),
        device("b", "This"),
    ]));
    assert!(matches!(
        page.handle(ConnectivityInput::DeviceQrRequest)[0],
        ConnectivityOutput::DeviceQr
    ));
    assert!(page.busy);
    page.handle(ConnectivityInput::SetQr(QrLink::Pending {
        token: vec![1, 2, 3],
        expires: 0,
    }));
    assert!(!page.busy);
    assert!(matches!(
        page.handle(ConnectivityInput::DeviceQrRefresh)[0],
        ConnectivityOutput::DeviceQrPoll
    ));
    assert!(page.busy);
    page.handle(ConnectivityInput::SetQr(QrLink::Idle));
    page.handle(ConnectivityInput::SetQr(QrLink::PasswordPending));
    assert!(matches!(page.qr, QrLink::PasswordPending));
    page.handle(ConnectivityInput::SetQr(QrLink::Expired));
    assert!(matches!(page.qr, QrLink::Expired));
    page.handle(ConnectivityInput::SetQr(QrLink::Invalid));
    assert!(matches!(page.qr, QrLink::Invalid));
    page.handle(ConnectivityInput::SetQr(QrLink::Linked));
    assert!(matches!(page.qr, QrLink::Linked));
    page.handle(ConnectivityInput::SetQr(QrLink::Idle));

    assert!(
        page.handle(ConnectivityInput::DeviceRevokeRequest("missing".into()))
            .is_empty()
    );
    page.handle(ConnectivityInput::DeviceRevokeRequest("a".into()));
    assert_eq!(page.confirming_revoke.as_deref(), Some("a"));
    page.handle(ConnectivityInput::DeviceRevokeCancel);
    assert!(page.confirming_revoke.is_none());
    page.handle(ConnectivityInput::DeviceRevokeRequest("a".into()));
    assert!(matches!(
        page.handle(ConnectivityInput::DeviceRevokeConfirm)[0],
        ConnectivityOutput::DeviceRevoke(ref hash) if hash == "a"
    ));
}

#[test]
fn proxy_probe_direct_and_check_all_emit_the_matching_outputs() {
    let mut first = page();
    first.handle(ConnectivityInput::SetProxies(vec![record(1), record(2)]));
    assert!(matches!(
        first.handle(ConnectivityInput::ProxyProbe(1))[0],
        ConnectivityOutput::ProxyProbe(1)
    ));
    assert!(!first.busy);
    assert!(matches!(
        first.handle(ConnectivityInput::ProxyProbeAll)[0],
        ConnectivityOutput::ProxyProbeAll
    ));
    assert!(!first.busy);
    assert!(matches!(
        first.handle(ConnectivityInput::ProxyDirect)[0],
        ConnectivityOutput::ProxyDirect
    ));
    assert!(first.busy);

    let mut empty = page();
    assert!(empty.handle(ConnectivityInput::ProxyProbeAll).is_empty());
}

#[test]
fn connectivity_data_inputs_and_refresh_are_idempotent() {
    let mut page = page();
    page.handle(ConnectivityInput::SetI18n(Rc::new(
        I18n::new("ru").unwrap(),
    )));
    page.handle(ConnectivityInput::SetBusy(true));
    assert!(page.busy);
    page.handle(ConnectivityInput::SetError(Some("boom".into())));
    assert!(!page.busy);
    assert_eq!(page.error.as_deref(), Some("boom"));
    page.handle(ConnectivityInput::SetError(None));
    assert!(matches!(
        page.handle(ConnectivityInput::Refresh)[0],
        ConnectivityOutput::Refresh
    ));
    assert!(page.busy);
    assert!(page.handle(ConnectivityInput::Refresh).is_empty());
    let _ = local_notification_policy_path();
}

#[test]
fn connectivity_labels_exist_in_every_supported_locale() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "feature-auth-auth_connection",
            "desktop-back",
            "desktop-cancel",
            "desktop-loading",
            "desktop-error-generic",
            "desktop-notifications",
            "feature-settings-settings_proxy",
            "feature-settings-settings_proxy_sub",
            "feature-settings-settings_proxy_add",
            "feature-settings-settings_proxy_add_clipboard",
            "feature-settings-settings_proxy_check_all",
            "feature-settings-settings_proxy_direct_title",
            "feature-settings-settings_proxy_empty",
            "feature-settings-settings_proxy_active",
            "feature-settings-settings_proxy_edit",
            "feature-settings-settings_proxy_remove",
            "feature-settings-settings_proxy_remove_title",
            "feature-settings-settings_proxy_save",
            "feature-settings-settings_proxy_test",
            "feature-settings-settings_proxy_test_failed",
            "feature-settings-settings_proxy_not_checked",
            "feature-settings-settings_proxy_type",
            "feature-settings-settings_proxy_type_socks5",
            "feature-settings-settings_proxy_type_http",
            "feature-settings-settings_proxy_type_https",
            "feature-settings-settings_proxy_type_mtproto",
            "feature-settings-settings_proxy_host_hint",
            "feature-settings-settings_proxy_port",
            "feature-settings-settings_proxy_username",
            "feature-settings-settings_proxy_password",
            "feature-settings-settings_proxy_secret",
            "feature-settings-settings_proxy_clipboard_invalid",
            "feature-settings-settings_notifications",
            "feature-settings-settings_notifications_sub",
            "feature-settings-settings_notifications_preview",
            "feature-settings-settings_notifications_private",
            "feature-settings-settings_notifications_groups",
            "feature-settings-settings_notifications_channels",
            "feature-settings-settings_notifications_chat_mentions",
            "feature-settings-settings_notifications_exceptions_empty",
            "feature-settings-settings_notifications_reset",
            "feature-settings-settings_devices",
            "feature-settings-settings_devices_sub",
            "feature-settings-settings_devices_empty",
            "feature-settings-settings_devices_current_title",
            "feature-settings-settings_devices_terminate",
            "feature-settings-settings_devices_terminate_confirm",
            "feature-settings-settings_devices_terminate_action",
            "feature-settings-settings_devices_link",
            "feature-settings-settings_devices_refresh",
            "feature-settings-settings_devices_password_pending",
            "feature-settings-settings_devices_expired",
            "feature-settings-settings_devices_invalid",
            "feature-settings-settings_devices_linked",
        ] {
            let value = if key.ends_with("_ping_result") || key.ends_with("_last_active") {
                i18n.text(key, &[("arg1", "42")]).unwrap()
            } else {
                i18n.text(key, &[]).unwrap()
            };
            assert!(!value.is_empty(), "{locale}: {key}");
        }
    }
}
