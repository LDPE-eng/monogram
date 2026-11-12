use super::*;

fn challenge(value: &str) -> Challenge {
    Challenge::parse(value).unwrap()
}

fn login_fixture() -> String {
    r#"{"publicKey":{"challenge":"aGVsbG8","rpId":"telegram.org","timeout":60000,"userVerification":"required","allowCredentials":[{"type":"public-key","id":"Y3JlZGVudGlhbA","transports":["usb","nfc"]}]}}"#.to_owned()
}

fn creation_fixture() -> String {
    r#"{"publicKey":{"challenge":"aGVsbG8","rp":{"id":"telegram.org","name":"Telegram"},"user":{"id":"aGVsbG8","name":"user","displayName":"User"},"timeout":60000,"authenticatorSelection":{"residentKey":"required","userVerification":"required"}}}"#.to_owned()
}

#[test]
fn client_data_json_is_well_formed_and_fixed_to_telegram_origin() {
    for kind in [ClientDataKind::Create, ClientDataKind::Get] {
        let json = client_data_json(kind, &challenge("aGVsbG8")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], kind.as_str());
        assert_eq!(value["challenge"], "aGVsbG8");
        assert_eq!(value["origin"], "https://telegram.org");
        assert_eq!(value["crossOrigin"], false);
        assert!(json.len() < MAX_CLIENT_DATA_BYTES);
    }
    assert_eq!(
        client_data_json(
            ClientDataKind::Get,
            &Challenge {
                encoded: String::new(),
                bytes: Vec::new(),
            }
        ),
        Err(PasskeyError::InvalidChallenge)
    );
}

#[test]
fn client_data_hash_matches_sha256_reference_vectors() {
    let expected: [u8; 32] = [
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22,
        0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00,
        0x15, 0xad,
    ];
    assert_eq!(client_data_hash("abc").unwrap(), expected);
    let empty: [u8; 32] = [
        0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f, 0xb9,
        0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b, 0x78, 0x52,
        0xb8, 0x55,
    ];
    assert_eq!(client_data_hash("").unwrap(), empty);
}

#[test]
fn request_options_parse_challenge_rp_and_allow_list() {
    let options = parse_request_options(&login_fixture()).unwrap();
    assert_eq!(options.rp_id, "telegram.org");
    assert_eq!(options.challenge.encoded(), "aGVsbG8");
    assert_eq!(options.challenge.bytes(), b"hello");
    assert_eq!(options.timeout_ms, Some(60_000));
    assert_eq!(options.user_verification, UserVerification::Required);
    assert_eq!(options.allow_credentials.len(), 1);
    assert_eq!(options.allow_credentials[0].id_b64url, "Y3JlZGVudGlhbA");
    assert_eq!(options.allow_credentials[0].id, b"credential");
    assert_eq!(
        options.allow_credentials[0].transports,
        vec![Transport::Usb, Transport::Nfc]
    );
}

#[test]
fn request_options_reject_foreign_rp_and_missing_challenge() {
    let foreign = login_fixture().replace("telegram.org", "example.org");
    assert_eq!(
        parse_request_options(&foreign),
        Err(PasskeyError::InvalidOptions)
    );
    let no_challenge = r#"{"publicKey":{"rpId":"telegram.org"}}"#;
    assert_eq!(
        parse_request_options(no_challenge),
        Err(PasskeyError::InvalidChallenge)
    );
}

#[test]
fn creation_options_parse_rp_user_and_resident_key() {
    let options = parse_creation_options(&creation_fixture()).unwrap();
    assert_eq!(options.rp_id, "telegram.org");
    assert_eq!(options.challenge.bytes(), b"hello");
    assert_eq!(options.user.id, b"hello");
    assert_eq!(options.user.name, "user");
    assert_eq!(options.user.display_name, "User");
    assert!(options.resident_key_required);
    assert_eq!(options.user_verification, UserVerification::Required);
}

#[test]
fn creation_options_reject_foreign_rp_and_invalid_user() {
    let foreign = creation_fixture().replace("telegram.org", "example.org");
    assert_eq!(
        parse_creation_options(&foreign),
        Err(PasskeyError::InvalidOptions)
    );
    let no_user = r#"{"publicKey":{"challenge":"aGVsbG8","rp":{"id":"telegram.org"}}}"#;
    assert_eq!(
        parse_creation_options(no_user),
        Err(PasskeyError::InvalidOptions)
    );
}

