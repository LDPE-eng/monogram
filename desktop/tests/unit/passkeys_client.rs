use super::*;

#[test]
fn passkey_errors_map_to_client_errors() {
    use crate::domain::ClientError;

    assert_eq!(
        passkey_error(pk::PasskeyError::Cancelled),
        ClientError::Cancelled
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::NoDevice),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::DeviceBusy),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::NotSupported),
        ClientError::UnsupportedContent
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::InvalidOptions),
        ClientError::Protocol
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::InvalidChallenge),
        ClientError::Protocol
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::InvalidCredential),
        ClientError::Protocol
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::Crypto),
        ClientError::Protocol
    );
    assert_eq!(
        passkey_error(pk::PasskeyError::Protocol("boom".into())),
        ClientError::Protocol
    );
}

#[test]
fn credential_maps_every_mono_field() {
    let credential = pk::PasskeyCredential {
        id: "id".into(),
        raw_id: "raw".into(),
        client_data: "{}".into(),
        authenticator_data: vec![1],
        signature: vec![2],
        user_handle: "2:1".into(),
        attestation_data: vec![3],
    };
    let dto = credential.into_dto();
    assert_eq!(dto.id, "id");
    assert_eq!(dto.raw_id, "raw");
    assert_eq!(dto.client_data, "{}");
    assert_eq!(dto.authenticator_data, vec![1]);
    assert_eq!(dto.signature, vec![2]);
    assert_eq!(dto.user_handle, "2:1");
    assert_eq!(dto.attestation_data, vec![3]);
}

#[test]
fn command_and_response_carry_plain_telegram_types() {
    let command = PasskeyCommand::FinishLogin {
        options: LoginOptions {
            options_json: "{}".into(),
            dc_id: 2,
            auth_key_id: 7,
        },
    };
    assert!(matches!(
        command,
        PasskeyCommand::FinishLogin { options }
            if options.dc_id == 2 && options.auth_key_id == 7 && options.options_json == "{}"
    ));
    assert!(matches!(
        PasskeyResponse::SignedIn {
            user_id: 1,
            dc_id: 2
        },
        PasskeyResponse::SignedIn {
            user_id: 1,
            dc_id: 2
        }
    ));
}

#[test]
fn request_cancellation_reaches_the_authenticator() {
    struct FakeAuthenticator {
        cancelled: std::sync::atomic::AtomicBool,
    }

    impl pk::PasskeyAuthenticator for FakeAuthenticator {
        fn make_credential(
            &self,
            _options: &pk::CreationOptions,
            _client_data_hash: &[u8; 32],
        ) -> Result<pk::RegistrationResult, pk::PasskeyError> {
            Err(pk::PasskeyError::NotSupported)
        }

        fn get_assertion(
            &self,
            _options: &pk::RequestOptions,
            _client_data_hash: &[u8; 32],
        ) -> Result<pk::AssertionResult, pk::PasskeyError> {
            Err(pk::PasskeyError::NotSupported)
        }

        fn cancel(&self) -> Result<(), pk::PasskeyError> {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            Ok(())
        }
    }

    let fake = std::sync::Arc::new(FakeAuthenticator {
        cancelled: std::sync::atomic::AtomicBool::new(false),
    });
    let authenticator: std::sync::Arc<dyn pk::PasskeyAuthenticator> = fake.clone();
    let request_id = request_control::create();
    let previous = request_control::bind(request_id);
    let result = with_cancellation(&authenticator, || {
        request_control::cancel(request_id);
        for _ in 0..100 {
            if fake.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(pk::PasskeyError::Protocol(
            "authenticator cancellation was not called".into(),
        ))
    });
    request_control::bind(previous);
    request_control::release(request_id);

    assert!(fake.cancelled.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(result, Err(pk::PasskeyError::Cancelled));
}
