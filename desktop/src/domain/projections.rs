use mono_proto_client::{CloudDraft, FolderDto, ProfileDto};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct DraftRevision {
    pub draft: CloudDraft,
    pub revision: i64,
    pub pending: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "FolderDto")]
struct FolderFields {
    id: i32,
    title: String,
    chat_ids: Vec<i64>,
    exclude_chat_ids: Vec<i64>,
    include_contacts: bool,
    include_non_contacts: bool,
    include_groups: bool,
    include_channels: bool,
    include_bots: bool,
    exclude_muted: bool,
    exclude_read: bool,
    exclude_archived: bool,
    emoticon: String,
    pinned_chat_ids: Vec<i64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StoredFolder(#[serde(with = "FolderFields")] pub FolderDto);

#[derive(Serialize, Deserialize)]
#[serde(remote = "ProfileDto")]
struct ProfileFields {
    id: i64,
    kind: String,
    title: String,
    username: Option<String>,
    about: Option<String>,
    avatar_cache_key: Option<String>,
    is_self: bool,
    status: Option<String>,
    status_at: Option<i64>,
    extra_json: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct StoredProfile(#[serde(with = "ProfileFields")] pub ProfileDto);