#[test]
fn registration_credential_maps_id_and_attestation_and_rejects_bad_client_data() {
    let options = parse_creation_options(&creation_fixture()).unwrap();
    let client_data = client_data_json(ClientDataKind::Create, &options.challenge).unwrap();
    let result = RegistrationResult {
        credential_id: b"credential".to_vec(),
        attestation_object: vec![0xa1, 0x02, 0x03],
    };
    let credential = registration_credential(&client_data, &options, &result).unwrap();
    assert_eq!(credential.id, "Y3JlZGVudGlhbA");
    assert_eq!(credential.raw_id, "Y3JlZGVudGlhbA");
    assert_eq!(credential.client_data, client_data);
    assert_eq!(credential.attestation_data, vec![0xa1, 0x02, 0x03]);
    assert!(credential.authenticator_data.is_empty());
    assert!(credential.signature.is_empty());
    assert!(credential.user_handle.is_empty());

    let wrong_origin = client_data.replace("https://telegram.org", "https://example.org");
    assert_eq!(
        registration_credential(&wrong_origin, &options, &result),
        Err(PasskeyError::InvalidCredential)
    );

    let empty_attestation = RegistrationResult {
        credential_id: b"credential".to_vec(),
        attestation_object: Vec::new(),
    };
    assert_eq!(
        registration_credential(&client_data, &options, &empty_attestation),
        Err(PasskeyError::InvalidCredential)
    );
}

#[test]
fn assertion_credential_maps_signature_and_validates_user_handle() {
    let options = parse_request_options(&login_fixture()).unwrap();
    let client_data = client_data_json(ClientDataKind::Get, &options.challenge).unwrap();
    let result = AssertionResult {
        credential_id: b"credential".to_vec(),
        authenticator_data: vec![0; 37],
        signature: vec![1, 2, 3],
        user_handle: b"2:123".to_vec(),
    };
    let credential = assertion_credential(&client_data, &options, &result).unwrap();
    assert_eq!(credential.id, "Y3JlZGVudGlhbA");
    assert_eq!(credential.raw_id, "Y3JlZGVudGlhbA");
    assert_eq!(credential.client_data, client_data);
    assert_eq!(credential.authenticator_data, vec![0; 37]);
    assert_eq!(credential.signature, vec![1, 2, 3]);
    assert_eq!(credential.user_handle, "2:123");
    assert!(credential.attestation_data.is_empty());

    for bad in [
        b"0:1".to_vec(),
        b"6:1".to_vec(),
        b"2:0".to_vec(),
        b"nope".to_vec(),
    ] {
        let mut invalid = result.clone();
        invalid.user_handle = bad;
        assert_eq!(
            assertion_credential(&client_data, &options, &invalid),
            Err(PasskeyError::InvalidCredential)
        );
    }

    let mut short_authdata = result;
    short_authdata.authenticator_data = vec![0; 36];
    assert_eq!(
        assertion_credential(&client_data, &options, &short_authdata),
        Err(PasskeyError::InvalidCredential)
    );
}

#[test]
fn base64url_round_trips_and_matches_reference() {
    assert_eq!(encode_b64url(b"hello"), "aGVsbG8");
    assert_eq!(encode_b64url(&[0xfb, 0xff]), "-_8");
    assert_eq!(decode_b64url("aGVsbG8").unwrap(), b"hello");
    assert_eq!(decode_b64url("-_8").unwrap(), vec![0xfb, 0xff]);
    assert!(decode_b64url("not!base64").is_err());
}

#[test]
fn user_handle_accepts_only_valid_dc_user_routes() {
    assert_eq!(
        validate_user_handle("2:1234567890123").unwrap(),
        (2, 1234567890123)
    );
    for handle in [
        "",
        "0:1",
        "6:1",
        "2:0",
        "2:-1",
        "2:1:3",
        "2:9223372036854775808",
    ] {
        assert_eq!(
            validate_user_handle(handle),
            Err(PasskeyError::InvalidCredential),
            "{handle:?}"
        );
    }
}

#[test]
fn validate_client_data_enforces_operation_origin_and_challenge() {
    let challenge = challenge("aGVsbG8");
    let json = client_data_json(ClientDataKind::Get, &challenge).unwrap();
    assert!(validate_client_data(&json, ClientDataKind::Get, &challenge).is_ok());
    assert_eq!(
        validate_client_data(&json, ClientDataKind::Create, &challenge),
        Err(PasskeyError::InvalidCredential)
    );
    assert_eq!(
        validate_client_data(
            &json.replace("aGVsbG8", "b3RoZXI"),
            ClientDataKind::Get,
            &challenge
        ),
        Err(PasskeyError::InvalidCredential)
    );
    let oversized = format!(
        r#"{{"type":"webauthn.get","challenge":"aGVsbG8","origin":"https://telegram.org","pad":"{}"}}"#,
        "x".repeat(MAX_CLIENT_DATA_BYTES)
    );
    assert_eq!(
        validate_client_data(&oversized, ClientDataKind::Get, &challenge),
        Err(PasskeyError::InvalidCredential)
    );
}

