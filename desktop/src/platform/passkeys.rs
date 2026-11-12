//! Linux WebAuthn / CTAP2 provider for Telegram passkeys.
//!
//! This module implements the client side of WebAuthn for `telegram.org`:
//! client data JSON construction, SHA-256 hashing, base64url encoding,
//! Telegram passkey option parsing, USB HID authenticator operations, and
//! mapping authenticator results into the credential shape consumed by
//! `mono-proto-client`.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use thiserror::Error;

pub const RP_ID: &str = "telegram.org";
pub const ORIGIN: &str = "https://telegram.org";

const MAX_CLIENT_DATA_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasskeyError {
    #[error("no FIDO2 authenticator is available")]
    NoDevice,
    #[error("the FIDO2 authenticator is busy")]
    DeviceBusy,
    #[error("the passkey operation was cancelled")]
    Cancelled,
    #[error("the FIDO2 authenticator does not support the requested options")]
    NotSupported,
    #[error("passkey options are malformed")]
    InvalidOptions,
    #[error("the passkey challenge is missing or invalid")]
    InvalidChallenge,
    #[error("the authenticator returned an invalid credential")]
    InvalidCredential,
    #[error("SHA-256 is unavailable")]
    Crypto,
    #[error("the FIDO2 transport failed: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDataKind {
    Create,
    Get,
}

impl ClientDataKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "webauthn.create",
            Self::Get => "webauthn.get",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserVerification {
    Required,
    Preferred,
    #[default]
    Discouraged,
}

impl UserVerification {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("required") => Self::Required,
            Some("preferred") => Self::Preferred,
            _ => Self::Discouraged,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Nfc,
    Internal,
    Hybrid,
}

impl Transport {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "usb" => Some(Self::Usb),
            "nfc" => Some(Self::Nfc),
            "internal" => Some(Self::Internal),
            "hybrid" | "cable" | "ble" => Some(Self::Hybrid),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    encoded: String,
    bytes: Vec<u8>,
}

impl Challenge {
    fn parse(encoded: &str) -> Result<Self, PasskeyError> {
        if encoded.is_empty() {
            return Err(PasskeyError::InvalidChallenge);
        }
        let bytes = decode_b64url(encoded).map_err(|_| PasskeyError::InvalidChallenge)?;
        Ok(Self {
            encoded: encoded.to_owned(),
            bytes,
        })
    }

