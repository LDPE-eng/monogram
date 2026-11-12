use super::*;

#[test]
fn api_requires_positive_id_and_exact_hex_hash() {
    assert!(valid_api(1, &"aF09".repeat(8)));
    assert!(!valid_api(0, &"a".repeat(32)));
    assert!(!valid_api(-1, &"a".repeat(32)));
    assert!(!valid_api(1, &"g".repeat(32)));
    assert!(!valid_api(1, &"a".repeat(31)));
    assert!(!valid_api(1, &format!(" {}", "a".repeat(32))));
}

#[test]
fn phone_normalization_rejects_extensions_and_embedded_plus() {
    assert_eq!(
        normalize_phone(" +49 (151) 123-4567 ").as_deref(),
        Some("+491511234567")
    );
    assert_eq!(
        normalize_phone("49 151 1234567").as_deref(),
        Some("+491511234567")
    );
    for phone in [
        "",
        "+123",
        "+1234567890123456",
        "+49+1234567",
        "123456 ext 2",
        "１２３４５",
    ] {
        assert!(normalize_phone(phone).is_none(), "{phone}");
    }
}

#[test]
fn code_validation_respects_server_length_and_word_types() {
    assert!(valid_code("123456", "app", 6));
    assert!(!valid_code("12345", "sms", 6));
    assert!(!valid_code("12345x", "email", 6));
    assert!(valid_code("a phrase", "sms_phrase", 5));
    assert!(valid_code("word", "sms_word", 5));
    assert!(!valid_code("", "sms_word", 5));
}

#[test]
fn auth_debug_never_exposes_credentials_or_qr_tokens() {
    let secret = "sensitive credential";
    let messages = [
        AuthOutput::Configure {
            api_id: 1,
            api_hash: secret.into(),
        },
        AuthOutput::SendCode(secret.into()),
        AuthOutput::SignIn(secret.into()),
        AuthOutput::CheckPassword(secret.into()),
    ];
    for message in messages {
        assert!(!format!("{message:?}").contains(secret));
    }
    let input = AuthInput::QrToken {
        token: secret.as_bytes().to_vec(),
        expires: 123,
    };
    assert_eq!(format!("{input:?}"), "AuthInput::QrToken(..)");
}

#[test]
fn qr_is_real_rgb_matrix_with_four_module_quiet_zone() {
    let token = [251, 255, 254, 1];
    assert_eq!(URL_SAFE_NO_PAD.encode(token), "-__-AQ");
    let (pixels, size) = qr_pixels(&token).unwrap();
    assert_eq!(pixels.len(), size * size * 3);
    assert!(pixels[..24 * size * 3].iter().all(|value| *value == 255));
    assert_eq!(
        &pixels[(24 * size + 24) * 3..(24 * size + 24) * 3 + 3],
        &[0, 0, 0]
    );
    assert!(qr_pixels(&[]).is_none());
}
