use serde::{Deserialize, Serialize};

use crate::domain::ClientError;

pub const APP_PREFERENCES_VERSION: u32 = 1;
pub const MAX_PREFERENCE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceNamespace {
    Notifications,
    Proxy,
    Wallpapers,
    UpdateState,
    LogSettings,
    Connectivity,
    Appearance,
    Passkeys,
    Maintenance,
}

impl PreferenceNamespace {
    pub const ALL: [Self; 9] = [
        Self::Notifications,
        Self::Proxy,
        Self::Wallpapers,
        Self::UpdateState,
        Self::LogSettings,
        Self::Connectivity,
        Self::Appearance,
        Self::Passkeys,
        Self::Maintenance,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notifications => "notifications",
            Self::Proxy => "proxy",
            Self::Wallpapers => "wallpapers",
            Self::UpdateState => "update_state",
            Self::LogSettings => "log_settings",
            Self::Connectivity => "connectivity",
            Self::Appearance => "appearance",
            Self::Passkeys => "passkeys",
            Self::Maintenance => "maintenance",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|namespace| namespace.as_str() == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferenceDocument {
    pub version: u32,
    pub payload_json: String,
}

impl PreferenceDocument {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.version == 0
            || self.version > APP_PREFERENCES_VERSION
            || self.payload_json.len() > MAX_PREFERENCE_BYTES
        {
            return Err(ClientError::InvalidRequest);
        }
        serde_json::from_str::<serde_json::Value>(&self.payload_json)
            .map_err(|_| ClientError::InvalidRequest)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPreferences {
    pub documents: Vec<(PreferenceNamespace, PreferenceDocument)>,
}
