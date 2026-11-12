#[path = "../src/i18n.rs"]
mod i18n;

use i18n::{I18n, I18nError, SUPPORTED_LOCALES};

type Vector = (&'static str, &'static str, Option<u64>, &'static str);
const VECTORS: &[Vector] = include!(env!("MONOGRAM_I18N_VECTORS"));
const ARGS: [(&str, &str); 5] = [
    ("arg1", "A1{ $arg2 }<>&\"\\"),
    ("arg2", "A2"),
    ("arg3", "A3"),
    ("arg4", "A4"),
    ("arg5", "A5"),
];

#[test]
fn every_android_and_desktop_string_and_integer_plural_matches_runtime() {
    let mut checked = 0;
    for locale in SUPPORTED_LOCALES {
        let catalog = I18n::new(locale).unwrap();
        assert_eq!(catalog.locale(), locale);
        for &(language, key, count, expected) in VECTORS {
            if language != locale {
                continue;
            }
            let actual = match count {
                Some(count) => catalog.plural(key, count, &ARGS),
                None => catalog.text(key, &ARGS),
            };
            assert_eq!(actual.unwrap(), expected, "{locale}:{key}:{count:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, VECTORS.len());
    assert_eq!(checked, 19300);
}

#[test]
fn locale_negotiation_is_explicit() {
    for (requested, expected) in [
        ("ru_RU.UTF-8", "ru"),
        ("pt_BR.UTF-8", "pt-BR"),
        ("pt-BR", "pt-BR"),
        ("pt", "pt-BR"),
        ("zh-Hans-CN", "zh"),
        ("uk_UA@variant", "uk"),
        ("ja-JP", "ja"),
        ("de_DE", "en"),
        ("pt-PT", "en"),
        ("C", "en"),
        ("", "en"),
    ] {
        assert_eq!(I18n::new(requested).unwrap().locale(), expected);
    }
}

#[test]
fn errors_are_not_silently_displayed_as_translations() {
    let catalog = I18n::new("en").unwrap();
    assert_eq!(
        catalog.text("not-a-resource", &[]),
        Err(I18nError::UnknownKey("not-a-resource".into()))
    );
    assert_eq!(
        catalog.text("core-ui-status_flood_wait", &[]),
        Err(I18nError::MissingArgument("arg1".into()))
    );
    assert!(matches!(
        catalog.text("feature-chats-chats_unread_count", &[]),
        Err(I18nError::WrongKind(_))
    ));
    assert!(matches!(
        catalog.plural("core-ui-status_syncing", 1, &[]),
        Err(I18nError::WrongKind(_))
    ));
    assert!(matches!(
        catalog.plural("not-a-resource", 1, &[]),
        Err(I18nError::UnknownKey(_))
    ));
    assert_eq!(catalog.text("core-ui-service_empty", &[]).unwrap(), "");
}

#[test]
fn quantity_does_not_replace_the_printf_argument() {
    let catalog = I18n::new("en").unwrap();
    assert_eq!(
        catalog
            .plural("feature-chats-chats_unread_count", 1, &[("arg1", "999")])
            .unwrap(),
        "999 unread message"
    );
    assert_eq!(
        catalog
            .plural("feature-chats-chats_unread_count", 2, &[("arg1", "999")])
            .unwrap(),
        "999 unread messages"
    );
}
