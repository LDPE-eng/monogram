use serde::{Deserialize, Serialize};

use crate::domain::ClientError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaDownloadKind {
    Full,
    Thumb,
    Display,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaDownloadMode {
    Manual,
    Automatic,
    Autoplay,
    Preload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCategory {
    Photo,
    Video,
    Document,
    Avatar,
    Other,
}

impl MediaCategory {
    pub fn from_media_kind(kind: Option<&str>) -> Self {
        match kind {
            Some("photo") => Self::Photo,
            Some("video" | "gif" | "video_note") => Self::Video,
            Some("document" | "audio" | "voice" | "sticker") => Self::Document,
            Some(_) => Self::Other,
            None => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkClass {
    Wifi,
    Mobile,
    Roaming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheLimit {
    Limited(u64),
    Unlimited,
}

impl CacheLimit {
    pub fn validate(self) -> Result<(), ClientError> {
        match self {
            Self::Limited(bytes) if bytes == 0 || bytes > i64::MAX as u64 => {
                Err(ClientError::InvalidRequest)
            }
            _ => Ok(()),
        }
    }

    pub fn bytes(self) -> Option<u64> {
        match self {
            Self::Limited(bytes) => Some(bytes),
            Self::Unlimited => None,
        }
    }
}

impl Default for CacheLimit {
    fn default() -> Self {
        Self::Limited(2 * 1024 * 1024 * 1024)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub wifi: bool,
    pub mobile: bool,
    pub roaming: bool,
}

impl NetworkPolicy {
    pub fn allows(self, network: NetworkClass) -> bool {
        match network {
            NetworkClass::Wifi => self.wifi,
            NetworkClass::Mobile => self.mobile,
            NetworkClass::Roaming => self.roaming,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoDownloadRule {
    pub enabled: bool,
    pub max_bytes: u64,
}

impl AutoDownloadRule {
    fn validate(self) -> Result<(), ClientError> {
        if self.max_bytes == 0 {
            Err(ClientError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoDownloadPolicy {
    pub photo: AutoDownloadRule,
    pub video: AutoDownloadRule,
    pub document: AutoDownloadRule,
}

impl AutoDownloadPolicy {
    pub fn rule(self, category: MediaCategory) -> Option<AutoDownloadRule> {
        match category {
            MediaCategory::Photo => Some(self.photo),
            MediaCategory::Video => Some(self.video),
            MediaCategory::Document => Some(self.document),
            MediaCategory::Avatar | MediaCategory::Other => None,
        }
    }

    fn validate(self) -> Result<(), ClientError> {
        self.photo.validate()?;
        self.video.validate()?;
        self.document.validate()
    }
}

impl Default for AutoDownloadPolicy {
    fn default() -> Self {
        let disabled = AutoDownloadRule {
            enabled: false,
            max_bytes: 16 * 1024 * 1024,
        };
        Self {
            photo: disabled,
            video: disabled,
            document: disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoDownloadSettings {
    pub wifi: AutoDownloadPolicy,
    pub mobile: AutoDownloadPolicy,
    pub roaming: AutoDownloadPolicy,
}

impl AutoDownloadSettings {
    pub fn policy(self, network: NetworkClass) -> AutoDownloadPolicy {
        match network {
            NetworkClass::Wifi => self.wifi,
            NetworkClass::Mobile => self.mobile,
            NetworkClass::Roaming => self.roaming,
        }
    }

    pub fn validate(self) -> Result<(), ClientError> {
        self.wifi.validate()?;
        self.mobile.validate()?;
        self.roaming.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaDecision {
    pub autoplay: bool,
    pub preload: bool,
    pub auto_download: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaSettings {
    pub cache_limit: CacheLimit,
    pub network: NetworkClass,
    pub auto_download: AutoDownloadSettings,
    pub autoplay: NetworkPolicy,
    pub preload: NetworkPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaCategoryUsage {
    pub category: MediaCategory,
    pub complete_bytes: u64,
    pub partial_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaChatUsage {
    pub chat_id: i64,
    pub complete_bytes: u64,
    pub partial_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaUsage {
    pub complete_bytes: u64,
    pub partial_bytes: u64,
    pub pinned_entries: usize,
    pub cache_limit: CacheLimit,
    pub by_category: Vec<MediaCategoryUsage>,
    pub by_chat: Vec<MediaChatUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearMediaScope {
    Category(MediaCategory),
    Chat(i64),
    ChatCategory {
        chat_id: i64,
        category: MediaCategory,
    },
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaUpload {
    pub path: String,
    pub kind: String,
    pub mime_type: String,
    pub file_name: String,
    pub caption: String,
    pub entities_json: Option<String>,
    pub duration: i32,
    pub width: i32,
    pub height: i32,
}

impl MediaUpload {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.path.is_empty()
            || !matches!(self.kind.as_str(), "photo" | "video" | "document")
            || self.duration < 0
            || self.width < 0
            || self.height < 0
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

    pub fn replacement_validate(&self) -> Result<(), ClientError> {
        self.validate()?;
        if !self.caption.is_empty() || self.entities_json.is_some() {
            Err(ClientError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaSend {
    pub chat_id: i64,
    pub reply_to_msg_id: i32,
    pub top_msg_id: i32,
    pub silent: bool,
    pub schedule_date: Option<i32>,
    pub items: Vec<MediaUpload>,
}

impl MediaSend {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.chat_id == 0
            || self.reply_to_msg_id < 0
            || self.top_msg_id < 0
            || self.items.is_empty()
            || self.items.len() > 10
            || self
                .schedule_date
                .is_some_and(|date| date <= 0 || date == i32::MAX)
        {
            return Err(ClientError::InvalidRequest);
        }
        for item in &self.items {
            item.validate()?;
        }
        let documents = self
            .items
            .iter()
            .filter(|item| item.kind == "document")
            .count();
        if documents > 0 && documents != self.items.len() {
            return Err(ClientError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PendingMediaSend {
    pub album_id: i64,
    pub random_ids: Vec<i64>,
    pub send: MediaSend,
    pub uncertain: bool,
}
