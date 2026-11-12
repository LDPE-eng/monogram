use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    rc::Rc,
    time::Duration,
};

use mono_proto_client::{
    ChatDto, CloudDraft, MessageDto, OutgoingMessageResult, ScheduledMessageDto,
};
use monogram_markup::MarkupEntityDto;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, SimpleComponent,
    gtk, gtk::prelude::*,
};
use serde::Deserialize;

use crate::{
    client::{
        WorkerCommand, WorkerResponse,
        interactions::{InteractionCommand, InteractionResponse, OutboxRead, ReactionPeersPage},
        web::{INSTANT_VIEW_MEDIA_MESSAGE_ID, WebCommand, WebResponse},
    },
    domain::{
        ClientError, DraftRevision, MediaDownloadKind, MediaDownloadMode, MediaSend, MediaUpload,
        PendingMediaSend, PendingSend, SpecialContent, SpecialSend, TextSend,
    },
    i18n::I18n,
    media::{
        playback::{MediaSource, Track},
        sticker::StickerSource,
    },
    ui::{
        appearance::{WallpaperConfig, WallpaperMode, wallpaper_texture},
        markup::{Block, Document, MAX_ENTITIES, MAX_INPUT_BYTES, ParseMode, RichText, TextIndex},
        media::{MediaInput, MediaPlayer},
        sticker::StickerView,
        web::{WebInput, WebOutput, WebView},
    },
};

pub enum DialogInput {
    SetChat(ChatDto),
    SetWallpaper(WallpaperConfig),
    SetForwardRecipient(mono_proto_client::ProfileDto),
    SetHistory {
        chat_id: i64,
        messages: Vec<MessageDto>,
        append: bool,
    },
    UpsertMessage(MessageDto),
    RemoveMessage {
        chat_id: i64,
        message_id: i32,
    },
    SetBusy(bool),
    SetError(Option<ClientError>),
    MediaNotice(&'static str),
    SetOnline(bool),
    SetPending(Vec<PendingSend>),
    SetPendingMedia(Vec<PendingMediaSend>),
    MediaQueued {
        token: u64,
        album_id: i64,
        random_ids: Vec<i64>,
    },
    MediaProgress {
        token: u64,
        transfer_id: u64,
        downloaded: u64,
        total: Option<u64>,
    },
    OutgoingQueued {
        random_id: i64,
    },
    OutgoingResult {
        random_id: i64,
        result: Result<OutgoingMessageResult, ClientError>,
    },
    Response {
        token: u64,
        response: WorkerResponse,
    },
    Failed {
        token: u64,
        error: ClientError,
    },
    SetActive(bool),
    DraftChanged(DraftRevision),
    ScheduledChanged(ScheduledMessageDto),
    ScheduledDeleted {
        chat_id: i64,
        message_ids: Vec<i32>,
    },
    OpenThread {
        chat_id: i64,
        message_id: i32,
    },
    LeaveThread,
    LoadPinned,
    JumpTo(i32),
    LoadScheduled,
    QueueAction {
        message_id: i32,
        action: ScheduledAction,
    },
    DiscardDraft,
    ComposerChanged,
    DraftDebounced {
        generation: u64,
        revision: u64,
    },
    ViewportChanged,
    Back,
    StopPlayback,
    LoadMore,
    Submit,
    Cancel,
    Action {
        message_id: i32,
        action: MessageAction,
    },
    Media {
        message_id: i32,
        action: MediaAction,
    },
    OpenGeo(i32),
    TogglePollOption {
        message_id: i32,
        index: usize,
    },
    SubmitPollVote(i32),
    RetractPollVote(i32),
    ClosePoll(i32),
    LoadPollVoters {
        message_id: i32,
        append: bool,
    },
    Checklist {
        message_id: i32,
        action: ChecklistAction,
    },
    ReactionPeers {
        message_id: i32,
        append: bool,
    },
    QuickReact {
        message_id: i32,
        emoticon: String,
        document_id: i64,
    },
    LoadReadReceipt(i32),
    LoadForums,
    OpenForumTopic(i32),
    ToggleGeneralHidden,
    OpenPicker(PickerKind),
    OpenStickerPack(usize),
    LoadInlineNext,
    InsertEmoji(String),
    InsertInlineResult(usize),
    SelectMention(usize),
    SendSticker(i64),
    SendGif(i64),
    Keyboard {
        message_id: i32,
        row: usize,
        index: usize,
    },
    OpenInstantView(i32),
    WebBack,
    WebCommand(WebCommand),
    WebExternal(String),
    WebMedia(String),
    RetryMedia(i64),
    ChooseAttachments(AttachmentKind),
    AttachmentsChosen(Vec<SelectedAttachment>),
    RemoveAttachment(usize),
    Retry(i64),
}

pub enum DialogOutput {
    Back,
    LoadHistory {
        chat_id: i64,
        limit: i32,
        offset_id: i32,
        offset_date: i32,
        add_offset: i32,
    },
    SendText(TextSend),
    RetryText(i64),
    EditText {
        chat_id: i64,
        message_id: i32,
        text: String,
        entities_json: Option<String>,
    },
    DeleteMessage {
        chat_id: i64,
        message_id: i32,
        revoke: bool,
    },
    Forward {
        from_chat_id: i64,
        message_ids: Vec<i32>,
        to_chat_id: i64,
        drop_author: bool,
    },
    React {
        chat_id: i64,
        message_id: i32,
        emoticon: String,
        document_id: i64,
    },
    Command {
        token: u64,
        command: WorkerCommand,
    },
    ClearActiveDialog,
}

impl fmt::Debug for DialogInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DialogInput(..)")
    }
}