    pub fn encoded(&self) -> &str {
        &self.encoded
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowCredential {
    pub id: Vec<u8>,
    pub id_b64url: String,
    pub transports: Vec<Transport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeyUser {
    pub id: Vec<u8>,
    pub name: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOptions {
    pub rp_id: String,
    pub challenge: Challenge,
    pub allow_credentials: Vec<AllowCredential>,
    pub timeout_ms: Option<u64>,
    pub user_verification: UserVerification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationOptions {
    pub rp_id: String,
    pub challenge: Challenge,
    pub user: PublicKeyUser,
    pub timeout_ms: Option<u64>,
    pub user_verification: UserVerification,
    pub resident_key_required: bool,
}

#[derive(Debug, Clone)]
pub struct RegistrationResult {
    pub credential_id: Vec<u8>,
    pub attestation_object: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct AssertionResult {
    pub credential_id: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub user_handle: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyInfo {
    pub id: String,
    pub name: String,
    pub date: i32,
    pub software_emoji_id: Option<i64>,
    pub last_usage_date: Option<i32>,
}

/// Credential in exactly the field layout `mono-proto-client` expects in
/// `PasskeyCredentialDto`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyCredential {
    pub id: String,
    pub raw_id: String,
    pub client_data: String,
    pub authenticator_data: Vec<u8>,
    pub signature: Vec<u8>,
    pub user_handle: String,
    pub attestation_data: Vec<u8>,
}

pub trait PasskeyAuthenticator: Send + Sync {
    fn make_credential(
        &self,
        options: &CreationOptions,
        client_data_hash: &[u8; 32],
    ) -> Result<RegistrationResult, PasskeyError>;

    fn get_assertion(
        &self,
        options: &RequestOptions,
        client_data_hash: &[u8; 32],
    ) -> Result<AssertionResult, PasskeyError>;

    fn cancel(&self) -> Result<(), PasskeyError>;
}

pub fn system_authenticator() -> Result<std::sync::Arc<dyn PasskeyAuthenticator>, PasskeyError> {
    fido2::Fido2Authenticator::new().map(|authenticator| {
        let authenticator: std::sync::Arc<dyn PasskeyAuthenticator> =
            std::sync::Arc::new(authenticator);
        authenticator
    })
}

pub fn encode_b64url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode_b64url(encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
    URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('='))
}

/// Builds the UTF-8 client data JSON that is later hashed and signed by the
/// authenticator. `challenge` must be the base64url challenge from the server.
pub fn client_data_json(
    kind: ClientDataKind,
    challenge: &Challenge,
) -> Result<String, PasskeyError> {
    if challenge.bytes().is_empty() {
        return Err(PasskeyError::InvalidChallenge);
    }
    Ok(serde_json::json!({
        "type": kind.as_str(),
        "challenge": challenge.encoded(),
        "origin": ORIGIN,
        "crossOrigin": false,
    })
    .to_string())
}

pub fn client_data_hash(client_data_json: &str) -> Result<[u8; 32], PasskeyError> {
    let mut checksum = relm4::gtk::glib::Checksum::new(relm4::gtk::glib::ChecksumType::Sha256)
        .ok_or(PasskeyError::Crypto)?;
    checksum.update(client_data_json.as_bytes());
    let digest = checksum.digest();
    digest.try_into().map_err(|_| PasskeyError::Crypto)
}

/// Validates that `client_data_json` is the expected WebAuthn operation for
/// telegram.org and carries the exact challenge from the server.
pub fn validate_client_data(
    client_data_json: &str,
    kind: ClientDataKind,
    challenge: &Challenge,
) -> Result<(), PasskeyError> {
    if client_data_json.len() > MAX_CLIENT_DATA_BYTES {
        return Err(PasskeyError::InvalidCredential);
    }
    let value: Value =
        serde_json::from_str(client_data_json).map_err(|_| PasskeyError::InvalidCredential)?;
    let expected = kind.as_str();
    if value.get("type").and_then(Value::as_str) != Some(expected)
        || value.get("origin").and_then(Value::as_str) != Some(ORIGIN)
        || value.get("challenge").and_then(Value::as_str) != Some(challenge.encoded())
    {
        return Err(PasskeyError::InvalidCredential);
    }
    Ok(())
}

/// Parses the `{"publicKey": ...}` login options JSON returned by
/// `auth.passkeyLoginOptions`.
pub fn parse_request_options(json: &str) -> Result<RequestOptions, PasskeyError> {
    let public_key = public_key_object(json)?;
    let challenge = Challenge::parse(
        public_key
            .get("challenge")
            .and_then(Value::as_str)
            .ok_or(PasskeyError::InvalidChallenge)?,
    )?;
    let rp_id = public_key
        .get("rpId")
        .and_then(Value::as_str)
        .unwrap_or(RP_ID)
        .to_owned();
    if rp_id != RP_ID {
        return Err(PasskeyError::InvalidOptions);
    }
    let allow_credentials = parse_allow_credentials(public_key.get("allowCredentials"))?;
    let timeout_ms = public_key.get("timeout").and_then(Value::as_u64);
    let user_verification =
        UserVerification::parse(public_key.get("userVerification").and_then(Value::as_str));
    Ok(RequestOptions {
        rp_id,
        challenge,
        allow_credentials,
        timeout_ms,
        user_verification,
    })
}

/// Parses the `{"publicKey": ...}` registration options JSON returned by
/// `account.passkeyRegistrationOptions`.
pub fn parse_creation_options(json: &str) -> Result<CreationOptions, PasskeyError> {
    let public_key = public_key_object(json)?;
    let challenge = Challenge::parse(
        public_key
            .get("challenge")
            .and_then(Value::as_str)
            .ok_or(PasskeyError::InvalidChallenge)?,
    )?;
    let rp = public_key
        .get("rp")
        .and_then(Value::as_object)
        .ok_or(PasskeyError::InvalidOptions)?;
    let rp_id = rp
        .get("id")
        .and_then(Value::as_str)
        .ok_or(PasskeyError::InvalidOptions)?;
    if rp_id != RP_ID {
        return Err(PasskeyError::InvalidOptions);
    }
    let user = public_key
        .get("user")
        .and_then(Value::as_object)
        .ok_or(PasskeyError::InvalidOptions)?;
    let user_id = decode_b64url(
        user.get("id")
            .and_then(Value::as_str)
            .ok_or(PasskeyError::InvalidOptions)?,
    )
    .map_err(|_| PasskeyError::InvalidOptions)?;
    if user_id.is_empty() {
        return Err(PasskeyError::InvalidOptions);
    }
    let timeout_ms = public_key.get("timeout").and_then(Value::as_u64);
    let selection = public_key
        .get("authenticatorSelection")
        .and_then(Value::as_object);
    let user_verification = UserVerification::parse(
        selection
            .and_then(|s| s.get("userVerification"))
            .and_then(Value::as_str)
            .or_else(|| public_key.get("userVerification").and_then(Value::as_str)),
    );
    let resident_key_required = selection
        .and_then(|s| s.get("residentKey"))
        .and_then(Value::as_str)
        == Some("required")
        || selection
            .and_then(|s| s.get("requireResidentKey"))
            .and_then(Value::as_bool)
            == Some(true);
    Ok(CreationOptions {
        rp_id: rp_id.to_owned(),
        challenge,
        user: PublicKeyUser {
            id: user_id,
            name: user
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            display_name: user
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        },
        timeout_ms,
        user_verification,
        resident_key_required,
    })
}

/// Maps a `makeCredential` result into the credential Telegram expects.
pub fn registration_credential(
    client_data_json: &str,
    options: &CreationOptions,
    result: &RegistrationResult,
) -> Result<PasskeyCredential, PasskeyError> {
    validate_client_data(client_data_json, ClientDataKind::Create, &options.challenge)?;
    if result.credential_id.is_empty() || result.attestation_object.is_empty() {
        return Err(PasskeyError::InvalidCredential);
    }
    let id = encode_b64url(&result.credential_id);
    Ok(PasskeyCredential {
        id: id.clone(),
        raw_id: id,
        client_data: client_data_json.to_owned(),
        authenticator_data: Vec::new(),
        signature: Vec::new(),
        user_handle: String::new(),
        attestation_data: result.attestation_object.clone(),
    })
}

/// Maps a `getAssertion` result into the credential Telegram expects.
pub fn assertion_credential(
    client_data_json: &str,
    options: &RequestOptions,
    result: &AssertionResult,
) -> Result<PasskeyCredential, PasskeyError> {
    validate_client_data(client_data_json, ClientDataKind::Get, &options.challenge)?;
    if result.credential_id.is_empty()
        || result.authenticator_data.len() < 37
        || result.signature.is_empty()
    {
        return Err(PasskeyError::InvalidCredential);
    }
    let user_handle =
        std::str::from_utf8(&result.user_handle).map_err(|_| PasskeyError::InvalidCredential)?;
    validate_user_handle(user_handle)?;
    let id = encode_b64url(&result.credential_id);
    Ok(PasskeyCredential {
        id: id.clone(),
        raw_id: id,
        client_data: client_data_json.to_owned(),
        authenticator_data: result.authenticator_data.clone(),
        signature: result.signature.clone(),
        user_handle: user_handle.to_owned(),
        attestation_data: Vec::new(),
    })
}

/// Validates and splits the Telegram passkey user handle (`dc_id:user_id`).
pub fn validate_user_handle(handle: &str) -> Result<(i32, i64), PasskeyError> {
    let invalid = || PasskeyError::InvalidCredential;
    let (dc, user) = handle.split_once(':').ok_or_else(invalid)?;
    let dc = dc.parse::<i32>().map_err(|_| invalid())?;
    let user = user.parse::<i64>().map_err(|_| invalid())?;
    if !(1..=5).contains(&dc) || user <= 0 {
        return Err(invalid());
    }
    Ok((dc, user))
}

fn public_key_object(json: &str) -> Result<serde_json::Map<String, Value>, PasskeyError> {
    let root: Value = serde_json::from_str(json).map_err(|_| PasskeyError::InvalidOptions)?;
    root.get("publicKey")
        .and_then(Value::as_object)
        .cloned()
        .ok_or(PasskeyError::InvalidOptions)
}

fn parse_allow_credentials(value: Option<&Value>) -> Result<Vec<AllowCredential>, PasskeyError> {
    let Some(array) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut credentials = Vec::with_capacity(array.len());
    for entry in array {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let Some(id_b64url) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Ok(id) = decode_b64url(id_b64url) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let transports = entry
            .get("transports")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .filter_map(Transport::parse)
            .collect();
        credentials.push(AllowCredential {
            id,
            id_b64url: id_b64url.to_owned(),
            transports,
        });
    }
    Ok(credentials)
}

mod fido2 {
    use super::*;
    use authenticator::authenticatorservice::{AuthenticatorService, RegisterArgs, SignArgs};
    use authenticator::crypto::COSEAlgorithm;
    use authenticator::ctap2::commands::{CommandError, StatusCode};
    use authenticator::ctap2::server::{
        AuthenticationExtensionsClientInputs, PublicKeyCredentialDescriptor,
        PublicKeyCredentialParameters, PublicKeyCredentialUserEntity, RelyingParty,
        ResidentKeyRequirement, Transport as CtapTransport, UserVerificationRequirement,
    };
    use authenticator::errors::{AuthenticatorError, HIDError, U2FTokenError};
    use authenticator::statecallback::StateCallback;
    use authenticator::{RegisterResult, SignResult, StatusUpdate};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    const DEFAULT_TIMEOUT_MS: u64 = 60_000;

    pub struct Fido2Authenticator {
        service: Mutex<AuthenticatorService>,
        operation: Mutex<()>,
        cancelled: AtomicBool,
    }

    impl Fido2Authenticator {
        pub fn new() -> Result<Self, PasskeyError> {
            let mut service = AuthenticatorService::new().map_err(map_error)?;
            service.add_u2f_usb_hid_platform_transports();
            Ok(Self {
                service: Mutex::new(service),
                operation: Mutex::new(()),
                cancelled: AtomicBool::new(false),
            })
        }

        fn run<T, F>(&self, start: F) -> Result<T, PasskeyError>
        where
            T: Send + 'static,
            F: FnOnce(
                &mut AuthenticatorService,
                mpsc::Sender<StatusUpdate>,
                StateCallback<authenticator::Result<T>>,
            ) -> authenticator::Result<()>,
        {
            let _operation = self
                .operation
                .lock()
                .map_err(|_| PasskeyError::Protocol("passkey operation lock is poisoned".into()))?;
            self.cancelled.store(false, Ordering::Release);
            let (status_tx, status_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let callback = StateCallback::new(Box::new(move |result| {
                let _ = result_tx.send(result);
            }));
            {
                let mut service = self
                    .service
                    .lock()
                    .map_err(|_| PasskeyError::Protocol("FIDO2 service lock is poisoned".into()))?;
                start(&mut service, status_tx, callback).map_err(map_error)?;
            }

            let mut saw_device = false;
            loop {
                while let Ok(status) = status_rx.try_recv() {
                    match status {
                        StatusUpdate::PresenceRequired | StatusUpdate::SelectDeviceNotice => {
                            saw_device = true;
                        }
                        StatusUpdate::PinUvError(_) | StatusUpdate::SelectResultNotice(_, _) => {
                            self.cancel()?;
                            return Err(PasskeyError::NotSupported);
                        }
                        StatusUpdate::InteractiveManagement(_) => {
                            self.cancel()?;
                            return Err(PasskeyError::Protocol(
                                "unexpected interactive authenticator request".into(),
                            ));
                        }
                    }
                }
                match result_rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(Ok(result)) => return Ok(result),
                    Ok(Err(_)) if self.cancelled.load(Ordering::Acquire) => {
                        return Err(PasskeyError::Cancelled);
                    }
                    Ok(Err(error)) => return Err(map_error_with_device(error, saw_device)),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected)
                        if self.cancelled.load(Ordering::Acquire) =>
                    {
                        return Err(PasskeyError::Cancelled);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(PasskeyError::Protocol(
                            "FIDO2 service stopped without a result".into(),
                        ));
                    }
                }
            }
        }
    }

    impl PasskeyAuthenticator for Fido2Authenticator {
        fn make_credential(
            &self,
            options: &CreationOptions,
            client_data_hash: &[u8; 32],
        ) -> Result<RegistrationResult, PasskeyError> {
            let args = register_args(options, *client_data_hash);
            let timeout_ms = timeout(options.timeout_ms);
            let result = self.run(|service, status, callback| {
                service.register(timeout_ms, args, status, callback)
            })?;
            registration_from(result)
        }

        fn get_assertion(
            &self,
            options: &RequestOptions,
            client_data_hash: &[u8; 32],
        ) -> Result<AssertionResult, PasskeyError> {
            let args = sign_args(options, *client_data_hash);
            let timeout_ms = timeout(options.timeout_ms);
            let result = self.run(|service, status, callback| {
                service.sign(timeout_ms, args, status, callback)
            })?;
            assertion_from(result, options)
        }

        fn cancel(&self) -> Result<(), PasskeyError> {
            self.service
                .lock()
                .map_err(|_| PasskeyError::Protocol("FIDO2 service lock is poisoned".into()))?
                .cancel()
                .map_err(map_error)?;
            self.cancelled.store(true, Ordering::Release);
            Ok(())
        }
    }

    pub(super) fn timeout(timeout_ms: Option<u64>) -> u64 {
        timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)
    }

    pub(super) fn register_args(
        options: &CreationOptions,
        client_data_hash: [u8; 32],
    ) -> RegisterArgs {
        RegisterArgs {
            client_data_hash,
            relying_party: RelyingParty::from(options.rp_id.clone()),
            origin: ORIGIN.into(),
            user: PublicKeyCredentialUserEntity {
                id: options.user.id.clone(),
                name: Some(options.user.name.clone()),
                display_name: Some(options.user.display_name.clone()),
            },
            pub_cred_params: vec![PublicKeyCredentialParameters {
                alg: COSEAlgorithm::ES256,
            }],
            exclude_list: Vec::new(),
            user_verification_req: user_verification(options.user_verification),
            resident_key_req: if options.resident_key_required {
                ResidentKeyRequirement::Required
            } else {
                ResidentKeyRequirement::Discouraged
            },
            extensions: AuthenticationExtensionsClientInputs::default(),
            pin: None,
            use_ctap1_fallback: false,
        }
    }

    pub(super) fn sign_args(options: &RequestOptions, client_data_hash: [u8; 32]) -> SignArgs {
        SignArgs {
            client_data_hash,
            origin: ORIGIN.into(),
            relying_party_id: options.rp_id.clone(),
            allow_list: options
                .allow_credentials
                .iter()
                .map(|credential| PublicKeyCredentialDescriptor {
                    id: credential.id.clone(),
                    transports: credential.transports.iter().map(ctap_transport).collect(),
                })
                .collect(),
            user_verification_req: user_verification(options.user_verification),
            user_presence_req: true,
            extensions: AuthenticationExtensionsClientInputs::default(),
            pin: None,
            use_ctap1_fallback: false,
        }
    }

    fn user_verification(value: UserVerification) -> UserVerificationRequirement {
        match value {
            UserVerification::Required => UserVerificationRequirement::Required,
            UserVerification::Preferred => UserVerificationRequirement::Preferred,
            UserVerification::Discouraged => UserVerificationRequirement::Discouraged,
        }
    }

    fn ctap_transport(value: &Transport) -> CtapTransport {
        match value {
            Transport::Usb => CtapTransport::USB,
            Transport::Nfc => CtapTransport::NFC,
            Transport::Internal => CtapTransport::Internal,
            Transport::Hybrid => CtapTransport::BLE,
        }
    }

    fn registration_from(result: RegisterResult) -> Result<RegistrationResult, PasskeyError> {
        registration_from_parts(result.att_obj)
    }

    pub(super) fn registration_from_parts(
        attestation: authenticator::AttestationObject,
    ) -> Result<RegistrationResult, PasskeyError> {
        let credential_id = attestation
            .auth_data
            .credential_data
            .as_ref()
            .map(|data| data.credential_id.clone())
            .filter(|id| !id.is_empty())
            .ok_or(PasskeyError::InvalidCredential)?;
        let attestation_object =
            serde_cbor::to_vec(&attestation).map_err(|_| PasskeyError::InvalidCredential)?;
        Ok(RegistrationResult {
            credential_id,
            attestation_object,
        })
    }

    pub(super) fn assertion_from(
        result: SignResult,
        options: &RequestOptions,
    ) -> Result<AssertionResult, PasskeyError> {
        let assertion = result.assertion;
        let credential_id = match assertion.credentials {
            Some(credential) if !credential.id.is_empty() => credential.id,
            _ if options.allow_credentials.len() == 1 => options.allow_credentials[0].id.clone(),
            _ => return Err(PasskeyError::InvalidCredential),
        };
        if assertion.signature.is_empty() {
            return Err(PasskeyError::InvalidCredential);
        }
        let encoded_auth_data = serde_cbor::to_vec(&assertion.auth_data)
            .map_err(|_| PasskeyError::InvalidCredential)?;
        let serde_cbor::Value::Bytes(authenticator_data) =
            serde_cbor::from_slice(&encoded_auth_data)
                .map_err(|_| PasskeyError::InvalidCredential)?
        else {
            return Err(PasskeyError::InvalidCredential);
        };
        Ok(AssertionResult {
            credential_id,
            authenticator_data,
            signature: assertion.signature,
            user_handle: assertion.user.map(|user| user.id).unwrap_or_default(),
        })
    }

    pub(super) fn map_error(error: AuthenticatorError) -> PasskeyError {
        map_error_with_device(error, false)
    }

    pub(super) fn map_error_with_device(
        error: AuthenticatorError,
        saw_device: bool,
    ) -> PasskeyError {
        match error {
            AuthenticatorError::NoConfiguredTransports => PasskeyError::NoDevice,
            AuthenticatorError::CancelledByUser => PasskeyError::Cancelled,
            AuthenticatorError::UnsupportedOption(_)
            | AuthenticatorError::HIDError(HIDError::DeviceNotSupported)
            | AuthenticatorError::HIDError(HIDError::UnsupportedCommand)
            | AuthenticatorError::U2FToken(U2FTokenError::NotSupported) => {
                PasskeyError::NotSupported
            }
            AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(
                StatusCode::ChannelBusy,
                _,
            ))) => PasskeyError::DeviceBusy,
            AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(
                StatusCode::KeepaliveCancel | StatusCode::OperationDenied,
                _,
            ))) => PasskeyError::Cancelled,
            AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(
                StatusCode::UnsupportedAlgorithm
                | StatusCode::UnsupportedOption
                | StatusCode::InvalidOption,
                _,
            ))) => PasskeyError::NotSupported,
            AuthenticatorError::HIDError(HIDError::Command(CommandError::StatusCode(
                StatusCode::NoCredentials,
                _,
            ))) => PasskeyError::InvalidCredential,
            AuthenticatorError::HIDError(HIDError::IO(_, ref error))
                if error.kind() == std::io::ErrorKind::ResourceBusy =>
            {
                PasskeyError::DeviceBusy
            }
            AuthenticatorError::U2FToken(U2FTokenError::NotAllowed) if !saw_device => {
                PasskeyError::NoDevice
            }
            AuthenticatorError::U2FToken(U2FTokenError::NotAllowed) => {
                PasskeyError::Protocol("FIDO2 operation timed out".into())
            }
            other => PasskeyError::Protocol(other.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/passkeys.rs"]
mod tests;