fn sample_auth_data() -> &'static [u8] {
    &[
        0x58, 0xa2, 0xc2, 0x89, 0xc5, 0xca, 0x9b, 0x04, 0x60, 0xf9, 0x34, 0x6a, 0xb4, 0xe4, 0x2d,
        0x84, 0x27, 0x43, 0x40, 0x4d, 0x31, 0xf4, 0x84, 0x68, 0x25, 0xa6, 0xd0, 0x65, 0xbe, 0x59,
        0x7a, 0x87, 0x05, 0x1d, 0xc1, 0x00, 0x00, 0x00, 0x0b, 0xf8, 0xa0, 0x11, 0xf3, 0x8c, 0x0a,
        0x4d, 0x15, 0x80, 0x06, 0x17, 0x11, 0x1f, 0x9e, 0xdc, 0x7d, 0x00, 0x10, 0x89, 0x59, 0xce,
        0xad, 0x5b, 0x5c, 0x48, 0x16, 0x4e, 0x8a, 0xbc, 0xd6, 0xd9, 0x43, 0x5c, 0x6f, 0xa5, 0x01,
        0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20, 0xa5, 0xfd, 0x5c, 0xe1, 0xb1, 0xc4, 0x58,
        0xc5, 0x30, 0xa5, 0x4f, 0xa6, 0x1b, 0x31, 0xbf, 0x6b, 0x04, 0xbe, 0x8b, 0x97, 0xaf, 0xde,
        0x54, 0xdd, 0x8c, 0xbb, 0x69, 0x27, 0x5a, 0x8a, 0x1b, 0xe1, 0x22, 0x58, 0x20, 0xfa, 0x3a,
        0x32, 0x31, 0xdd, 0x9d, 0xee, 0xd9, 0xd1, 0x89, 0x7b, 0xe5, 0xa6, 0x22, 0x8c, 0x59, 0x50,
        0x1e, 0x4b, 0xcd, 0x12, 0x97, 0x5d, 0x3d, 0xff, 0x73, 0x0f, 0x01, 0x27, 0x8e, 0xa6, 0x1c,
        0xa1, 0x6b, 0x68, 0x6d, 0x61, 0x63, 0x2d, 0x73, 0x65, 0x63, 0x72, 0x65, 0x74, 0xf5,
    ]
}

#[test]
fn ctap_args_preserve_hash_rp_user_allow_list_and_options() {
    use authenticator::ctap2::server::{
        ResidentKeyRequirement, Transport as CtapTransport, UserVerificationRequirement,
    };

    let hash = [0x42; 32];
    let creation = parse_creation_options(&creation_fixture()).unwrap();
    let register = fido2::register_args(&creation, hash);
    assert_eq!(register.client_data_hash, hash);
    assert_eq!(register.relying_party.id, "telegram.org");
    assert_eq!(register.origin, ORIGIN);
    assert_eq!(register.user.id, b"hello");
    assert_eq!(register.user.name.as_deref(), Some("user"));
    assert_eq!(register.user.display_name.as_deref(), Some("User"));
    assert_eq!(
        register.user_verification_req,
        UserVerificationRequirement::Required
    );
    assert_eq!(register.resident_key_req, ResidentKeyRequirement::Required);
    assert!(!register.use_ctap1_fallback);

    let request = parse_request_options(&login_fixture()).unwrap();
    let sign = fido2::sign_args(&request, hash);
    assert_eq!(sign.client_data_hash, hash);
    assert_eq!(sign.relying_party_id, "telegram.org");
    assert_eq!(sign.allow_list.len(), 1);
    assert_eq!(sign.allow_list[0].id, b"credential");
    assert_eq!(
        sign.allow_list[0].transports,
        vec![CtapTransport::USB, CtapTransport::NFC]
    );
    assert_eq!(
        sign.user_verification_req,
        UserVerificationRequirement::Required
    );
    assert!(sign.user_presence_req);
    assert!(!sign.use_ctap1_fallback);
    assert_eq!(fido2::timeout(None), 60_000);
    assert_eq!(fido2::timeout(Some(5_000)), 5_000);
}