impl fmt::Debug for DialogOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DialogOutput(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledAction {
    Edit,
    Reschedule,
    SendNow,
    Delete,
}

#[derive(Clone, Copy)]
enum Operation {
    Replies {
        append: bool,
    },
    Pinned,
    Jump(i32),
    DraftLoad,
    DraftSave {
        revision: u64,
        discard: bool,
    },
    ScheduledLoad,
    ScheduledMutation,
    Read(i32),
    PendingMedia,
    MediaSend,
    MediaDownload {
        chat_id: i64,
        message_id: i32,
        kind: MediaDownloadKind,
        mode: MediaDownloadMode,
    },
    MediaRange {
        chat_id: i64,
        message_id: i32,
    },
    CustomEmoji {
        document_id: i64,
    },
    PollMutation(i32),
    PollVoters {
        message_id: i32,
        append: bool,
    },
    ChecklistMutation(i32),
    ReactionPeers {
        message_id: i32,
        append: bool,
    },
    ReadReceipt(i32),
    ForumTopics,
    GeneralHidden,
    ResolveInline,
    InlineResults,
    MentionSearch,
    StickerCatalog,
    StickerPack,
    SavedGifs,
    RecentReactions,
    BotCallback(i32),
    InstantView,
    InstantMedia,
    SendInlineResult,
    MediaCancel,
}

struct DialogRequest {
    generation: u64,
    operation: Operation,
}

#[derive(Default, Clone)]
struct DraftGuard {
    revision: u64,
    saved_revision: u64,
    server_revision: i64,
    ready: bool,
    unsupported: bool,
    conflict: bool,
}

impl DraftGuard {
    fn changed(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
    fn dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    fn accept(&mut self, incoming: &DraftRevision) -> bool {
        if incoming.revision < self.server_revision {
            return false;
        }
        self.server_revision = incoming.revision;
        self.ready = true;
        if self.dirty() {
            return false;
        }
        self.unsupported = incoming.draft.has_unsupported_content;
        true
    }
    fn acknowledge(&mut self, local_revision: u64, server_revision: i64) {
        self.server_revision = self.server_revision.max(server_revision);
        self.saved_revision = self.saved_revision.max(local_revision.min(self.revision));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAction {
    Reply,
    Edit,
    Delete,
    Forward,
    React,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    Download,
    CancelDownload,
    RetryDownload,
    Save,
    SaveAs,
    OpenExternal,
    Copy,
    View,
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentKind {
    Photo,
    Video,
    Document,
}

impl AttachmentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::Document => "document",
        }
    }

    fn localization_key(self) -> &'static str {
        match self {
            Self::Photo => "feature-dialog-dialog_attach_photo",
            Self::Video => "feature-dialog-dialog_attach_video",
            Self::Document => "feature-dialog-dialog_attach_file",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SelectedAttachment {
    path: String,
    kind: AttachmentKind,
    file_name: String,
    mime_type: String,
}

fn extend_attachments(
    current: &[SelectedAttachment],
    additions: Vec<SelectedAttachment>,
) -> Result<Vec<SelectedAttachment>, ClientError> {
    if additions.is_empty() || current.len().saturating_add(additions.len()) > 10 {
        return Err(ClientError::InvalidRequest);
    }
    let document_album = current
        .iter()
        .chain(&additions)
        .any(|item| item.kind == AttachmentKind::Document);
    let media_album = current
        .iter()
        .chain(&additions)
        .any(|item| item.kind != AttachmentKind::Document);
    if document_album && media_album {
        return Err(ClientError::InvalidRequest);
    }
    let mut result = current.to_vec();
    for item in additions {
        if item.path.is_empty()
            || item.file_name.is_empty()
            || result.iter().any(|old| old.path == item.path)
        {
            return Err(ClientError::InvalidRequest);
        }
        result.push(item);
    }
    Ok(result)
}

fn media_uploads(
    attachments: &[SelectedAttachment],
    caption: &str,
    entities_json: Option<String>,
) -> Vec<MediaUpload> {
    attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| MediaUpload {
            path: attachment.path.clone(),
            kind: attachment.kind.as_str().into(),
            mime_type: attachment.mime_type.clone(),
            file_name: attachment.file_name.clone(),
            caption: if index == 0 {
                caption.to_owned()
            } else {
                String::new()
            },
            entities_json: if index == 0 {
                entities_json.clone()
            } else {
                None
            },
            duration: 0,
            width: 0,
            height: 0,
        })
        .collect()
}

fn custom_emoji_ids(text: &str, json: Option<&str>) -> Vec<i64> {
    let Ok(entities) = entities(text, json) else {
        return Vec::new();
    };
    entities
        .into_iter()
        .filter(|entity| entity.kind == "custom_emoji")
        .filter_map(|entity| entity.extra?.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sticker_source(path: &std::path::Path, file_name: Option<&str>) -> Option<StickerSource> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .ok()?;
    if magic.starts_with(&[0x1f, 0x8b]) {
        if metadata.len() > 8 * 1024 * 1024 {
            return None;
        }
        return std::fs::read(path).ok().map(StickerSource::Tgs);
    }
    let webm_extension = file_name.is_some_and(|name| {
        std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("webm"))
    });
    if webm_extension || magic == [0x1a, 0x45, 0xdf, 0xa3] {
        return Some(StickerSource::Webm(path.to_path_buf()));
    }
    None
}

#[derive(Default)]
struct History {
    messages: BTreeMap<i32, MessageDto>,
    live: BTreeSet<i32>,
    deleted: BTreeSet<i32>,
}

fn derived_media_cache_key(prefix: &str, key: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}:{hash:016x}")
}

fn album_positions(messages: &BTreeMap<i32, MessageDto>) -> BTreeMap<i32, (usize, usize)> {
    let mut albums: BTreeMap<(i64, i64), Vec<i32>> = BTreeMap::new();
    for message in messages.values() {
        if let Some(grouped_id) = message.grouped_id.filter(|id| *id != 0) {
            albums
                .entry((message.chat_id, grouped_id))
                .or_default()
                .push(message.id);
        }
    }
    albums
        .into_values()
        .filter(|ids| ids.len() > 1)
        .flat_map(|ids| {
            let total = ids.len();
            ids.into_iter()
                .enumerate()
                .map(move |(index, id)| (id, (index + 1, total)))
        })
        .collect()
}

impl History {
    fn insert(&mut self, message: MessageDto, live: bool) {
        let id = message.id;
        if id <= 0 || self.deleted.contains(&id) || (!live && self.live.contains(&id)) {
            return;
        }
        if self
            .messages
            .get(&id)
            .is_some_and(|old| old.edit_date.unwrap_or(0) > message.edit_date.unwrap_or(0))
        {
            return;
        }
        if live {
            self.live.insert(id);
        }
        self.messages.insert(id, message);
    }

    fn delete(&mut self, id: i32) {
        self.deleted.insert(id);
        self.messages.remove(&id);
        self.live.remove(&id);
    }
}

struct PendingRow {
    message: TextSend,
    failed: bool,
    uncertain: bool,
}

#[derive(Clone)]
struct MediaSummary {
    kind_key: &'static str,
    album_position: Option<(usize, usize)>,
    media_cache_key: Option<String>,
    file_name: Option<String>,
    file_size: Option<i64>,
    duration: Option<i32>,
    width: Option<i32>,
    height: Option<i32>,
    supports_streaming: bool,
    protected: bool,
    webpage: Option<WebpageSummary>,
    interaction: Option<InteractionCard>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct WebpageSummary {
    #[serde(rename = "u")]
    url: Option<String>,
    #[serde(rename = "t")]
    title: Option<String>,
    #[serde(rename = "s")]
    site_name: Option<String>,
    #[serde(rename = "d")]
    description: Option<String>,
    #[serde(rename = "e")]
    embed_url: Option<String>,
    #[serde(rename = "y")]
    kind: Option<String>,
    #[serde(rename = "iv", default)]
    instant_view: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum InteractionCard {
    Poll(PollCard),
    Checklist(ChecklistCard),
    Contact(ContactCard),
    Geo(GeoCard),
    Dice(DiceCard),
    Announcement(AnnouncementCard),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct PollCard {
    #[serde(default)]
    id: String,
    #[serde(default)]
    creator: i32,
    #[serde(default)]
    r: i32,
    #[serde(default)]
    q: String,
    #[serde(default)]
    a: Vec<PollAnswer>,
    #[serde(default)]
    n: i32,
    #[serde(default)]
    z: i32,
    #[serde(default)]
    m: i32,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    o: i32,
    #[serde(default)]
    s: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct PollAnswer {
    #[serde(default)]
    t: String,
    #[serde(default)]
    c: i32,
    #[serde(default)]
    k: i32,
    #[serde(default)]
    v: i32,
    #[serde(default)]
    o: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ChecklistCard {
    #[serde(default)]
    t: String,
    #[serde(default)]
    a: i32,
    #[serde(default)]
    c: i32,
    #[serde(default)]
    i: Vec<ChecklistItem>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ChecklistItem {
    #[serde(default)]
    id: i32,
    #[serde(default)]
    t: String,
    #[serde(default)]
    d: i32,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    by: i64,
    #[serde(default)]
    at: i32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ContactCard {
    #[serde(default)]
    p: String,
    #[serde(default)]
    f: String,
    #[serde(default)]
    l: String,
    #[serde(default)]
    c: String,
    #[serde(default)]
    u: i64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct GeoCard {
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    long: Option<f64>,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    a: Option<String>,
    #[serde(default)]
    live: Option<i32>,
    #[serde(default)]
    p: Option<i32>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct DiceCard {
    #[serde(default)]
    v: i32,
    #[serde(default)]
    e: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct AnnouncementCard {
    #[serde(default)]
    t: String,
    #[serde(default)]
    s: String,
    #[serde(default)]
    d: String,
    #[serde(default)]
    q: i32,
    #[serde(default)]
    m: i32,
    #[serde(default)]
    x: i64,
    #[serde(default)]
    w: i32,
    #[serde(default)]
    u: i32,
    #[serde(default)]
    i: i32,
    #[serde(default)]
    n: i32,
    #[serde(default)]
    r: i32,
}

fn parse_interaction(message: &MessageDto) -> Option<InteractionCard> {
    let kind = message.media_kind.as_deref()?;
    let json = message.file_name.as_deref()?;
    match kind {
        "poll" => serde_json::from_str(json).ok().map(InteractionCard::Poll),
        "todo" => serde_json::from_str(json)
            .ok()
            .map(InteractionCard::Checklist),
        "contact" => serde_json::from_str(json)
            .ok()
            .map(InteractionCard::Contact),
        "geo" | "venue" => serde_json::from_str(json).ok().map(InteractionCard::Geo),
        "dice" => serde_json::from_str(json).ok().map(InteractionCard::Dice),
        "giveaway" | "giveaway_results" | "paid_media" | "game" | "invoice" | "story"
        | "video_stream" => serde_json::from_str(json)
            .ok()
            .map(InteractionCard::Announcement),
        _ => None,
    }
}

fn geo_url(card: &GeoCard) -> Option<String> {
    let latitude = card.lat?;
    let longitude = card.long?;
    if !latitude.is_finite()
        || !longitude.is_finite()
        || !(-90.0..=90.0).contains(&latitude)
        || !(-180.0..=180.0).contains(&longitude)
    {
        return None;
    }
    Some(format!(
        "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=18/{latitude}/{longitude}"
    ))
}

fn poll_option_bytes(answer: &PollAnswer) -> Option<Vec<u8>> {
    let bytes = answer.o.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

fn poll_vote_allowed(poll: &PollCard, options: &[Vec<u8>]) -> bool {
    if poll.x != 0
        || poll.a.iter().any(|answer| answer.c != 0) && (poll.z != 0 || poll.r != 0)
        || options.is_empty()
        || poll.m == 0 && options.len() > 1
    {
        return false;
    }
    let available = poll
        .a
        .iter()
        .filter_map(poll_option_bytes)
        .collect::<Vec<_>>();
    let mut unique = options.to_vec();
    unique.sort();
    unique.dedup();
    unique.len() == options.len()
        && options
            .iter()
            .all(|option| !option.is_empty() && available.contains(option))
}

fn poll_retract_allowed(poll: &PollCard) -> bool {
    poll.x == 0 && poll.r == 0 && poll.z == 0 && poll.a.iter().any(|answer| answer.c != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReactionSummary {
    emoticon: String,
    document_id: i64,
    count: i32,
    chosen: bool,
}

fn reaction_summaries(json: Option<&str>) -> Vec<ReactionSummary> {
    let Some(json) = json else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(32)
        .filter_map(|value| {
            let count = value.get("c")?.as_i64()? as i32;
            let chosen = value.get("me").and_then(serde_json::Value::as_i64) == Some(1);
            if let Some(emoticon) = value.get("e").and_then(serde_json::Value::as_str) {
                return Some(ReactionSummary {
                    emoticon: emoticon.to_owned(),
                    document_id: 0,
                    count,
                    chosen,
                });
            }
            value
                .get("d")
                .and_then(serde_json::Value::as_i64)
                .map(|document_id| ReactionSummary {
                    emoticon: String::new(),
                    document_id,
                    count,
                    chosen,
                })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum KeyboardAction {
    Url(String),
    Callback(String),
    Copy(String),
    SwitchInline(String),
    Text(String),
    Web(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyboardButton {
    label: String,
    action: KeyboardAction,
}

fn http_url(value: &str) -> bool {
    let value = value.trim();
    (value
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://")))
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
}

fn external_uri_allowed(value: &str) -> bool {
    let value = value.trim();
    if http_url(value) {
        let normalized = value.to_ascii_lowercase();
        if [
            "https://t.me/",
            "http://t.me/",
            "https://www.t.me/",
            "http://www.t.me/",
            "https://telegram.me/",
            "http://telegram.me/",
            "https://www.telegram.me/",
            "http://www.telegram.me/",
            "https://telegram.dog/",
            "http://telegram.dog/",
        ]
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
        {
            return crate::platform::external_action_allowed(value);
        }
        return true;
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    match scheme.to_ascii_lowercase().as_str() {
        "tg" => {
            let normalized = if value.starts_with("tg:") {
                value.to_owned()
            } else {
                format!("tg:{rest}")
            };
            crate::platform::external_action_allowed(&normalized)
        }
        "mailto" | "tel" | "geo" => true,
        _ => false,
    }
}

fn json_flag(value: Option<&serde_json::Value>) -> bool {
    value.and_then(|value| {
        value
            .as_bool()
            .or_else(|| value.as_i64().map(|value| value != 0))
    }) == Some(true)
}

fn keyboard_rows(json: Option<&str>) -> Vec<Vec<KeyboardButton>> {
    let Some(json) = json.filter(|json| json.len() <= 65_536) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let markup_kind = value.get("k").and_then(serde_json::Value::as_str);
    let mut total = 0usize;
    value
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(20)
        .filter_map(|row| {
            let buttons = row
                .as_array()
                .into_iter()
                .flatten()
                .take(20)
                .filter_map(|button| {
                    total += 1;
                    if total > 100 {
                        return None;
                    }
                    let label = button.get("x")?.as_str()?.to_owned();
                    if label.is_empty() || label.encode_utf16().count() > 256 {
                        return None;
                    }
                    let kind = button.get("t")?.as_str()?;
                    let text = |key| button.get(key).and_then(serde_json::Value::as_str);
                    let action = match kind {
                        "url" if text("u").is_some_and(http_url) => {
                            KeyboardAction::Url(text("u")?.to_owned())
                        }
                        "cb" if !json_flag(button.get("pw"))
                            && text("d").is_some_and(|data| {
                                !data.is_empty()
                                    && data.len() <= 128
                                    && data.len().is_multiple_of(2)
                                    && data.bytes().all(|byte| byte.is_ascii_hexdigit())
                            }) =>
                        {
                            KeyboardAction::Callback(text("d")?.to_owned())
                        }
                        "copy" if text("c").is_some_and(|text| !text.is_empty()) => {
                            KeyboardAction::Copy(text("c")?.to_owned())
                        }
                        "sw" if json_flag(button.get("same")) => {
                            KeyboardAction::SwitchInline(text("q").unwrap_or_default().to_owned())
                        }
                        "text" => KeyboardAction::Text(label.clone()),
                        "web" if text("u").is_some_and(http_url) => {
                            KeyboardAction::Web(text("u")?.to_owned())
                        }
                        _ => return None,
                    };
                    (markup_kind == Some("inline")
                        || markup_kind == Some("keyboard")
                            && matches!(&action, KeyboardAction::Text(_)))
                    .then_some(KeyboardButton { label, action })
                })
                .collect::<Vec<_>>();
            (!buttons.is_empty()).then_some(buttons)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ComposerInlineQuery {
    username: String,
    query: String,
}

fn composer_inline_query(draft: &str) -> Option<ComposerInlineQuery> {
    let draft = draft.trim_end();
    let token = draft.strip_prefix('@')?;
    let (username, query) = token
        .split_once(char::is_whitespace)
        .map_or((token, ""), |(username, query)| (username, query));
    let mut chars = username.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && (3..=32).contains(&username.len())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    valid.then(|| ComposerInlineQuery {
        username: username.to_ascii_lowercase(),
        query: query.to_owned(),
    })
}

fn composer_mention_query(draft: &str) -> Option<String> {
    let draft = draft.trim_end();
    let start = draft
        .rfind(|ch: char| ch.is_whitespace())
        .map_or(0, |index| index + 1);
    let query = draft[start..].strip_prefix('@')?;
    (!query.is_empty()
        && query.len() <= 64
        && query
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
    .then(|| query.to_ascii_lowercase())
}

impl MediaSummary {
    fn new(message: &MessageDto, album_position: Option<(usize, usize)>) -> Option<Self> {
        let kind = message.media_kind.as_deref()?;
        let kind_key = match kind {
            "photo" => "feature-dialog-dialog_media_photo",
            "video" => "feature-dialog-dialog_media_video",
            "video_note" => "feature-dialog-dialog_media_video_note",
            "gif" => "feature-dialog-dialog_media_gif",
            "audio" => "feature-dialog-dialog_media_audio",
            "voice" => "feature-dialog-dialog_media_voice",
            "document" => "feature-dialog-dialog_media_document",
            "sticker" => "feature-dialog-dialog_media_sticker",
            "webpage" => "feature-dialog-dialog_media_link",
            "poll" => "feature-dialog-dialog_media_poll",
            "todo" => "feature-dialog-dialog_media_checklist",
            "contact" => "feature-dialog-dialog_media_contact",
            "geo" | "venue" => "feature-dialog-dialog_media_location",
            "dice" => "feature-dialog-dialog_media_dice",
            "giveaway" => "feature-dialog-dialog_media_giveaway",
            "giveaway_results" => "feature-dialog-dialog_media_giveaway_results",
            "paid_media" => "feature-dialog-dialog_media_paid_media",
            "game" => "feature-dialog-dialog_media_game",
            "invoice" => "feature-dialog-dialog_media_invoice",
            "story" => "feature-dialog-dialog_media_story",
            "video_stream" => "feature-dialog-dialog_media_video_stream",
            _ => "core-ui-media_kind_unknown",
        };
        let webpage = (kind == "webpage")
            .then(|| {
                message
                    .file_name
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok())
            })
            .flatten();
        let interaction = parse_interaction(message);
        Some(Self {
            kind_key,
            album_position,
            media_cache_key: message.media_cache_key.clone(),
            file_name: (kind != "webpage" && interaction.is_none())
                .then(|| message.file_name.clone())
                .flatten(),
            file_size: message.file_size,
            duration: message.media_duration,
            width: message.media_width,
            height: message.media_height,
            supports_streaming: message.supports_streaming,
            protected: message.noforwards || message.media_preview_restricted == Some(true),
            webpage,
            interaction,
        })
    }

    fn supported(&self) -> bool {
        self.kind_key != "core-ui-media_kind_unknown"
    }
}

#[derive(Default)]
struct InlineState {
    username: String,
    bot_id: i64,
    query: String,
    next_offset: String,
    query_id: i64,
    results: Vec<mono_proto_client::InlineBotResultDto>,
}

fn inline_bot_send(
    inline: &InlineState,
    index: usize,
    chat_id: i64,
    reply_to_msg_id: i32,
    top_msg_id: i32,
) -> Option<SpecialSend> {
    let result = inline.results.get(index)?;
    if chat_id == 0 || inline.query_id == 0 || result.id.is_empty() {
        return None;
    }
    Some(SpecialSend {
        chat_id,
        reply_to_msg_id,
        top_msg_id,
        silent: false,
        schedule_date: None,
        content: SpecialContent::InlineBotResult {
            query_id: inline.query_id,
            result_id: result.id.clone(),
        },
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstantMediaTarget {
    Message(i64),
    Peer(i64),
}

fn instant_media_target(key: &str) -> Option<InstantMediaTarget> {
    let (kind, rest) = key.split_once(':')?;
    let id = rest.split(':').next()?.parse::<i64>().ok()?;
    if id == 0 {
        return None;
    }
    match kind {
        "photo" | "doc" | "geo" => Some(InstantMediaTarget::Message(id)),
        "avatar" => Some(InstantMediaTarget::Peer(id)),
        _ => None,
    }
}

#[derive(Clone, PartialEq, Eq)]
struct MentionChoice {
    peer_id: i64,
    title: String,
    username: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Emoji,
    Stickers,
    Gifs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChecklistAction {
    Toggle(i32),
    Append(Vec<String>),
    EditTitle(String),
    EditItem(i32, String),
    DeleteItem(i32),
}

#[derive(Default)]
struct SpecialState {
    poll_selections: BTreeMap<(i64, i32), BTreeSet<Vec<u8>>>,
    busy: BTreeSet<i32>,
    poll_voters: BTreeMap<i32, mono_proto_client::adapter::messages::extras_rpc::PollVotersDto>,
    poll_next: BTreeMap<i32, Option<String>>,
    reaction_peers: BTreeMap<i32, ReactionPeersPage>,
    reaction_next: BTreeMap<i32, Option<String>>,
    read_receipts: BTreeMap<i32, OutboxRead>,
    forums: BTreeMap<i32, mono_proto_client::ForumTopicDto>,
    inline: Option<InlineState>,
    mention_query: Option<String>,
    mentions: Vec<MentionChoice>,
    sticker_sets: Vec<mono_proto_client::StickerPackDto>,
    sticker_pack: Option<mono_proto_client::StickerPackDto>,
    gifs: Vec<mono_proto_client::SavedGifDto>,
    reaction_choices: Vec<mono_proto_client::ReactionChoiceDto>,
    picker: Option<PickerKind>,
    instant_media_key: Option<String>,
    callback_notice: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum MediaStatus {
    #[default]
    Missing,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MediaPurpose {
    Thumb,
    Display,
    Full,
}

impl MediaPurpose {
    fn download_kind(self) -> MediaDownloadKind {
        match self {
            Self::Thumb => MediaDownloadKind::Thumb,
            Self::Display => MediaDownloadKind::Display,
            Self::Full => MediaDownloadKind::Full,
        }
    }
}

#[derive(Default)]
struct MediaTransfer {
    status: MediaStatus,
    transfer_id: Option<u64>,
    downloaded: u64,
    total: Option<u64>,
    lease: Option<crate::media::cache::CacheLease>,
}

struct StreamTransfer {
    path: std::path::PathBuf,
    next_offset: u64,
    total: Option<u64>,
    transfer_id: Option<u64>,
    started: bool,
    finished: bool,
}

struct StreamChunk {
    transfer_id: u64,
    cache_key: String,
    offset: u64,
    bytes: Vec<u8>,
    done: bool,
    total: Option<u64>,
}

struct StickerSlot {
    view: StickerView,
    cache_key: String,
}

fn media_action_allowed(summary: &MediaSummary, action: MediaAction, status: MediaStatus) -> bool {
    let downloadable = summary.media_cache_key.is_some();
    match action {
        MediaAction::Download => downloadable && status == MediaStatus::Missing,
        MediaAction::CancelDownload => downloadable && status == MediaStatus::Loading,
        MediaAction::RetryDownload => downloadable && status == MediaStatus::Failed,
        MediaAction::Save | MediaAction::SaveAs | MediaAction::OpenExternal => {
            downloadable && status == MediaStatus::Ready && !summary.protected
        }
        MediaAction::Copy => {
            downloadable
                && status == MediaStatus::Ready
                && !summary.protected
                && matches!(
                    summary.kind_key,
                    "feature-dialog-dialog_media_photo"
                        | "feature-dialog-dialog_media_video"
                        | "feature-dialog-dialog_media_video_note"
                        | "feature-dialog-dialog_media_gif"
                        | "feature-dialog-dialog_media_sticker"
                )
        }
        MediaAction::View => {
            downloadable
                && status == MediaStatus::Ready
                && matches!(
                    summary.kind_key,
                    "feature-dialog-dialog_media_photo"
                        | "feature-dialog-dialog_media_video"
                        | "feature-dialog-dialog_media_video_note"
                        | "feature-dialog-dialog_media_gif"
                        | "feature-dialog-dialog_media_sticker"
                        | "feature-dialog-dialog_media_link"
                )
        }
        MediaAction::Play => {
            downloadable
                && (status == MediaStatus::Ready || summary.supports_streaming)
                && matches!(
                    summary.kind_key,
                    "feature-dialog-dialog_media_video"
                        | "feature-dialog-dialog_media_video_note"
                        | "feature-dialog-dialog_media_gif"
                        | "feature-dialog-dialog_media_audio"
                        | "feature-dialog-dialog_media_voice"
                )
        }
    }
}

struct DialogExtra {
    generation: u64,
    next_token: u64,
    requests: BTreeMap<u64, DialogRequest>,
    thread: Option<(i64, i32)>,
    histories: BTreeMap<(i64, i32), History>,
    drafts: BTreeMap<(i64, i32), (CloudDraft, DraftGuard)>,
    draft: DraftGuard,
    original_draft: Option<CloudDraft>,
    draft_inflight: bool,
    debounce: Option<gtk::glib::SourceId>,
    suppress: Rc<Cell<bool>>,
    visibility_source: Option<gtk::glib::SourceId>,
    active: bool,
    read_requested: i32,
    pinned: BTreeMap<i32, MessageDto>,
    scheduled: BTreeMap<i32, ScheduledMessageDto>,
    scheduled_snapshot: Option<ScheduledMessageDto>,
    queue_action: Option<(i32, ScheduledAction)>,
    queue_busy: bool,
    discard_confirm: bool,
    focus_message: Option<i32>,
    notice: Option<&'static str>,
    root: gtk::Box,
    scroller: gtk::ScrolledWindow,
    pinned_rows: gtk::Box,
    scheduled_rows: gtk::Box,
    leave_thread: gtk::Button,
    draft_status: gtk::Label,
    discard: gtk::Button,
}

impl DialogExtra {
    fn new(
        root: &gtk::Box,
        scroller: &gtk::ScrolledWindow,
        i18n: &I18n,
        sender: &ComponentSender<DialogPage>,
    ) -> Self {
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let leave_thread = gtk::Button::with_label(&localized(i18n, "desktop-back"));
        connect(&leave_thread, sender, || DialogInput::LeaveThread);
        toolbar.append(&leave_thread);
        let pinned = gtk::Button::with_label(&localized(i18n, "feature-dialog-dialog_pinned_list"));
        connect(&pinned, sender, || DialogInput::LoadPinned);
        toolbar.append(&pinned);
        let scheduled =
            gtk::Button::with_label(&localized(i18n, "feature-dialog-dialog_scheduled_messages"));
        connect(&scheduled, sender, || DialogInput::LoadScheduled);
        toolbar.append(&scheduled);
        root.append(&toolbar);
        let pinned_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let scheduled_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for (key, rows) in [
            ("feature-dialog-dialog_pinned_list", &pinned_rows),
            ("feature-dialog-dialog_scheduled_messages", &scheduled_rows),
        ] {
            let expander = gtk::Expander::new(Some(&localized(i18n, key)));
            let scroll = gtk::ScrolledWindow::builder()
                .max_content_height(160)
                .propagate_natural_height(true)
                .child(rows)
                .build();
            expander.set_child(Some(&scroll));
            root.append(&expander);
        }
        let draft_status = gtk::Label::new(None);
        draft_status.set_wrap(true);
        root.append(&draft_status);
        let discard =
            gtk::Button::with_label(&localized(i18n, "feature-dialog-dialog_discard_draft"));
        connect(&discard, sender, || DialogInput::DiscardDraft);
        root.append(&discard);
        let input = sender.input_sender().clone();
        scroller
            .vadjustment()
            .connect_value_changed(move |_| input.emit(DialogInput::ViewportChanged));
        let input = sender.input_sender().clone();
        let visibility_source =
            gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
                input.emit(DialogInput::ViewportChanged);
                gtk::glib::ControlFlow::Continue
            });
        Self {
            generation: 0,
            next_token: 0,
            requests: BTreeMap::new(),
            thread: None,
            histories: BTreeMap::new(),
            drafts: BTreeMap::new(),
            draft: DraftGuard::default(),
            original_draft: None,
            draft_inflight: false,
            debounce: None,
            suppress: Rc::new(Cell::new(false)),
            visibility_source: Some(visibility_source),
            active: true,
            read_requested: 0,
            pinned: BTreeMap::new(),
            scheduled: BTreeMap::new(),
            scheduled_snapshot: None,
            queue_action: None,
            queue_busy: false,
            discard_confirm: false,
            focus_message: None,
            notice: None,
            root: root.clone(),
            scroller: scroller.clone(),
            pinned_rows,
            scheduled_rows,
            leave_thread,
            draft_status,
            discard,
        }
    }
}

pub struct DialogPage {
    extra: DialogExtra,
    i18n: Rc<I18n>,
    wallpaper_config: WallpaperConfig,
    wallpaper: gtk::Picture,
    chat: Option<ChatDto>,
    history: History,
    pending: BTreeMap<i64, PendingRow>,
    submitted: VecDeque<TextSend>,
    rows: BTreeMap<i32, gtk::Box>,
    pending_rows: gtk::Box,
    list: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    spinner: gtk::Spinner,
    load: gtk::Button,
    send: gtk::Button,
    attach: gtk::MenuButton,
    attachments: Vec<SelectedAttachment>,
    attachment_rows: gtk::Box,
    media_send: Option<MediaSend>,
    media_album_id: Option<i64>,
    media_submitting: bool,
    pending_media: BTreeMap<i64, PendingMediaSend>,
    media_transfers: BTreeMap<(i64, i32, MediaPurpose), MediaTransfer>,
    streams: BTreeMap<(i64, i32), StreamTransfer>,
    custom_emoji: BTreeMap<i64, MediaTransfer>,
    sticker_views: BTreeMap<(i64, i32), StickerSlot>,
    custom_emoji_views: BTreeMap<(i64, i32, i64), StickerSlot>,
    media_sticker_viewer: Option<StickerView>,
    player: Controller<MediaPlayer>,
    media_viewer: Option<gtk::Window>,
    special: SpecialState,
    web: Controller<WebView>,
    web_open: bool,
    suggestion_rows: gtk::Box,
    picker_rows: gtk::Box,
    forum_expander: gtk::Expander,
    forum_rows: gtk::Box,
    editor: gtk::TextView,
    reply: gtk::Entry,
    top: gtk::Entry,
    schedule: gtk::Entry,
    silent: gtk::CheckButton,
    markdown: gtk::CheckButton,
    options: gtk::Expander,
    action_label: gtk::Label,
    target: gtk::Entry,
    revoke: gtk::CheckButton,
    cancel: gtk::Button,
    action: Option<(i32, MessageAction)>,
    busy: bool,
    online: bool,
    error: Option<ClientError>,
    exhausted: bool,
}

impl SimpleComponent for DialogPage {
    type Init = Rc<I18n>;
    type Input = DialogInput;
    type Output = DialogOutput;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 8)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tr = |key: &str| localized(&i18n, key);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back = gtk::Button::with_label(&tr("desktop-back"));
        connect(&back, &sender, || DialogInput::Back);
        header.append(&back);
        let title = gtk::Label::new(None);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.add_css_class("title-2");
        header.append(&title);
        root.append(&header);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.update_property(&[gtk::accessible::Property::Label(&tr("desktop-message"))]);
        root.append(&status);
        let spinner = gtk::Spinner::new();
        spinner.update_property(&[gtk::accessible::Property::Label(&tr("desktop-loading"))]);
        root.append(&spinner);
        let load = gtk::Button::with_label(&tr("feature-dialog-dialog_peer_list_load_more"));
        connect(&load, &sender, || DialogInput::LoadMore);
        root.append(&load);
        let scroller = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        list.update_property(&[gtk::accessible::Property::Label(&tr("desktop-message"))]);
        let pending_rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let timeline = gtk::Box::new(gtk::Orientation::Vertical, 8);
        timeline.append(&list);
        timeline.append(&pending_rows);
        scroller.set_child(Some(&timeline));
        root.append(&scroller);
        let action_label = gtk::Label::new(None);
        action_label.set_xalign(0.0);
        root.append(&action_label);
        let target = gtk::Entry::new();
        root.append(&target);
        let revoke = gtk::CheckButton::with_label(&tr("feature-dialog-dialog_delete_for_everyone"));
        root.append(&revoke);
        let editor = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .top_margin(8)
            .bottom_margin(8)
            .left_margin(8)
            .right_margin(8)
            .build();
        editor.update_property(&[gtk::accessible::Property::Label(&tr("desktop-message"))]);
        let edit_scroll = gtk::ScrolledWindow::builder()
            .min_content_height(72)
            .max_content_height(180)
            .propagate_natural_height(true)
            .child(&editor)
            .build();
        let attachment_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.append(&attachment_rows);
        root.append(&edit_scroll);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let input = sender.input_sender().clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter)
                && !modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK)
            {
                input.emit(DialogInput::Submit);
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        editor.add_controller(keys);
        let options = gtk::Expander::new(Some(&tr("feature-dialog-dialog_editor_title")));
        let fields = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let reply = numeric_field(&fields, &tr("feature-dialog-dialog_reply"));
        let top = numeric_field(&fields, &tr("feature-dialog-dialog_comments_title"));
        let schedule = numeric_field(&fields, &tr("feature-dialog-dialog_send_schedule"));
        schedule.set_placeholder_text(Some("0"));
        let silent = gtk::CheckButton::with_label(&tr("feature-dialog-dialog_send_silent"));
        fields.append(&silent);
        let markdown = gtk::CheckButton::with_label(&tr("feature-dialog-dialog_markdown_editor"));
        fields.append(&markdown);
        options.set_child(Some(&fields));
        root.append(&options);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let attach = gtk::MenuButton::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text(tr("feature-dialog-dialog_attach"))
            .build();
        attach.update_property(&[gtk::accessible::Property::Label(&tr(
            "feature-dialog-dialog_attach",
        ))]);
        let attach_popover = gtk::Popover::new();
        let attach_actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for kind in [
            AttachmentKind::Photo,
            AttachmentKind::Video,
            AttachmentKind::Document,
        ] {
            let button = gtk::Button::with_label(&tr(kind.localization_key()));
            let input = sender.input_sender().clone();
            let popover = attach_popover.clone();
            button.connect_clicked(move |_| {
                popover.popdown();
                input.emit(DialogInput::ChooseAttachments(kind));
            });
            attach_actions.append(&button);
        }
        attach_popover.set_child(Some(&attach_actions));
        attach.set_popover(Some(&attach_popover));
        buttons.append(&attach);
        for (kind, key) in [
            (PickerKind::Emoji, "feature-dialog-dialog_emoji_open"),
            (PickerKind::Stickers, "feature-dialog-dialog_panel_stickers"),
            (PickerKind::Gifs, "feature-dialog-dialog_saved_gifs"),
        ] {
            let button = gtk::Button::with_label(&tr(key));
            connect(&button, &sender, move || DialogInput::OpenPicker(kind));
            buttons.append(&button);
        }
        let cancel = gtk::Button::with_label(&tr("desktop-cancel"));
        connect(&cancel, &sender, || DialogInput::Cancel);
        buttons.append(&cancel);
        let send = gtk::Button::with_label(&tr("desktop-send"));
        send.add_css_class("suggested-action");
        connect(&send, &sender, || DialogInput::Submit);
        buttons.append(&send);
        root.append(&buttons);
        let player = MediaPlayer::builder().launch(i18n.clone()).detach();
        root.append(player.widget());
        let suggestion_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        suggestion_rows.set_visible(false);
        root.append(&suggestion_rows);
        let picker_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        picker_rows.set_visible(false);
        root.append(&picker_rows);
        let forum_expander = gtk::Expander::new(Some(&tr("feature-dialog-dialog_comments_title")));
        forum_expander.set_visible(false);
        let forum_rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        forum_expander.set_child(Some(&forum_rows));
        root.append(&forum_expander);
        let web =
            WebView::builder()
                .launch(i18n.clone())
                .forward(sender.input_sender(), |output| match output {
                    WebOutput::Back => DialogInput::WebBack,
                    WebOutput::Command(command) => DialogInput::WebCommand(command),
                    WebOutput::OpenExternal(uri) => DialogInput::WebExternal(uri),
                    WebOutput::OpenMedia(key) => DialogInput::WebMedia(key),
                });
        web.widget().set_visible(false);
        root.append(web.widget());
        let extra = DialogExtra::new(&root, &scroller, &i18n, &sender);
        let input = sender.input_sender().clone();
        let suppress = extra.suppress.clone();
        editor.buffer().connect_changed(move |_| {
            if !suppress.get() {
                input.emit(DialogInput::ComposerChanged);
            }
        });
        let input = sender.input_sender().clone();
        let suppress = extra.suppress.clone();
        reply.connect_changed(move |_| {
            if !suppress.get() {
                input.emit(DialogInput::ComposerChanged);
            }
        });
        let input = sender.input_sender().clone();
        let suppress = extra.suppress.clone();
        markdown.connect_toggled(move |_| {
            if !suppress.get() {
                input.emit(DialogInput::ComposerChanged);
            }
        });
        top.set_editable(false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_hexpand(true);
        content.set_vexpand(true);
        while let Some(child) = root.first_child() {
            root.remove(&child);
            content.append(&child);
        }
        let wallpaper = gtk::Picture::new();
        wallpaper.set_widget_name("dialog-wallpaper");
        wallpaper.set_content_fit(gtk::ContentFit::Cover);
        wallpaper.set_can_shrink(true);
        wallpaper.set_hexpand(true);
        wallpaper.set_vexpand(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&wallpaper));
        overlay.add_overlay(&content);
        root.append(&overlay);
        let model = Self {
            extra,
            i18n,
            wallpaper_config: WallpaperConfig::default(),
            wallpaper,
            chat: None,
            history: History::default(),
            pending: BTreeMap::new(),
            submitted: VecDeque::new(),
            rows: BTreeMap::new(),
            list,
            pending_rows,
            title,
            status,
            spinner,
            load,
            send,
            attach,
            attachments: Vec::new(),
            attachment_rows,
            media_send: None,
            media_album_id: None,
            media_submitting: false,
            pending_media: BTreeMap::new(),
            media_transfers: BTreeMap::new(),
            streams: BTreeMap::new(),
            custom_emoji: BTreeMap::new(),
            sticker_views: BTreeMap::new(),
            custom_emoji_views: BTreeMap::new(),
            media_sticker_viewer: None,
            player,
            media_viewer: None,
            special: SpecialState::default(),
            web,
            web_open: false,
            suggestion_rows,
            picker_rows,
            forum_expander,
            forum_rows,
            editor,
            reply,
            top,
            schedule,
            silent,
            markdown,
            options,
            action_label,
            target,
            revoke,
            cancel,
            action: None,
            busy: false,
            online: true,
            error: None,
            exhausted: false,
        };
        model.update_wallpaper();
        model.refresh(&sender);
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        match input {
            DialogInput::SetWallpaper(config) => {
                self.wallpaper_config = config;
                self.update_wallpaper();
            }
            DialogInput::SetForwardRecipient(profile) => {
                if profile.id != 0
                    && self
                        .action
                        .is_some_and(|(_, action)| action == MessageAction::Forward)
                {
                    self.target.set_text(&profile.id.to_string());
                }
            }
            DialogInput::Response { token, response } => self.response(token, response, &sender),
            DialogInput::Failed { token, error } => self.failed(token, error, &sender),
            DialogInput::SetActive(active) => {
                if !active {
                    self.flush_draft(&sender);
                }
                self.extra.active = active;
                for slot in self
                    .sticker_views
                    .values()
                    .chain(self.custom_emoji_views.values())
                {
                    slot.view.set_active(active);
                }
            }
            DialogInput::DraftChanged(value) => self.apply_draft(value),
            DialogInput::ScheduledChanged(value) => {
                if self
                    .context()
                    .is_some_and(|context| context.0 == value.message.chat_id)
                {
                    self.extra.scheduled.insert(value.message.id, value);
                }
            }
            DialogInput::ScheduledDeleted {
                chat_id,
                message_ids,
            } => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    for id in message_ids {
                        self.extra.scheduled.remove(&id);
                    }
                }
            }
            DialogInput::OpenThread {
                chat_id,
                message_id,
            } => {
                if chat_id != 0 && message_id > 0 {
                    self.stash_context(&sender);
                    self.extra.thread = Some((chat_id, message_id));
                    self.restore_context(&sender);
                }
            }
            DialogInput::LeaveThread => {
                if self.extra.thread.is_some() {
                    self.stash_context(&sender);
                    self.extra.thread = None;
                    self.restore_context(&sender);
                }
            }
            DialogInput::LoadPinned => {
                if let Some((chat_id, _)) = self.context() {
                    self.command(
                        WorkerCommand::GetPinnedMessages {
                            chat_id,
                            limit: 100,
                        },
                        Operation::Pinned,
                        &sender,
                    );
                }
            }
            DialogInput::JumpTo(id) => self.jump_to(id, &sender),
            DialogInput::LoadScheduled => {
                if let Some((chat_id, _)) = self.context() {
                    self.command(
                        WorkerCommand::GetCachedScheduledHistory { chat_id },
                        Operation::ScheduledLoad,
                        &sender,
                    );
                    self.command(
                        WorkerCommand::GetScheduledHistory { chat_id },
                        Operation::ScheduledLoad,
                        &sender,
                    );
                }
            }
            DialogInput::QueueAction { message_id, action } => {
                self.begin_queue_action(message_id, action)
            }
            DialogInput::DiscardDraft => self.extra.discard_confirm = true,
            DialogInput::ComposerChanged => {
                self.schedule_draft(&sender);
                self.update_suggestions(&sender);
                self.refresh(&sender);
                return;
            }
            DialogInput::DraftDebounced {
                generation,
                revision,
            } => {
                if generation == self.extra.generation && revision == self.extra.draft.revision {
                    self.extra.debounce = None;
                    self.flush_draft(&sender);
                }
            }
            DialogInput::ViewportChanged => {
                self.mark_visible(&sender);
                return;
            }
            DialogInput::SetChat(chat) => {
                let changed = self.chat.as_ref().is_none_or(|old| old.id != chat.id);
                if changed {
                    self.stash_context(&sender);
                    self.extra.thread = None;
                }
                self.chat = Some(chat);
                if changed {
                    self.restore_context(&sender);
                }
            }
            DialogInput::SetHistory {
                chat_id,
                messages,
                append,
            } => {
                if self.extra.thread.is_none() && self.context() == Some((chat_id, 0)) {
                    self.busy = false;
                    if append && messages.is_empty() {
                        self.exhausted = true;
                    }
                    for message in messages
                        .into_iter()
                        .filter(|message| message.chat_id == chat_id)
                    {
                        self.history.insert(message, false);
                    }
                }
            }
            DialogInput::UpsertMessage(message) => {
                if in_context(&message, self.context()) {
                    self.history.insert(message, true);
                } else {
                    for (context, history) in &mut self.extra.histories {
                        if in_context(&message, Some(*context)) {
                            history.insert(message.clone(), true);
                        }
                    }
                }
            }
            DialogInput::RemoveMessage {
                chat_id,
                message_id,
            } => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    self.history.delete(message_id);
                }
                for (context, history) in &mut self.extra.histories {
                    if context.0 == chat_id {
                        history.delete(message_id);
                    }
                }
            }
            DialogInput::SetBusy(busy) => self.busy = busy,
            DialogInput::SetError(error) => {
                if error.is_some() {
                    self.submitted.clear();
                }
                self.error = error;
                self.busy = false;
            }
            DialogInput::MediaNotice(key) => self.extra.notice = Some(key),
            DialogInput::SetOnline(online) => self.online = online,
            DialogInput::SetPending(pending) => {
                self.pending = pending
                    .into_iter()
                    .map(|pending| {
                        (
                            pending.random_id,
                            PendingRow {
                                message: pending.message,
                                failed: false,
                                uncertain: pending.uncertain,
                            },
                        )
                    })
                    .collect();
            }
            DialogInput::SetPendingMedia(pending) => {
                self.pending_media = pending
                    .into_iter()
                    .map(|pending| (pending.album_id, pending))
                    .collect();
            }
            DialogInput::MediaQueued {
                token,
                album_id,
                random_ids,
            } => {
                let current = self.extra.requests.get(&token).is_some_and(|request| {
                    request.generation == self.extra.generation
                        && matches!(request.operation, Operation::MediaSend)
                });
                if current {
                    self.media_album_id = Some(album_id);
                    if let Some(send) = &self.media_send {
                        self.pending_media.insert(
                            album_id,
                            PendingMediaSend {
                                album_id,
                                random_ids,
                                send: send.clone(),
                                uncertain: false,
                            },
                        );
                    }
                }
            }
            DialogInput::MediaProgress {
                token,
                transfer_id,
                downloaded,
                total,
            } => self.media_progress(token, transfer_id, downloaded, total),
            DialogInput::OutgoingQueued { random_id } => {
                if let Some(message) = self.submitted.pop_front() {
                    if self.context() == Some((message.chat_id, message.top_msg_id)) {
                        self.extra.suppress.set(true);
                        self.editor.buffer().set_text("");
                        self.reply.set_text("");
                        self.schedule.set_text("");
                        self.extra.original_draft = None;
                        self.extra.suppress.set(false);
                        self.extra.draft.changed();
                        self.flush_draft(&sender);
                    }
                    self.pending.insert(
                        random_id,
                        PendingRow {
                            message,
                            failed: false,
                            uncertain: false,
                        },
                    );
                }
            }
            DialogInput::OutgoingResult { random_id, result } => match result {
                Ok(OutgoingMessageResult::Sent { message }) => {
                    self.pending.remove(&random_id);
                    if in_context(&message, self.context()) {
                        self.history.insert(message, true);
                    }
                }
                Ok(OutgoingMessageResult::Scheduled { message }) => {
                    self.pending.remove(&random_id);
                    if self
                        .context()
                        .is_some_and(|context| context.0 == message.message.chat_id)
                    {
                        self.extra.scheduled.insert(message.message.id, message);
                    }
                }
                Err(error) => {
                    if let Some(pending) = self.pending.get_mut(&random_id) {
                        pending.failed = true;
                        pending.uncertain = true;
                    }
                    self.error = Some(error);
                }
            },
            DialogInput::Back => {
                self.stop_playback(&sender);
                self.web_open = false;
                self.stash_context(&sender);
                self.extra.generation = self.extra.generation.saturating_add(1);
                self.extra.requests.clear();
                self.chat = None;
                self.extra.thread = None;
                let _ = sender.output(DialogOutput::ClearActiveDialog);
                let _ = sender.output(DialogOutput::Back);
            }
            DialogInput::StopPlayback => self.stop_playback(&sender),
            DialogInput::LoadMore => self.load_history(true, &sender),
            DialogInput::Submit => {
                let result = if self.extra.queue_action.is_some() || self.extra.discard_confirm {
                    self.submit_queue(&sender)
                } else {
                    self.submit(&sender)
                };
                if let Err(error) = result {
                    self.error = Some(error);
                }
            }
            DialogInput::Cancel => {
                self.cancel_action();
                self.attachments.clear();
                if self.extra.draft.dirty() {
                    self.flush_draft(&sender);
                }
            }
            DialogInput::Action { message_id, action } => self.begin_action(message_id, action),
            DialogInput::Media { message_id, action } => {
                self.media_action(message_id, action, &sender)
            }
            DialogInput::OpenGeo(message_id) => self.open_geo(message_id, &sender),
            DialogInput::TogglePollOption { message_id, index } => {
                self.toggle_poll_option(message_id, index)
            }
            DialogInput::SubmitPollVote(message_id) => self.submit_poll_vote(message_id, &sender),
            DialogInput::RetractPollVote(message_id) => self.retract_poll_vote(message_id, &sender),
            DialogInput::ClosePoll(message_id) => self.close_poll(message_id, &sender),
            DialogInput::LoadPollVoters { message_id, append } => {
                self.load_poll_voters(message_id, append, &sender)
            }
            DialogInput::Checklist { message_id, action } => {
                self.checklist_action(message_id, action, &sender)
            }
            DialogInput::ReactionPeers { message_id, append } => {
                self.load_reaction_peers(message_id, append, &sender)
            }
            DialogInput::QuickReact {
                message_id,
                emoticon,
                document_id,
            } => self.quick_react(message_id, emoticon, document_id, &sender),
            DialogInput::LoadReadReceipt(message_id) => self.load_read_receipt(message_id, &sender),
            DialogInput::LoadForums => self.load_forums(&sender),
            DialogInput::OpenForumTopic(topic_id) => self.open_forum_topic(topic_id, &sender),
            DialogInput::ToggleGeneralHidden => self.toggle_general_hidden(&sender),
            DialogInput::OpenPicker(kind) => self.open_picker(kind, &sender),
            DialogInput::OpenStickerPack(index) => self.open_sticker_pack(index, &sender),
            DialogInput::LoadInlineNext => self.load_inline_next(&sender),
            DialogInput::InsertEmoji(value) => self.insert_composer_text(&value),
            DialogInput::InsertInlineResult(index) => self.insert_inline_result(index, &sender),
            DialogInput::SelectMention(index) => self.select_mention(index),
            DialogInput::SendSticker(document_id) => self.send_sticker(document_id, false, &sender),
            DialogInput::SendGif(document_id) => self.send_sticker(document_id, true, &sender),
            DialogInput::Keyboard {
                message_id,
                row,
                index,
            } => self.keyboard_action(message_id, row, index, &sender),
            DialogInput::OpenInstantView(message_id) => self.open_instant_view(message_id, &sender),
            DialogInput::WebBack => self.web_open = false,
            DialogInput::WebCommand(command) => {
                self.command(WorkerCommand::Web(command), Operation::InstantView, &sender)
            }
            DialogInput::WebExternal(uri) => self.launch_uri(&uri, &sender),
            DialogInput::WebMedia(key) => self.open_instant_media(key, &sender),
            DialogInput::RetryMedia(album_id) => {
                let pending = (!self.media_submitting)
                    .then(|| self.pending_media.get(&album_id).cloned())
                    .flatten()
                    .filter(|pending| {
                        self.context().is_some_and(|context| {
                            (context.0, context.1)
                                == (pending.send.chat_id, pending.send.top_msg_id)
                        })
                    });
                if let Some(pending) = pending {
                    self.media_album_id = Some(album_id);
                    self.media_send = Some(pending.send);
                    self.media_submitting = true;
                    self.command(
                        WorkerCommand::RetryMedia { album_id },
                        Operation::MediaSend,
                        &sender,
                    );
                }
            }
            DialogInput::ChooseAttachments(kind) => self.choose_attachments(kind, &sender),
            DialogInput::AttachmentsChosen(additions) => {
                match extend_attachments(&self.attachments, additions) {
                    Ok(attachments) => {
                        self.attachments = attachments;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            DialogInput::RemoveAttachment(index) => {
                if index < self.attachments.len() {
                    self.attachments.remove(index);
                }
            }
            DialogInput::Retry(random_id) => {
                if let Some(pending) = self.pending.get_mut(&random_id)
                    && (pending.failed || pending.uncertain)
                {
                    pending.failed = false;
                    pending.uncertain = false;
                    let _ = sender.output(DialogOutput::RetryText(random_id));
                }
            }
        }
        self.sync_rows(&sender);
        self.refresh(&sender);
        self.focus_message();
    }
}

impl DialogPage {
    fn update_wallpaper(&self) {
        let texture = wallpaper_texture(&self.wallpaper_config, 960, 640);
        self.wallpaper.set_paintable(texture.as_ref());
        self.wallpaper
            .set_visible(self.wallpaper_config.mode != WallpaperMode::None && texture.is_some());
    }

    fn context(&self) -> Option<(i64, i32)> {
        self.extra
            .thread
            .or_else(|| self.chat.as_ref().map(|chat| (chat.id, 0)))
    }

    fn command(
        &mut self,
        command: WorkerCommand,
        operation: Operation,
        sender: &ComponentSender<Self>,
    ) {
        self.extra.next_token = self.extra.next_token.saturating_add(1);
        let token = self.extra.next_token;
        self.extra.requests.insert(
            token,
            DialogRequest {
                generation: self.extra.generation,
                operation,
            },
        );
        if sender
            .output(DialogOutput::Command { token, command })
            .is_err()
        {
            self.extra.requests.remove(&token);
            self.error = Some(ClientError::Stopped);
        }
    }

    fn ensure_media_previews(&mut self, sender: &ComponentSender<Self>) {
        let requests = self
            .history
            .messages
            .values()
            .filter(|message| {
                message.media_preview_restricted != Some(true)
                    && message.media_cache_key.is_some()
                    && matches!(
                        message.media_kind.as_deref(),
                        Some("photo" | "video" | "video_note" | "gif" | "sticker" | "webpage")
                    )
                    && !self.media_transfers.contains_key(&(
                        message.chat_id,
                        message.id,
                        MediaPurpose::Display,
                    ))
                    && !self.media_transfers.contains_key(&(
                        message.chat_id,
                        message.id,
                        MediaPurpose::Thumb,
                    ))
            })
            .take(8)
            .map(|message| (message.chat_id, message.id))
            .collect::<Vec<_>>();
        for (chat_id, message_id) in requests {
            self.start_media_download(
                chat_id,
                message_id,
                MediaPurpose::Display,
                MediaDownloadMode::Automatic,
                sender,
            );
        }
        let custom = self
            .history
            .messages
            .values()
            .filter(|message| message.media_preview_restricted != Some(true))
            .flat_map(|message| {
                custom_emoji_ids(
                    message.text.as_deref().unwrap_or_default(),
                    message.entities_json.as_deref(),
                )
            })
            .filter(|document_id| !self.custom_emoji.contains_key(document_id))
            .take(16)
            .collect::<Vec<_>>();
        for document_id in custom {
            self.custom_emoji.insert(
                document_id,
                MediaTransfer {
                    status: MediaStatus::Loading,
                    ..MediaTransfer::default()
                },
            );
            self.command(
                WorkerCommand::DownloadCustomEmoji {
                    document_id,
                    mode: MediaDownloadMode::Automatic,
                },
                Operation::CustomEmoji { document_id },
                sender,
            );
        }
    }

    fn ensure_sticker_views(&mut self) {
        let stickers = self
            .history
            .messages
            .values()
            .filter(|message| message.media_kind.as_deref() == Some("sticker"))
            .filter_map(|message| {
                let transfer = self
                    .media_transfers
                    .get(&(message.chat_id, message.id, MediaPurpose::Full))
                    .filter(|transfer| transfer.status == MediaStatus::Ready)?;
                let lease = transfer.lease.as_ref()?;
                let source = sticker_source(lease.path(), message.file_name.as_deref())?;
                let cache_key = message.media_cache_key.clone()?;
                Some((message.chat_id, message.id, cache_key, source))
            })
            .collect::<Vec<_>>();
        for (chat_id, message_id, cache_key, source) in stickers {
            let key = (chat_id, message_id);
            if self
                .sticker_views
                .get(&key)
                .is_some_and(|slot| slot.cache_key == cache_key)
            {
                continue;
            }
            let view =
                StickerView::with_max_dimension(&self.i18n, 256).expect("valid sticker dimensions");
            view.load(source, None);
            view.set_active(self.extra.active);
            self.sticker_views
                .insert(key, StickerSlot { view, cache_key });
        }
        let custom = self
            .history
            .messages
            .values()
            .flat_map(|message| {
                custom_emoji_ids(
                    message.text.as_deref().unwrap_or_default(),
                    message.entities_json.as_deref(),
                )
                .into_iter()
                .map(|document_id| (message.chat_id, message.id, document_id))
                .collect::<Vec<_>>()
            })
            .filter_map(|(chat_id, message_id, document_id)| {
                let transfer = self
                    .custom_emoji
                    .get(&document_id)
                    .filter(|transfer| transfer.status == MediaStatus::Ready)?;
                let source = sticker_source(transfer.lease.as_ref()?.path(), None)?;
                Some((chat_id, message_id, document_id, source))
            })
            .collect::<Vec<_>>();
        for (chat_id, message_id, document_id, source) in custom {
            let key = (chat_id, message_id, document_id);
            let cache_key = format!("custom-emoji:{document_id}");
            if self
                .custom_emoji_views
                .get(&key)
                .is_some_and(|slot| slot.cache_key == cache_key)
            {
                continue;
            }
            let view =
                StickerView::with_max_dimension(&self.i18n, 48).expect("valid sticker dimensions");
            view.load(source, None);
            view.set_active(self.extra.active);
            self.custom_emoji_views
                .insert(key, StickerSlot { view, cache_key });
        }
    }

    fn start_media_download(
        &mut self,
        chat_id: i64,
        message_id: i32,
        purpose: MediaPurpose,
        mode: MediaDownloadMode,
        sender: &ComponentSender<Self>,
    ) {
        let kind = purpose.download_kind();
        let transfer = self
            .media_transfers
            .entry((chat_id, message_id, purpose))
            .or_default();
        transfer.status = MediaStatus::Loading;
        transfer.downloaded = 0;
        transfer.total = None;
        self.command(
            WorkerCommand::DownloadMessageMedia {
                chat_id,
                message_id,
                kind,
                mode,
            },
            Operation::MediaDownload {
                chat_id,
                message_id,
                kind,
                mode,
            },
            sender,
        );
    }

    fn media_progress(
        &mut self,
        token: u64,
        transfer_id: u64,
        downloaded: u64,
        total: Option<u64>,
    ) {
        let Some(request) = self.extra.requests.get(&token) else {
            return;
        };
        if request.generation != self.extra.generation {
            return;
        }
        match request.operation {
            Operation::MediaDownload {
                chat_id,
                message_id,
                kind,
                ..
            } => {
                let purpose = match kind {
                    MediaDownloadKind::Thumb => MediaPurpose::Thumb,
                    MediaDownloadKind::Display => MediaPurpose::Display,
                    MediaDownloadKind::Full => MediaPurpose::Full,
                };
                let transfer = self
                    .media_transfers
                    .entry((chat_id, message_id, purpose))
                    .or_default();
                if transfer.status == MediaStatus::Loading {
                    transfer.transfer_id = Some(transfer_id);
                    transfer.downloaded = downloaded;
                    transfer.total = total;
                }
            }
            Operation::MediaRange {
                chat_id,
                message_id,
            } => {
                if let Some(stream) = self.streams.get_mut(&(chat_id, message_id))
                    && !stream.finished
                {
                    stream.transfer_id = Some(transfer_id);
                    stream.total = total;
                }
            }
            Operation::CustomEmoji { document_id } => {
                let transfer = self.custom_emoji.entry(document_id).or_default();
                if transfer.status == MediaStatus::Loading {
                    transfer.transfer_id = Some(transfer_id);
                    transfer.downloaded = downloaded;
                    transfer.total = total;
                }
            }
            _ => {}
        }
    }

    fn poll_for_message(&self, message_id: i32) -> Option<(i64, PollCard)> {
        let message = self.history.messages.get(&message_id)?;
        let Some(InteractionCard::Poll(poll)) = parse_interaction(message) else {
            return None;
        };
        Some((message.chat_id, poll))
    }

    fn toggle_poll_option(&mut self, message_id: i32, index: usize) {
        if self.special.busy.contains(&message_id) {
            return;
        }
        let Some((chat_id, poll)) = self.poll_for_message(message_id) else {
            return;
        };
        let Some(option) = poll.a.get(index).and_then(poll_option_bytes) else {
            return;
        };
        let selection = self
            .special
            .poll_selections
            .entry((chat_id, message_id))
            .or_default();
        if poll.m == 0 {
            selection.clear();
            selection.insert(option);
        } else if !selection.insert(option.clone()) {
            selection.remove(&option);
        }
    }

    fn begin_poll_mutation(
        &mut self,
        message_id: i32,
        command: WorkerCommand,
        sender: &ComponentSender<Self>,
    ) {
        self.special.busy.insert(message_id);
        self.command(command, Operation::PollMutation(message_id), sender);
    }

    fn submit_poll_vote(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some((chat_id, poll)) = self.poll_for_message(message_id) else {
            return;
        };
        let options = self
            .special
            .poll_selections
            .get(&(chat_id, message_id))
            .map(|selection| selection.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if self.special.busy.contains(&message_id) || !poll_vote_allowed(&poll, &options) {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        self.begin_poll_mutation(
            message_id,
            WorkerCommand::VotePoll {
                chat_id,
                message_id,
                options,
            },
            sender,
        );
    }

    fn retract_poll_vote(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some((chat_id, poll)) = self.poll_for_message(message_id) else {
            return;
        };
        if self.special.busy.contains(&message_id) || !poll_retract_allowed(&poll) {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        self.begin_poll_mutation(
            message_id,
            WorkerCommand::RetractPollVote {
                chat_id,
                message_id,
            },
            sender,
        );
    }

    fn close_poll(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some((chat_id, poll)) = self.poll_for_message(message_id) else {
            return;
        };
        if self.special.busy.contains(&message_id) || poll.creator == 0 || poll.x != 0 {
            self.error = Some(ClientError::Forbidden);
            return;
        }
        self.begin_poll_mutation(
            message_id,
            WorkerCommand::ClosePoll {
                chat_id,
                message_id,
            },
            sender,
        );
    }

    fn load_poll_voters(&mut self, message_id: i32, append: bool, sender: &ComponentSender<Self>) {
        let Some((chat_id, poll)) = self.poll_for_message(message_id) else {
            return;
        };
        if poll.o == 0 {
            return;
        }
        let offset = append
            .then(|| self.special.poll_next.get(&message_id).cloned().flatten())
            .flatten();
        if append && offset.is_none() {
            return;
        }
        self.command(
            WorkerCommand::PollVoters {
                chat_id,
                message_id,
                offset,
            },
            Operation::PollVoters { message_id, append },
            sender,
        );
    }

    fn checklist_action(
        &mut self,
        message_id: i32,
        action: ChecklistAction,
        sender: &ComponentSender<Self>,
    ) {
        if self.special.busy.contains(&message_id) {
            return;
        }
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(InteractionCard::Checklist(list)) = parse_interaction(message) else {
            return;
        };
        let chat_id = message.chat_id;
        let outgoing = message.outgoing;
        let command = match action {
            ChecklistAction::Toggle(item_id) => {
                if !outgoing && list.c == 0 {
                    return;
                }
                let Some(item) = list.i.iter().find(|item| item.id == item_id) else {
                    return;
                };
                let (completed, incompleted) = if item.d == 0 {
                    (vec![item_id], Vec::new())
                } else {
                    (Vec::new(), vec![item_id])
                };
                WorkerCommand::ToggleChecklistCompleted {
                    chat_id,
                    message_id,
                    completed,
                    incompleted,
                }
            }
            ChecklistAction::Append(titles) => {
                if !outgoing && list.a == 0 {
                    return;
                }
                let first_id = list.i.iter().map(|item| item.id).max().unwrap_or(0) + 1;
                if crate::domain::validate_append_checklist(first_id, &titles).is_err() {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                }
                WorkerCommand::AppendChecklist {
                    chat_id,
                    message_id,
                    first_id,
                    titles,
                }
            }
            ChecklistAction::EditTitle(text) => {
                if !outgoing {
                    return;
                }
                let edit = crate::domain::ChecklistTextEdit {
                    item_id: None,
                    text,
                    entities_json: None,
                    expected_text: list.t.clone(),
                    expected_entities_json: list.e.clone(),
                };
                if edit.validate(false).is_err() {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                }
                WorkerCommand::EditChecklistTitle {
                    chat_id,
                    message_id,
                    edit,
                }
            }
            ChecklistAction::EditItem(item_id, text) => {
                if !outgoing {
                    return;
                }
                let Some(item) = list.i.iter().find(|item| item.id == item_id) else {
                    return;
                };
                let edit = crate::domain::ChecklistTextEdit {
                    item_id: Some(item_id),
                    text,
                    entities_json: None,
                    expected_text: item.t.clone(),
                    expected_entities_json: item.e.clone(),
                };
                if edit.validate(false).is_err() {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                }
                WorkerCommand::EditChecklistItem {
                    chat_id,
                    message_id,
                    edit,
                }
            }
            ChecklistAction::DeleteItem(item_id) => {
                if !outgoing || list.i.len() <= 1 {
                    return;
                }
                let Some(item) = list.i.iter().find(|item| item.id == item_id) else {
                    return;
                };
                let edit = crate::domain::ChecklistTextEdit {
                    item_id: Some(item_id),
                    text: String::new(),
                    entities_json: None,
                    expected_text: item.t.clone(),
                    expected_entities_json: item.e.clone(),
                };
                if edit.validate(true).is_err() {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                }
                WorkerCommand::DeleteChecklistItem {
                    chat_id,
                    message_id,
                    edit,
                }
            }
        };
        self.special.busy.insert(message_id);
        self.command(command, Operation::ChecklistMutation(message_id), sender);
    }

    fn quick_react(
        &mut self,
        message_id: i32,
        emoticon: String,
        document_id: i64,
        sender: &ComponentSender<Self>,
    ) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        if (emoticon.is_empty() && document_id <= 0) || emoticon.encode_utf16().count() > 64 {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        let _ = sender.output(DialogOutput::React {
            chat_id: message.chat_id,
            message_id,
            emoticon,
            document_id,
        });
    }

    fn load_reaction_peers(
        &mut self,
        message_id: i32,
        append: bool,
        sender: &ComponentSender<Self>,
    ) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let offset = append
            .then(|| {
                self.special
                    .reaction_next
                    .get(&message_id)
                    .cloned()
                    .flatten()
            })
            .flatten();
        if append && offset.is_none() {
            return;
        }
        self.command(
            WorkerCommand::Interaction(InteractionCommand::ReactionPeers {
                chat_id: message.chat_id,
                message_id,
                offset,
            }),
            Operation::ReactionPeers { message_id, append },
            sender,
        );
    }

    fn load_read_receipt(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        if !message.outgoing {
            return;
        }
        self.command(
            WorkerCommand::Interaction(InteractionCommand::OutboxReadDate {
                chat_id: message.chat_id,
                message_id,
            }),
            Operation::ReadReceipt(message_id),
            sender,
        );
    }

    fn load_forums(&mut self, sender: &ComponentSender<Self>) {
        let Some(chat) = &self.chat else {
            return;
        };
        if !chat.is_forum {
            return;
        }
        self.command(
            WorkerCommand::Interaction(InteractionCommand::ForumTopics {
                chat_id: chat.id,
                offset_date: 0,
                offset_id: 0,
                offset_topic: 0,
                limit: 100,
            }),
            Operation::ForumTopics,
            sender,
        );
    }

    fn open_forum_topic(&mut self, topic_id: i32, sender: &ComponentSender<Self>) {
        let Some(chat_id) = self.context().map(|context| context.0) else {
            return;
        };
        if topic_id == 1 {
            sender.input_sender().emit(DialogInput::LeaveThread);
        } else if topic_id > 1 {
            sender.input_sender().emit(DialogInput::OpenThread {
                chat_id,
                message_id: topic_id,
            });
        }
    }

    fn toggle_general_hidden(&mut self, sender: &ComponentSender<Self>) {
        let Some(chat) = &self.chat else {
            return;
        };
        let hidden = !self
            .special
            .forums
            .get(&1)
            .is_some_and(|topic| topic.hidden);
        self.command(
            WorkerCommand::Interaction(InteractionCommand::SetGeneralTopicHidden {
                chat_id: chat.id,
                hidden,
            }),
            Operation::GeneralHidden,
            sender,
        );
    }

    fn insert_composer_text(&mut self, value: &str) {
        if value.is_empty() || value.len() > MAX_INPUT_BYTES {
            return;
        }
        self.editor.buffer().insert_at_cursor(value);
        self.editor.grab_focus();
    }

    fn open_picker(&mut self, kind: PickerKind, sender: &ComponentSender<Self>) {
        if self.special.picker == Some(kind) {
            self.special.picker = None;
            return;
        }
        self.special.picker = Some(kind);
        match kind {
            PickerKind::Emoji => self.command(
                WorkerCommand::Interaction(InteractionCommand::RecentReactions),
                Operation::RecentReactions,
                sender,
            ),
            PickerKind::Stickers => self.command(
                WorkerCommand::Interaction(InteractionCommand::StickerCatalog {
                    emoji: false,
                    hash: 0,
                }),
                Operation::StickerCatalog,
                sender,
            ),
            PickerKind::Gifs => self.command(
                WorkerCommand::Interaction(InteractionCommand::SavedGifs),
                Operation::SavedGifs,
                sender,
            ),
        }
    }

    fn open_sticker_pack(&mut self, index: usize, sender: &ComponentSender<Self>) {
        let Some(pack) = self.special.sticker_sets.get(index) else {
            return;
        };
        self.command(
            WorkerCommand::Interaction(InteractionCommand::StickerPack {
                set_id: pack.id,
                access_hash: pack.access_hash,
            }),
            Operation::StickerPack,
            sender,
        );
    }

    fn load_inline_next(&mut self, sender: &ComponentSender<Self>) {
        let Some(inline) = &self.special.inline else {
            return;
        };
        if inline.bot_id <= 0 || inline.next_offset.is_empty() {
            return;
        }
        let Some(chat_id) = self.context().map(|context| context.0) else {
            return;
        };
        self.command(
            WorkerCommand::Interaction(InteractionCommand::InlineBotResults {
                chat_id,
                bot_id: inline.bot_id,
                query: inline.query.clone(),
                offset: inline.next_offset.clone(),
            }),
            Operation::InlineResults,
            sender,
        );
    }

    fn insert_inline_result(&mut self, index: usize, sender: &ComponentSender<Self>) {
        let Some((chat_id, top_msg_id)) = self.context() else {
            return;
        };
        let reply_to_msg_id = optional_number(&self.reply.text())
            .ok()
            .flatten()
            .unwrap_or(0);
        let Some(send) = self.special.inline.as_ref().and_then(|inline| {
            inline_bot_send(inline, index, chat_id, reply_to_msg_id, top_msg_id)
        }) else {
            return;
        };
        self.command(
            WorkerCommand::SendSpecial(send),
            Operation::SendInlineResult,
            sender,
        );
    }

    fn select_mention(&mut self, index: usize) {
        let Some(username) = self
            .special
            .mentions
            .get(index)
            .and_then(|mention| mention.username.as_deref())
        else {
            return;
        };
        self.insert_composer_text(&format!("@{username} "));
    }

    fn send_sticker(&mut self, document_id: i64, gif: bool, sender: &ComponentSender<Self>) {
        let Some((chat_id, top_msg_id)) = self.context() else {
            return;
        };
        if document_id <= 0
            || !self
                .chat
                .as_ref()
                .is_some_and(|chat| chat.can_send_photos && !chat.left)
        {
            self.error = Some(ClientError::Forbidden);
            return;
        }
        let reply_to_msg_id = optional_number(&self.reply.text())
            .ok()
            .flatten()
            .unwrap_or(0);
        let command = if gif {
            WorkerCommand::SendSavedGif {
                chat_id,
                document_id,
                reply_to_msg_id,
                top_msg_id,
            }
        } else {
            WorkerCommand::SendSticker {
                chat_id,
                document_id,
                reply_to_msg_id,
                top_msg_id,
            }
        };
        self.command(command, Operation::SavedGifs, sender);
    }

    fn keyboard_action(
        &mut self,
        message_id: i32,
        row: usize,
        index: usize,
        sender: &ComponentSender<Self>,
    ) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(button) = keyboard_rows(message.reply_markup_json.as_deref())
            .get(row)
            .and_then(|row| row.get(index))
            .cloned()
        else {
            return;
        };
        match button.action {
            KeyboardAction::Url(uri) | KeyboardAction::Web(uri) => self.launch_uri(&uri, sender),
            KeyboardAction::Copy(text) => {
                if !message.noforwards
                    && let Some(display) = gtk::gdk::Display::default()
                {
                    display.clipboard().set_text(&text);
                }
            }
            KeyboardAction::SwitchInline(query) => {
                let bot = message
                    .via_bot
                    .as_deref()
                    .or(message.sender_name.as_deref())
                    .unwrap_or_default()
                    .trim_start_matches('@')
                    .to_owned();
                if composer_inline_query(&format!("@{bot} {query}")).is_some() {
                    self.insert_composer_text(&format!("@{bot} {query}"));
                }
            }
            KeyboardAction::Text(text) => self.insert_composer_text(&text),
            KeyboardAction::Callback(data_hex) => self.command(
                WorkerCommand::Interaction(InteractionCommand::BotCallback {
                    chat_id: message.chat_id,
                    message_id,
                    data_hex,
                }),
                Operation::BotCallback(message_id),
                sender,
            ),
        }
    }

    fn update_suggestions(&mut self, sender: &ComponentSender<Self>) {
        let source = self.composer_text();
        if let Some(query) = composer_inline_query(&source) {
            let unchanged = self.special.inline.as_ref().is_some_and(|inline| {
                inline.username == query.username && inline.query == query.query
            });
            if !unchanged {
                self.special.inline = Some(InlineState {
                    username: query.username.clone(),
                    query: query.query.clone(),
                    ..InlineState::default()
                });
                self.special.mentions.clear();
                self.special.mention_query = None;
                self.command(
                    WorkerCommand::Interaction(InteractionCommand::ResolveInlineBot {
                        username: query.username,
                    }),
                    Operation::ResolveInline,
                    sender,
                );
            }
            return;
        }
        self.special.inline = None;
        let mention = composer_mention_query(&source);
        if mention == self.special.mention_query {
            return;
        }
        self.special.mention_query = mention.clone();
        self.special.mentions.clear();
        let Some(query) = mention else {
            return;
        };
        let Some(chat) = &self.chat else {
            return;
        };
        let (command, operation) = if chat.is_group || chat.is_channel {
            (
                WorkerCommand::Interaction(InteractionCommand::SearchMentions {
                    chat_id: chat.id,
                    query,
                    offset: 0,
                    limit: 20,
                }),
                Operation::MentionSearch,
            )
        } else {
            (
                WorkerCommand::Interaction(InteractionCommand::SearchMentionContacts {
                    query,
                    limit: 20,
                }),
                Operation::MentionSearch,
            )
        };
        self.command(command, operation, sender);
    }

    fn open_instant_view(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(webpage) = MediaSummary::new(message, None)
            .and_then(|summary| summary.webpage)
            .filter(|_| !message.noforwards)
        else {
            return;
        };
        if !webpage.instant_view {
            return;
        }
        let Some(url) = webpage.url.filter(|url| http_url(url)) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        self.web_open = true;
        self.web.emit(WebInput::Loading);
        self.command(
            WorkerCommand::Web(WebCommand::Page { url, hash: 0 }),
            Operation::InstantView,
            sender,
        );
    }

    fn open_instant_media(&mut self, key: String, sender: &ComponentSender<Self>) {
        let Some(target) = instant_media_target(&key) else {
            return;
        };
        self.special.instant_media_key = Some(key);
        let command = match target {
            InstantMediaTarget::Message(media_id) => WorkerCommand::DownloadMessageMedia {
                chat_id: media_id,
                message_id: INSTANT_VIEW_MEDIA_MESSAGE_ID,
                kind: MediaDownloadKind::Full,
                mode: MediaDownloadMode::Manual,
            },
            InstantMediaTarget::Peer(peer_id) => WorkerCommand::DownloadPeerAvatar {
                peer_id,
                kind: MediaDownloadKind::Full,
                mode: MediaDownloadMode::Manual,
            },
        };
        self.command(command, Operation::InstantMedia, sender);
    }

    fn stream_media(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let chat_id = message.chat_id;
        let key = (chat_id, message_id);
        if let Some(stream) = self.streams.get(&key) {
            if stream.finished {
                let path = stream.path.clone();
                self.start_stream_playback(message_id, &path);
            }
            return;
        }
        let result = (|| {
            let directory = xdg::BaseDirectories::with_prefix("monogram")
                .create_cache_directory("streams")
                .map_err(|_| ClientError::Storage)?;
            let path = directory.join(format!("{chat_id}-{message_id}-{}", self.extra.generation));
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(ClientError::Storage),
            }
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| ClientError::Storage)?;
            Ok(path)
        })();
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.streams.insert(
            key,
            StreamTransfer {
                path,
                next_offset: 0,
                total: None,
                transfer_id: None,
                started: false,
                finished: false,
            },
        );
        self.command(
            WorkerCommand::DownloadMediaRange {
                chat_id,
                message_id,
                offset: 0,
                mode: MediaDownloadMode::Autoplay,
            },
            Operation::MediaRange {
                chat_id,
                message_id,
            },
            sender,
        );
    }

    fn start_stream_playback(&mut self, message_id: i32, path: &std::path::Path) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(summary) = MediaSummary::new(message, None) else {
            return;
        };
        let Ok(uri) = gtk::glib::filename_to_uri(path, None) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        self.player.emit(MediaInput::Play {
            tracks: vec![Track {
                id: summary
                    .file_name
                    .unwrap_or_else(|| self.tr(summary.kind_key)),
                source: MediaSource::Uri(uri.to_string()),
            }],
            start: 0,
            autoplay: true,
        });
        if matches!(
            summary.kind_key,
            "feature-dialog-dialog_media_video"
                | "feature-dialog-dialog_media_video_note"
                | "feature-dialog-dialog_media_gif"
        ) && let Some(parent) = self.parent_window()
        {
            self.player.emit(MediaInput::OpenViewer(parent));
        }
    }

    fn fail_stream(&mut self, chat_id: i64, message_id: i32) {
        if let Some(stream) = self.streams.remove(&(chat_id, message_id)) {
            let _ = std::fs::remove_file(stream.path);
            if stream.started {
                self.player.emit(MediaInput::Close);
            }
        }
    }

    fn cancel_stream(&mut self, chat_id: i64, message_id: i32, sender: &ComponentSender<Self>) {
        let transfer_id = self
            .streams
            .get(&(chat_id, message_id))
            .and_then(|stream| stream.transfer_id);
        self.extra.requests.retain(|_, request| {
            !matches!(
                request.operation,
                Operation::MediaRange {
                    chat_id: request_chat,
                    message_id: request_message,
                } if request_chat == chat_id && request_message == message_id
            )
        });
        if let Some(transfer_id) = transfer_id {
            self.command(
                WorkerCommand::CancelMedia { transfer_id },
                Operation::MediaCancel,
                sender,
            );
        }
        self.fail_stream(chat_id, message_id);
    }

    fn cleanup_streams(&mut self) {
        for (_, stream) in std::mem::take(&mut self.streams) {
            let _ = std::fs::remove_file(stream.path);
        }
    }

    fn media_chunk(
        &mut self,
        chat_id: i64,
        message_id: i32,
        chunk: StreamChunk,
        sender: &ComponentSender<Self>,
    ) -> Result<(), ClientError> {
        let StreamChunk {
            transfer_id,
            cache_key,
            offset,
            bytes,
            done,
            total,
        } = chunk;
        let valid_key = self
            .history
            .messages
            .get(&message_id)
            .is_some_and(|message| {
                message.chat_id == chat_id
                    && message.supports_streaming
                    && message.media_cache_key.as_deref() == Some(cache_key.as_str())
            });
        if !valid_key {
            return Err(ClientError::Protocol);
        }
        let (path, next_offset, start, finished) = {
            let stream = self
                .streams
                .get_mut(&(chat_id, message_id))
                .ok_or(ClientError::Protocol)?;
            if stream.finished || stream.next_offset != offset || (bytes.is_empty() && !done) {
                return Err(ClientError::Protocol);
            }
            if total != stream.total && stream.total.is_some() {
                return Err(ClientError::Protocol);
            }
            let next_offset = offset
                .checked_add(bytes.len() as u64)
                .ok_or(ClientError::Protocol)?;
            if total.is_some_and(|total| next_offset > total) {
                return Err(ClientError::Protocol);
            }
            OpenOptions::new()
                .append(true)
                .open(&stream.path)
                .and_then(|mut file| file.write_all(&bytes))
                .map_err(|_| ClientError::Storage)?;
            stream.next_offset = next_offset;
            stream.total = total;
            stream.transfer_id = Some(transfer_id);
            stream.finished = done;
            let start = !stream.started;
            stream.started = true;
            (stream.path.clone(), next_offset, start, done)
        };
        if start {
            self.start_stream_playback(message_id, &path);
        }
        if !finished {
            self.command(
                WorkerCommand::DownloadMediaRange {
                    chat_id,
                    message_id,
                    offset: next_offset,
                    mode: MediaDownloadMode::Autoplay,
                },
                Operation::MediaRange {
                    chat_id,
                    message_id,
                },
                sender,
            );
        }
        Ok(())
    }

    fn media_action(
        &mut self,
        message_id: i32,
        action: MediaAction,
        sender: &ComponentSender<Self>,
    ) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(summary) = MediaSummary::new(message, None) else {
            return;
        };
        let chat_id = message.chat_id;
        if action == MediaAction::CancelDownload
            && self
                .streams
                .get(&(chat_id, message_id))
                .is_some_and(|stream| !stream.finished)
        {
            self.cancel_stream(chat_id, message_id, sender);
            return;
        }
        if action == MediaAction::OpenExternal
            && !summary.protected
            && let Some(url) = summary
                .webpage
                .as_ref()
                .and_then(|webpage| webpage.url.as_deref())
        {
            self.launch_uri(url, sender);
            return;
        }
        let full_status = self
            .media_transfers
            .get(&(chat_id, message_id, MediaPurpose::Full))
            .map_or(MediaStatus::Missing, |transfer| transfer.status);
        let display_ready = self
            .media_transfers
            .get(&(chat_id, message_id, MediaPurpose::Display))
            .is_some_and(|transfer| transfer.status == MediaStatus::Ready);
        let action_status = if action == MediaAction::View && display_ready {
            MediaStatus::Ready
        } else {
            full_status
        };
        if !media_action_allowed(&summary, action, action_status) {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        if action == MediaAction::Play
            && summary.supports_streaming
            && full_status != MediaStatus::Ready
        {
            self.stream_media(message_id, sender);
            return;
        }
        if matches!(action, MediaAction::Download | MediaAction::RetryDownload)
            && self.streams.contains_key(&(chat_id, message_id))
        {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        match action {
            MediaAction::Download | MediaAction::RetryDownload => {
                let transfer = self
                    .media_transfers
                    .entry((chat_id, message_id, MediaPurpose::Full))
                    .or_default();
                transfer.status = MediaStatus::Loading;
                transfer.downloaded = 0;
                transfer.total = None;
                self.command(
                    WorkerCommand::DownloadMessageMedia {
                        chat_id,
                        message_id,
                        kind: MediaDownloadKind::Full,
                        mode: MediaDownloadMode::Manual,
                    },
                    Operation::MediaDownload {
                        chat_id,
                        message_id,
                        kind: MediaDownloadKind::Full,
                        mode: MediaDownloadMode::Manual,
                    },
                    sender,
                );
            }
            MediaAction::CancelDownload => {
                let Some(transfer_id) = self
                    .media_transfers
                    .get(&(chat_id, message_id, MediaPurpose::Full))
                    .and_then(|transfer| transfer.transfer_id)
                else {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                };
                self.command(
                    WorkerCommand::CancelMedia { transfer_id },
                    Operation::MediaCancel,
                    sender,
                );
            }
            MediaAction::Save => self.save_media(message_id, false, sender),
            MediaAction::SaveAs => self.save_media(message_id, true, sender),
            MediaAction::OpenExternal => self.open_media_external(message_id, sender),
            MediaAction::Copy => self.copy_media(message_id, sender),
            MediaAction::View => self.view_media(message_id, sender),
            MediaAction::Play => self.play_media(message_id, sender),
        }
    }

    fn parent_window(&self) -> Option<gtk::Window> {
        self.extra
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
    }

    fn ready_full_media(
        &self,
        message_id: i32,
        action: MediaAction,
    ) -> Option<(MediaSummary, std::path::PathBuf)> {
        let message = self.history.messages.get(&message_id)?;
        let summary = MediaSummary::new(message, None)?;
        if !media_action_allowed(&summary, action, MediaStatus::Ready) {
            return None;
        }
        let path = self
            .media_transfers
            .get(&(message.chat_id, message_id, MediaPurpose::Full))
            .filter(|transfer| transfer.status == MediaStatus::Ready)
            .and_then(|transfer| transfer.lease.as_ref())?
            .path()
            .to_path_buf();
        Some((summary, path))
    }

    fn save_media(&mut self, message_id: i32, save_as: bool, sender: &ComponentSender<Self>) {
        let action = if save_as {
            MediaAction::SaveAs
        } else {
            MediaAction::Save
        };
        let Some((summary, path)) = self.ready_full_media(message_id, action) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        let source = gtk::gio::File::for_path(path);
        let file_name = summary
            .file_name
            .unwrap_or_else(|| format!("message-{message_id}"));
        let input = sender.input_sender().clone();
        if !save_as {
            let Some(directory) = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads)
            else {
                self.extra.notice = Some("feature-dialog-media_action_save_failed");
                return;
            };
            Self::copy_media_file(
                &source,
                &gtk::gio::File::for_path(directory.join(file_name)),
                false,
                input,
            );
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(self.tr("core-ui-media_action_save_as"))
            .initial_name(file_name)
            .modal(true)
            .build();
        let parent = self.parent_window();
        dialog.save(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let destination = match result {
                    Ok(destination) => destination,
                    Err(error) => {
                        if error.kind::<gtk::gio::IOErrorEnum>()
                            != Some(gtk::gio::IOErrorEnum::Cancelled)
                        {
                            input.emit(DialogInput::MediaNotice(
                                "feature-dialog-media_action_save_failed",
                            ));
                        }
                        return;
                    }
                };
                Self::copy_media_file(&source, &destination, true, input);
            },
        );
    }

    fn copy_media_file(
        source: &gtk::gio::File,
        destination: &gtk::gio::File,
        overwrite: bool,
        input: relm4::Sender<DialogInput>,
    ) {
        let flags = if overwrite {
            gtk::gio::FileCopyFlags::OVERWRITE
        } else {
            gtk::gio::FileCopyFlags::NONE
        };
        source.copy_async(
            destination,
            flags,
            gtk::glib::Priority::DEFAULT,
            gtk::gio::Cancellable::NONE,
            None,
            move |result| {
                input.emit(DialogInput::MediaNotice(if result.is_ok() {
                    "feature-dialog-media_action_saved"
                } else {
                    "feature-dialog-media_action_save_failed"
                }));
            },
        );
    }

    fn open_media_external(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some((_, path)) = self.ready_full_media(message_id, MediaAction::OpenExternal) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        let launcher = gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(path)));
        let parent = self.parent_window();
        let input = sender.input_sender().clone();
        launcher.launch(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if result.is_err() {
                    input.emit(DialogInput::MediaNotice(
                        "feature-dialog-dialog_media_open_failed",
                    ));
                }
            },
        );
    }

    fn open_geo(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        if message.noforwards {
            self.error = Some(ClientError::Forbidden);
            return;
        }
        let Some(InteractionCard::Geo(card)) = parse_interaction(message) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        let Some(url) = geo_url(&card) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        self.launch_uri(&url, sender);
    }

    fn launch_uri(&self, uri: &str, sender: &ComponentSender<Self>) {
        if !external_uri_allowed(uri) {
            sender
                .input_sender()
                .emit(DialogInput::SetError(Some(ClientError::InvalidRequest)));
            return;
        }
        let launcher = gtk::UriLauncher::new(uri);
        let parent = self.parent_window();
        let input = sender.input_sender().clone();
        launcher.launch(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                if result.is_err() {
                    input.emit(DialogInput::MediaNotice(
                        "feature-dialog-media_action_no_app",
                    ));
                }
            },
        );
    }

    fn copy_media(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some((summary, path)) = self.ready_full_media(message_id, MediaAction::Copy) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        if summary
            .file_size
            .is_some_and(|size| size > 512 * 1024 * 1024)
        {
            self.extra.notice = Some("core-ui-media_action_copy_failed");
            return;
        }
        let video = matches!(
            summary.kind_key,
            "feature-dialog-dialog_media_video"
                | "feature-dialog-dialog_media_video_note"
                | "feature-dialog-dialog_media_gif"
        );
        let file = gtk::gio::File::for_path(path);
        let input = sender.input_sender().clone();
        file.load_contents_async(gtk::gio::Cancellable::NONE, move |result| {
            let Ok((contents, _)) = result else {
                input.emit(DialogInput::MediaNotice("core-ui-media_action_copy_failed"));
                return;
            };
            if contents.len() > 512 * 1024 * 1024 {
                input.emit(DialogInput::MediaNotice("core-ui-media_action_copy_failed"));
                return;
            }
            let mime = gtk::gio::content_type_guess(None::<&std::path::Path>, contents.as_ref()).0;
            let provider = gtk::gdk::ContentProvider::for_bytes(
                mime.as_str(),
                &gtk::glib::Bytes::from_owned(contents.to_vec()),
            );
            let copied = gtk::gdk::Display::default()
                .is_some_and(|display| display.clipboard().set_content(Some(&provider)).is_ok());
            input.emit(DialogInput::MediaNotice(if copied {
                if video {
                    "core-ui-media_action_copied_video"
                } else {
                    "core-ui-media_action_copied_image"
                }
            } else {
                "core-ui-media_action_copy_failed"
            }));
        });
    }

    fn view_media(&mut self, message_id: i32, sender: &ComponentSender<Self>) {
        let Some(message) = self.history.messages.get(&message_id) else {
            return;
        };
        let Some(summary) = MediaSummary::new(message, None) else {
            return;
        };
        if matches!(
            summary.kind_key,
            "feature-dialog-dialog_media_video"
                | "feature-dialog-dialog_media_video_note"
                | "feature-dialog-dialog_media_gif"
        ) {
            self.play_media(message_id, sender);
            return;
        }
        let path = [MediaPurpose::Display, MediaPurpose::Full]
            .into_iter()
            .find_map(|purpose| {
                self.media_transfers
                    .get(&(message.chat_id, message_id, purpose))
                    .filter(|transfer| transfer.status == MediaStatus::Ready)
                    .and_then(|transfer| transfer.lease.as_ref())
                    .map(|lease| lease.path().to_path_buf())
            });
        let Some(path) =
            path.filter(|_| media_action_allowed(&summary, MediaAction::View, MediaStatus::Ready))
        else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        if let Some(viewer) = &self.media_viewer {
            viewer.set_visible(false);
        }
        let window = gtk::Window::builder()
            .title(self.tr("core-ui-media_viewer_label"))
            .default_width(960)
            .default_height(640)
            .hide_on_close(true)
            .build();
        if let Some(parent) = self.parent_window() {
            window.set_transient_for(Some(&parent));
            window.set_destroy_with_parent(true);
            if let Some(application) = parent.application() {
                window.set_application(Some(&application));
            }
        }
        if let Some(view) = self.media_sticker_viewer.take() {
            view.clear();
        }
        if summary.kind_key == "feature-dialog-dialog_media_sticker"
            && let Some(source) = sticker_source(&path, summary.file_name.as_deref())
        {
            let view = StickerView::new(&self.i18n);
            let fallback = gtk::gdk::Texture::from_filename(&path).ok();
            view.load(source, fallback);
            view.set_active(true);
            view.widget().set_hexpand(true);
            view.widget().set_vexpand(true);
            window.set_child(Some(view.widget()));
            window.present();
            self.media_sticker_viewer = Some(view);
            self.media_viewer = Some(window);
            return;
        }
        let picture = gtk::Picture::for_filename(path);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        window.set_child(Some(&picture));
        window.present();
        self.media_viewer = Some(window);
    }

    fn play_media(&mut self, message_id: i32, _sender: &ComponentSender<Self>) {
        let Some((summary, path)) = self.ready_full_media(message_id, MediaAction::Play) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        let Ok(uri) = gtk::glib::filename_to_uri(path, None) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        self.player.emit(MediaInput::Play {
            tracks: vec![Track {
                id: summary
                    .file_name
                    .unwrap_or_else(|| self.tr(summary.kind_key)),
                source: MediaSource::Uri(uri.to_string()),
            }],
            start: 0,
            autoplay: true,
        });
        if matches!(
            summary.kind_key,
            "feature-dialog-dialog_media_video"
                | "feature-dialog-dialog_media_video_note"
                | "feature-dialog-dialog_media_gif"
        ) && let Some(parent) = self.parent_window()
        {
            self.player.emit(MediaInput::OpenViewer(parent));
        }
    }

    fn stop_playback(&mut self, sender: &ComponentSender<Self>) {
        let transfers = self
            .streams
            .values()
            .filter_map(|stream| stream.transfer_id)
            .collect::<Vec<_>>();
        self.extra
            .requests
            .retain(|_, request| !matches!(request.operation, Operation::MediaRange { .. }));
        for transfer_id in transfers {
            self.command(
                WorkerCommand::CancelMedia { transfer_id },
                Operation::MediaCancel,
                sender,
            );
        }
        self.cleanup_streams();
        self.player.emit(MediaInput::Close);
        for slot in self
            .sticker_views
            .values()
            .chain(self.custom_emoji_views.values())
        {
            slot.view.set_active(false);
        }
        if let Some(view) = self.media_sticker_viewer.take() {
            view.clear();
        }
        if let Some(viewer) = &self.media_viewer {
            viewer.set_visible(false);
        }
    }

    fn choose_attachments(&self, kind: AttachmentKind, sender: &ComponentSender<Self>) {
        let allowed = self.chat.as_ref().is_some_and(|chat| {
            !chat.left
                && match kind {
                    AttachmentKind::Photo | AttachmentKind::Video => chat.can_send_photos,
                    AttachmentKind::Document => chat.can_send_plain,
                }
        });
        if !allowed {
            sender
                .input_sender()
                .emit(DialogInput::SetError(Some(ClientError::Forbidden)));
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title(self.tr(kind.localization_key()))
            .modal(true)
            .build();
        if kind != AttachmentKind::Document {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&self.tr(kind.localization_key())));
            filter.add_mime_type(match kind {
                AttachmentKind::Photo => "image/*",
                AttachmentKind::Video => "video/*",
                AttachmentKind::Document => unreachable!(),
            });
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));
        }
        let parent = self
            .extra
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        let input = sender.input_sender().clone();
        dialog.open_multiple(
            parent.as_ref(),
            gtk::gio::Cancellable::NONE,
            move |result| {
                let files = match result {
                    Ok(files) => files,
                    Err(error) => {
                        if error.kind::<gtk::gio::IOErrorEnum>()
                            != Some(gtk::gio::IOErrorEnum::Cancelled)
                        {
                            input.emit(DialogInput::SetError(Some(ClientError::Protocol)));
                        }
                        return;
                    }
                };
                let mut attachments = Vec::new();
                for index in 0..files.n_items() {
                    let Some(file) = files
                        .item(index)
                        .and_then(|item| item.downcast::<gtk::gio::File>().ok())
                    else {
                        input.emit(DialogInput::SetError(Some(ClientError::InvalidRequest)));
                        return;
                    };
                    let Some(path) = file
                        .path()
                        .and_then(|path| path.into_os_string().into_string().ok())
                    else {
                        input.emit(DialogInput::SetError(Some(ClientError::InvalidRequest)));
                        return;
                    };
                    let Some(file_name) = std::path::Path::new(&path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                    else {
                        input.emit(DialogInput::SetError(Some(ClientError::InvalidRequest)));
                        return;
                    };
                    let mime_type = file
                        .query_info(
                            "standard::content-type",
                            gtk::gio::FileQueryInfoFlags::NONE,
                            gtk::gio::Cancellable::NONE,
                        )
                        .ok()
                        .and_then(|info| info.content_type())
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| {
                            gtk::gio::content_type_guess(Some(&path), None)
                                .0
                                .to_string()
                        });
                    let valid_mime = mime_type.is_empty()
                        || match kind {
                            AttachmentKind::Photo => mime_type.starts_with("image/"),
                            AttachmentKind::Video => mime_type.starts_with("video/"),
                            AttachmentKind::Document => true,
                        };
                    if !valid_mime {
                        input.emit(DialogInput::SetError(Some(ClientError::InvalidRequest)));
                        return;
                    }
                    attachments.push(SelectedAttachment {
                        path,
                        kind,
                        file_name,
                        mime_type,
                    });
                }
                input.emit(DialogInput::AttachmentsChosen(attachments));
            },
        );
    }

    fn composer_text(&self) -> String {
        let buffer = self.editor.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    fn compose_media_send(&self) -> Result<MediaSend, ClientError> {
        let (chat_id, top_msg_id) = self.context().ok_or(ClientError::InvalidRequest)?;
        let (caption, entities_json) =
            outgoing_text(&self.composer_text(), self.markdown.is_active())?;
        let schedule_date = optional_number(&self.schedule.text())?
            .map(|_| {
                let now = gtk::glib::DateTime::now_utc()
                    .map_err(|_| ClientError::InvalidRequest)?
                    .to_unix();
                schedule_timestamp(&self.schedule.text(), now)
            })
            .transpose()?;
        let send = MediaSend {
            chat_id,
            reply_to_msg_id: optional_number(&self.reply.text())?.unwrap_or(0),
            top_msg_id,
            silent: self.silent.is_active(),
            schedule_date,
            items: media_uploads(&self.attachments, &caption, entities_json),
        };
        send.validate()?;
        Ok(send)
    }

    fn composer_draft(&self) -> Result<CloudDraft, ClientError> {
        let (chat_id, thread_id) = self.context().ok_or(ClientError::InvalidRequest)?;
        let source = self.composer_text();
        let (text, mut entities_json) = outgoing_text(&source, self.markdown.is_active())?;
        let mut no_webpage = false;
        if let Some(original) = &self.extra.original_draft {
            no_webpage = original.no_webpage;
            if !self.markdown.is_active() && original.text == text {
                entities_json = original.entities_json.clone();
            }
        }
        Ok(CloudDraft {
            chat_id,
            thread_id,
            text,
            entities_json,
            reply_to_message_id: optional_number(&self.reply.text())?.unwrap_or(0),
            date: gtk::glib::DateTime::now_utc()
                .map_err(|_| ClientError::InvalidRequest)?
                .to_unix()
                .try_into()
                .map_err(|_| ClientError::InvalidRequest)?,
            no_webpage,
            has_unsupported_content: false,
        })
    }

    fn stash_context(&mut self, sender: &ComponentSender<Self>) {
        if self.action.is_some() || self.extra.queue_action.is_some() {
            self.cancel_action();
        }
        self.flush_draft(sender);
        if let Some(context) = self.context() {
            self.extra
                .histories
                .insert(context, std::mem::take(&mut self.history));
            if let Ok(draft) = self.composer_draft() {
                self.extra
                    .drafts
                    .insert(context, (draft, self.extra.draft.clone()));
            }
        }
        if let Some(source) = self.extra.debounce.take() {
            source.remove();
        }
    }

    fn restore_context(&mut self, sender: &ComponentSender<Self>) {
        self.extra.generation = self.extra.generation.saturating_add(1);
        self.extra.requests.clear();
        self.extra.draft_inflight = false;
        self.extra.queue_action = None;
        self.extra.queue_busy = false;
        self.extra.discard_confirm = false;
        self.extra.scheduled_snapshot = None;
        self.extra.read_requested = 0;
        self.extra.notice = None;
        self.extra.suppress.set(true);
        self.cancel_action();
        self.attachments.clear();
        self.media_send = None;
        self.media_album_id = None;
        self.media_submitting = false;
        self.pending_media.clear();
        self.media_transfers.clear();
        self.stop_playback(sender);
        self.sticker_views.clear();
        self.custom_emoji_views.clear();
        self.custom_emoji.clear();
        self.special = SpecialState::default();
        self.web_open = false;
        self.editor.buffer().set_text("");
        self.extra.draft = DraftGuard::default();
        self.extra.original_draft = None;
        if let Some(context) = self.context() {
            self.history = self.extra.histories.remove(&context).unwrap_or_default();
            self.top.set_text(&context.1.to_string());
            if let Some((draft, guard)) = self.extra.drafts.remove(&context) {
                self.extra.draft = guard;
                self.editor.buffer().set_text(&draft.text);
                self.reply.set_text(&draft.reply_to_message_id.to_string());
                self.markdown.set_active(false);
                self.extra.original_draft = Some(draft);
            }
        }
        self.extra.suppress.set(false);
        self.rows.clear();
        clear_box(&self.list);
        self.busy = false;
        self.exhausted = false;
        self.error = None;
        self.extra.pinned.clear();
        self.extra.scheduled.clear();
        self.command(
            WorkerCommand::GetCachedCloudDrafts,
            Operation::DraftLoad,
            sender,
        );
        self.command(WorkerCommand::GetCloudDrafts, Operation::DraftLoad, sender);
        self.command(WorkerCommand::PendingMedia, Operation::PendingMedia, sender);
        self.load_history(false, sender);
        if self.chat.as_ref().is_some_and(|chat| chat.is_forum) {
            self.load_forums(sender);
        }
    }

    fn schedule_draft(&mut self, sender: &ComponentSender<Self>) {
        if self.action.is_some() || self.extra.queue_action.is_some() || self.extra.suppress.get() {
            return;
        }
        self.extra.draft.changed();
        self.extra.draft.conflict = false;
        if let Some(source) = self.extra.debounce.take() {
            source.remove();
        }
        let generation = self.extra.generation;
        let revision = self.extra.draft.revision;
        let input = sender.input_sender().clone();
        self.extra.debounce = Some(gtk::glib::timeout_add_local_once(
            Duration::from_millis(400),
            move || {
                input.emit(DialogInput::DraftDebounced {
                    generation,
                    revision,
                });
            },
        ));
    }

    fn flush_draft(&mut self, sender: &ComponentSender<Self>) {
        if !self.extra.draft.ready
            || !self.extra.draft.dirty()
            || self.extra.draft.unsupported
            || self.extra.draft.conflict
            || self.extra.draft_inflight
            || self.action.is_some()
            || self.extra.queue_action.is_some()
        {
            return;
        }
        match self.composer_draft() {
            Ok(draft) => {
                self.extra.draft_inflight = true;
                self.command(
                    WorkerCommand::SaveCloudDraft {
                        draft,
                        expected_revision: self.extra.draft.server_revision,
                    },
                    Operation::DraftSave {
                        revision: self.extra.draft.revision,
                        discard: false,
                    },
                    sender,
                );
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn apply_draft(&mut self, incoming: DraftRevision) {
        if self.context() != Some((incoming.draft.chat_id, incoming.draft.thread_id)) {
            return;
        }
        if self.extra.draft.accept(&incoming) {
            if self.action.is_none() && self.extra.queue_action.is_none() {
                self.extra.suppress.set(true);
                self.editor.buffer().set_text(&incoming.draft.text);
                self.reply
                    .set_text(&incoming.draft.reply_to_message_id.to_string());
                self.markdown.set_active(false);
                self.extra.suppress.set(false);
            }
            self.extra.original_draft = Some(incoming.draft);
        }
    }

    fn media_failure(&mut self, operation: Operation, error: &ClientError) -> bool {
        match operation {
            Operation::MediaSend => {
                self.media_submitting = false;
                if let Some(album_id) = self.media_album_id
                    && let Some(pending) = self.pending_media.get_mut(&album_id)
                {
                    pending.uncertain = true;
                }
            }
            Operation::MediaDownload {
                chat_id,
                message_id,
                kind,
                mode,
            } => {
                let purpose = match kind {
                    MediaDownloadKind::Thumb => MediaPurpose::Thumb,
                    MediaDownloadKind::Display => MediaPurpose::Display,
                    MediaDownloadKind::Full => MediaPurpose::Full,
                };
                let transfer = self
                    .media_transfers
                    .entry((chat_id, message_id, purpose))
                    .or_default();
                transfer.status = if *error == ClientError::Cancelled {
                    MediaStatus::Missing
                } else {
                    MediaStatus::Failed
                };
                transfer.transfer_id = None;
                if mode == MediaDownloadMode::Automatic {
                    return true;
                }
            }
            Operation::CustomEmoji { document_id } => {
                let transfer = self.custom_emoji.entry(document_id).or_default();
                transfer.status = MediaStatus::Failed;
                transfer.transfer_id = None;
                return true;
            }
            Operation::MediaRange {
                chat_id,
                message_id,
            } => self.fail_stream(chat_id, message_id),
            _ => {}
        }
        false
    }

    fn special_response(
        &mut self,
        operation: Operation,
        response: WorkerResponse,
        sender: &ComponentSender<Self>,
    ) {
        match (operation, response) {
            (Operation::PollMutation(message_id), WorkerResponse::Done)
            | (Operation::ChecklistMutation(message_id), WorkerResponse::Done) => {
                self.special.busy.remove(&message_id);
                if let Some(chat_id) = self.context().map(|context| context.0) {
                    self.command(
                        WorkerCommand::Interaction(InteractionCommand::GetMessage {
                            chat_id,
                            message_id,
                        }),
                        operation,
                        sender,
                    );
                }
            }
            (
                Operation::PollMutation(message_id),
                WorkerResponse::Interaction(InteractionResponse::Message(message)),
            )
            | (
                Operation::ChecklistMutation(message_id),
                WorkerResponse::Interaction(InteractionResponse::Message(message)),
            ) => {
                self.special.busy.remove(&message_id);
                if let Some(message) = message {
                    self.history.insert(*message, true);
                } else {
                    self.error = Some(ClientError::Protocol);
                }
            }
            (
                Operation::PollVoters { message_id, append },
                WorkerResponse::PollVoters {
                    chat_id,
                    message_id: response_id,
                    page,
                },
            ) if Some(chat_id) == self.context().map(|context| context.0)
                && response_id == message_id =>
            {
                let next_offset = page.next_offset.clone();
                if append {
                    let voters = self
                        .special
                        .poll_voters
                        .entry(message_id)
                        .or_insert_with(|| {
                            mono_proto_client::adapter::messages::extras_rpc::PollVotersDto {
                                next_offset: None,
                                voters: Vec::new(),
                                count: 0,
                            }
                        });
                    let mut seen = voters
                        .voters
                        .iter()
                        .map(|voter| voter.peer_id)
                        .collect::<BTreeSet<_>>();
                    voters.voters.extend(
                        page.voters
                            .into_iter()
                            .filter(|voter| seen.insert(voter.peer_id)),
                    );
                    voters.next_offset = page.next_offset;
                    voters.count = page.count;
                } else {
                    self.special.poll_voters.insert(message_id, page);
                }
                self.special.poll_next.insert(message_id, next_offset);
            }
            (
                Operation::ReactionPeers { message_id, append },
                WorkerResponse::Interaction(InteractionResponse::ReactionPeers(page)),
            ) => {
                let next_offset = page.next_offset.clone();
                if append {
                    let peers = self
                        .special
                        .reaction_peers
                        .entry(message_id)
                        .or_insert_with(|| crate::client::interactions::ReactionPeersPage {
                            next_offset: None,
                            peers: Vec::new(),
                            count: 0,
                        });
                    let mut seen = peers
                        .peers
                        .iter()
                        .map(|peer| peer.peer_id)
                        .collect::<BTreeSet<_>>();
                    peers.peers.extend(
                        page.peers
                            .into_iter()
                            .filter(|peer| seen.insert(peer.peer_id)),
                    );
                    peers.next_offset = page.next_offset;
                    peers.count = page.count;
                } else {
                    self.special.reaction_peers.insert(message_id, page);
                }
                self.special.reaction_next.insert(message_id, next_offset);
            }
            (
                Operation::ReadReceipt(message_id),
                WorkerResponse::Interaction(InteractionResponse::OutboxReadDate(read)),
            ) => {
                self.special.read_receipts.insert(message_id, read);
            }
            (
                Operation::ForumTopics,
                WorkerResponse::Interaction(InteractionResponse::ForumTopics(page)),
            ) => {
                for topic in page.topics {
                    self.special.forums.insert(topic.id, topic);
                }
            }
            (Operation::GeneralHidden, WorkerResponse::Interaction(InteractionResponse::Done)) => {
                self.load_forums(sender)
            }
            (
                Operation::ResolveInline,
                WorkerResponse::Interaction(InteractionResponse::ResolvedPeer(peer)),
            ) => {
                if !peer.is_bot {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                }
                let next = self
                    .special
                    .inline
                    .as_ref()
                    .map(|inline| (peer.peer_id, inline.query.clone()));
                if let Some((bot_id, query)) = next {
                    if let Some(inline) = &mut self.special.inline {
                        inline.bot_id = bot_id;
                    }
                    if let Some(chat_id) = self.context().map(|context| context.0) {
                        self.command(
                            WorkerCommand::Interaction(InteractionCommand::InlineBotResults {
                                chat_id,
                                bot_id,
                                query,
                                offset: String::new(),
                            }),
                            Operation::InlineResults,
                            sender,
                        );
                    }
                }
            }
            (
                Operation::InlineResults,
                WorkerResponse::Interaction(InteractionResponse::InlineBotResults(page)),
            ) => {
                if let Some(inline) = &mut self.special.inline {
                    let existing = inline
                        .results
                        .iter()
                        .map(|result| result.id.clone())
                        .collect::<BTreeSet<_>>();
                    inline.results.extend(
                        page.results
                            .into_iter()
                            .filter(|result| !existing.contains(&result.id)),
                    );
                    inline.query_id = page.query_id;
                    inline.next_offset = page.next_offset.unwrap_or_default();
                }
            }
            (
                Operation::MentionSearch,
                WorkerResponse::Interaction(InteractionResponse::Mentions(page)),
            ) => {
                self.special.mentions = page
                    .members
                    .into_iter()
                    .map(|member| MentionChoice {
                        peer_id: member.peer_id,
                        title: member.title,
                        username: member.username,
                    })
                    .collect();
            }
            (
                Operation::MentionSearch,
                WorkerResponse::Interaction(InteractionResponse::MentionContacts(page)),
            ) => {
                self.special.mentions = page
                    .people
                    .into_iter()
                    .chain(page.chats)
                    .map(|peer| MentionChoice {
                        peer_id: peer.peer_id,
                        title: peer.title,
                        username: peer.username,
                    })
                    .collect();
            }
            (
                Operation::StickerCatalog,
                WorkerResponse::Interaction(InteractionResponse::StickerCatalog(catalog)),
            ) => {
                self.special.sticker_sets = catalog.sets;
                self.special.sticker_pack = None;
            }
            (
                Operation::StickerPack,
                WorkerResponse::Interaction(InteractionResponse::StickerPack(pack)),
            ) => self.special.sticker_pack = Some(pack),
            (
                Operation::SavedGifs,
                WorkerResponse::Interaction(InteractionResponse::SavedGifs(gifs)),
            ) => self.special.gifs = gifs,
            (
                Operation::SavedGifs,
                WorkerResponse::Interaction(InteractionResponse::Message(Some(message))),
            ) => self.history.insert(*message, true),
            (
                Operation::RecentReactions,
                WorkerResponse::Interaction(InteractionResponse::RecentReactions(choices)),
            ) => self.special.reaction_choices = choices,
            (
                Operation::BotCallback(message_id),
                WorkerResponse::Interaction(InteractionResponse::BotCallback(answer)),
            ) => {
                if let Some(url) = answer.url.filter(|url| http_url(url)) {
                    self.launch_uri(&url, sender);
                } else {
                    self.special.callback_notice = answer.message;
                }
                let _ = message_id;
            }
            (Operation::InstantView, WorkerResponse::Web(WebResponse::Page(page))) => {
                self.web.emit(WebInput::Page(*page))
            }
            (Operation::InstantMedia, WorkerResponse::MediaReady { lease, .. }) => {
                let expected = self.special.instant_media_key.take();
                if expected.is_none() {
                    self.error = Some(ClientError::Protocol);
                    return;
                }
                let launcher =
                    gtk::FileLauncher::new(Some(&gtk::gio::File::for_path(lease.path())));
                let parent = self.parent_window();
                let input = sender.input_sender().clone();
                launcher.launch(
                    parent.as_ref(),
                    gtk::gio::Cancellable::NONE,
                    move |result| {
                        if result.is_err() {
                            input.emit(DialogInput::MediaNotice(
                                "feature-dialog-dialog_media_open_failed",
                            ));
                        }
                    },
                );
            }
            (Operation::SendInlineResult, WorkerResponse::SpecialOutgoing { result, .. }) => {
                match *result {
                    OutgoingMessageResult::Sent { message } => {
                        if in_context(&message, self.context()) {
                            self.history.insert(message, true);
                        }
                    }
                    OutgoingMessageResult::Scheduled { message } => {
                        if self
                            .context()
                            .is_some_and(|context| context.0 == message.message.chat_id)
                        {
                            self.extra.scheduled.insert(message.message.id, message);
                        }
                    }
                }
                self.special.inline = None;
                self.extra.suppress.set(true);
                self.editor.buffer().set_text("");
                self.reply.set_text("");
                self.schedule.set_text("");
                self.extra.original_draft = None;
                self.extra.suppress.set(false);
                self.extra.draft.changed();
                self.flush_draft(sender);
            }
            (
                Operation::PollMutation(_)
                | Operation::ChecklistMutation(_)
                | Operation::PollVoters { .. }
                | Operation::ReactionPeers { .. }
                | Operation::ReadReceipt(_)
                | Operation::ForumTopics
                | Operation::GeneralHidden
                | Operation::ResolveInline
                | Operation::InlineResults
                | Operation::MentionSearch
                | Operation::StickerCatalog
                | Operation::StickerPack
                | Operation::SavedGifs
                | Operation::RecentReactions
                | Operation::BotCallback(_)
                | Operation::InstantView
                | Operation::InstantMedia,
                _,
            ) => self.error = Some(ClientError::Protocol),
            _ => self.error = Some(ClientError::Protocol),
        }
    }

    fn response(&mut self, token: u64, response: WorkerResponse, sender: &ComponentSender<Self>) {
        let Some(request) = self.extra.requests.remove(&token) else {
            return;
        };
        if request.generation != self.extra.generation {
            return;
        }
        if matches!(
            request.operation,
            Operation::PollMutation(_)
                | Operation::ChecklistMutation(_)
                | Operation::PollVoters { .. }
                | Operation::ReactionPeers { .. }
                | Operation::ReadReceipt(_)
                | Operation::ForumTopics
                | Operation::GeneralHidden
                | Operation::ResolveInline
                | Operation::InlineResults
                | Operation::MentionSearch
                | Operation::StickerCatalog
                | Operation::StickerPack
                | Operation::SavedGifs
                | Operation::RecentReactions
                | Operation::BotCallback(_)
                | Operation::InstantView
                | Operation::InstantMedia
                | Operation::SendInlineResult
        ) {
            self.special_response(request.operation, response, sender);
            return;
        }
        match (request.operation, response) {
            (Operation::Replies { append }, WorkerResponse::History { chat_id, messages }) => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    self.busy = false;
                    self.exhausted = append && messages.is_empty();
                    for message in messages
                        .into_iter()
                        .filter(|message| message.chat_id == chat_id)
                    {
                        self.history.insert(message, false);
                    }
                }
            }
            (Operation::Pinned, WorkerResponse::History { chat_id, messages }) => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    self.extra.pinned = messages
                        .into_iter()
                        .filter(|message| message.chat_id == chat_id)
                        .map(|message| (message.id, message))
                        .collect();
                }
            }
            (Operation::Jump(id), WorkerResponse::History { chat_id, messages }) => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    for message in messages
                        .into_iter()
                        .filter(|message| message.chat_id == chat_id)
                    {
                        self.history.insert(message, false);
                    }
                    self.extra.focus_message = Some(id);
                }
            }
            (Operation::DraftLoad, WorkerResponse::Drafts(drafts)) => {
                if let Some(draft) = drafts.into_iter().find(|value| {
                    self.context() == Some((value.draft.chat_id, value.draft.thread_id))
                }) {
                    self.apply_draft(draft);
                } else {
                    self.extra.draft.ready = true;
                }
                if self.extra.debounce.is_none() {
                    self.flush_draft(sender);
                }
            }
            (Operation::DraftSave { revision, discard }, WorkerResponse::Draft(value)) => {
                if self.context() == Some((value.draft.chat_id, value.draft.thread_id)) {
                    self.extra.draft_inflight = false;
                    self.extra.draft.acknowledge(revision, value.revision);
                    if revision == self.extra.draft.revision {
                        self.extra.original_draft = Some(value.draft.clone());
                        if discard {
                            self.extra.suppress.set(true);
                            self.editor.buffer().set_text("");
                            self.reply.set_text("");
                            self.extra.suppress.set(false);
                            self.extra.draft.unsupported = false;
                            self.extra.discard_confirm = false;
                        }
                    }
                    if self.extra.debounce.is_none() {
                        self.flush_draft(sender);
                    }
                }
            }
            (
                Operation::ScheduledLoad | Operation::ScheduledMutation,
                WorkerResponse::Scheduled { chat_id, messages },
            ) => {
                if self.context().is_some_and(|context| context.0 == chat_id) {
                    self.extra.scheduled = messages
                        .into_iter()
                        .filter(|value| value.message.chat_id == chat_id)
                        .map(|value| (value.message.id, value))
                        .collect();
                    self.extra.queue_busy = false;
                    if matches!(request.operation, Operation::ScheduledMutation) {
                        self.cancel_action();
                    }
                }
            }
            (Operation::PendingMedia, WorkerResponse::PendingMedia(pending)) => {
                self.pending_media = pending
                    .into_iter()
                    .map(|pending| (pending.album_id, pending))
                    .collect();
            }
            (
                Operation::MediaDownload {
                    chat_id,
                    message_id,
                    kind,
                    ..
                },
                WorkerResponse::MediaReady {
                    transfer_id,
                    cache_key,
                    lease,
                },
            ) => {
                let purpose = match kind {
                    MediaDownloadKind::Thumb => MediaPurpose::Thumb,
                    MediaDownloadKind::Display => MediaPurpose::Display,
                    MediaDownloadKind::Full => MediaPurpose::Full,
                };
                let valid = self
                    .history
                    .messages
                    .get(&message_id)
                    .is_some_and(|message| {
                        message.chat_id == chat_id
                            && match kind {
                                MediaDownloadKind::Full => {
                                    message.media_cache_key.as_deref() == Some(cache_key.as_str())
                                }
                                MediaDownloadKind::Thumb => {
                                    message
                                        .thumb_cache_key
                                        .as_deref()
                                        .or(message.media_cache_key.as_deref())
                                        == Some(cache_key.as_str())
                                }
                                MediaDownloadKind::Display => {
                                    message.media_cache_key.as_deref().is_some_and(|key| {
                                        cache_key == derived_media_cache_key("display", key)
                                    })
                                }
                            }
                    });
                if !valid {
                    self.media_failure(request.operation, &ClientError::Protocol);
                    self.error = Some(ClientError::Protocol);
                    return;
                }
                self.media_transfers.insert(
                    (chat_id, message_id, purpose),
                    MediaTransfer {
                        status: MediaStatus::Ready,
                        transfer_id: Some(transfer_id),
                        downloaded: 0,
                        total: None,
                        lease: Some(*lease),
                    },
                );
            }
            (
                Operation::CustomEmoji { document_id },
                WorkerResponse::MediaReady {
                    transfer_id,
                    cache_key,
                    lease,
                },
            ) => {
                if cache_key != format!("custom-emoji:{document_id}") {
                    self.media_failure(request.operation, &ClientError::Protocol);
                    self.error = Some(ClientError::Protocol);
                    return;
                }
                self.custom_emoji.insert(
                    document_id,
                    MediaTransfer {
                        status: MediaStatus::Ready,
                        transfer_id: Some(transfer_id),
                        downloaded: 0,
                        total: None,
                        lease: Some(*lease),
                    },
                );
            }
            (
                Operation::MediaSend,
                WorkerResponse::MediaOutgoing {
                    album_id,
                    random_ids,
                    results,
                },
            ) => {
                let expected = self.media_album_id == Some(album_id)
                    && self
                        .pending_media
                        .get(&album_id)
                        .is_some_and(|pending| pending.random_ids == random_ids);
                if !expected || results.is_empty() || results.len() != random_ids.len() {
                    self.media_failure(request.operation, &ClientError::Protocol);
                    self.error = Some(ClientError::Protocol);
                    return;
                }
                for result in results {
                    match result {
                        OutgoingMessageResult::Sent { message } => {
                            if in_context(&message, self.context()) {
                                self.history.insert(message, true);
                            }
                        }
                        OutgoingMessageResult::Scheduled { message } => {
                            if self
                                .context()
                                .is_some_and(|context| context.0 == message.message.chat_id)
                            {
                                self.extra.scheduled.insert(message.message.id, message);
                            }
                        }
                    }
                }
                self.pending_media.remove(&album_id);
                self.media_album_id = None;
                self.media_send = None;
                self.media_submitting = false;
                self.attachments.clear();
                self.extra.suppress.set(true);
                self.editor.buffer().set_text("");
                self.reply.set_text("");
                self.schedule.set_text("");
                self.extra.original_draft = None;
                self.extra.suppress.set(false);
                self.extra.draft.changed();
                self.flush_draft(sender);
            }
            (
                Operation::MediaRange {
                    chat_id,
                    message_id,
                },
                WorkerResponse::MediaChunk {
                    transfer_id,
                    cache_key,
                    offset,
                    bytes,
                    done,
                    total,
                },
            ) => {
                if let Err(error) = self.media_chunk(
                    chat_id,
                    message_id,
                    StreamChunk {
                        transfer_id,
                        cache_key,
                        offset,
                        bytes,
                        done,
                        total,
                    },
                    sender,
                ) {
                    self.fail_stream(chat_id, message_id);
                    self.error = Some(error);
                }
            }
            (Operation::MediaCancel, WorkerResponse::Done) => {}
            (Operation::Read(max_id), WorkerResponse::Done) => {
                if let Some(chat) = self.chat.as_mut() {
                    chat.read_inbox_max_id = chat.read_inbox_max_id.max(max_id);
                }
            }
            _ => {
                self.media_failure(request.operation, &ClientError::Protocol);
                self.error = Some(ClientError::Protocol);
            }
        }
    }

    fn failed(&mut self, token: u64, error: ClientError, sender: &ComponentSender<Self>) {
        let Some(request) = self.extra.requests.remove(&token) else {
            return;
        };
        if request.generation != self.extra.generation {
            return;
        }
        match request.operation {
            Operation::DraftSave { .. } => {
                self.extra.draft_inflight = false;
                self.extra.draft.conflict = true;
                self.command(WorkerCommand::GetCloudDrafts, Operation::DraftLoad, sender);
            }
            Operation::Read(_) => self.extra.read_requested = 0,
            Operation::ScheduledLoad | Operation::ScheduledMutation => {
                self.extra.queue_busy = false
            }
            Operation::Replies { .. } => self.busy = false,
            Operation::PollMutation(message_id) | Operation::ChecklistMutation(message_id) => {
                self.special.busy.remove(&message_id);
            }
            Operation::InstantMedia => self.special.instant_media_key = None,
            _ => {}
        }
        let fallback = match request.operation {
            Operation::MediaDownload {
                chat_id,
                message_id,
                kind: MediaDownloadKind::Display,
                mode: MediaDownloadMode::Automatic,
            } if matches!(
                error,
                ClientError::Protocol | ClientError::UnsupportedContent
            ) =>
            {
                self.history
                    .messages
                    .get(&message_id)
                    .filter(|message| {
                        message.chat_id == chat_id
                            && message.media_preview_restricted != Some(true)
                            && message.media_cache_key.is_some()
                    })
                    .map(|_| (chat_id, message_id))
            }
            _ => None,
        };
        let suppressed = self.media_failure(request.operation, &error);
        if let Some((chat_id, message_id)) = fallback {
            self.start_media_download(
                chat_id,
                message_id,
                MediaPurpose::Thumb,
                MediaDownloadMode::Automatic,
                sender,
            );
        }
        if !suppressed {
            self.error = Some(error);
        }
    }

    fn jump_to(&mut self, id: i32, sender: &ComponentSender<Self>) {
        let Some((chat_id, _)) = self.context() else {
            return;
        };
        if id <= 0 {
            return;
        }
        if self.history.messages.contains_key(&id) {
            self.extra.focus_message = Some(id);
        } else {
            self.command(
                WorkerCommand::LoadHistory {
                    chat_id,
                    limit: 100,
                    offset_id: id,
                    offset_date: 0,
                    add_offset: -50,
                },
                Operation::Jump(id),
                sender,
            );
        }
    }

    fn mark_visible(&mut self, sender: &ComponentSender<Self>) {
        if !self.extra.active || !self.extra.root.is_mapped() || !self.online {
            return;
        }
        let active = self
            .extra
            .root
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .is_some_and(|window| window.is_active());
        if !active {
            return;
        }
        let Some((chat_id, _)) = self.context() else {
            return;
        };
        let adjustment = self.extra.scroller.vadjustment();
        let start = adjustment.value() as f32;
        let end = start + adjustment.page_size() as f32;
        let max_id = self
            .rows
            .iter()
            .filter_map(|(id, row)| {
                let message = self.history.messages.get(id)?;
                let bounds = row.compute_bounds(&self.list)?;
                (!message.outgoing && bounds.y() < end && bounds.y() + bounds.height() > start)
                    .then_some(*id)
            })
            .max()
            .unwrap_or(0);
        if max_id > self.extra.read_requested {
            self.extra.read_requested = max_id;
            self.command(
                WorkerCommand::ReadHistory { chat_id, max_id },
                Operation::Read(max_id),
                sender,
            );
        }
    }

    fn begin_queue_action(&mut self, id: i32, action: ScheduledAction) {
        let Some(value) = self.extra.scheduled.get(&id).cloned() else {
            return;
        };
        if self.extra.queue_busy
            || (action == ScheduledAction::Edit
                && (value.has_unsupported_content || value.message.text.is_none()))
        {
            return;
        }
        if self.action.is_none() && self.extra.queue_action.is_none() {
            self.extra.original_draft = self.composer_draft().ok();
        }
        self.extra.suppress.set(true);
        self.action = None;
        self.extra.queue_action = Some((id, action));
        self.extra.discard_confirm = false;
        self.extra.scheduled_snapshot = Some(value.clone());
        self.schedule.set_text(&value.message.date.to_string());
        if action == ScheduledAction::Edit {
            self.editor
                .buffer()
                .set_text(value.message.text.as_deref().unwrap_or_default());
            self.markdown.set_active(false);
        }
        self.extra.suppress.set(false);
    }

    fn submit_queue(&mut self, sender: &ComponentSender<Self>) -> Result<(), ClientError> {
        let (chat_id, _) = self.context().ok_or(ClientError::InvalidRequest)?;
        if self.extra.queue_busy {
            return Err(ClientError::InvalidRequest);
        }
        if self.extra.discard_confirm {
            self.extra.draft_inflight = true;
            self.command(
                WorkerCommand::DiscardCloudDraft {
                    chat_id,
                    thread_id: self.context().unwrap().1,
                    expected_revision: self.extra.draft.server_revision,
                },
                Operation::DraftSave {
                    revision: self.extra.draft.revision,
                    discard: true,
                },
                sender,
            );
            return Ok(());
        }
        let (id, action) = self.extra.queue_action.ok_or(ClientError::InvalidRequest)?;
        let current = self
            .extra
            .scheduled
            .get(&id)
            .ok_or(ClientError::InvalidRequest)?;
        if !self
            .extra
            .scheduled_snapshot
            .as_ref()
            .is_some_and(|snapshot| scheduled_unchanged(snapshot, current))
        {
            self.extra.notice = Some("feature-dialog-dialog_scheduled_edit_changed");
            return Err(ClientError::InvalidRequest);
        }
        let now = gtk::glib::DateTime::now_utc()
            .map_err(|_| ClientError::InvalidRequest)?
            .to_unix();
        let command = match action {
            ScheduledAction::SendNow => WorkerCommand::SendScheduledNow {
                chat_id,
                message_ids: vec![id],
            },
            ScheduledAction::Delete => WorkerCommand::DeleteScheduled {
                chat_id,
                message_ids: vec![id],
            },
            ScheduledAction::Reschedule => WorkerCommand::Reschedule {
                chat_id,
                message_id: id,
                schedule_date: schedule_timestamp(&self.schedule.text(), now)?,
            },
            ScheduledAction::Edit => {
                if current.has_unsupported_content {
                    return Err(ClientError::InvalidRequest);
                }
                let schedule_date = schedule_timestamp(&current.message.date.to_string(), now)?;
                let (text, mut entities_json) =
                    outgoing_text(&self.composer_text(), self.markdown.is_active())?;
                if text.trim().is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                if !self.markdown.is_active() && current.message.text.as_deref() == Some(&text) {
                    entities_json = current.message.entities_json.clone();
                }
                WorkerCommand::EditScheduled {
                    chat_id,
                    message_id: id,
                    schedule_date,
                    text,
                    entities_json,
                    no_webpage: false,
                }
            }
        };
        self.extra.queue_busy = true;
        self.command(command, Operation::ScheduledMutation, sender);
        Ok(())
    }

    fn refresh_extra(&self, sender: &ComponentSender<Self>) {
        self.extra
            .leave_thread
            .set_visible(self.extra.thread.is_some());
        self.extra
            .draft_status
            .set_text(&self.tr(if self.extra.draft.unsupported {
                "feature-dialog-dialog_draft_unsupported"
            } else if self.extra.draft.conflict {
                "desktop-error-generic"
            } else {
                "feature-dialog-dialog_outbox_waiting"
            }));
        self.extra.draft_status.set_visible(
            self.extra.draft.unsupported || self.extra.draft.conflict || self.extra.draft_inflight,
        );
        self.extra.discard.set_visible(
            self.extra.draft.unsupported
                || self.extra.draft.conflict
                || !self.composer_text().is_empty(),
        );
        self.extra
            .discard
            .set_sensitive(self.extra.draft.ready && !self.extra.draft_inflight);
        clear_box(&self.extra.pinned_rows);
        for (id, value) in &self.extra.pinned {
            let button = gtk::Button::with_label(&format!(
                "{} · {}",
                id,
                value.sender_name.as_deref().unwrap_or_default()
            ));
            let id = *id;
            connect(&button, sender, move || DialogInput::JumpTo(id));
            self.extra.pinned_rows.append(&button);
        }
        clear_box(&self.extra.scheduled_rows);
        if self.extra.scheduled.is_empty() {
            append_label(
                &self.extra.scheduled_rows,
                &self.tr("feature-dialog-dialog_scheduled_empty"),
            );
        }
        for (id, value) in &self.extra.scheduled {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
            append_label(&row, &value.message.date.to_string());
            render_body(
                &row,
                value.message.text.as_deref().unwrap_or_default(),
                value.message.entities_json.as_deref(),
            );
            let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (action, key) in [
                (ScheduledAction::Edit, "feature-dialog-dialog_edit"),
                (
                    ScheduledAction::Reschedule,
                    "feature-dialog-dialog_scheduled_reschedule",
                ),
                (
                    ScheduledAction::SendNow,
                    "feature-dialog-dialog_scheduled_send_now",
                ),
                (
                    ScheduledAction::Delete,
                    "feature-dialog-dialog_scheduled_cancel",
                ),
            ] {
                if action == ScheduledAction::Edit
                    && (value.has_unsupported_content || value.message.text.is_none())
                {
                    continue;
                }
                let button = gtk::Button::with_label(&self.tr(key));
                button.set_sensitive(!self.extra.queue_busy);
                let id = *id;
                connect(&button, sender, move || DialogInput::QueueAction {
                    message_id: id,
                    action,
                });
                buttons.append(&button);
            }
            row.append(&buttons);
            self.extra.scheduled_rows.append(&row);
        }
    }

    fn focus_message(&mut self) {
        let Some(id) = self.extra.focus_message.take() else {
            return;
        };
        let Some(row) = self.rows.get(&id).cloned() else {
            return;
        };
        let list = self.list.clone();
        let scroller = self.extra.scroller.clone();
        gtk::glib::idle_add_local_once(move || {
            if !row.is_mapped() {
                return;
            }
            row.set_focusable(true);
            row.grab_focus();
            if let Some(bounds) = row.compute_bounds(&list) {
                scroller.vadjustment().set_value(f64::from(bounds.y()));
            }
        });
    }

    fn tr(&self, key: &str) -> String {
        localized(&self.i18n, key)
    }

    fn load_history(&mut self, older: bool, sender: &ComponentSender<Self>) {
        let Some(chat) = &self.chat else { return };
        if self.busy || (!chat.can_view && self.extra.thread.is_none()) || (older && self.exhausted)
        {
            return;
        }
        let Some((chat_id, thread_id)) = self.context() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let offset_id = if older {
            self.history.messages.keys().next().copied().unwrap_or(0)
        } else {
            0
        };
        if thread_id > 0 {
            self.command(
                WorkerCommand::GetReplies {
                    chat_id,
                    message_id: thread_id,
                    limit: 100,
                    offset_id,
                    add_offset: 0,
                },
                Operation::Replies { append: older },
                sender,
            );
        } else {
            let _ = sender.output(DialogOutput::LoadHistory {
                chat_id,
                limit: 100,
                offset_id,
                offset_date: 0,
                add_offset: 0,
            });
        }
    }

    fn cancel_action(&mut self) {
        let had_action = self.action.take().is_some() | self.extra.queue_action.take().is_some();
        self.extra.discard_confirm = false;
        self.extra.scheduled_snapshot = None;
        self.extra.suppress.set(true);
        if had_action {
            self.editor.buffer().set_text(
                self.extra
                    .original_draft
                    .as_ref()
                    .map_or("", |draft| draft.text.as_str()),
            );
            self.reply.set_text(
                &self
                    .extra
                    .original_draft
                    .as_ref()
                    .map_or(0, |draft| draft.reply_to_message_id)
                    .to_string(),
            );
            self.markdown.set_active(false);
        } else {
            self.reply.set_text("");
        }
        self.target.set_text("");
        self.revoke.set_active(false);
        self.extra.suppress.set(false);
    }

    fn begin_action(&mut self, id: i32, action: MessageAction) {
        if self.action.is_none() && self.extra.queue_action.is_none() {
            self.extra.original_draft = self.composer_draft().ok();
        }
        let Some(message) = self.history.messages.get(&id) else {
            return;
        };
        let Some(chat) = &self.chat else { return };
        if action == MessageAction::Reply {
            if chat.can_send_plain {
                self.reply.set_text(&id.to_string());
                self.options.set_expanded(true);
                self.editor.grab_focus();
            }
            return;
        }
        if (action == MessageAction::Edit
            && (!message.outgoing || message.text.is_none() || !chat.can_send_plain))
            || (action == MessageAction::Forward && (message.noforwards || !chat.can_forward))
            || (action == MessageAction::Delete && !message.outgoing && !chat.can_delete_others)
        {
            return;
        }
        self.action = Some((id, action));
        self.target.set_text("");
        self.revoke.set_active(false);
        if action == MessageAction::Edit {
            self.editor
                .buffer()
                .set_text(message.text.as_deref().unwrap_or_default());
            self.markdown.set_active(false);
            self.editor.grab_focus();
        } else {
            self.target.grab_focus();
        }
    }

    fn submit(&mut self, sender: &ComponentSender<Self>) -> Result<(), ClientError> {
        let chat = self.chat.as_ref().ok_or(ClientError::InvalidRequest)?;
        if self.action.is_none() && !self.attachments.is_empty() {
            if self.media_submitting {
                return Err(ClientError::InvalidRequest);
            }
            let (send, command) = if let Some(album_id) = self.media_album_id {
                let send = self
                    .pending_media
                    .get(&album_id)
                    .map(|pending| pending.send.clone())
                    .or_else(|| self.media_send.clone())
                    .ok_or(ClientError::InvalidRequest)?;
                (send, WorkerCommand::RetryMedia { album_id })
            } else {
                let send = self.compose_media_send()?;
                let command = WorkerCommand::SendMedia(send.clone());
                (send, command)
            };
            if self.context() != Some((send.chat_id, send.top_msg_id)) {
                return Err(ClientError::InvalidRequest);
            }
            self.media_send = Some(send);
            self.media_submitting = true;
            self.command(command, Operation::MediaSend, sender);
            self.error = None;
            return Ok(());
        }
        let (chat_id, top_msg_id) = self.context().ok_or(ClientError::InvalidRequest)?;
        let source = self.composer_text();
        let output = match self.action {
            Some((id, MessageAction::Forward)) => {
                let to_chat_id = self
                    .target
                    .text()
                    .trim()
                    .parse::<i64>()
                    .map_err(|_| ClientError::InvalidRequest)?;
                if to_chat_id == 0 {
                    return Err(ClientError::InvalidRequest);
                }
                DialogOutput::Forward {
                    from_chat_id: chat_id,
                    message_ids: vec![id],
                    to_chat_id,
                    drop_author: false,
                }
            }
            Some((id, MessageAction::React)) => {
                let emoticon = self.target.text().trim().to_owned();
                if emoticon.encode_utf16().count() > 64 {
                    return Err(ClientError::InvalidRequest);
                }
                DialogOutput::React {
                    chat_id,
                    message_id: id,
                    emoticon,
                    document_id: 0,
                }
            }
            Some((id, MessageAction::Delete)) => DialogOutput::DeleteMessage {
                chat_id,
                message_id: id,
                revoke: self.revoke.is_active(),
            },
            action => {
                if (!chat.can_send_plain && self.extra.thread.is_none())
                    || chat.left
                    || self.extra.draft.unsupported
                {
                    return Err(ClientError::Forbidden);
                }
                let (text, mut entities_json) = outgoing_text(&source, self.markdown.is_active())?;
                if text.trim().is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                if let Some((id, MessageAction::Edit)) = action {
                    if let Some(original) = self.history.messages.get(&id)
                        && original.text.as_deref() == Some(&text)
                        && !self.markdown.is_active()
                    {
                        entities_json = original.entities_json.clone();
                    }
                    DialogOutput::EditText {
                        chat_id,
                        message_id: id,
                        text,
                        entities_json,
                    }
                } else {
                    if !self.submitted.is_empty() {
                        return Err(ClientError::InvalidRequest);
                    }
                    if let Some(original) = &self.extra.original_draft
                        && original.text == text
                        && !self.markdown.is_active()
                    {
                        entities_json = original.entities_json.clone();
                    }
                    let schedule_date = optional_number(&self.schedule.text())?
                        .map(|_| {
                            let now = gtk::glib::DateTime::now_utc()
                                .map_err(|_| ClientError::InvalidRequest)?
                                .to_unix();
                            schedule_timestamp(&self.schedule.text(), now)
                        })
                        .transpose()?;
                    let message = TextSend {
                        chat_id,
                        text,
                        entities_json,
                        reply_to_msg_id: optional_number(&self.reply.text())?.unwrap_or(0),
                        top_msg_id,
                        schedule_date,
                        silent: self.silent.is_active(),
                        webpage_url: None,
                        no_webpage: self
                            .extra
                            .original_draft
                            .as_ref()
                            .is_some_and(|draft| draft.no_webpage),
                    };
                    message.validate()?;
                    self.submitted.push_back(message.clone());
                    DialogOutput::SendText(message)
                }
            }
        };
        sender.output(output).map_err(|_| ClientError::Stopped)?;
        self.error = None;
        Ok(())
    }

    fn append_interaction_actions(
        &self,
        parent: &gtk::Box,
        message: &MessageDto,
        summary: &MediaSummary,
        sender: &ComponentSender<Self>,
    ) {
        let message_id = message.id;
        let busy = self.special.busy.contains(&message_id);
        let composer = self.composer_text();
        match &summary.interaction {
            Some(InteractionCard::Poll(poll)) => {
                let selection = self
                    .special
                    .poll_selections
                    .get(&(message.chat_id, message_id));
                for (index, answer) in poll.a.iter().enumerate() {
                    let button = gtk::CheckButton::with_label(&answer.t);
                    let chosen = selection
                        .and_then(|selection| {
                            poll_option_bytes(answer).map(|option| (selection, option))
                        })
                        .is_some_and(|(selection, option)| selection.contains(&option));
                    button.set_active(chosen || selection.is_none() && answer.c != 0);
                    button.set_sensitive(!busy && poll.x == 0);
                    let input = sender.input_sender().clone();
                    button.connect_toggled(move |_| {
                        input.emit(DialogInput::TogglePollOption { message_id, index });
                    });
                    parent.append(&button);
                }
                if busy {
                    append_label(parent, &self.tr("feature-dialog-dialog_poll_updating"));
                }
                let options = selection
                    .map(|selection| selection.iter().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                if !busy && poll_vote_allowed(poll, &options) {
                    let button =
                        gtk::Button::with_label(&self.tr("feature-dialog-dialog_poll_submit"));
                    connect(&button, sender, move || {
                        DialogInput::SubmitPollVote(message_id)
                    });
                    parent.append(&button);
                }
                if !busy && poll_retract_allowed(poll) {
                    let button =
                        gtk::Button::with_label(&self.tr("feature-dialog-dialog_poll_retract"));
                    connect(&button, sender, move || {
                        DialogInput::RetractPollVote(message_id)
                    });
                    parent.append(&button);
                }
                if !busy && poll.creator != 0 && poll.x == 0 {
                    let button =
                        gtk::Button::with_label(&self.tr("feature-dialog-dialog_poll_close"));
                    connect(&button, sender, move || DialogInput::ClosePoll(message_id));
                    parent.append(&button);
                }
                if poll.o != 0 {
                    let button = gtk::Button::with_label(
                        &self.tr("feature-dialog-dialog_poll_voters_title"),
                    );
                    connect(&button, sender, move || DialogInput::LoadPollVoters {
                        message_id,
                        append: false,
                    });
                    parent.append(&button);
                }
                if let Some(voters) = self.special.poll_voters.get(&message_id) {
                    for voter in &voters.voters {
                        append_label(parent, &format!("{} · {}", voter.title, voter.date));
                    }
                    if self
                        .special
                        .poll_next
                        .get(&message_id)
                        .is_some_and(Option::is_some)
                    {
                        let button = gtk::Button::with_label(
                            &self.tr("feature-dialog-dialog_peer_list_load_more"),
                        );
                        connect(&button, sender, move || DialogInput::LoadPollVoters {
                            message_id,
                            append: true,
                        });
                        parent.append(&button);
                    }
                }
            }
            Some(InteractionCard::Checklist(list)) => {
                let done = list.i.iter().filter(|item| item.d != 0).count();
                append_label(
                    parent,
                    &self
                        .i18n
                        .text(
                            "feature-dialog-dialog_checklist_progress",
                            &[
                                ("arg1", &done.to_string()),
                                ("arg2", &list.i.len().to_string()),
                            ],
                        )
                        .unwrap_or_default(),
                );
                for item in &list.i {
                    let button = gtk::CheckButton::with_label(&item.t);
                    button.set_active(item.d != 0);
                    button.set_sensitive(!busy && (message.outgoing || list.c != 0));
                    let input = sender.input_sender().clone();
                    let item_id = item.id;
                    button.connect_toggled(move |_| {
                        input.emit(DialogInput::Checklist {
                            message_id,
                            action: ChecklistAction::Toggle(item_id),
                        });
                    });
                    parent.append(&button);
                    if message.outgoing && !busy {
                        if !composer.trim().is_empty() && composer != item.t {
                            let edit =
                                gtk::Button::with_label(&self.tr("feature-dialog-dialog_edit"));
                            let text = composer.clone();
                            connect(&edit, sender, move || DialogInput::Checklist {
                                message_id,
                                action: ChecklistAction::EditItem(item_id, text.clone()),
                            });
                            parent.append(&edit);
                        }
                        if list.i.len() > 1 {
                            let delete =
                                gtk::Button::with_label(&self.tr("feature-dialog-dialog_delete"));
                            connect(&delete, sender, move || DialogInput::Checklist {
                                message_id,
                                action: ChecklistAction::DeleteItem(item_id),
                            });
                            parent.append(&delete);
                        }
                    }
                }
                if !busy && (message.outgoing || list.a != 0) && !composer.trim().is_empty() {
                    let button = gtk::Button::with_label(
                        &self.tr("feature-dialog-dialog_checklist_add_action"),
                    );
                    let text = composer.clone();
                    connect(&button, sender, move || DialogInput::Checklist {
                        message_id,
                        action: ChecklistAction::Append(vec![text.clone()]),
                    });
                    parent.append(&button);
                }
                if !busy && message.outgoing && !composer.trim().is_empty() && composer != list.t {
                    let button = gtk::Button::with_label(&self.tr("feature-dialog-dialog_edit"));
                    let text = composer.clone();
                    connect(&button, sender, move || DialogInput::Checklist {
                        message_id,
                        action: ChecklistAction::EditTitle(text.clone()),
                    });
                    parent.append(&button);
                }
            }
            _ => {}
        }
        let reactions = reaction_summaries(message.reactions_json.as_deref());
        if !reactions.is_empty() {
            let buttons = gtk::FlowBox::new();
            buttons.set_selection_mode(gtk::SelectionMode::None);
            for reaction in reactions {
                let label = if reaction.emoticon.is_empty() {
                    format!(
                        "{} · {}",
                        self.tr("feature-dialog-dialog_media_sticker"),
                        reaction.count
                    )
                } else {
                    format!("{} · {}", reaction.emoticon, reaction.count)
                };
                let button = gtk::Button::with_label(&label);
                button.add_css_class(if reaction.chosen {
                    "suggested-action"
                } else {
                    "flat"
                });
                let emoticon = reaction.emoticon.clone();
                connect(&button, sender, move || DialogInput::QuickReact {
                    message_id,
                    emoticon: emoticon.clone(),
                    document_id: reaction.document_id,
                });
                buttons.insert(&button, -1);
            }
            parent.append(&buttons);
            let button =
                gtk::Button::with_label(&self.tr("feature-dialog-dialog_reaction_users_title"));
            connect(&button, sender, move || DialogInput::ReactionPeers {
                message_id,
                append: false,
            });
            parent.append(&button);
            if let Some(peers) = self.special.reaction_peers.get(&message_id) {
                for peer in &peers.peers {
                    append_label(
                        parent,
                        &format!("{} · {} · {}", peer.title, peer.emoticon, peer.date),
                    );
                }
                if self
                    .special
                    .reaction_next
                    .get(&message_id)
                    .is_some_and(Option::is_some)
                {
                    let button = gtk::Button::with_label(
                        &self.tr("feature-dialog-dialog_peer_list_load_more"),
                    );
                    connect(&button, sender, move || DialogInput::ReactionPeers {
                        message_id,
                        append: true,
                    });
                    parent.append(&button);
                }
            }
        }
        if message.outgoing {
            let button = gtk::Button::with_label(&self.tr("feature-dialog-dialog_read_show_when"));
            connect(&button, sender, move || {
                DialogInput::LoadReadReceipt(message_id)
            });
            parent.append(&button);
            if let Some(read) = self.special.read_receipts.get(&message_id) {
                let text = if let Some(date) = read.date {
                    self.i18n
                        .text(
                            "feature-dialog-dialog_read_at",
                            &[("arg1", &date.to_string())],
                        )
                        .unwrap_or_default()
                } else {
                    self.tr(match read.issue {
                        Some(crate::client::interactions::ReadReceiptIssue::ChatTooBig)
                        | Some(crate::client::interactions::ReadReceiptIssue::MessageTooOld) => {
                            "feature-dialog-dialog_read_receipts_expired"
                        }
                        Some(crate::client::interactions::ReadReceiptIssue::MessageNotRead) => {
                            "feature-dialog-dialog_read_unread"
                        }
                        _ => "feature-dialog-dialog_read_unknown",
                    })
                };
                append_label(parent, &text);
            }
        }
        if summary
            .webpage
            .as_ref()
            .is_some_and(|webpage| webpage.instant_view)
            && !summary.protected
        {
            let button =
                gtk::Button::with_label(&self.tr("feature-dialog-dialog_instant_view_open"));
            connect(&button, sender, move || {
                DialogInput::OpenInstantView(message_id)
            });
            parent.append(&button);
        }
        for (row_index, row) in keyboard_rows(message.reply_markup_json.as_deref())
            .into_iter()
            .enumerate()
        {
            let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (index, keyboard) in row.into_iter().enumerate() {
                let button = gtk::Button::with_label(&keyboard.label);
                connect(&button, sender, move || DialogInput::Keyboard {
                    message_id,
                    row: row_index,
                    index,
                });
                buttons.append(&button);
            }
            parent.append(&buttons);
        }
    }

    fn append_media_actions(
        &self,
        parent: &gtk::Box,
        message: &MessageDto,
        summary: &MediaSummary,
        sender: &ComponentSender<Self>,
    ) {
        let full = self
            .media_transfers
            .get(&(message.chat_id, message.id, MediaPurpose::Full));
        let status = full.map_or(MediaStatus::Missing, |transfer| transfer.status);
        let stream = self.streams.get(&(message.chat_id, message.id));
        let stream_active = stream.is_some_and(|stream| !stream.finished);
        if let Some(stream) = stream.filter(|stream| !stream.finished) {
            let unknown = self.tr("core-ui-media_info_unknown");
            let total = stream
                .total
                .map(|total| total.to_string())
                .unwrap_or(unknown);
            let progress = gtk::ProgressBar::new();
            progress.set_show_text(true);
            progress.set_text(Some(
                &self
                    .i18n
                    .text(
                        "feature-dialog-dialog_media_download_progress",
                        &[("arg1", &stream.next_offset.to_string()), ("arg2", &total)],
                    )
                    .unwrap_or_default(),
            ));
            if let Some(total) = stream.total.filter(|total| *total > 0) {
                progress.set_fraction(stream.next_offset as f64 / total as f64);
            }
            parent.append(&progress);
        }
        if let Some(transfer) = full.filter(|transfer| transfer.status == MediaStatus::Loading) {
            let unknown = self.tr("core-ui-media_info_unknown");
            let total = transfer
                .total
                .map(|total| total.to_string())
                .unwrap_or(unknown);
            let progress = gtk::ProgressBar::new();
            progress.set_show_text(true);
            progress.set_text(Some(
                &self
                    .i18n
                    .text(
                        "feature-dialog-dialog_media_download_progress",
                        &[("arg1", &transfer.downloaded.to_string()), ("arg2", &total)],
                    )
                    .unwrap_or_default(),
            ));
            if let Some(total) = transfer.total.filter(|total| *total > 0) {
                progress.set_fraction(transfer.downloaded as f64 / total as f64);
            }
            parent.append(&progress);
        }
        if status == MediaStatus::Failed {
            append_label(parent, &self.tr("feature-dialog-dialog_media_failed"));
        } else if self
            .media_transfers
            .get(&(message.chat_id, message.id, MediaPurpose::Display))
            .is_some_and(|transfer| transfer.status == MediaStatus::Loading)
        {
            append_label(parent, &self.tr("feature-dialog-dialog_media_loading"));
        }
        let display_ready = self
            .media_transfers
            .get(&(message.chat_id, message.id, MediaPurpose::Display))
            .is_some_and(|transfer| transfer.status == MediaStatus::Ready);
        let view_status = if display_ready {
            MediaStatus::Ready
        } else {
            status
        };
        let video = matches!(
            summary.kind_key,
            "feature-dialog-dialog_media_video"
                | "feature-dialog-dialog_media_video_note"
                | "feature-dialog-dialog_media_gif"
        );
        let actions = gtk::FlowBox::new();
        actions.set_selection_mode(gtk::SelectionMode::None);
        actions.set_max_children_per_line(4);
        for (action, key, action_status) in [
            (
                MediaAction::Download,
                "feature-dialog-dialog_instant_view_media_download",
                status,
            ),
            (
                MediaAction::CancelDownload,
                "feature-dialog-dialog_media_cancel",
                status,
            ),
            (MediaAction::RetryDownload, "desktop-retry", status),
            (MediaAction::View, "core-ui-media_viewer_label", view_status),
            (
                MediaAction::Play,
                "feature-dialog-dialog_media_play",
                status,
            ),
            (MediaAction::Save, "core-ui-media_action_save", status),
            (MediaAction::SaveAs, "core-ui-media_action_save_as", status),
            (
                MediaAction::OpenExternal,
                "core-ui-media_action_open_externally",
                status,
            ),
            (
                MediaAction::Copy,
                if video {
                    "core-ui-media_action_copy_video"
                } else {
                    "core-ui-media_action_copy_image"
                },
                status,
            ),
        ] {
            let stream_cancel = action == MediaAction::CancelDownload
                && stream_active
                && stream.and_then(|stream| stream.transfer_id).is_some();
            let allowed = stream_cancel || media_action_allowed(summary, action, action_status);
            if !allowed
                || (action == MediaAction::CancelDownload
                    && !stream_cancel
                    && full.and_then(|transfer| transfer.transfer_id).is_none())
                || (action == MediaAction::Play && stream_active)
                || (matches!(action, MediaAction::Download | MediaAction::RetryDownload)
                    && stream.is_some())
            {
                continue;
            }
            let button = gtk::Button::with_label(&self.tr(key));
            let message_id = message.id;
            connect(&button, sender, move || DialogInput::Media {
                message_id,
                action,
            });
            actions.insert(&button, -1);
        }
        if !summary.protected
            && let Some(InteractionCard::Geo(card)) = &summary.interaction
            && geo_url(card).is_some()
        {
            let button =
                gtk::Button::with_label(&self.tr("feature-dialog-dialog_location_open_maps"));
            let message_id = message.id;
            connect(&button, sender, move || DialogInput::OpenGeo(message_id));
            actions.insert(&button, -1);
        }
        if !summary.protected
            && summary
                .webpage
                .as_ref()
                .and_then(|webpage| webpage.url.as_ref())
                .is_some_and(|url| url.starts_with("https://") || url.starts_with("http://"))
        {
            let button = gtk::Button::with_label(&self.tr("core-ui-media_action_open_externally"));
            let input = sender.input_sender().clone();
            let message_id = message.id;
            button.connect_clicked(move |_| {
                input.emit(DialogInput::Media {
                    message_id,
                    action: MediaAction::OpenExternal,
                });
            });
            button.set_widget_name(&format!("webpage-{message_id}"));
            actions.insert(&button, -1);
        }
        parent.append(&actions);
    }

    fn sync_rows(&mut self, sender: &ComponentSender<Self>) {
        self.ensure_media_previews(sender);
        self.ensure_sticker_views();
        self.rows.retain(|id, row| {
            if self.history.messages.contains_key(id) {
                true
            } else {
                self.list.remove(row);
                false
            }
        });
        let current_chat = self.chat.as_ref().map(|chat| chat.id);
        self.sticker_views.retain(|(chat_id, message_id), _| {
            Some(*chat_id) == current_chat && self.history.messages.contains_key(message_id)
        });
        self.custom_emoji_views
            .retain(|(chat_id, message_id, _), _| {
                Some(*chat_id) == current_chat && self.history.messages.contains_key(message_id)
            });
        let Some(chat) = &self.chat else { return };
        let album_positions = album_positions(&self.history.messages);
        let mut previous: Option<gtk::Box> = None;
        for (id, message) in &self.history.messages {
            let row = self
                .rows
                .entry(*id)
                .or_insert_with(|| gtk::Box::new(gtk::Orientation::Vertical, 4))
                .clone();
            if row.parent().is_none() {
                self.list.insert_child_after(&row, previous.as_ref());
            }
            previous = Some(row.clone());
            clear_box(&row);
            row.set_halign(if message.outgoing {
                gtk::Align::End
            } else {
                gtk::Align::Start
            });
            row.set_widget_name(&format!("message-{}-{}", chat.id, id));
            let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let name = gtk::Label::new(message.sender_name.as_deref());
            name.set_xalign(0.0);
            name.set_hexpand(true);
            meta.append(&name);
            let menu = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .build();
            menu.update_property(&[gtk::accessible::Property::Label(&localized(
                &self.i18n,
                "core-ui-media_more_actions",
            ))]);
            let popover = gtk::Popover::new();
            let actions = gtk::Box::new(gtk::Orientation::Vertical, 2);
            for (action, key, allowed) in [
                (
                    MessageAction::Reply,
                    "feature-dialog-dialog_reply",
                    chat.can_send_plain,
                ),
                (
                    MessageAction::Edit,
                    "feature-dialog-dialog_edit",
                    message.outgoing && message.text.is_some() && chat.can_send_plain,
                ),
                (
                    MessageAction::Delete,
                    "feature-dialog-dialog_delete",
                    message.outgoing || chat.can_delete_others,
                ),
                (
                    MessageAction::Forward,
                    "feature-dialog-dialog_forward",
                    chat.can_forward && !message.noforwards,
                ),
                (
                    MessageAction::React,
                    "feature-dialog-dialog_add_reaction",
                    true,
                ),
            ] {
                if !allowed {
                    continue;
                }
                let button = gtk::Button::with_label(&localized(&self.i18n, key));
                let id = *id;
                let input = sender.input_sender().clone();
                let pop = popover.clone();
                button.connect_clicked(move |_| {
                    pop.popdown();
                    input.emit(DialogInput::Action {
                        message_id: id,
                        action,
                    });
                });
                actions.append(&button);
            }
            if !message.noforwards
                && let Some(text) = message.text.as_deref().filter(|text| !text.is_empty())
            {
                let button = gtk::Button::with_label(&localized(
                    &self.i18n,
                    "core-ui-media_action_copy_caption",
                ));
                let text = text.to_owned();
                button.connect_clicked(move |_| {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                    }
                });
                actions.append(&button);
            }
            popover.set_child(Some(&actions));
            menu.set_popover(Some(&popover));
            meta.append(&menu);
            row.append(&meta);
            if (message.replies_count > 0 || message.forum_topic)
                && (!chat.is_channel || chat.is_group)
            {
                let button = gtk::Button::with_label(&format!(
                    "{} · {}",
                    localized(&self.i18n, "feature-dialog-dialog_comments_title"),
                    message.replies_count
                ));
                let chat_id = message.chat_id;
                let message_id = message.reply_to_top_id.unwrap_or(*id);
                connect(&button, sender, move || DialogInput::OpenThread {
                    chat_id,
                    message_id,
                });
                row.append(&button);
            }
            if let Some(reply) = message.reply_to_msg_id {
                let button = gtk::Button::with_label(&format!(
                    "{} · {}",
                    localized(&self.i18n, "feature-dialog-dialog_reply"),
                    reply
                ));
                connect(&button, sender, move || DialogInput::JumpTo(reply));
                row.append(&button);
            }
            if message.fwd_from.is_some() {
                append_label(
                    &row,
                    &localized(&self.i18n, "feature-dialog-dialog_forwarded_message"),
                );
            }
            render_body(
                &row,
                message.text.as_deref().unwrap_or_default(),
                message.entities_json.as_deref(),
            );
            if let Some(summary) = MediaSummary::new(message, album_positions.get(id).copied()) {
                if summary.supported() {
                    let animated_sticker = self
                        .sticker_views
                        .contains_key(&(message.chat_id, message.id));
                    let preview = [
                        MediaPurpose::Display,
                        MediaPurpose::Thumb,
                        MediaPurpose::Full,
                    ]
                    .into_iter()
                    .filter(|purpose| {
                        !animated_sticker
                            && (*purpose != MediaPurpose::Full
                                || matches!(
                                    message.media_kind.as_deref(),
                                    Some("photo" | "webpage")
                                ))
                    })
                    .find_map(|purpose| {
                        self.media_transfers
                            .get(&(message.chat_id, *id, purpose))
                            .filter(|transfer| transfer.status == MediaStatus::Ready)
                            .and_then(|transfer| transfer.lease.as_ref())
                            .map(|lease| lease.path().to_path_buf())
                    });
                    append_media_summary(&row, &summary, &self.i18n, preview.as_deref());
                    if let Some(slot) = self.sticker_views.get(&(message.chat_id, message.id)) {
                        row.append(slot.view.widget());
                    }
                    self.append_interaction_actions(&row, message, &summary, sender);
                    self.append_media_actions(&row, message, &summary, sender);
                } else {
                    append_label(
                        &row,
                        &localized(&self.i18n, "feature-dialog-dialog_instant_view_unsupported"),
                    );
                }
            }
            let emoji_ids = custom_emoji_ids(
                message.text.as_deref().unwrap_or_default(),
                message.entities_json.as_deref(),
            );
            if !emoji_ids.is_empty() {
                let emoji_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                for document_id in emoji_ids {
                    if let Some(slot) =
                        self.custom_emoji_views
                            .get(&(message.chat_id, message.id, document_id))
                    {
                        emoji_row.append(slot.view.widget());
                    } else if let Some(texture) = self
                        .custom_emoji
                        .get(&document_id)
                        .filter(|transfer| transfer.status == MediaStatus::Ready)
                        .and_then(|transfer| transfer.lease.as_ref())
                        .and_then(|lease| gtk::gdk::Texture::from_filename(lease.path()).ok())
                    {
                        let picture = gtk::Picture::for_paintable(&texture);
                        picture.set_size_request(48, 48);
                        picture.set_content_fit(gtk::ContentFit::Contain);
                        picture.update_property(&[gtk::accessible::Property::Label(&localized(
                            &self.i18n,
                            "feature-dialog-dialog_media_sticker",
                        ))]);
                        emoji_row.append(&picture);
                    }
                }
                if emoji_row.first_child().is_some() {
                    row.append(&emoji_row);
                }
            }
            let state = if message.outgoing {
                localized(
                    &self.i18n,
                    if *id <= chat.read_outbox_max_id {
                        "core-ui-status_outgoing_read"
                    } else {
                        "core-ui-status_outgoing_sent"
                    },
                )
            } else {
                String::new()
            };
            let footer = format!(
                "{} · {}{}",
                message.date,
                state,
                if message.edit_date.is_some() {
                    format!(
                        " · {}",
                        localized(&self.i18n, "feature-dialog-dialog_edited")
                    )
                } else {
                    String::new()
                }
            );
            let label = append_label(&row, &footer);
            label.add_css_class("dim-label");
        }
    }

    fn refresh_special(&self, sender: &ComponentSender<Self>) {
        self.web.widget().set_visible(self.web_open);
        self.extra.scroller.set_visible(!self.web_open);
        self.forum_expander
            .set_visible(!self.web_open && self.chat.as_ref().is_some_and(|chat| chat.is_forum));
        clear_box(&self.forum_rows);
        if self.special.forums.is_empty() {
            append_label(
                &self.forum_rows,
                &self.tr("feature-dialog-dialog_topics_empty"),
            );
        } else {
            for (id, topic) in &self.special.forums {
                let label = if topic.hidden {
                    format!(
                        "{} · {}",
                        topic.title,
                        self.tr("feature-dialog-dialog_topic_hidden")
                    )
                } else {
                    topic.title.clone()
                };
                let button = gtk::Button::with_label(&label);
                let id = *id;
                connect(&button, sender, move || DialogInput::OpenForumTopic(id));
                self.forum_rows.append(&button);
            }
            let hidden = self
                .special
                .forums
                .get(&1)
                .is_some_and(|topic| topic.hidden);
            let button = gtk::Button::with_label(&self.tr(if hidden {
                "feature-dialog-dialog_topic_show_general"
            } else {
                "feature-dialog-dialog_topic_hide_general"
            }));
            connect(&button, sender, || DialogInput::ToggleGeneralHidden);
            self.forum_rows.append(&button);
        }
        self.picker_rows
            .set_visible(!self.web_open && self.special.picker.is_some());
        clear_box(&self.picker_rows);
        match self.special.picker {
            Some(PickerKind::Emoji) => {
                let choices = if self.special.reaction_choices.is_empty() {
                    ["❤️", "👍", "👎", "😂", "😮", "😢", "🙏", "🔥", "🎉"]
                        .into_iter()
                        .map(|value| mono_proto_client::ReactionChoiceDto {
                            emoticon: value.into(),
                            document_id: 0,
                        })
                        .collect::<Vec<_>>()
                } else {
                    self.special.reaction_choices.clone()
                };
                for choice in choices {
                    if choice.emoticon.is_empty() {
                        continue;
                    }
                    let button = gtk::Button::with_label(&choice.emoticon);
                    let value = choice.emoticon.clone();
                    connect(&button, sender, move || {
                        DialogInput::InsertEmoji(value.clone())
                    });
                    self.picker_rows.append(&button);
                }
            }
            Some(PickerKind::Stickers) => {
                for (index, pack) in self.special.sticker_sets.iter().enumerate() {
                    let button = gtk::Button::with_label(&pack.title);
                    connect(&button, sender, move || DialogInput::OpenStickerPack(index));
                    self.picker_rows.append(&button);
                }
                if self.special.sticker_sets.is_empty() {
                    append_label(
                        &self.picker_rows,
                        &self.tr("feature-dialog-dialog_sticker_sets_empty"),
                    );
                }
                if let Some(pack) = &self.special.sticker_pack {
                    append_label(&self.picker_rows, &pack.title);
                    for (index, document_id) in pack.preview_document_ids.iter().enumerate() {
                        let button = gtk::Button::with_label(&format!(
                            "{} {}",
                            self.tr("feature-dialog-dialog_media_sticker"),
                            index + 1
                        ));
                        let document_id = *document_id;
                        connect(&button, sender, move || {
                            DialogInput::SendSticker(document_id)
                        });
                        self.picker_rows.append(&button);
                    }
                }
            }
            Some(PickerKind::Gifs) => {
                if self.special.gifs.is_empty() {
                    append_label(
                        &self.picker_rows,
                        &self.tr("feature-dialog-dialog_saved_gifs_empty"),
                    );
                }
                for gif in &self.special.gifs {
                    let button = gtk::Button::with_label(&format!(
                        "{} · {}×{}",
                        self.tr("feature-dialog-dialog_media_gif"),
                        gif.width.unwrap_or(0),
                        gif.height.unwrap_or(0)
                    ));
                    let document_id = gif.document_id;
                    connect(&button, sender, move || DialogInput::SendGif(document_id));
                    self.picker_rows.append(&button);
                }
            }
            None => {}
        }
        let has_suggestions = self
            .special
            .inline
            .as_ref()
            .is_some_and(|inline| !inline.results.is_empty())
            || !self.special.mentions.is_empty();
        self.suggestion_rows
            .set_visible(!self.web_open && has_suggestions);
        clear_box(&self.suggestion_rows);
        if let Some(inline) = &self.special.inline {
            for (index, result) in inline.results.iter().enumerate() {
                let title = result
                    .title
                    .as_deref()
                    .or(result.description.as_deref())
                    .unwrap_or_default();
                let button = gtk::Button::with_label(title);
                connect(&button, sender, move || {
                    DialogInput::InsertInlineResult(index)
                });
                self.suggestion_rows.append(&button);
                if let Some(description) = &result.description {
                    append_label(&self.suggestion_rows, description);
                }
            }
            if !inline.next_offset.is_empty() {
                let button =
                    gtk::Button::with_label(&self.tr("feature-dialog-dialog_peer_list_load_more"));
                connect(&button, sender, || DialogInput::LoadInlineNext);
                self.suggestion_rows.append(&button);
            }
        }
        for (index, mention) in self.special.mentions.iter().enumerate() {
            let button = gtk::Button::with_label(&mention.title);
            button.set_sensitive(mention.username.is_some());
            connect(&button, sender, move || DialogInput::SelectMention(index));
            self.suggestion_rows.append(&button);
        }
        if let Some(notice) = &self.special.callback_notice {
            self.status.set_text(notice);
            self.status.set_visible(true);
        }
    }

    fn refresh(&self, sender: &ComponentSender<Self>) {
        self.title
            .set_text(self.chat.as_ref().map_or("", |chat| chat.title.as_str()));
        self.spinner.set_spinning(self.busy);
        self.spinner.set_visible(self.busy);
        let status = if !self.online || self.error == Some(ClientError::Network) {
            self.tr("core-ui-status_network")
        } else if self.error.is_some() {
            self.tr("desktop-error-generic")
        } else if self.chat.as_ref().is_some_and(|chat| !chat.can_view) {
            self.tr("feature-dialog-dialog_cant_view")
        } else if self
            .chat
            .as_ref()
            .is_some_and(|chat| !chat.can_send_plain || chat.left)
        {
            self.tr("feature-dialog-dialog_cant_send")
        } else if self.busy {
            self.tr("desktop-loading")
        } else if self.history.messages.is_empty() {
            self.tr("desktop-empty-dialog")
        } else {
            String::new()
        };
        self.status.set_text(&status);
        self.status.set_visible(!status.is_empty());
        self.load.set_sensitive(
            !self.busy && !self.exhausted && self.chat.as_ref().is_some_and(|chat| chat.can_view),
        );
        self.load.set_label(&self.tr(if self.error.is_some() {
            "desktop-retry"
        } else {
            "feature-dialog-dialog_peer_list_load_more"
        }));
        let action = self.action.map(|(_, action)| action);
        let can_send = self
            .chat
            .as_ref()
            .is_some_and(|chat| chat.can_send_plain && !chat.left);
        let can_send_attachments = !self.attachments.is_empty()
            && self.chat.as_ref().is_some_and(|chat| {
                !chat.left
                    && if self
                        .attachments
                        .iter()
                        .any(|item| item.kind == AttachmentKind::Document)
                    {
                        chat.can_send_plain
                    } else {
                        chat.can_send_photos
                    }
            });
        let media_composer_locked = self.media_submitting
            || (self.media_album_id.is_some() && !self.attachments.is_empty());
        self.attach.set_sensitive(
            action.is_none()
                && self.submitted.is_empty()
                && !media_composer_locked
                && self.chat.as_ref().is_some_and(|chat| {
                    !chat.left && (chat.can_send_plain || chat.can_send_photos)
                }),
        );
        self.editor.set_sensitive(
            (can_send || can_send_attachments)
                && self.submitted.is_empty()
                && !media_composer_locked,
        );
        self.editor
            .set_visible(action.is_none_or(|action| action == MessageAction::Edit));
        self.options
            .set_visible(action.is_none() || action == Some(MessageAction::Edit));
        self.options
            .set_sensitive((can_send || can_send_attachments) && !media_composer_locked);
        self.target.set_visible(matches!(
            action,
            Some(MessageAction::Forward | MessageAction::React)
        ));
        self.revoke
            .set_visible(action == Some(MessageAction::Delete));
        self.cancel.set_visible(
            self.action.is_some() || !self.reply.text().is_empty() || !self.attachments.is_empty(),
        );
        self.send.set_sensitive(
            self.chat.is_some()
                && !self.media_submitting
                && (self.action.is_some()
                    || ((can_send || can_send_attachments) && self.submitted.is_empty())),
        );
        let key = match action {
            Some(MessageAction::Edit) => "feature-dialog-dialog_editing",
            Some(MessageAction::Delete) => "feature-dialog-dialog_delete_confirm",
            Some(MessageAction::Forward) => "feature-dialog-dialog_forward_to",
            Some(MessageAction::React) => "feature-dialog-dialog_add_reaction",
            _ => "desktop-send",
        };
        self.action_label.set_text(&self.tr(key));
        self.action_label.set_visible(self.action.is_some());
        self.send.set_label(&self.tr(match action {
            Some(MessageAction::Edit) => "feature-dialog-dialog_edit",
            Some(MessageAction::Delete) => "feature-dialog-dialog_delete",
            Some(MessageAction::Forward) => "feature-dialog-dialog_forward",
            Some(MessageAction::React) => "feature-dialog-dialog_add_reaction",
            _ => "desktop-send",
        }));
        self.target
            .update_property(&[gtk::accessible::Property::Label(&self.tr(key))]);
        self.target.set_placeholder_text(Some(&self.tr(key)));
        clear_box(&self.attachment_rows);
        for (index, attachment) in self.attachments.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_widget_name(&format!("attachment-{index}"));
            let label = append_label(
                &row,
                &format!(
                    "{} · {}",
                    self.tr(attachment.kind.localization_key()),
                    attachment.file_name
                ),
            );
            label.set_hexpand(true);
            let remove = gtk::Button::with_label(&self.tr("feature-dialog-dialog_attach_remove"));
            remove.set_sensitive(
                action.is_none() && self.submitted.is_empty() && !media_composer_locked,
            );
            connect(&remove, sender, move || {
                DialogInput::RemoveAttachment(index)
            });
            row.append(&remove);
            self.attachment_rows.append(&row);
        }
        clear_box(&self.pending_rows);
        for (id, pending) in &self.pending {
            if !self
                .chat
                .as_ref()
                .is_some_and(|chat| chat.id == pending.message.chat_id)
            {
                continue;
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_widget_name(&format!("pending-{id}"));
            let key = if pending.failed {
                "feature-dialog-dialog_outbox_failed"
            } else if pending.uncertain {
                "feature-dialog-dialog_outbox_uncertain"
            } else {
                "core-ui-status_outgoing_sending"
            };
            append_label(&row, &self.tr(key));
            render_body(
                &row,
                &pending.message.text,
                pending.message.entities_json.as_deref(),
            );
            if pending.failed || pending.uncertain {
                let button = gtk::Button::with_label(&self.tr("desktop-retry"));
                let id = *id;
                connect(&button, sender, move || DialogInput::Retry(id));
                row.append(&button);
            }
            self.pending_rows.append(&row);
        }
        for (album_id, pending) in &self.pending_media {
            if self.context() != Some((pending.send.chat_id, pending.send.top_msg_id)) {
                continue;
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_widget_name(&format!("pending-media-{album_id}"));
            append_label(
                &row,
                &self.tr(
                    if self.media_submitting && self.media_album_id == Some(*album_id) {
                        "feature-dialog-dialog_media_uploading"
                    } else if pending.uncertain {
                        "feature-dialog-dialog_outbox_uncertain"
                    } else {
                        "feature-dialog-dialog_outbox_waiting"
                    },
                ),
            );
            if let Some(item) = pending.send.items.first() {
                append_label(&row, &item.file_name);
                if !item.caption.is_empty() {
                    render_body(&row, &item.caption, item.entities_json.as_deref());
                }
            }
            if !self.media_submitting {
                let album_id = *album_id;
                let button = gtk::Button::with_label(&self.tr("desktop-retry"));
                connect(&button, sender, move || DialogInput::RetryMedia(album_id));
                row.append(&button);
            }
            self.pending_rows.append(&row);
        }
        self.schedule.set_editable(true);
        if self.extra.draft.unsupported {
            self.editor.set_sensitive(false);
            self.send.set_sensitive(false);
        }
        if let Some((_, queue_action)) = self.extra.queue_action {
            let key = match queue_action {
                ScheduledAction::Edit => "feature-dialog-dialog_editing",
                ScheduledAction::Reschedule => "feature-dialog-dialog_scheduled_reschedule",
                ScheduledAction::SendNow => "feature-dialog-dialog_scheduled_send_confirm",
                ScheduledAction::Delete => "feature-dialog-dialog_scheduled_cancel_confirm",
            };
            self.action_label.set_text(&self.tr(key));
            self.action_label.set_visible(true);
            self.send.set_label(&self.tr(match queue_action {
                ScheduledAction::Edit => "feature-dialog-dialog_edit",
                ScheduledAction::Reschedule => "feature-dialog-dialog_scheduled_reschedule",
                ScheduledAction::SendNow => "feature-dialog-dialog_scheduled_send_now",
                ScheduledAction::Delete => "feature-dialog-dialog_scheduled_cancel",
            }));
            self.editor
                .set_visible(queue_action == ScheduledAction::Edit);
            self.editor.set_sensitive(!self.extra.queue_busy);
            self.options.set_visible(matches!(
                queue_action,
                ScheduledAction::Edit | ScheduledAction::Reschedule
            ));
            self.options.set_sensitive(!self.extra.queue_busy);
            self.schedule
                .set_editable(queue_action == ScheduledAction::Reschedule);
            self.target.set_visible(false);
            self.revoke.set_visible(false);
            self.cancel.set_visible(true);
            self.send.set_sensitive(!self.extra.queue_busy);
        }
        if self.extra.discard_confirm {
            self.action_label
                .set_text(&self.tr("feature-dialog-dialog_discard_draft_confirm"));
            self.action_label.set_visible(true);
            self.send
                .set_label(&self.tr("feature-dialog-dialog_discard_draft"));
            self.send
                .set_sensitive(self.extra.draft.ready && !self.extra.draft_inflight);
            self.cancel.set_visible(true);
        }
        if let Some(key) = self.extra.notice {
            self.status.set_text(&self.tr(key));
            self.status.set_visible(true);
        }
        self.refresh_extra(sender);
        self.refresh_special(sender);
    }
}

impl Drop for DialogPage {
    fn drop(&mut self) {
        self.cleanup_streams();
        if let Some(source) = self.extra.debounce.take() {
            source.remove();
        }
        if let Some(source) = self.extra.visibility_source.take() {
            source.remove();
        }
    }
}

fn in_context(message: &MessageDto, context: Option<(i64, i32)>) -> bool {
    context.is_some_and(|(chat_id, top)| {
        message.chat_id == chat_id
            && (top == 0
                || message.id == top
                || message.reply_to_top_id == Some(top)
                || message.reply_to_msg_id == Some(top))
    })
}

fn localized(i18n: &I18n, key: &str) -> String {
    i18n.text(key, &[])
        .or_else(|_| i18n.text("desktop-error-generic", &[]))
        .unwrap_or_default()
}

fn connect(
    button: &gtk::Button,
    sender: &ComponentSender<DialogPage>,
    input: impl Fn() -> DialogInput + 'static,
) {
    let sender = sender.input_sender().clone();
    button.connect_clicked(move |_| sender.emit(input()));
}

fn numeric_field(parent: &gtk::Box, label: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .placeholder_text(label)
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(label)]);
    parent.append(&gtk::Label::new(Some(label)));
    parent.append(&entry);
    entry
}

fn optional_number(value: &str) -> Result<Option<i32>, ClientError> {
    if value.trim().is_empty() || value.trim() == "0" {
        return Ok(None);
    }
    let value = value
        .trim()
        .parse::<i32>()
        .map_err(|_| ClientError::InvalidRequest)?;
    if value <= 0 || value == i32::MAX {
        return Err(ClientError::InvalidRequest);
    }
    Ok(Some(value))
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn append_label(parent: &gtk::Box, text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("monogram-message-text");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    parent.append(&label);
    label
}

fn append_interaction_summary(
    parent: &gtk::Box,
    interaction: &InteractionCard,
    kind_key: &str,
    i18n: &I18n,
) {
    match interaction {
        InteractionCard::Poll(poll) => {
            append_label(parent, &poll.q);
            for (index, answer) in poll.a.iter().enumerate() {
                if answer.k != 0 {
                    append_label(
                        parent,
                        &i18n
                            .text(
                                "feature-dialog-dialog_poll_correct_option",
                                &[("arg1", &(index + 1).to_string())],
                            )
                            .unwrap_or_default(),
                    );
                }
                append_label(parent, &answer.t);
                if answer.v > 0 {
                    append_label(
                        parent,
                        &format!(
                            "{}: {}",
                            localized(i18n, "feature-dialog-dialog_poll_voters_title"),
                            answer.v
                        ),
                    );
                }
            }
            if poll.n > 0 {
                append_label(
                    parent,
                    &format!(
                        "{}: {}",
                        localized(i18n, "feature-dialog-dialog_poll_voters_title"),
                        poll.n
                    ),
                );
            }
            if poll.x != 0 {
                append_label(
                    parent,
                    &localized(i18n, "feature-dialog-dialog_poll_closed"),
                );
            }
            if !poll.s.is_empty() {
                append_label(
                    parent,
                    &i18n
                        .text("feature-dialog-dialog_poll_solution", &[("arg1", &poll.s)])
                        .unwrap_or_default(),
                );
            }
        }
        InteractionCard::Checklist(checklist) => {
            append_label(parent, &checklist.t);
            for item in &checklist.i {
                append_label(
                    parent,
                    &format!(
                        "{} · {}",
                        localized(
                            i18n,
                            if item.d != 0 {
                                "feature-dialog-dialog_task_done"
                            } else {
                                "feature-dialog-dialog_task_open"
                            }
                        ),
                        item.t
                    ),
                );
            }
        }
        InteractionCard::Contact(contact) => {
            append_label(parent, format!("{} {}", contact.f, contact.l).trim());
            if !contact.p.is_empty() {
                append_label(parent, &contact.p);
            }
        }
        InteractionCard::Geo(geo) => {
            for value in [geo.t.as_deref(), geo.a.as_deref()].into_iter().flatten() {
                if !value.is_empty() {
                    append_label(parent, value);
                }
            }
            if let (Some(latitude), Some(longitude)) = (geo.lat, geo.long) {
                append_label(parent, &format!("{latitude}, {longitude}"));
            }
            if geo.live.is_some_and(|live| live != 0) {
                append_label(
                    parent,
                    &localized(i18n, "feature-dialog-dialog_location_live"),
                );
            }
        }
        InteractionCard::Dice(dice) => {
            append_label(parent, &dice.e);
            append_label(
                parent,
                &if dice.v > 0 {
                    i18n.text(
                        "feature-dialog-dialog_dice_result",
                        &[("arg1", &dice.v.to_string())],
                    )
                    .unwrap_or_default()
                } else {
                    localized(i18n, "feature-dialog-dialog_dice_pending")
                },
            );
        }
        InteractionCard::Announcement(announcement) => {
            for value in [
                announcement.t.as_str(),
                announcement.s.as_str(),
                announcement.d.as_str(),
            ] {
                if !value.is_empty() {
                    append_label(parent, value);
                }
            }
            match kind_key {
                "feature-dialog-dialog_media_giveaway" => {
                    if announcement.m > 0 {
                        append_label(
                            parent,
                            &i18n
                                .text(
                                    "feature-dialog-dialog_media_giveaway_months",
                                    &[("arg1", &announcement.m.to_string())],
                                )
                                .unwrap_or_default(),
                        );
                    }
                    if announcement.q > 0 {
                        append_label(parent, &announcement.q.to_string());
                    }
                    if announcement.x > 0 {
                        append_label(
                            parent,
                            &i18n
                                .text(
                                    "feature-dialog-dialog_media_giveaway_stars",
                                    &[("arg1", &announcement.x.to_string())],
                                )
                                .unwrap_or_default(),
                        );
                    }
                }
                "feature-dialog-dialog_media_giveaway_results" => {
                    if announcement.w > 0 || announcement.u > 0 {
                        append_label(
                            parent,
                            &i18n
                                .text(
                                    "feature-dialog-dialog_media_giveaway_winners",
                                    &[
                                        ("arg1", &announcement.w.to_string()),
                                        ("arg2", &announcement.u.to_string()),
                                    ],
                                )
                                .unwrap_or_default(),
                        );
                    }
                }
                "feature-dialog-dialog_media_paid_media" => {
                    append_label(
                        parent,
                        &i18n
                            .text(
                                "feature-dialog-dialog_media_paid_media_items",
                                &[
                                    ("arg1", &announcement.x.to_string()),
                                    ("arg2", &announcement.i.to_string()),
                                ],
                            )
                            .unwrap_or_default(),
                    );
                }
                "feature-dialog-dialog_media_story" => {
                    if announcement.n > 0 {
                        append_label(
                            parent,
                            &i18n
                                .text(
                                    "feature-dialog-dialog_media_story_number",
                                    &[("arg1", &announcement.n.to_string())],
                                )
                                .unwrap_or_default(),
                        );
                    }
                }
                "feature-dialog-dialog_media_video_stream" if announcement.r != 0 => {
                    append_label(
                        parent,
                        &localized(i18n, "feature-dialog-dialog_media_rtmp_stream"),
                    );
                }
                _ => {}
            }
        }
    }
}

fn append_media_summary(
    parent: &gtk::Box,
    summary: &MediaSummary,
    i18n: &I18n,
    preview: Option<&std::path::Path>,
) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 3);
    container.add_css_class("card");
    container.set_margin_top(4);
    container.set_margin_bottom(4);
    if let Some(path) = preview {
        let picture = gtk::Picture::for_filename(path);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_size_request(
            summary.width.unwrap_or(320).clamp(96, 480),
            summary.height.unwrap_or(240).clamp(90, 360),
        );
        picture.update_property(&[gtk::accessible::Property::Label(&localized(
            i18n,
            summary.kind_key,
        ))]);
        container.append(&picture);
    }
    let mut heading = localized(i18n, summary.kind_key);
    if let Some((position, total)) = summary.album_position {
        let position = i18n
            .text(
                "core-ui-media_viewer_page_position",
                &[
                    ("arg1", &position.to_string()),
                    ("arg2", &total.to_string()),
                ],
            )
            .unwrap_or_else(|_| format!("{position} / {total}"));
        heading = format!("{heading} · {position}");
    }
    append_label(&container, &heading).add_css_class("heading");
    if let Some(webpage) = &summary.webpage {
        for value in [
            webpage.title.as_deref(),
            webpage.site_name.as_deref(),
            webpage.description.as_deref(),
            webpage.url.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        {
            append_label(&container, value);
        }
    }
    if let Some(interaction) = &summary.interaction {
        append_interaction_summary(&container, interaction, summary.kind_key, i18n);
    }
    if let Some(name) = summary.file_name.as_deref().filter(|name| !name.is_empty()) {
        append_label(
            &container,
            &format!("{}: {name}", localized(i18n, "core-ui-media_info_name")),
        );
    }
    if let (Some(width), Some(height)) = (summary.width, summary.height)
        && width > 0
        && height > 0
    {
        let value = i18n
            .text(
                "core-ui-media_info_dimensions_value",
                &[("arg1", &width.to_string()), ("arg2", &height.to_string())],
            )
            .unwrap_or_else(|_| format!("{width} × {height}"));
        append_label(&container, &value);
    }
    if let Some(duration) = summary.duration.filter(|duration| *duration >= 0) {
        append_label(
            &container,
            &format!(
                "{}: {duration}",
                localized(i18n, "core-ui-media_info_duration")
            ),
        );
    }
    if let Some(size) = summary.file_size.filter(|size| *size >= 0) {
        append_label(
            &container,
            &format!("{}: {size}", localized(i18n, "core-ui-media_info_size")),
        );
    }
    if summary.supports_streaming {
        append_label(
            &container,
            &localized(i18n, "feature-dialog-dialog_media_video_stream"),
        );
    }
    parent.append(&container);
}

#[derive(Deserialize)]
struct WireEntity {
    kind: String,
    offset: i32,
    length: i32,
    #[serde(default, alias = "extra")]
    url: Option<String>,
}

fn entities(text: &str, json: Option<&str>) -> Result<Vec<MarkupEntityDto>, ClientError> {
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    if json.len() > MAX_INPUT_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    let wire: Vec<WireEntity> =
        serde_json::from_str(json).map_err(|_| ClientError::InvalidRequest)?;
    if wire.len() > MAX_ENTITIES {
        return Err(ClientError::InvalidRequest);
    }
    let index = TextIndex::new(text);
    wire.into_iter()
        .map(|entity| {
            index
                .utf16_range(entity.offset, entity.length)
                .ok_or(ClientError::InvalidRequest)?;
            Ok(MarkupEntityDto {
                kind: entity.kind,
                offset: entity.offset,
                length: entity.length,
                extra: entity.url,
            })
        })
        .collect()
}

fn outgoing_text(source: &str, markdown: bool) -> Result<(String, Option<String>), ClientError> {
    if source.len() > MAX_INPUT_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    if !markdown {
        return Ok((source.to_owned(), None));
    }
    let parsed = monogram_markup::parse_telegram_markdown(source.to_owned());
    let wire: Vec<_> = parsed.entities.iter().map(|entity| serde_json::json!({
        "kind": entity.kind, "offset": entity.offset, "length": entity.length, "url": entity.extra,
    })).collect();
    let json = serde_json::to_string(&wire).map_err(|_| ClientError::InvalidRequest)?;
    entities(&parsed.text, Some(&json))?;
    Ok((parsed.text, Some(json)))
}

fn render_rich(parent: &gtk::Box, rich: &RichText) {
    let text = rich
        .selected_text(0..rich.text().chars().count(), &[])
        .unwrap_or_default();
    let label = append_label(parent, &text);
    if text == rich.text() {
        label.set_attributes(Some(&rich.pango_attributes(&[])));
    }
}

fn render_body(parent: &gtk::Box, text: &str, json: Option<&str>) {
    let document = entities(text, json)
        .ok()
        .and_then(|entities| Document::parse(text, entities, ParseMode::TelegramEntities).ok());
    let Some(document) = document else {
        append_label(parent, text);
        return;
    };
    for block in document.blocks {
        match block {
            Block::Paragraph(text)
            | Block::Heading { text, .. }
            | Block::Quote { text, .. }
            | Block::Tasks { text, .. }
            | Block::Details { text, .. }
            | Block::Unsupported { text, .. } => render_rich(parent, &text),
            Block::Code(code) => render_rich(parent, &code.text),
            Block::Table { headers, rows } => {
                for cell in headers.into_iter().chain(rows.into_iter().flatten()) {
                    render_rich(parent, &cell);
                }
            }
            Block::Rule => parent.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
        }
    }
}

fn schedule_timestamp(value: &str, now: i64) -> Result<i32, ClientError> {
    let date = optional_number(value)?.ok_or(ClientError::InvalidRequest)?;
    let delta = i64::from(date) - now;
    if !(60..=365 * 24 * 60 * 60).contains(&delta) {
        return Err(ClientError::InvalidRequest);
    }
    Ok(date)
}

fn scheduled_unchanged(left: &ScheduledMessageDto, right: &ScheduledMessageDto) -> bool {
    left.message.chat_id == right.message.chat_id
        && left.message.id == right.message.id
        && left.message.date == right.message.date
        && left.message.edit_date == right.message.edit_date
        && left.message.text == right.message.text
        && left.message.entities_json == right.message.entities_json
        && left.has_unsupported_content == right.has_unsupported_content
}

#[cfg(test)]
#[path = "../../tests/unit/dialog_ui.rs"]
pub(crate) mod tests;
