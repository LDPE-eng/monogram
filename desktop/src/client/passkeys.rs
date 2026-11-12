use super::*;
use crate::platform::passkeys as pk;

pub type PasskeyDto = proto::adapter::types::auth_types::PasskeyDto;
pub type PasskeyCredentialDto = proto::adapter::types::auth_types::PasskeyCredentialDto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOptions {
    pub options_json: String,
    pub dc_id: i32,
    pub auth_key_id: i64,
}

#[derive(Debug, Clone)]
pub enum PasskeyCommand {
    Enabled,
    InitLogin,
    FinishLogin { options: LoginOptions },
    InitRegistration,
    Register { options_json: String },
    List,
    Delete { id: String },
}

#[derive(Debug, Clone)]
pub enum PasskeyResponse {
    Enabled(bool),
    LoginOptions(LoginOptions),
    RegistrationOptions(String),
    SignedIn { user_id: i64, dc_id: i32 },
    Registered(pk::PasskeyInfo),
    Keys(Vec<pk::PasskeyInfo>),
    Deleted(bool),
    Done,
}

impl Runtime {
    pub(super) fn execute_passkey(
        &mut self,
        command: PasskeyCommand,
    ) -> Result<PasskeyResponse, ClientError> {
        match command {
            PasskeyCommand::Enabled => {
                let enabled =
                    proto::passkeys::passkeys_enabled(self.handle()?).map_err(ClientError::from)?;
                Ok(PasskeyResponse::Enabled(enabled))
            }
            PasskeyCommand::InitLogin => {
                let dto = proto::passkeys::init_passkey_login(self.handle()?)
                    .map_err(ClientError::from)?;
                Ok(PasskeyResponse::LoginOptions(LoginOptions {
                    options_json: dto.options,
                    dc_id: dto.dc_id,
                    auth_key_id: dto.auth_key_id,
                }))
            }
            PasskeyCommand::FinishLogin { options } => {
                let request =
                    pk::parse_request_options(&options.options_json).map_err(passkey_error)?;
                let authenticator = pk::system_authenticator().map_err(passkey_error)?;
                let client_data = pk::client_data_json(pk::ClientDataKind::Get, &request.challenge)
                    .map_err(passkey_error)?;
                let hash = pk::client_data_hash(&client_data).map_err(passkey_error)?;
                let assertion = with_cancellation(&authenticator, || {
                    authenticator.get_assertion(&request, &hash)
                })
                .map_err(passkey_error)?;
                let credential = pk::assertion_credential(&client_data, &request, &assertion)
                    .map_err(passkey_error)?;
                let signed = proto::passkeys::finish_passkey_login(
                    self.handle()?,
                    credential.into_dto(),
                    options.dc_id,
                    options.auth_key_id,
                )
                .map_err(ClientError::from)?;
                self.signed_in(signed.user_id)?;
                Ok(PasskeyResponse::SignedIn {
                    user_id: signed.user_id,
                    dc_id: signed.dc_id,
                })
            }
            PasskeyCommand::InitRegistration => {
                let options = proto::passkeys::init_passkey_registration(self.authorized()?)
                    .map_err(ClientError::from)?;
                Ok(PasskeyResponse::RegistrationOptions(options))
            }
            PasskeyCommand::Register { options_json } => {
                let options = pk::parse_creation_options(&options_json).map_err(passkey_error)?;
                let authenticator = pk::system_authenticator().map_err(passkey_error)?;
                let client_data =
                    pk::client_data_json(pk::ClientDataKind::Create, &options.challenge)
                        .map_err(passkey_error)?;
                let hash = pk::client_data_hash(&client_data).map_err(passkey_error)?;
                let result = with_cancellation(&authenticator, || {
                    authenticator.make_credential(&options, &hash)
                })
                .map_err(passkey_error)?;
                let credential = pk::registration_credential(&client_data, &options, &result)
                    .map_err(passkey_error)?;
                let dto =
                    proto::passkeys::register_passkey(self.authorized()?, credential.into_dto())
                        .map_err(ClientError::from)?;
                Ok(PasskeyResponse::Registered(info_from(dto)))
            }
            PasskeyCommand::List => {
                let keys =
                    proto::passkeys::get_passkeys(self.authorized()?).map_err(ClientError::from)?;
                Ok(PasskeyResponse::Keys(
                    keys.into_iter().map(info_from).collect(),
                ))
            }
            PasskeyCommand::Delete { id } => {
                if id.is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                let deleted = proto::passkeys::delete_passkey(self.authorized()?, id)
                    .map_err(ClientError::from)?;
                Ok(PasskeyResponse::Deleted(deleted))
            }
        }
    }
}

fn with_cancellation<T>(
    authenticator: &std::sync::Arc<dyn pk::PasskeyAuthenticator>,
    operation: impl FnOnce() -> Result<T, pk::PasskeyError>,
) -> Result<T, pk::PasskeyError> {
    let request_id = request_control::current_id();
    if request_id == 0 {
        return operation();
    }
    request_control::check().map_err(|_| pk::PasskeyError::Cancelled)?;

    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor_finished = finished.clone();
    let monitor_authenticator = authenticator.clone();
    let monitor = std::thread::Builder::new()
        .name("passkey-cancel".into())
        .spawn(move || {
            let previous = request_control::bind(request_id);
            while !monitor_finished.load(std::sync::atomic::Ordering::Acquire) {
                if request_control::check().is_err() {
                    let _ = monitor_authenticator.cancel();
                    break;
                }
                std::thread::park_timeout(std::time::Duration::from_millis(20));
            }
            request_control::bind(previous);
        })
        .map_err(|error| pk::PasskeyError::Protocol(error.to_string()))?;

    let result = operation();
    finished.store(true, std::sync::atomic::Ordering::Release);
    monitor.thread().unpark();
    monitor
        .join()
        .map_err(|_| pk::PasskeyError::Protocol("passkey cancellation monitor panicked".into()))?;
    request_control::check().map_err(|_| pk::PasskeyError::Cancelled)?;
    result
}

fn info_from(value: PasskeyDto) -> pk::PasskeyInfo {
    pk::PasskeyInfo {
        id: value.id,
        name: value.name,
        date: value.date,
        software_emoji_id: value.software_emoji_id,
        last_usage_date: value.last_usage_date,
    }
}

impl pk::PasskeyCredential {
    fn into_dto(self) -> PasskeyCredentialDto {
        PasskeyCredentialDto {
            id: self.id,
            raw_id: self.raw_id,
            client_data: self.client_data,
            authenticator_data: self.authenticator_data,
            signature: self.signature,
            user_handle: self.user_handle,
            attestation_data: self.attestation_data,
        }
    }
}

fn passkey_error(error: pk::PasskeyError) -> ClientError {
    match error {
        pk::PasskeyError::Cancelled => ClientError::Cancelled,
        pk::PasskeyError::NoDevice
        | pk::PasskeyError::DeviceBusy
        | pk::PasskeyError::NotSupported => ClientError::UnsupportedContent,
        pk::PasskeyError::InvalidOptions
        | pk::PasskeyError::InvalidChallenge
        | pk::PasskeyError::InvalidCredential
        | pk::PasskeyError::Crypto
        | pk::PasskeyError::Protocol(_) => ClientError::Protocol,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/passkeys_client.rs"]
mod tests;