#[test]
fn ctap_errors_map_to_the_passkey_contract() {
    use authenticator::ctap2::commands::{CommandError, StatusCode};
    use authenticator::errors::{AuthenticatorError, HIDError, U2FTokenError, UnsupportedOption};

    let status = |status| {
        AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(status, None)))
    };
    assert_eq!(
        fido2::map_error(status(StatusCode::ChannelBusy)),
        PasskeyError::DeviceBusy
    );
    assert_eq!(
        fido2::map_error(status(StatusCode::KeepaliveCancel)),
        PasskeyError::Cancelled
    );
    assert_eq!(
        fido2::map_error(status(StatusCode::OperationDenied)),
        PasskeyError::Cancelled
    );
    assert_eq!(
        fido2::map_error(status(StatusCode::NoCredentials)),
        PasskeyError::InvalidCredential
    );
    assert_eq!(
        fido2::map_error(status(StatusCode::UnsupportedAlgorithm)),
        PasskeyError::NotSupported
    );
    assert_eq!(
        fido2::map_error(AuthenticatorError::UnsupportedOption(
            UnsupportedOption::ResidentKey,
        )),
        PasskeyError::NotSupported
    );
    assert_eq!(
        fido2::map_error(AuthenticatorError::CancelledByUser),
        PasskeyError::Cancelled
    );
    assert_eq!(
        fido2::map_error(AuthenticatorError::U2FToken(U2FTokenError::NotAllowed)),
        PasskeyError::NoDevice
    );
    assert_eq!(
        fido2::map_error_with_device(
            AuthenticatorError::U2FToken(U2FTokenError::NotAllowed),
            true,
        ),
        PasskeyError::Protocol("FIDO2 operation timed out".into())
    );
    assert_eq!(
        fido2::map_error(AuthenticatorError::HIDError(HIDError::IO(
            None,
            std::io::Error::new(std::io::ErrorKind::ResourceBusy, "busy"),
        ))),
        PasskeyError::DeviceBusy
    );
}

#[test]
fn registration_uses_credential_id_and_cbor_attestation_fixture() {
    let attestation = authenticator::AttestationObject {
        auth_data: serde_cbor::from_slice(sample_auth_data()).unwrap(),
        att_stmt: authenticator::ctap2::attestation::AttestationStatement::None,
    };
    let result = fido2::registration_from_parts(attestation).unwrap();
    assert_eq!(
        result.credential_id,
        [
            0x89, 0x59, 0xce, 0xad, 0x5b, 0x5c, 0x48, 0x16, 0x4e, 0x8a, 0xbc, 0xd6, 0xd9, 0x43,
            0x5c, 0x6f,
        ]
    );
    let serde_cbor::Value::Map(entries) =
        serde_cbor::from_slice(&result.attestation_object).unwrap()
    else {
        panic!("attestation object is not a CBOR map");
    };
    assert!(entries.iter().any(|(key, value)| {
        key == &serde_cbor::Value::Text("fmt".into())
            && value == &serde_cbor::Value::Text("none".into())
    }));
    assert!(entries.iter().any(|(key, value)| {
        key == &serde_cbor::Value::Text("authData".into())
            && value == &serde_cbor::Value::Bytes(sample_auth_data()[2..].to_vec())
    }));

    let options = parse_creation_options(&creation_fixture()).unwrap();
    let client_data = client_data_json(ClientDataKind::Create, &options.challenge).unwrap();
    let credential = registration_credential(&client_data, &options, &result).unwrap();
    assert_eq!(credential.attestation_data, result.attestation_object);
}

#[test]
fn assertion_extracts_raw_auth_data_and_ctap21_user_handle() {
    use authenticator::ctap2::server::{AuthenticatorAttachment, PublicKeyCredentialUserEntity};

    let options = parse_request_options(&login_fixture()).unwrap();
    let result = authenticator::GetAssertionResult {
        assertion: authenticator::Assertion {
            credentials: None,
            auth_data: serde_cbor::from_slice(sample_auth_data()).unwrap(),
            signature: vec![0x30, 0x02, 0x01, 0x01],
            user: Some(PublicKeyCredentialUserEntity {
                id: b"2:123".to_vec(),
                name: Some("user".into()),
                display_name: None,
            }),
        },
        attachment: AuthenticatorAttachment::CrossPlatform,
        extensions: Default::default(),
    };
    let assertion = fido2::assertion_from(result, &options).unwrap();
    assert_eq!(assertion.credential_id, b"credential");
    assert_eq!(assertion.authenticator_data, sample_auth_data()[2..]);
    assert_eq!(assertion.signature, [0x30, 0x02, 0x01, 0x01]);
    assert_eq!(assertion.user_handle, b"2:123");

    let client_data = client_data_json(ClientDataKind::Get, &options.challenge).unwrap();
    let credential = assertion_credential(&client_data, &options, &assertion).unwrap();
    assert_eq!(credential.user_handle, "2:123");
    assert_eq!(credential.authenticator_data, sample_auth_data()[2..]);
}
