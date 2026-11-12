use super::*;
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
};

use proto::{
    NotifyExceptionDto, NotifySettingsDto,
    adapter::protocol::latest::api::{
        Bool, BoolFalseConstructor, BoolTrueConstructor, InputPeerNotifySettings,
        InputPeerNotifySettingsConstructor, NotificationSound, NotificationSoundDefaultConstructor,
        NotificationSoundNoneConstructor, PeerNotifySettings,
    },
    adapter::socket::{ProxyConfig, ProxyKind, TransportMode},
    adapter::types::auth_types::DeviceSessionDto,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const SECRET_SERVICE: &str = "io.github.monogram.desktop";
const CONFIG_FILE: &str = "connectivity.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyScope {
    Users,
    Chats,
    Broadcasts,
}

impl NotifyScope {
    fn peer_kind(self) -> &'static str {
        match self {
            Self::Users => "users",
            Self::Chats => "chats",
            Self::Broadcasts => "broadcasts",
        }
    }
}

pub struct ProxySpec {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub mtproto_secret: Option<Zeroizing<[u8; 16]>>,
    pub fake_tls_domain: Option<String>,
}

impl fmt::Debug for ProxySpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProxySpec")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field(
                "mtproto_secret",
                &self.mtproto_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("fake_tls_domain", &self.fake_tls_domain)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ProxyDto {
    pub id: u64,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub fake_tls_domain: Option<String>,
    pub has_secret: bool,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub id: u64,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NotifyUpdate {
    pub show_previews: Option<bool>,
    pub silent: bool,
    pub mute_until: i32,
    pub stories_muted: bool,
    pub sound: String,
}

impl NotifyUpdate {
    pub fn reset() -> Self {
        Self {
            show_previews: None,
            silent: false,
            mute_until: 0,
            stories_muted: false,
            sound: "default".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotifyDefaults {
    pub users: NotifySettingsDto,
    pub chats: NotifySettingsDto,
    pub broadcasts: NotifySettingsDto,
}

pub enum ConnectivityCommand {
    ListProxies,
    AddProxy {
        spec: ProxySpec,
    },
    EditProxy {
        id: u64,
        spec: ProxySpec,
    },
    DeleteProxy {
        id: u64,
    },
    ActivateProxy {
        id: u64,
    },
    SetDirect,
    GetActiveProxy,
    ImportProxyLink {
        link: String,
    },
    GetTransportMode,
    SetTransportMode {
        mode: TransportMode,
    },
    ProbeProxies {
        ids: Vec<u64>,
    },
    GetNotifyDefaults,
    SetNotifyDefault {
        scope: NotifyScope,
        settings: NotifyUpdate,
    },
    GetNotifyExceptions,
    ResetNotify,
    GetPeerNotify {
        chat_id: i64,
    },
    SetPeerNotify {
        chat_id: i64,
        settings: NotifyUpdate,
    },
    ResetPeerNotify {
        chat_id: i64,
    },
    GetTopicNotify {
        chat_id: i64,
        topic_id: i32,
    },
    SetTopicNotify {
        chat_id: i64,
        topic_id: i32,
        settings: NotifyUpdate,
    },
    ResetTopicNotify {
        chat_id: i64,
        topic_id: i32,
    },
    GetDeviceSessions,
    AcceptDeviceSession {
        token: Vec<u8>,
    },
    RevokeDeviceSession {
        hash: i64,
    },
    PollPending2fa,
}

pub enum ConnectivityResponse {
    Done,
    Proxy(Box<ProxyDto>),
    Proxies(Vec<ProxyDto>),
    ActiveProxy(Option<Box<ProxyDto>>),
    TransportMode(TransportMode),
    Probes(Vec<ProbeResult>),
    NotifyDefaults(Box<NotifyDefaults>),
    NotifySettings(NotifySettingsDto),
    NotifyExceptions(Vec<NotifyExceptionDto>),
    DeviceSessions(Vec<DeviceSessionDto>),
    DeviceSession(Box<DeviceSessionDto>),
    Pending2fa(Vec<DeviceSessionDto>),
}

impl Runtime {
    pub(super) fn execute_connectivity(
        &mut self,
        command: ConnectivityCommand,
    ) -> Result<ConnectivityResponse, ClientError> {
        match command {
            ConnectivityCommand::ListProxies => ProxyStore::open()?
                .list()
                .map(ConnectivityResponse::Proxies),
            ConnectivityCommand::AddProxy { spec } => {
                let proxy = ProxyStore::open()?.add(spec)?;
                Ok(ConnectivityResponse::Proxy(Box::new(proxy)))
            }
            ConnectivityCommand::EditProxy { id, spec } => {
                let proxy = ProxyStore::open()?.edit(id, spec)?;
                Ok(ConnectivityResponse::Proxy(Box::new(proxy)))
            }
            ConnectivityCommand::DeleteProxy { id } => {
                ProxyStore::open()?.delete(id)?;
                Ok(ConnectivityResponse::Done)
            }
            ConnectivityCommand::ActivateProxy { id } => {
                let store = ProxyStore::open()?;
                let config = store.config_for(id)?;
                let mode = store.transport()?;
                validate_transport_combo(mode, Some(&config))?;
                let proxy = store.activate(id)?;
                set_active_proxy(Some(config))?;
                Ok(ConnectivityResponse::ActiveProxy(Some(Box::new(proxy))))
            }
            ConnectivityCommand::SetDirect => {
                ProxyStore::open()?.set_direct()?;
                set_active_proxy(None)?;
                Ok(ConnectivityResponse::ActiveProxy(None))
            }
            ConnectivityCommand::GetActiveProxy => Ok(ConnectivityResponse::ActiveProxy(
                ProxyStore::open()?.active()?.map(Box::new),
            )),
            ConnectivityCommand::ImportProxyLink { link } => {
                let spec = parse_proxy_link(&link)?;
                let proxy = ProxyStore::open()?.add(spec)?;
                Ok(ConnectivityResponse::Proxy(Box::new(proxy)))
            }
            ConnectivityCommand::GetTransportMode => Ok(ConnectivityResponse::TransportMode(
                ProxyStore::open()?.transport()?,
            )),
            ConnectivityCommand::SetTransportMode { mode } => {
                let store = ProxyStore::open()?;
                let active = store.active_config()?;
                validate_transport_combo(mode, active.as_ref())?;
                store.set_transport(mode)?;
                apply_transport_mode(mode);
                Ok(ConnectivityResponse::TransportMode(mode))
            }
            ConnectivityCommand::ProbeProxies { ids } => {
                validate_probe_ids(&ids)?;
                let store = ProxyStore::open()?;
                let mut targets = Vec::with_capacity(ids.len());
                for id in ids {
                    targets.push((id, store.config_for(id)?));
                }
                Ok(ConnectivityResponse::Probes(run_probes(
                    &targets,
                    &real_probe,
                )))
            }
            ConnectivityCommand::GetNotifyDefaults => {
                let handle = self.authorized()?;
                let users = notify_settings(handle, "users", 0)?;
                let chats = notify_settings(handle, "chats", 0)?;
                let broadcasts = notify_settings(handle, "broadcasts", 0)?;
                Ok(ConnectivityResponse::NotifyDefaults(Box::new(
                    NotifyDefaults {
                        users,
                        chats,
                        broadcasts,
                    },
                )))
            }
            ConnectivityCommand::SetNotifyDefault { scope, settings } => {
                validate_notify_update(&settings)?;
                let handle = self.authorized()?;
                update_notify_settings(handle, scope.peer_kind(), 0, &settings)?;
                Ok(ConnectivityResponse::NotifySettings(notify_settings(
                    handle,
                    scope.peer_kind(),
                    0,
                )?))
            }
            ConnectivityCommand::GetNotifyExceptions => {
                let handle = self.authorized()?;
                let exceptions = proto::dispatch::get_notify_exceptions(handle, true)
                    .map_err(ClientError::from)?;
                Ok(ConnectivityResponse::NotifyExceptions(exceptions))
            }
            ConnectivityCommand::ResetNotify => {
                let handle = self.authorized()?;
                proto::dispatch::reset_notify_settings(handle).map_err(ClientError::from)?;
                Ok(ConnectivityResponse::Done)
            }
            ConnectivityCommand::GetPeerNotify { chat_id } => {
                features::valid_chat(chat_id)?;
                let handle = self.authorized()?;
                Ok(ConnectivityResponse::NotifySettings(notify_settings(
                    handle, "peer", chat_id,
                )?))
            }
            ConnectivityCommand::SetPeerNotify { chat_id, settings } => {
                features::valid_chat(chat_id)?;
                validate_notify_update(&settings)?;
                let handle = self.authorized()?;
                update_notify_settings(handle, "peer", chat_id, &settings)?;
                Ok(ConnectivityResponse::NotifySettings(notify_settings(
                    handle, "peer", chat_id,
                )?))
            }
            ConnectivityCommand::ResetPeerNotify { chat_id } => {
                features::valid_chat(chat_id)?;
                let handle = self.authorized()?;
                update_notify_settings(handle, "peer", chat_id, &NotifyUpdate::reset())?;
                Ok(ConnectivityResponse::NotifySettings(notify_settings(
                    handle, "peer", chat_id,
                )?))
            }
            ConnectivityCommand::GetTopicNotify { chat_id, topic_id } => {
                features::valid_chat(chat_id)?;
                validate_topic(topic_id)?;
                let handle = self.authorized()?;
                Ok(ConnectivityResponse::NotifySettings(topic_settings(
                    handle, chat_id, topic_id,
                )?))
            }
            ConnectivityCommand::SetTopicNotify {
                chat_id,
                topic_id,
                settings,
            } => {
                features::valid_chat(chat_id)?;
                validate_topic(topic_id)?;
                validate_notify_update(&settings)?;
                let handle = self.authorized()?;
                proto::forum_management::set_topic_notifications(
                    handle,
                    chat_id,
                    topic_id,
                    topic_notify_settings(&settings),
                )
                .map_err(ClientError::from)?;
                Ok(ConnectivityResponse::NotifySettings(topic_settings(
                    handle, chat_id, topic_id,
                )?))
            }
            ConnectivityCommand::ResetTopicNotify { chat_id, topic_id } => {
                features::valid_chat(chat_id)?;
                validate_topic(topic_id)?;
                let handle = self.authorized()?;
                proto::forum_management::set_topic_notifications(
                    handle,
                    chat_id,
                    topic_id,
                    topic_notify_settings(&NotifyUpdate::reset()),
                )
                .map_err(ClientError::from)?;
                Ok(ConnectivityResponse::NotifySettings(topic_settings(
                    handle, chat_id, topic_id,
                )?))
            }
            ConnectivityCommand::GetDeviceSessions => {
                let handle = self.authorized()?;
                let sessions =
                    proto::auth::get_device_sessions(handle).map_err(ClientError::from)?;
                Ok(ConnectivityResponse::DeviceSessions(sessions))
            }
            ConnectivityCommand::AcceptDeviceSession { token } => {
                let handle = self.authorized()?;
                let session =
                    proto::auth::accept_login_token(handle, token).map_err(ClientError::from)?;
                Ok(ConnectivityResponse::DeviceSession(Box::new(session)))
            }
            ConnectivityCommand::RevokeDeviceSession { hash } => {
                let handle = self.authorized()?;
                proto::auth::revoke_device_session(handle, hash).map_err(ClientError::from)?;
                Ok(ConnectivityResponse::Done)
            }
            ConnectivityCommand::PollPending2fa => {
                let handle = self.authorized()?;
                let sessions =
                    proto::auth::get_device_sessions(handle).map_err(ClientError::from)?;
                let pending = sessions
                    .into_iter()
                    .filter(|session| session.password_pending)
                    .collect();
                Ok(ConnectivityResponse::Pending2fa(pending))
            }
        }
    }
}

pub(super) fn command_class(command: &ConnectivityCommand) -> scheduler::RequestClass {
    match command {
        ConnectivityCommand::ListProxies
        | ConnectivityCommand::GetActiveProxy
        | ConnectivityCommand::GetTransportMode
        | ConnectivityCommand::GetNotifyDefaults
        | ConnectivityCommand::GetNotifyExceptions
        | ConnectivityCommand::GetPeerNotify { .. }
        | ConnectivityCommand::GetTopicNotify { .. }
        | ConnectivityCommand::GetDeviceSessions
        | ConnectivityCommand::PollPending2fa => scheduler::RequestClass::InteractiveRead,
        ConnectivityCommand::ProbeProxies { .. } => scheduler::RequestClass::BackgroundRead,
        _ => scheduler::RequestClass::InteractiveWrite,
    }
}

fn set_active_proxy(config: Option<ProxyConfig>) -> Result<(), ClientError> {
    proto::adapter::runtime::connection::tcp::set_proxy(config)
        .map_err(|_| ClientError::InvalidRequest)
}

fn apply_transport_mode(mode: TransportMode) {
    proto::adapter::runtime::connection::tcp::set_transport_mode(mode);
}

fn real_probe(config: ProxyConfig) -> Result<i64, String> {
    proto::adapter::runtime::connection::tcp::probe_proxy(config).map_err(|error| error.to_string())
}

fn notify_settings(
    handle: u64,
    peer_kind: &str,
    chat_id: i64,
) -> Result<NotifySettingsDto, ClientError> {
    proto::dispatch::get_notify_settings(handle, peer_kind.to_owned(), chat_id)
        .map_err(ClientError::from)
}

fn update_notify_settings(
    handle: u64,
    peer_kind: &str,
    chat_id: i64,
    settings: &NotifyUpdate,
) -> Result<(), ClientError> {
    proto::dispatch::update_notify_settings(
        handle,
        peer_kind.to_owned(),
        chat_id,
        settings.show_previews,
        settings.silent,
        settings.mute_until,
        settings.stories_muted,
        settings.sound.clone(),
    )
    .map_err(ClientError::from)
}

fn topic_settings(
    handle: u64,
    chat_id: i64,
    topic_id: i32,
) -> Result<NotifySettingsDto, ClientError> {
    let raw = proto::forum_management::get_topic_notifications(handle, chat_id, topic_id)
        .map_err(ClientError::from)?;
    Ok(settings_from_topic(&raw))
}

fn validate_notify_update(update: &NotifyUpdate) -> Result<(), ClientError> {
    if update.mute_until < 0 || !matches!(update.sound.as_str(), "" | "keep" | "none" | "default") {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_topic(topic_id: i32) -> Result<(), ClientError> {
    if topic_id <= 0 {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn tl_bool(value: bool) -> Box<Bool> {
    if value {
        Box::new(Bool::BoolTrue(BoolTrueConstructor {}))
    } else {
        Box::new(Bool::BoolFalse(BoolFalseConstructor {}))
    }
}

fn bool_of(flag: &Bool) -> bool {
    matches!(flag, Bool::BoolTrue(_))
}

fn topic_sound(name: &str) -> Option<Box<NotificationSound>> {
    match name {
        "" | "keep" => None,
        "none" => Some(Box::new(NotificationSound::NotificationSoundNone(
            NotificationSoundNoneConstructor {},
        ))),
        _ => Some(Box::new(NotificationSound::NotificationSoundDefault(
            NotificationSoundDefaultConstructor {},
        ))),
    }
}

fn topic_sound_name(sound: Option<&NotificationSound>) -> String {
    match sound {
        Some(NotificationSound::NotificationSoundNone(_)) => "none".into(),
        Some(NotificationSound::NotificationSoundDefault(_)) => "default".into(),
        Some(_) => "custom".into(),
        None => "default".into(),
    }
}

fn topic_notify_settings(update: &NotifyUpdate) -> InputPeerNotifySettings {
    let mut flags = InputPeerNotifySettingsConstructor::SILENT_FLAG
        | InputPeerNotifySettingsConstructor::MUTE_UNTIL_FLAG
        | InputPeerNotifySettingsConstructor::STORIES_MUTED_FLAG;
    let sound = topic_sound(&update.sound);
    if update.show_previews.is_some() {
        flags |= InputPeerNotifySettingsConstructor::SHOW_PREVIEWS_FLAG;
    }
    if sound.is_some() {
        flags |= InputPeerNotifySettingsConstructor::SOUND_FLAG;
    }
    InputPeerNotifySettings::InputPeerNotifySettings(InputPeerNotifySettingsConstructor {
        flags,
        show_previews: update.show_previews.map(tl_bool),
        silent: Some(tl_bool(update.silent)),
        mute_until: Some(update.mute_until),
        sound,
        stories_muted: Some(tl_bool(update.stories_muted)),
        stories_hide_sender: None,
        stories_sound: None,
    })
}

fn settings_from_topic(settings: &PeerNotifySettings) -> NotifySettingsDto {
    let PeerNotifySettings::PeerNotifySettings(body) = settings;
    NotifySettingsDto {
        show_previews: body
            .show_previews
            .as_ref()
            .map(|flag| bool_of(flag.as_ref())),
        silent: body
            .silent
            .as_ref()
            .is_some_and(|flag| bool_of(flag.as_ref())),
        mute_until: body.mute_until.unwrap_or(0),
        stories_muted: body
            .stories_muted
            .as_ref()
            .is_some_and(|flag| bool_of(flag.as_ref())),
        stories_hide_sender: body
            .stories_hide_sender
            .as_ref()
            .is_some_and(|flag| bool_of(flag.as_ref())),
        sound: topic_sound_name(
            body.android_sound
                .as_deref()
                .or(body.other_sound.as_deref()),
        ),
    }
}

fn validate_transport_combo(
    mode: TransportMode,
    active: Option<&ProxyConfig>,
) -> Result<(), ClientError> {
    if mode == TransportMode::Http && active.is_some_and(|config| config.kind == ProxyKind::Mtproto)
    {
        return Err(ClientError::UnsupportedContent);
    }
    Ok(())
}

fn validate_probe_ids(ids: &[u64]) -> Result<(), ClientError> {
    if ids.is_empty() || ids.contains(&0) || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_spec(spec: &ProxySpec) -> Result<(), ClientError> {
    if spec.kind == ProxyKind::Mtproto {
        if spec.username.is_some() || spec.password.is_some() || spec.mtproto_secret.is_none() {
            return Err(ClientError::InvalidRequest);
        }
    } else if spec.mtproto_secret.is_some() || spec.fake_tls_domain.is_some() {
        return Err(ClientError::InvalidRequest);
    }
    let config = ProxyConfig {
        kind: spec.kind.clone(),
        host: spec.host.clone(),
        port: spec.port,
        username: spec.username.clone(),
        password: spec.password.as_deref().map(|password| password.to_owned()),
        secret: spec.mtproto_secret.as_ref().map(|secret| **secret),
        fake_tls_domain: spec.fake_tls_domain.clone(),
    };
    config.validate().map_err(|_| ClientError::InvalidRequest)
}

fn config_from_spec(spec: ProxySpec) -> Result<ProxyConfig, ClientError> {
    validate_spec(&spec)?;
    Ok(ProxyConfig {
        kind: spec.kind,
        host: spec.host,
        port: spec.port,
        username: spec.username,
        password: spec.password.as_deref().map(|password| password.to_owned()),
        secret: spec.mtproto_secret.map(|secret| *secret),
        fake_tls_domain: spec.fake_tls_domain,
    })
}

fn has_secret(config: &ProxyConfig) -> bool {
    match config.kind {
        ProxyKind::Mtproto => config.secret.is_some(),
        ProxyKind::Socks5 | ProxyKind::Http | ProxyKind::Https => config.password.is_some(),
    }
}

fn secret_value(config: &ProxyConfig) -> Result<String, ClientError> {
    match config.kind {
        ProxyKind::Mtproto => Ok(encode_mtproto_secret(
            &config.secret.ok_or(ClientError::InvalidRequest)?,
        )),
        ProxyKind::Socks5 | ProxyKind::Http | ProxyKind::Https => {
            config.password.clone().ok_or(ClientError::InvalidRequest)
        }
    }
}

fn secret_key(id: u64) -> String {
    format!("proxy-secret-v1:{id}")
}

fn kind_name(kind: ProxyKind) -> &'static str {
    match kind {
        ProxyKind::Socks5 => "socks5",
        ProxyKind::Http => "http",
        ProxyKind::Https => "https",
        ProxyKind::Mtproto => "mtproto",
    }
}

fn kind_from_name(name: &str) -> Option<ProxyKind> {
    match name {
        "socks5" => Some(ProxyKind::Socks5),
        "http" => Some(ProxyKind::Http),
        "https" => Some(ProxyKind::Https),
        "mtproto" => Some(ProxyKind::Mtproto),
        _ => None,
    }
}

fn transport_name(mode: TransportMode) -> &'static str {
    match mode {
        TransportMode::PaddedIntermediate => "padded-intermediate",
        TransportMode::Http => "http",
    }
}

fn transport_from_name(name: &str) -> Option<TransportMode> {
    match name {
        "padded-intermediate" => Some(TransportMode::PaddedIntermediate),
        "http" => Some(TransportMode::Http),
        _ => None,
    }
}

fn encode_mtproto_secret(secret: &[u8; 16]) -> String {
    secret.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_mtproto_secret_hex(encoded: &str) -> Result<[u8; 16], ClientError> {
    let bytes = hex_decode(encoded)?;
    if bytes.len() != 16 {
        return Err(ClientError::InvalidRequest);
    }
    let mut secret = [0_u8; 16];
    secret.copy_from_slice(&bytes);
    Ok(secret)
}

fn hex_decode(value: &str) -> Result<Vec<u8>, ClientError> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ClientError::InvalidRequest);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        bytes.push(
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| ClientError::InvalidRequest)?,
        );
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push(high << 4 | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.insert(percent_decode(key), percent_decode(value));
    }
    params
}

fn parse_proxy_link(link: &str) -> Result<ProxySpec, ClientError> {
    let link = link.trim();
    let (kind, query) = if let Some(rest) = link.strip_prefix("tg://proxy?") {
        (ProxyKind::Mtproto, rest)
    } else if let Some(rest) = link.strip_prefix("tg://socks?") {
        (ProxyKind::Socks5, rest)
    } else if let Some(rest) = link.strip_prefix("https://t.me/proxy?") {
        (ProxyKind::Mtproto, rest)
    } else if let Some(rest) = link.strip_prefix("https://t.me/socks?") {
        (ProxyKind::Socks5, rest)
    } else {
        return Err(ClientError::InvalidRequest);
    };
    let params = parse_query(query);
    let host = params
        .get("server")
        .filter(|host| !host.is_empty())
        .cloned()
        .ok_or(ClientError::InvalidRequest)?;
    let port = params
        .get("port")
        .ok_or(ClientError::InvalidRequest)?
        .parse::<u16>()
        .map_err(|_| ClientError::InvalidRequest)?;
    let spec = match kind {
        ProxyKind::Mtproto => {
            let secret = params.get("secret").ok_or(ClientError::InvalidRequest)?;
            let bytes = hex_decode(secret)?;
            let (key, domain) = ProxyConfig::decode_mtproto_secret(&bytes)
                .map_err(|_| ClientError::InvalidRequest)?
                .ok_or(ClientError::InvalidRequest)?;
            ProxySpec {
                kind: ProxyKind::Mtproto,
                host,
                port,
                username: None,
                password: None,
                mtproto_secret: Some(Zeroizing::new(key)),
                fake_tls_domain: domain,
            }
        }
        _ => ProxySpec {
            kind: ProxyKind::Socks5,
            host,
            port,
            username: params.get("user").cloned(),
            password: params
                .get("pass")
                .map(|password| Zeroizing::new(password.clone())),
            mtproto_secret: None,
            fake_tls_domain: None,
        },
    };
    validate_spec(&spec)?;
    Ok(spec)
}

fn fresh_id(proxies: &[StoredProxy]) -> u64 {
    use rand::{TryRng, rngs::SysRng};
    for _ in 0..8 {
        let Ok(id) = SysRng.try_next_u64() else {
            break;
        };
        if id != 0 && !proxies.iter().any(|proxy| proxy.id == id) {
            return id;
        }
    }
    let mut id = 1_u64;
    while proxies.iter().any(|proxy| proxy.id == id) {
        id = id.wrapping_add(1);
    }
    id
}

fn run_probes<F>(targets: &[(u64, ProxyConfig)], probe: &F) -> Vec<ProbeResult>
where
    F: Fn(ProxyConfig) -> Result<i64, String> + Sync,
{
    let mut results = Vec::with_capacity(targets.len());
    for chunk in targets.chunks(3) {
        let batch = std::thread::scope(|scope| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(id, config)| {
                    let id = *id;
                    let config = config.clone();
                    scope.spawn(move || match probe(config) {
                        Ok(latency_ms) => ProbeResult {
                            id,
                            latency_ms: Some(latency_ms),
                            error: None,
                        },
                        Err(error) => ProbeResult {
                            id,
                            latency_ms: None,
                            error: Some(error),
                        },
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("proxy probe panicked"))
                .collect::<Vec<_>>()
        });
        results.extend(batch);
    }
    results
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProxy {
    id: u64,
    kind: String,
    host: String,
    port: u16,
    username: Option<String>,
    fake_tls_domain: Option<String>,
    has_secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredConfig {
    #[serde(default)]
    active: Option<u64>,
    #[serde(default = "default_transport_mode")]
    transport_mode: String,
    #[serde(default)]
    proxies: Vec<StoredProxy>,
}

fn default_transport_mode() -> String {
    "padded-intermediate".into()
}

impl Default for StoredConfig {
    fn default() -> Self {
        Self {
            active: None,
            transport_mode: default_transport_mode(),
            proxies: Vec::new(),
        }
    }
}

trait SecretBackend {
    fn read(&self, key: &str) -> Result<Option<String>, ClientError>;
    fn write(&self, key: &str, value: &str) -> Result<(), ClientError>;
    fn delete(&self, key: &str) -> Result<(), ClientError>;
}

trait ConfigBackend {
    fn load(&self) -> Result<StoredConfig, ClientError>;
    fn save(&self, state: &StoredConfig) -> Result<(), ClientError>;
}

struct KeyringSecretBackend;

impl SecretBackend for KeyringSecretBackend {
    fn read(&self, key: &str) -> Result<Option<String>, ClientError> {
        let entry = keyring::Entry::new(SECRET_SERVICE, key).map_err(|_| ClientError::Storage)?;
        match entry.get_secret() {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| ClientError::Storage),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(ClientError::Storage),
        }
    }

    fn write(&self, key: &str, value: &str) -> Result<(), ClientError> {
        keyring::Entry::new(SECRET_SERVICE, key)
            .and_then(|entry| entry.set_password(value))
            .map_err(|_| ClientError::Storage)
    }

    fn delete(&self, key: &str) -> Result<(), ClientError> {
        let entry = keyring::Entry::new(SECRET_SERVICE, key).map_err(|_| ClientError::Storage)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(ClientError::Storage),
        }
    }
}

struct XdgConfigBackend {
    path: PathBuf,
}

impl XdgConfigBackend {
    fn open() -> Result<Self, ClientError> {
        let path = xdg::BaseDirectories::with_prefix("monogram")
            .get_config_file(CONFIG_FILE)
            .ok_or(ClientError::Storage)?;
        Ok(Self { path })
    }
}

impl ConfigBackend for XdgConfigBackend {
    fn load(&self) -> Result<StoredConfig, ClientError> {
        if !self.path.exists() {
            return Ok(StoredConfig::default());
        }
        let bytes = std::fs::read(&self.path).map_err(|_| ClientError::Storage)?;
        serde_json::from_slice(&bytes).map_err(|_| ClientError::Storage)
    }

    fn save(&self, state: &StoredConfig) -> Result<(), ClientError> {
        if let Some(parent) = self.path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|_| ClientError::Storage)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state).map_err(|_| ClientError::Storage)?;
        std::fs::write(&temporary, &bytes).map_err(|_| ClientError::Storage)?;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| ClientError::Storage)?;
        std::fs::rename(&temporary, &self.path).map_err(|_| ClientError::Storage)?;
        Ok(())
    }
}

struct ProxyStore {
    secrets: Box<dyn SecretBackend>,
    config: Box<dyn ConfigBackend>,
}

impl ProxyStore {
    fn open() -> Result<Self, ClientError> {
        Ok(Self {
            secrets: Box::new(KeyringSecretBackend),
            config: Box::new(XdgConfigBackend::open()?),
        })
    }

    #[cfg(test)]
    fn with_backends(secrets: Box<dyn SecretBackend>, config: Box<dyn ConfigBackend>) -> Self {
        Self { secrets, config }
    }

    fn list(&self) -> Result<Vec<ProxyDto>, ClientError> {
        let state = self.config.load()?;
        state
            .proxies
            .iter()
            .map(|proxy| self.dto(&state, proxy.id))
            .collect()
    }

    fn active(&self) -> Result<Option<ProxyDto>, ClientError> {
        let state = self.config.load()?;
        match state.active {
            Some(id) => self.dto(&state, id).map(Some),
            None => Ok(None),
        }
    }

    fn active_config(&self) -> Result<Option<ProxyConfig>, ClientError> {
        let state = self.config.load()?;
        match state.active {
            Some(id) => self.config_from_state(&state, id).map(Some),
            None => Ok(None),
        }
    }

    fn config_for(&self, id: u64) -> Result<ProxyConfig, ClientError> {
        let state = self.config.load()?;
        self.config_from_state(&state, id)
    }

    fn config_from_state(&self, state: &StoredConfig, id: u64) -> Result<ProxyConfig, ClientError> {
        let stored = state
            .proxies
            .iter()
            .find(|proxy| proxy.id == id)
            .ok_or(ClientError::InvalidRequest)?;
        self.build_config(stored)
    }

    fn build_config(&self, stored: &StoredProxy) -> Result<ProxyConfig, ClientError> {
        let kind = kind_from_name(&stored.kind).ok_or(ClientError::Storage)?;
        let (username, password, secret) = match kind {
            ProxyKind::Mtproto => {
                let secret = if stored.has_secret {
                    let encoded = self
                        .secrets
                        .read(&secret_key(stored.id))?
                        .ok_or(ClientError::Storage)?;
                    Some(decode_mtproto_secret_hex(&encoded)?)
                } else {
                    None
                };
                (None, None, secret)
            }
            ProxyKind::Socks5 | ProxyKind::Http | ProxyKind::Https => {
                let password = if stored.has_secret {
                    Some(
                        self.secrets
                            .read(&secret_key(stored.id))?
                            .ok_or(ClientError::Storage)?,
                    )
                } else {
                    None
                };
                (stored.username.clone(), password, None)
            }
        };
        Ok(ProxyConfig {
            kind,
            host: stored.host.clone(),
            port: stored.port,
            username,
            password,
            secret,
            fake_tls_domain: stored.fake_tls_domain.clone(),
        })
    }

    fn dto(&self, state: &StoredConfig, id: u64) -> Result<ProxyDto, ClientError> {
        let stored = state
            .proxies
            .iter()
            .find(|proxy| proxy.id == id)
            .ok_or(ClientError::InvalidRequest)?;
        Ok(ProxyDto {
            id: stored.id,
            kind: kind_from_name(&stored.kind).ok_or(ClientError::Storage)?,
            host: stored.host.clone(),
            port: stored.port,
            username: stored.username.clone(),
            fake_tls_domain: stored.fake_tls_domain.clone(),
            has_secret: stored.has_secret,
            active: state.active == Some(id),
        })
    }

    fn add(&self, spec: ProxySpec) -> Result<ProxyDto, ClientError> {
        let config = config_from_spec(spec)?;
        let mut state = self.config.load()?;
        let id = fresh_id(&state.proxies);
        let has_secret = has_secret(&config);
        let secret = if has_secret {
            Some(secret_value(&config)?)
        } else {
            None
        };
        let kind = kind_name(config.kind).to_owned();
        let host = config.host.clone();
        let username = config.username.clone();
        let fake_tls_domain = config.fake_tls_domain.clone();
        if let Some(secret) = secret {
            self.secrets.write(&secret_key(id), &secret)?;
        }
        state.proxies.push(StoredProxy {
            id,
            kind,
            host,
            port: config.port,
            username,
            fake_tls_domain,
            has_secret,
        });
        self.config.save(&state)?;
        self.dto(&state, id)
    }

    fn edit(&self, id: u64, spec: ProxySpec) -> Result<ProxyDto, ClientError> {
        let config = config_from_spec(spec)?;
        let mut state = self.config.load()?;
        let index = state
            .proxies
            .iter()
            .position(|proxy| proxy.id == id)
            .ok_or(ClientError::InvalidRequest)?;
        let has_secret = has_secret(&config);
        let secret = if has_secret {
            Some(secret_value(&config)?)
        } else {
            None
        };
        let kind = kind_name(config.kind).to_owned();
        let host = config.host.clone();
        let username = config.username.clone();
        let fake_tls_domain = config.fake_tls_domain.clone();
        if let Some(secret) = secret {
            self.secrets.write(&secret_key(id), &secret)?;
        } else {
            self.secrets.delete(&secret_key(id))?;
        }
        state.proxies[index] = StoredProxy {
            id,
            kind,
            host,
            port: config.port,
            username,
            fake_tls_domain,
            has_secret,
        };
        self.config.save(&state)?;
        self.dto(&state, id)
    }

    fn delete(&self, id: u64) -> Result<(), ClientError> {
        let mut state = self.config.load()?;
        let before = state.proxies.len();
        state.proxies.retain(|proxy| proxy.id != id);
        if state.proxies.len() == before {
            return Err(ClientError::InvalidRequest);
        }
        if state.active == Some(id) {
            state.active = None;
        }
        self.config.save(&state)?;
        self.secrets.delete(&secret_key(id))
    }

    fn activate(&self, id: u64) -> Result<ProxyDto, ClientError> {
        let mut state = self.config.load()?;
        if !state.proxies.iter().any(|proxy| proxy.id == id) {
            return Err(ClientError::InvalidRequest);
        }
        state.active = Some(id);
        self.config.save(&state)?;
        self.dto(&state, id)
    }

    fn set_direct(&self) -> Result<(), ClientError> {
        let mut state = self.config.load()?;
        state.active = None;
        self.config.save(&state)
    }

    fn transport(&self) -> Result<TransportMode, ClientError> {
        let state = self.config.load()?;
        transport_from_name(&state.transport_mode).ok_or(ClientError::Storage)
    }

    fn set_transport(&self, mode: TransportMode) -> Result<(), ClientError> {
        let mut state = self.config.load()?;
        state.transport_mode = transport_name(mode).to_owned();
        self.config.save(&state)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/connectivity.rs"]
mod tests;
