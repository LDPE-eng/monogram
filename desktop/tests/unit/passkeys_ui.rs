use super::*;

fn page() -> PasskeyPage {
    PasskeyPage::new(Rc::new(I18n::new("en").unwrap()))
}

fn record(id: &str) -> PasskeyRecord {
    PasskeyRecord {
        id: id.into(),
        label: format!("Passkey {id}"),
        created_at: None,
    }
}

#[test]
fn create_and_refresh_emit_worker_requests_and_block_duplicates() {
    let mut page = page();
    page.handle(PasskeyInput::SetI18n(Rc::new(I18n::new("ru").unwrap())));
    page.handle(PasskeyInput::SetBusy(true));
    assert!(page.busy);
    page.handle(PasskeyInput::SetError(Some("boom".into())));
    assert!(!page.busy);
    assert_eq!(page.error.as_deref(), Some("boom"));
    page.handle(PasskeyInput::SetError(None));
    let output = page.handle(PasskeyInput::Create);
    assert!(matches!(output, Some(PasskeyOutput::Create)));
    assert!(page.busy);
    assert!(page.handle(PasskeyInput::Create).is_none());
    assert!(page.handle(PasskeyInput::Refresh).is_none());

    page.handle(PasskeyInput::SetPasskeys(vec![record("a")]));
    assert!(!page.busy);
    assert!(matches!(
        page.handle(PasskeyInput::Refresh),
        Some(PasskeyOutput::Refresh)
    ));
}

#[test]
fn unsupported_provider_cannot_create_passkeys() {
    let mut page = page();
    page.handle(PasskeyInput::SetSupported(false));
    assert!(page.handle(PasskeyInput::Create).is_none());
    assert!(!page.busy);
    page.handle(PasskeyInput::SetSupported(true));
    assert!(matches!(
        page.handle(PasskeyInput::Create),
        Some(PasskeyOutput::Create)
    ));
}

#[test]
fn delete_requires_confirmation_and_ignores_unknown_ids() {
    let mut page = page();
    page.handle(PasskeyInput::SetPasskeys(vec![record("a"), record("b")]));
    assert!(
        page.handle(PasskeyInput::DeleteRequest("missing".into()))
            .is_none()
    );
    page.handle(PasskeyInput::DeleteRequest("a".into()));
    assert_eq!(page.confirming_delete.as_deref(), Some("a"));
    page.handle(PasskeyInput::DeleteCancel);
    assert!(page.confirming_delete.is_none());
    page.handle(PasskeyInput::DeleteRequest("a".into()));
    assert!(matches!(
        page.handle(PasskeyInput::DeleteConfirm),
        Some(PasskeyOutput::Delete(id)) if id == "a"
    ));
    assert!(page.busy);
    page.handle(PasskeyInput::SetPasskeys(vec![record("b")]));
    assert!(!page.busy);
    assert!(page.confirming_delete.is_none());
}

#[test]
fn passkey_labels_exist_in_every_supported_locale() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "feature-settings-settings_passkeys",
            "feature-settings-settings_passkeys_hint",
            "feature-settings-settings_passkeys_create",
            "feature-settings-settings_passkeys_delete",
            "feature-settings-settings_passkeys_empty",
            "desktop-back",
            "desktop-cancel",
            "desktop-loading",
        ] {
            assert!(!i18n.text(key, &[]).unwrap().is_empty(), "{locale}: {key}");
        }
    }
}
