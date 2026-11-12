use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

mod media;
mod preferences;
mod projections;
mod special;
pub use media::*;
pub use preferences::*;
pub use projections::DraftRevision;
pub(crate) use projections::{StoredFolder, StoredProfile};
pub use special::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupState {
    Starting,
    Ready,
    Failed,
}

pub struct ApiConfig {
    pub api_id: i32,
    pub api_hash: Zeroizing<String>,
}

impl ApiConfig {
    pub fn new(api_id: i32, api_hash: String) -> Result<Self, ClientError> {
        let api_hash = Zeroizing::new(api_hash);
        if api_id <= 0
            || api_hash.len() != 32
            || !api_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ClientError::InvalidConfiguration);
        }
        Ok(Self { api_id, api_hash })
    }

    pub fn from_secrets(contents: &str) -> Result<Self, ClientError> {
        let mut api_id = None;
        let mut api_hash = None;
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line).trim();
            let (key, value) = line
                .split_once('=')
                .ok_or(ClientError::InvalidConfiguration)?;
            let key = key.trim();
            let value = value.trim();
            let value = if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };
            match key {
                "API_ID" if api_id.is_none() => {
                    api_id = Some(
                        value
                            .parse::<i32>()
                            .map_err(|_| ClientError::InvalidConfiguration)?,
                    );
                }
                "API_HASH" if api_hash.is_none() => api_hash = Some(value.to_owned()),
                "API_ID" | "API_HASH" => return Err(ClientError::InvalidConfiguration),
                _ => {}
            }
        }
        Self::new(
            api_id.ok_or(ClientError::InvalidConfiguration)?,
            api_hash.ok_or(ClientError::InvalidConfiguration)?,
        )
    }
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiConfig")
            .field("api_id", &self.api_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Unconfigured,
    Disconnected,
    Connecting,
    Connected,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    SignedOut,
    CodeRequired { code_type: String, code_length: i32 },
    QrPending { expires: i32 },
    PasswordRequired,
    Authorized { user_id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientState {
    pub connection: ConnectionState,
    pub auth: AuthState,
    pub api_id: Option<i32>,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            connection: ConnectionState::Unconfigured,
            auth: AuthState::SignedOut,
            api_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("invalid API configuration")]
    InvalidConfiguration,
    #[error("API configuration is required")]
    NotConfigured,
    #[error("authentication is required")]
    Unauthorized,
    #[error("two-factor password is required")]
    PasswordRequired,
    #[error("new account registration is not supported")]
    RegistrationRequired,
    #[error("invalid phone number")]
    InvalidPhone,
    #[error("invalid or expired login code")]
    InvalidCode,
    #[error("invalid password")]
    InvalidPassword,
    #[error("request cancelled")]
    Cancelled,
    #[error("retry after {seconds} seconds")]
    FloodWait { seconds: u64 },
    #[error("network request failed")]
    Network,
    #[error("operation is not permitted")]
    Forbidden,
    #[error("invalid request or state")]
    InvalidRequest,
    #[error("draft changed; refresh before saving")]
    RevisionConflict,
    #[error("content cannot be safely represented")]
    UnsupportedContent,
    #[error("local database operation failed")]
    Storage,
    #[error("session key is unavailable")]
    SessionKey,
    #[error("encrypted session could not be opened")]
    Session,
    #[error("worker is stopped")]
    Stopped,
    #[error("protocol request failed")]
    Protocol,
}

impl From<mono_proto_client::MtprotoError> for ClientError {
    fn from(error: mono_proto_client::MtprotoError) -> Self {
        use mono_proto_client::MtprotoError;
        match error {
            MtprotoError::UnknownClient => Self::Stopped,
            MtprotoError::RegistrationRequired => Self::RegistrationRequired,
            MtprotoError::PasswordRequired => Self::PasswordRequired,
            MtprotoError::Message(message) => {
                let message = message.to_ascii_uppercase();
                for prefix in ["FLOOD_WAIT_", "FLOOD_PREMIUM_WAIT_", "SLOWMODE_WAIT_"] {
                    if let Some((_, suffix)) = message.split_once(prefix) {
                        let digits: String =
                            suffix.chars().take_while(char::is_ascii_digit).collect();
                        if let Ok(seconds) = digits.parse() {
                            return Self::FloodWait { seconds };
                        }
                    }
                }
                if message.contains("CANCEL") {
                    Self::Cancelled
                } else if message.contains("SESSION_PASSWORD_NEEDED") {
                    Self::PasswordRequired
                } else if message.contains("PHONE_NUMBER_INVALID") {
                    Self::InvalidPhone
                } else if message.contains("PHONE_CODE_") {
                    Self::InvalidCode
                } else if message.contains("PASSWORD_HASH_INVALID") {
                    Self::InvalidPassword
                } else if message.contains("AUTH_KEY_")
                    || message.contains("SESSION_REVOKED")
                    || message.contains("SESSION_EXPIRED")
                    || message.contains("SESSION INVALIDATED")
                {
                    Self::Unauthorized
                } else if message.contains("CHAT_WRITE_FORBIDDEN")
                    || message.contains("CHAT_ADMIN_REQUIRED")
                    || message.contains("USER_BANNED")
                {
                    Self::Forbidden
                } else if ["NETWORK", "CONNECTION", "TRANSPORT", "TIMEOUT", "IO ERROR"]
                    .iter()
                    .any(|word| message.contains(word))
                {
                    Self::Network
                } else {
                    Self::Protocol
                }
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TextSend {
    pub chat_id: i64,
    pub text: String,
    pub reply_to_msg_id: i32,
    pub top_msg_id: i32,
    pub entities_json: Option<String>,
    pub webpage_url: Option<String>,
    pub silent: bool,
    pub no_webpage: bool,
    pub schedule_date: Option<i32>,
}

impl TextSend {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.chat_id == 0
            || self.text.trim().is_empty()
            || self.reply_to_msg_id < 0
            || self.top_msg_id < 0
            || self
                .schedule_date
                .is_some_and(|date| date <= 0 || date == i32::MAX)
        {
            return Err(ClientError::InvalidRequest);
        }
        if let Some(entities) = &self.entities_json {
            let value: serde_json::Value =
                serde_json::from_str(entities).map_err(|_| ClientError::InvalidRequest)?;
            if !value.is_array() {
                return Err(ClientError::InvalidRequest);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PendingSend {
    pub random_id: i64,
    pub message: TextSend,
    pub uncertain: bool,
}

pub fn page_limit(limit: i32) -> Result<i32, ClientError> {
    if limit <= 0 {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(limit.min(100))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/domain.rs"]
mod tests;
