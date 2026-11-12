use std::{
    cell::Cell,
    fmt, fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use qrcode::{Color, QrCode};
use relm4::{adw, adw::prelude::*, gtk, gtk::glib, prelude::*};
use serde::{Deserialize, Serialize};

use crate::i18n::I18n;

const PROXY_KINDS: [ProxyKind; 4] = [
    ProxyKind::Socks5,
    ProxyKind::Http,
    ProxyKind::Https,
    ProxyKind::Mtproto,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    Socks5,
    Http,
    Https,
    Mtproto,
}

impl ProxyKind {
    fn type_key(self) -> &'static str {
        match self {
            Self::Socks5 => "feature-settings-settings_proxy_type_socks5",
            Self::Http => "feature-settings-settings_proxy_type_http",
            Self::Https => "feature-settings-settings_proxy_type_https",
            Self::Mtproto => "feature-settings-settings_proxy_type_mtproto",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Self::Socks5 => 1080,
            Self::Http => 8080,
            Self::Https => 443,
            Self::Mtproto => 443,
        }
    }

    fn uses_auth(self) -> bool {
        !matches!(self, Self::Mtproto)
    }

    fn uses_secret(self) -> bool {
        matches!(self, Self::Mtproto)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRecord {
    pub id: i64,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub has_password: bool,
    pub has_secret: bool,
    pub active: bool,
    pub checked: bool,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyProbe {
    pub id: i64,
    pub ok: bool,
    pub latency_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySubmission {
    pub id: i64,
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyDraft {
    id: i64,
    kind: ProxyKind,
    host: String,
    port: u16,
    port_text: String,
    username: String,
    password: String,
    secret: String,
    has_password: bool,
    has_secret: bool,
}

impl ProxyDraft {
    fn new() -> Self {
        Self {
            id: 0,
            kind: ProxyKind::Socks5,
            host: String::new(),
            port: 0,
            port_text: String::new(),
            username: String::new(),
            password: String::new(),
            secret: String::new(),
            has_password: false,
            has_secret: false,
        }
    }

    fn from_record(record: &ProxyRecord) -> Self {
        Self {
            id: record.id,
            kind: record.kind,
            host: record.host.clone(),
            port: record.port,
            port_text: record.port.to_string(),
            username: record.username.clone(),
            password: String::new(),
            secret: String::new(),
            has_password: record.has_password,
            has_secret: record.has_secret,
        }
    }

    fn set_port(&mut self, text: String) {
        self.port = text
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .unwrap_or(0);
        self.port_text = text;
    }

    fn set_kind(&mut self, index: usize) {
        let Some(kind) = PROXY_KINDS.get(index).copied() else {
            return;
        };
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        if self.port == 0 {
            let port = kind.default_port();
            self.port = port;
            self.port_text = port.to_string();
        }
    }

    fn valid(&self) -> bool {
        let host = !self.host.trim().is_empty();
        let port = (1..=65535).contains(&self.port);
        let auth = !self.kind.uses_auth()
            || (self.username.trim().is_empty() && self.password.is_empty())
            || (!self.username.trim().is_empty() && !self.password.is_empty());
        let secret =
            !self.kind.uses_secret() || self.has_secret || valid_mtproto_secret(&self.secret);
        host && port && auth && secret
    }

    fn submission(self) -> ProxySubmission {
        ProxySubmission {
            id: self.id,
            kind: self.kind,
            host: self.host.trim().to_string(),
            port: self.port,
            username: self.username.trim().to_string(),
            password: (!self.password.is_empty()).then_some(self.password),
            secret: (!self.secret.is_empty()).then_some(self.secret),
        }
    }
}

fn valid_mtproto_secret(secret: &str) -> bool {
    !secret.is_empty()
        && secret.len().is_multiple_of(2)
        && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_proxy_link(link: &str) -> Option<ProxyDraft> {
    let link = link.trim();
    let mut draft = ProxyDraft::new();
    if let Some(query) = link.strip_prefix("tg://proxy?") {
        draft.kind = ProxyKind::Mtproto;
        for part in query.split('&') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key {
                "server" => draft.host = value.to_string(),
                "port" => draft.set_port(value.to_string()),
                "secret" => draft.secret = value.to_string(),
                _ => {}
            }
        }
        return Some(draft);
    }
    if let Some(query) = link.strip_prefix("tg://socks?") {
        draft.kind = ProxyKind::Socks5;
        for part in query.split('&') {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key {
                "server" => draft.host = value.to_string(),
                "port" => draft.set_port(value.to_string()),
                "user" => draft.username = value.to_string(),
                "pass" => draft.password = value.to_string(),
                _ => {}
            }
        }
        return Some(draft);
    }
    let (kind, rest) = link
        .strip_prefix("socks5://")
        .map(|rest| (ProxyKind::Socks5, rest))
        .or_else(|| {
            link.strip_prefix("http://")
                .map(|rest| (ProxyKind::Http, rest))
        })
        .or_else(|| {
            link.strip_prefix("https://")
                .map(|rest| (ProxyKind::Https, rest))
        })?;
    draft.kind = kind;
    let rest = rest.split('/').next().unwrap_or_default();
    let (userinfo, hostport) = match rest.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, rest),
    };
    if let Some(userinfo) = userinfo {
        match userinfo.split_once(':') {
            Some((username, password)) => {
                draft.username = username.to_string();
                draft.password = password.to_string();
            }
            None => draft.username = userinfo.to_string(),
        }
    }
    match hostport.rsplit_once(':') {
        Some((host, port)) => {
            draft.host = host.to_string();
            draft.set_port(port.to_string());
        }
        None => draft.host = hostport.to_string(),
    }
    Some(draft)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRecord {
    pub hash: String,
    pub name: String,
    pub is_current: bool,
    pub last_active: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrLink {
    Idle,
    Pending { token: Vec<u8>, expires: i64 },
    PasswordPending,
    Expired,
    Invalid,
    Linked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationSettings {
    pub private: bool,
    pub groups: bool,
    pub channels: bool,
    pub mentions: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            private: true,
            groups: true,
            channels: true,
            mentions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationException {
    pub chat_id: i64,
    pub title: Option<String>,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNotificationPolicy {
    pub enabled: bool,
    pub preview: bool,
}

impl Default for LocalNotificationPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            preview: true,
        }
    }
}

impl LocalNotificationPolicy {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)
    }
}

pub fn local_notification_policy_path() -> Option<PathBuf> {
    xdg::BaseDirectories::with_prefix("monogram").get_config_file("notifications.json")
}

pub enum ConnectivityInput {
    SetI18n(Rc<I18n>),
    SetBusy(bool),
    SetError(Option<String>),
    SetProxies(Vec<ProxyRecord>),
    SetProxyProbe(ProxyProbe),
    SetDevices(Vec<DeviceRecord>),
    SetQr(QrLink),
    SetNotificationSettings(NotificationSettings),
    SetNotificationExceptions(Vec<NotificationException>),
    SetLocalPolicyPath(PathBuf),
    Back,
    Refresh,
    ProxyNew,
    ProxyEdit(i64),
    ProxyImport(String),
    ProxyCancel,
    ProxyHost(String),
    ProxyPort(String),
    ProxyKind(usize),
    ProxyUsername(String),
    ProxyPassword(String),
    ProxySecret(String),
    ProxySave,
    ProxyDeleteRequest(i64),
    ProxyDeleteCurrent,
    ProxyDeleteConfirm,
    ProxyDeleteCancel,
    ProxyActivate(i64),
    ProxyDirect,
    ProxyProbe(i64),
    ProxyProbeAll,
    NotificationEnabled(bool),
    NotificationPreview(bool),
    NotificationPrivate(bool),
    NotificationGroups(bool),
    NotificationChannels(bool),
    NotificationMentions(bool),
    NotificationMute { chat_id: i64 },
    NotificationUnmute { chat_id: i64 },
    NotificationReset,
    DeviceRevokeRequest(String),
    DeviceRevokeConfirm,
    DeviceRevokeCancel,
    DeviceQrRequest,
    DeviceQrRefresh,
}

impl fmt::Debug for ConnectivityInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SetI18n(_) => "SetI18n",
            Self::SetBusy(_) => "SetBusy",
            Self::SetError(_) => "SetError",
            Self::SetProxies(_) => "SetProxies",
            Self::SetProxyProbe(_) => "SetProxyProbe",
            Self::SetDevices(_) => "SetDevices",
            Self::SetQr(_) => "SetQr",
            Self::SetNotificationSettings(_) => "SetNotificationSettings",
            Self::SetNotificationExceptions(_) => "SetNotificationExceptions",
            Self::SetLocalPolicyPath(_) => "SetLocalPolicyPath",
            Self::Back => "Back",
            Self::Refresh => "Refresh",
            Self::ProxyNew => "ProxyNew",
            Self::ProxyEdit(_) => "ProxyEdit",
            Self::ProxyImport(_) => "ProxyImport",
            Self::ProxyCancel => "ProxyCancel",
            Self::ProxyHost(_) => "ProxyHost",
            Self::ProxyPort(_) => "ProxyPort",
            Self::ProxyKind(_) => "ProxyKind",
            Self::ProxyUsername(_) => "ProxyUsername",
            Self::ProxyPassword(_) => "ProxyPassword",
            Self::ProxySecret(_) => "ProxySecret",
            Self::ProxySave => "ProxySave",
            Self::ProxyDeleteRequest(_) => "ProxyDeleteRequest",
            Self::ProxyDeleteCurrent => "ProxyDeleteCurrent",
            Self::ProxyDeleteConfirm => "ProxyDeleteConfirm",
            Self::ProxyDeleteCancel => "ProxyDeleteCancel",
            Self::ProxyActivate(_) => "ProxyActivate",
            Self::ProxyDirect => "ProxyDirect",
            Self::ProxyProbe(_) => "ProxyProbe",
            Self::ProxyProbeAll => "ProxyProbeAll",
            Self::NotificationEnabled(_) => "NotificationEnabled",
            Self::NotificationPreview(_) => "NotificationPreview",
            Self::NotificationPrivate(_) => "NotificationPrivate",
            Self::NotificationGroups(_) => "NotificationGroups",
            Self::NotificationChannels(_) => "NotificationChannels",
            Self::NotificationMentions(_) => "NotificationMentions",
            Self::NotificationMute { .. } => "NotificationMute",
            Self::NotificationUnmute { .. } => "NotificationUnmute",
            Self::NotificationReset => "NotificationReset",
            Self::DeviceRevokeRequest(_) => "DeviceRevokeRequest",
            Self::DeviceRevokeConfirm => "DeviceRevokeConfirm",
            Self::DeviceRevokeCancel => "DeviceRevokeCancel",
            Self::DeviceQrRequest => "DeviceQrRequest",
            Self::DeviceQrRefresh => "DeviceQrRefresh",
        })
    }
}

pub enum ConnectivityOutput {
    Back,
    Refresh,
    ProxySave(ProxySubmission),
    ProxyDelete(i64),
    ProxyActivate(i64),
    ProxyDirect,
    ProxyProbe(i64),
    ProxyProbeAll,
    NotificationSave(NotificationSettings),
    NotificationMute { chat_id: i64 },
    NotificationUnmute { chat_id: i64 },
    NotificationReset,
    LocalPolicyChanged(LocalNotificationPolicy),
    DeviceRevoke(String),
    DeviceQr,
    DeviceQrPoll,
}

impl fmt::Debug for ConnectivityOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Back => "Back",
            Self::Refresh => "Refresh",
            Self::ProxySave(_) => "ProxySave(..)",
            Self::ProxyDelete(_) => "ProxyDelete(..)",
            Self::ProxyActivate(_) => "ProxyActivate(..)",
            Self::ProxyDirect => "ProxyDirect",
            Self::ProxyProbe(_) => "ProxyProbe(..)",
            Self::ProxyProbeAll => "ProxyProbeAll",
            Self::NotificationSave(_) => "NotificationSave(..)",
            Self::NotificationMute { .. } => "NotificationMute(..)",
            Self::NotificationUnmute { .. } => "NotificationUnmute(..)",
            Self::NotificationReset => "NotificationReset",
            Self::LocalPolicyChanged(_) => "LocalPolicyChanged(..)",
            Self::DeviceRevoke(_) => "DeviceRevoke(..)",
            Self::DeviceQr => "DeviceQr",
            Self::DeviceQrPoll => "DeviceQrPoll",
        })
    }
}

pub struct ConnectivityPage {
    i18n: Rc<I18n>,
    busy: bool,
    error: Option<String>,
    proxies: Vec<ProxyRecord>,
    draft: Option<ProxyDraft>,
    confirming_delete: Option<i64>,
    devices: Vec<DeviceRecord>,
    qr: QrLink,
    confirming_revoke: Option<String>,
    notification_settings: NotificationSettings,
    exceptions: Vec<NotificationException>,
    local_policy: LocalNotificationPolicy,
    local_policy_path: Option<PathBuf>,
    rendering: Rc<Cell<bool>>,
    revision: u64,
}

impl ConnectivityPage {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            busy: false,
            error: None,
            proxies: Vec::new(),
            draft: None,
            confirming_delete: None,
            devices: Vec::new(),
            qr: QrLink::Idle,
            confirming_revoke: None,
            notification_settings: NotificationSettings::default(),
            exceptions: Vec::new(),
            local_policy: LocalNotificationPolicy::default(),
            local_policy_path: None,
            rendering: Rc::new(Cell::new(false)),
            revision: 1,
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_default()
    }

    fn text_arg(&self, key: &str, value: &str) -> String {
        self.i18n.text(key, &[("arg1", value)]).unwrap_or_default()
    }

    fn persist_local_policy(&mut self) {
        if let Some(path) = &self.local_policy_path
            && self.local_policy.save(path).is_err()
        {
            self.error = Some(self.text("desktop-error-generic"));
        }
    }

    fn notification_command(&mut self, settings: NotificationSettings) -> Vec<ConnectivityOutput> {
        if self.busy || settings == self.notification_settings {
            return Vec::new();
        }
        self.busy = true;
        self.error = None;
        vec![ConnectivityOutput::NotificationSave(settings)]
    }

    fn handle(&mut self, input: ConnectivityInput) -> Vec<ConnectivityOutput> {
        match input {
            ConnectivityInput::SetI18n(i18n) => {
                self.i18n = i18n;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetBusy(busy) => self.busy = busy,
            ConnectivityInput::SetError(error) => {
                self.error = error;
                self.busy = false;
            }
            ConnectivityInput::SetProxies(proxies) => {
                self.proxies = proxies;
                self.busy = false;
                self.error = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetProxyProbe(probe) => {
                if let Some(record) = self.proxies.iter_mut().find(|record| record.id == probe.id) {
                    record.checked = true;
                    record.latency_ms = if probe.ok { probe.latency_ms } else { None };
                }
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetDevices(devices) => {
                self.devices = devices;
                self.busy = false;
                self.error = None;
                self.confirming_revoke = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetQr(qr) => {
                self.qr = qr;
                self.busy = false;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetNotificationSettings(settings) => {
                self.notification_settings = settings;
                self.busy = false;
                self.error = None;
            }
            ConnectivityInput::SetNotificationExceptions(exceptions) => {
                self.exceptions = exceptions;
                self.busy = false;
                self.error = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::SetLocalPolicyPath(path) => {
                self.local_policy = LocalNotificationPolicy::load(&path);
                self.local_policy_path = Some(path);
            }
            ConnectivityInput::Back => return vec![ConnectivityOutput::Back],
            ConnectivityInput::Refresh if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::Refresh];
            }
            ConnectivityInput::ProxyNew if !self.busy => {
                self.draft = Some(ProxyDraft::new());
                self.confirming_delete = None;
                self.error = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyEdit(id) if !self.busy => {
                if let Some(record) = self.proxies.iter().find(|record| record.id == id) {
                    self.draft = Some(ProxyDraft::from_record(record));
                    self.confirming_delete = None;
                    self.error = None;
                    self.revision = self.revision.saturating_add(1);
                }
            }
            ConnectivityInput::ProxyImport(link) if !self.busy => {
                match parse_proxy_link(&link) {
                    Some(draft) => {
                        self.draft = Some(draft);
                        self.confirming_delete = None;
                        self.error = None;
                    }
                    None => {
                        self.error =
                            Some(self.text("feature-settings-settings_proxy_clipboard_invalid"));
                    }
                }
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyCancel => {
                self.draft = None;
                self.confirming_delete = None;
                self.error = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyHost(host) => {
                if let Some(draft) = &mut self.draft {
                    draft.host = host;
                }
            }
            ConnectivityInput::ProxyPort(port) => {
                if let Some(draft) = &mut self.draft {
                    draft.set_port(port);
                }
            }
            ConnectivityInput::ProxyKind(index) => {
                if let Some(draft) = &mut self.draft {
                    draft.set_kind(index);
                }
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyUsername(username) => {
                if let Some(draft) = &mut self.draft {
                    draft.username = username;
                }
            }
            ConnectivityInput::ProxyPassword(password) => {
                if let Some(draft) = &mut self.draft {
                    draft.password = password;
                }
            }
            ConnectivityInput::ProxySecret(secret) => {
                if let Some(draft) = &mut self.draft {
                    draft.secret = secret;
                }
            }
            ConnectivityInput::ProxySave
                if !self.busy && self.draft.as_ref().is_some_and(ProxyDraft::valid) =>
            {
                let draft = self.draft.take().unwrap();
                self.busy = true;
                self.error = None;
                self.revision = self.revision.saturating_add(1);
                return vec![ConnectivityOutput::ProxySave(draft.submission())];
            }
            ConnectivityInput::ProxyDeleteRequest(id)
                if !self.busy && self.proxies.iter().any(|record| record.id == id) =>
            {
                self.confirming_delete = Some(id);
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyDeleteCurrent if !self.busy => {
                let id =
                    self.draft.as_ref().map(|draft| draft.id).filter(|id| {
                        *id != 0 && self.proxies.iter().any(|record| record.id == *id)
                    });
                if let Some(id) = id {
                    self.confirming_delete = Some(id);
                    self.revision = self.revision.saturating_add(1);
                }
            }
            ConnectivityInput::ProxyDeleteCancel => {
                self.confirming_delete = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::ProxyDeleteConfirm if !self.busy => {
                if let Some(id) = self.confirming_delete.take() {
                    self.busy = true;
                    self.error = None;
                    self.revision = self.revision.saturating_add(1);
                    return vec![ConnectivityOutput::ProxyDelete(id)];
                }
            }
            ConnectivityInput::ProxyActivate(id)
                if !self.busy && self.proxies.iter().any(|record| record.id == id) =>
            {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::ProxyActivate(id)];
            }
            ConnectivityInput::ProxyDirect if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::ProxyDirect];
            }
            ConnectivityInput::ProxyProbe(id)
                if self.proxies.iter().any(|record| record.id == id) =>
            {
                return vec![ConnectivityOutput::ProxyProbe(id)];
            }
            ConnectivityInput::ProxyProbeAll if !self.proxies.is_empty() => {
                return vec![ConnectivityOutput::ProxyProbeAll];
            }
            ConnectivityInput::NotificationEnabled(enabled) => {
                if self.local_policy.enabled == enabled {
                    return Vec::new();
                }
                self.local_policy.enabled = enabled;
                self.persist_local_policy();
                return vec![ConnectivityOutput::LocalPolicyChanged(self.local_policy)];
            }
            ConnectivityInput::NotificationPreview(preview) => {
                if self.local_policy.preview == preview {
                    return Vec::new();
                }
                self.local_policy.preview = preview;
                self.persist_local_policy();
                return vec![ConnectivityOutput::LocalPolicyChanged(self.local_policy)];
            }
            ConnectivityInput::NotificationPrivate(value) => {
                return self.notification_command(NotificationSettings {
                    private: value,
                    ..self.notification_settings
                });
            }
            ConnectivityInput::NotificationGroups(value) => {
                return self.notification_command(NotificationSettings {
                    groups: value,
                    ..self.notification_settings
                });
            }
            ConnectivityInput::NotificationChannels(value) => {
                return self.notification_command(NotificationSettings {
                    channels: value,
                    ..self.notification_settings
                });
            }
            ConnectivityInput::NotificationMentions(value) => {
                return self.notification_command(NotificationSettings {
                    mentions: value,
                    ..self.notification_settings
                });
            }
            ConnectivityInput::NotificationMute { chat_id } if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::NotificationMute { chat_id }];
            }
            ConnectivityInput::NotificationUnmute { chat_id } if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::NotificationUnmute { chat_id }];
            }
            ConnectivityInput::NotificationReset if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::NotificationReset];
            }
            ConnectivityInput::DeviceRevokeRequest(hash)
                if !self.busy && self.devices.iter().any(|device| device.hash == hash) =>
            {
                self.confirming_revoke = Some(hash);
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::DeviceRevokeCancel => {
                self.confirming_revoke = None;
                self.revision = self.revision.saturating_add(1);
            }
            ConnectivityInput::DeviceRevokeConfirm if !self.busy => {
                if let Some(hash) = self.confirming_revoke.take() {
                    self.busy = true;
                    self.error = None;
                    self.revision = self.revision.saturating_add(1);
                    return vec![ConnectivityOutput::DeviceRevoke(hash)];
                }
            }
            ConnectivityInput::DeviceQrRequest if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::DeviceQr];
            }
            ConnectivityInput::DeviceQrRefresh if !self.busy => {
                self.busy = true;
                self.error = None;
                return vec![ConnectivityOutput::DeviceQrPoll];
            }
            _ => {}
        }
        Vec::new()
    }

    fn proxy_title(&self, record: &ProxyRecord) -> String {
        format!(
            "{} · {}:{}",
            self.text(record.kind.type_key()),
            record.host,
            record.port
        )
    }

    fn proxy_subtitle(&self, record: &ProxyRecord) -> String {
        if record.active {
            self.text("feature-settings-settings_proxy_active")
        } else if record.checked {
            match record.latency_ms {
                Some(latency) => self.text_arg(
                    "feature-settings-settings_proxy_ping_result",
                    &latency.to_string(),
                ),
                None => self.text("feature-settings-settings_proxy_test_failed"),
            }
        } else {
            self.text("feature-settings-settings_proxy_not_checked")
        }
    }

    fn exception_title(&self, exception: &NotificationException) -> String {
        exception
            .title
            .clone()
            .unwrap_or_else(|| exception.chat_id.to_string())
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn qr_pixels(token: &[u8]) -> Option<(Vec<u8>, usize)> {
    if token.is_empty() {
        return None;
    }
    let uri = format!("tg://login?token={}", URL_SAFE_NO_PAD.encode(token));
    let code = QrCode::new(uri.as_bytes()).ok()?;
    let width = code.width() + 8;
    let scale = 6;
    let size = width * scale;
    let mut pixels = vec![255; size * size * 3];
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let offset = (((y + 4) * scale + dy) * size + (x + 4) * scale + dx) * 3;
                        pixels[offset..offset + 3].fill(0);
                    }
                }
            }
        }
    }
    Some((pixels, size))
}

fn qr_texture(token: &[u8]) -> Option<gtk::gdk::Texture> {
    let (pixels, size) = qr_pixels(token)?;
    let bytes = glib::Bytes::from_owned(pixels);
    Some(
        gtk::gdk::MemoryTexture::new(
            size as i32,
            size as i32,
            gtk::gdk::MemoryFormat::R8g8b8,
            &bytes,
            size * 3,
        )
        .upcast(),
    )
}

pub struct ConnectivityWidgets {
    heading: gtk::Label,
    status: gtk::Label,
    spinner: gtk::Spinner,
    import_entry: gtk::Entry,
    import_parse: gtk::Button,
    add_proxy: gtk::Button,
    proxy_list: gtk::Box,
    editor: gtk::Box,
    kind: adw::ComboRow,
    host: gtk::Entry,
    port: gtk::Entry,
    auth_row: gtk::Box,
    username: gtk::Entry,
    password: gtk::Entry,
    secret_row: gtk::Box,
    secret: gtk::Entry,
    save: gtk::Button,
    delete: gtk::Button,
    enabled: adw::SwitchRow,
    preview: adw::SwitchRow,
    private: adw::SwitchRow,
    groups: adw::SwitchRow,
    channels: adw::SwitchRow,
    mentions: adw::SwitchRow,
    exceptions_list: gtk::Box,
    reset: gtk::Button,
    devices_list: gtk::Box,
    link: gtk::Button,
    qr_box: gtk::Box,
    qr_refresh: gtk::Button,
    revision: u64,
}

impl SimpleComponent for ConnectivityPage {
    type Init = Rc<I18n>;
    type Input = ConnectivityInput;
    type Output = ConnectivityOutput;
    type Root = gtk::Box;
    type Widgets = ConnectivityWidgets;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Self::new(i18n);
        let header = adw::HeaderBar::new();
        let heading = gtk::Label::new(Some(&model.text("feature-auth-auth_connection")));
        header.set_title_widget(Some(&heading));
        let back = gtk::Button::with_label(&model.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(ConnectivityInput::Back));
        header.pack_start(&back);
        root.append(&header);

        let status = gtk::Label::new(None);
        status.set_wrap(true);
        root.append(&status);
        let spinner = gtk::Spinner::new();
        root.append(&spinner);

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        root.append(&scroll);
        let page = adw::PreferencesPage::builder().vexpand(true).build();
        scroll.set_child(Some(&page));

        let proxy_group = adw::PreferencesGroup::builder()
            .title(model.text("feature-settings-settings_proxy"))
            .description(model.text("feature-settings-settings_proxy_sub"))
            .build();
        let import_entry = gtk::Entry::new();
        import_entry.set_placeholder_text(Some(
            &model.text("feature-settings-settings_proxy_add_clipboard"),
        ));
        proxy_group.add(&import_entry);
        let import_parse =
            gtk::Button::with_label(&model.text("feature-settings-settings_proxy_add"));
        let entry = import_entry.clone();
        let input = sender.input_sender().clone();
        import_parse.connect_clicked(move |_| {
            input.emit(ConnectivityInput::ProxyImport(entry.text().to_string()));
        });
        proxy_group.add(&import_parse);
        let add_proxy = gtk::Button::with_label(&model.text("feature-settings-settings_proxy_add"));
        let input = sender.input_sender().clone();
        add_proxy.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyNew));
        proxy_group.add(&add_proxy);
        let check_all =
            gtk::Button::with_label(&model.text("feature-settings-settings_proxy_check_all"));
        let input = sender.input_sender().clone();
        check_all.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyProbeAll));
        proxy_group.add(&check_all);
        let direct =
            gtk::Button::with_label(&model.text("feature-settings-settings_proxy_direct_title"));
        let input = sender.input_sender().clone();
        direct.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyDirect));
        proxy_group.add(&direct);
        let proxy_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        proxy_group.add(&proxy_list);

        let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let kind_names = gtk::StringList::new(
            &PROXY_KINDS
                .map(|kind| model.text(kind.type_key()))
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let kind = adw::ComboRow::builder()
            .title(model.text("feature-settings-settings_proxy_type"))
            .model(&kind_names)
            .selected(0)
            .build();
        let input = sender.input_sender().clone();
        kind.connect_selected_notify(move |row| {
            input.emit(ConnectivityInput::ProxyKind(row.selected() as usize));
        });
        editor.append(&kind);
        let host = gtk::Entry::new();
        host.set_placeholder_text(Some(
            &model.text("feature-settings-settings_proxy_host_hint"),
        ));
        let input = sender.input_sender().clone();
        host.connect_changed(move |entry| {
            input.emit(ConnectivityInput::ProxyHost(entry.text().to_string()));
        });
        editor.append(&host);
        let port = gtk::Entry::new();
        port.set_placeholder_text(Some(&model.text("feature-settings-settings_proxy_port")));
        let input = sender.input_sender().clone();
        port.connect_changed(move |entry| {
            input.emit(ConnectivityInput::ProxyPort(entry.text().to_string()));
        });
        editor.append(&port);
        let auth_row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let username = gtk::Entry::new();
        username.set_placeholder_text(Some(
            &model.text("feature-settings-settings_proxy_username"),
        ));
        let input = sender.input_sender().clone();
        username.connect_changed(move |entry| {
            input.emit(ConnectivityInput::ProxyUsername(entry.text().to_string()));
        });
        auth_row.append(&username);
        let password = gtk::Entry::new();
        password.set_placeholder_text(Some(
            &model.text("feature-settings-settings_proxy_password"),
        ));
        password.set_visibility(false);
        let input = sender.input_sender().clone();
        password.connect_changed(move |entry| {
            input.emit(ConnectivityInput::ProxyPassword(entry.text().to_string()));
        });
        auth_row.append(&password);
        editor.append(&auth_row);
        let secret_row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let secret = gtk::Entry::new();
        secret.set_placeholder_text(Some(&model.text("feature-settings-settings_proxy_secret")));
        secret.set_visibility(false);
        let input = sender.input_sender().clone();
        secret.connect_changed(move |entry| {
            input.emit(ConnectivityInput::ProxySecret(entry.text().to_string()));
        });
        secret_row.append(&secret);
        editor.append(&secret_row);
        let save = gtk::Button::with_label(&model.text("feature-settings-settings_proxy_save"));
        let input = sender.input_sender().clone();
        save.connect_clicked(move |_| input.emit(ConnectivityInput::ProxySave));
        editor.append(&save);
        let delete = gtk::Button::with_label(&model.text("feature-settings-settings_proxy_remove"));
        delete.add_css_class("destructive-action");
        let input = sender.input_sender().clone();
        delete.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyDeleteCurrent));
        editor.append(&delete);
        let editor_cancel = gtk::Button::with_label(&model.text("desktop-cancel"));
        let input = sender.input_sender().clone();
        editor_cancel.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyCancel));
        editor.append(&editor_cancel);
        proxy_group.add(&editor);
        page.add(&proxy_group);

        let notifications_group = adw::PreferencesGroup::builder()
            .title(model.text("feature-settings-settings_notifications"))
            .description(model.text("feature-settings-settings_notifications_sub"))
            .build();
        let enabled = adw::SwitchRow::builder()
            .title(model.text("desktop-notifications"))
            .active(model.local_policy.enabled)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        enabled.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationEnabled(row.is_active()));
            }
        });
        notifications_group.add(&enabled);
        let preview = adw::SwitchRow::builder()
            .title(model.text("feature-settings-settings_notifications_preview"))
            .active(model.local_policy.preview)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        preview.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationPreview(row.is_active()));
            }
        });
        notifications_group.add(&preview);
        let private = adw::SwitchRow::builder()
            .title(model.text("feature-settings-settings_notifications_private"))
            .active(model.notification_settings.private)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        private.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationPrivate(row.is_active()));
            }
        });
        notifications_group.add(&private);
        let groups = adw::SwitchRow::builder()
            .title(model.text("feature-settings-settings_notifications_groups"))
            .active(model.notification_settings.groups)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        groups.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationGroups(row.is_active()));
            }
        });
        notifications_group.add(&groups);
        let channels = adw::SwitchRow::builder()
            .title(model.text("feature-settings-settings_notifications_channels"))
            .active(model.notification_settings.channels)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        channels.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationChannels(row.is_active()));
            }
        });
        notifications_group.add(&channels);
        let mentions = adw::SwitchRow::builder()
            .title(model.text("feature-settings-settings_notifications_chat_mentions"))
            .active(model.notification_settings.mentions)
            .build();
        let input = sender.input_sender().clone();
        let rendering = model.rendering.clone();
        mentions.connect_active_notify(move |row| {
            if !rendering.get() {
                input.emit(ConnectivityInput::NotificationMentions(row.is_active()));
            }
        });
        notifications_group.add(&mentions);
        let exceptions_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        notifications_group.add(&exceptions_list);
        let reset =
            gtk::Button::with_label(&model.text("feature-settings-settings_notifications_reset"));
        let input = sender.input_sender().clone();
        reset.connect_clicked(move |_| input.emit(ConnectivityInput::NotificationReset));
        notifications_group.add(&reset);
        page.add(&notifications_group);

        let devices_group = adw::PreferencesGroup::builder()
            .title(model.text("feature-settings-settings_devices"))
            .description(model.text("feature-settings-settings_devices_sub"))
            .build();
        let link = gtk::Button::with_label(&model.text("feature-settings-settings_devices_link"));
        let input = sender.input_sender().clone();
        link.connect_clicked(move |_| input.emit(ConnectivityInput::DeviceQrRequest));
        devices_group.add(&link);
        let qr_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        devices_group.add(&qr_box);
        let qr_refresh =
            gtk::Button::with_label(&model.text("feature-settings-settings_devices_refresh"));
        let input = sender.input_sender().clone();
        qr_refresh.connect_clicked(move |_| input.emit(ConnectivityInput::DeviceQrRefresh));
        devices_group.add(&qr_refresh);
        let devices_list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        devices_group.add(&devices_list);
        page.add(&devices_group);

        let mut widgets = ConnectivityWidgets {
            heading,
            status,
            spinner,
            import_entry,
            import_parse,
            add_proxy,
            proxy_list,
            editor,
            kind,
            host,
            port,
            auth_row,
            username,
            password,
            secret_row,
            secret,
            save,
            delete,
            enabled,
            preview,
            private,
            groups,
            channels,
            mentions,
            exceptions_list,
            reset,
            devices_list,
            link,
            qr_box,
            qr_refresh,
            revision: 0,
        };
        SimpleComponent::update_view(&model, &mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        for output in self.handle(input) {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, widgets: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.rendering.set(true);
        widgets
            .heading
            .set_label(&self.text("feature-auth-auth_connection"));
        widgets.status.set_label(
            &self
                .error
                .clone()
                .or_else(|| self.busy.then(|| self.text("desktop-loading")))
                .unwrap_or_default(),
        );
        widgets
            .status
            .set_visible(self.error.is_some() || self.busy);
        widgets.spinner.set_spinning(self.busy);
        widgets
            .add_proxy
            .set_sensitive(!self.busy && self.draft.is_none());
        widgets
            .import_entry
            .set_sensitive(!self.busy && self.draft.is_none());
        widgets
            .import_parse
            .set_sensitive(!self.busy && self.draft.is_none());
        widgets.editor.set_visible(self.draft.is_some());
        widgets.editor.set_sensitive(!self.busy);
        widgets
            .save
            .set_sensitive(!self.busy && self.draft.as_ref().is_some_and(ProxyDraft::valid));
        let draft_is_existing = self.draft.as_ref().is_some_and(|draft| {
            draft.id != 0 && self.proxies.iter().any(|record| record.id == draft.id)
        });
        widgets
            .delete
            .set_sensitive(!self.busy && draft_is_existing);
        widgets.enabled.set_active(self.local_policy.enabled);
        widgets.preview.set_active(self.local_policy.preview);
        widgets
            .private
            .set_active(self.notification_settings.private);
        widgets.groups.set_active(self.notification_settings.groups);
        widgets
            .channels
            .set_active(self.notification_settings.channels);
        widgets
            .mentions
            .set_active(self.notification_settings.mentions);
        widgets.reset.set_sensitive(
            !self.busy
                && (self.notification_settings != NotificationSettings::default()
                    || !self.exceptions.is_empty()),
        );
        widgets
            .link
            .set_sensitive(!self.busy && matches!(self.qr, QrLink::Idle));
        widgets
            .qr_refresh
            .set_sensitive(!self.busy && matches!(self.qr, QrLink::Pending { .. }));

        if widgets.revision == self.revision {
            self.rendering.set(false);
            return;
        }
        widgets.revision = self.revision;

        clear(&widgets.proxy_list);
        if self.proxies.is_empty() {
            widgets.proxy_list.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_proxy_empty"),
            )));
        }
        for record in &self.proxies {
            let row = adw::ActionRow::builder()
                .title(self.proxy_title(record))
                .subtitle(self.proxy_subtitle(record))
                .build();
            let active = record.active;
            if !active {
                let activate =
                    gtk::Button::with_label(&self.text("feature-settings-settings_proxy_active"));
                activate.set_sensitive(!self.busy);
                let id = record.id;
                let input = sender.input_sender().clone();
                activate.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyActivate(id)));
                row.add_suffix(&activate);
            }
            let edit = gtk::Button::with_label(&self.text("feature-settings-settings_proxy_edit"));
            edit.set_sensitive(!self.busy && self.draft.is_none());
            let id = record.id;
            let input = sender.input_sender().clone();
            edit.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyEdit(id)));
            row.add_suffix(&edit);
            let test = gtk::Button::with_label(&self.text("feature-settings-settings_proxy_test"));
            let id = record.id;
            let input = sender.input_sender().clone();
            test.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyProbe(id)));
            row.add_suffix(&test);
            let remove =
                gtk::Button::with_label(&self.text("feature-settings-settings_proxy_remove"));
            remove.add_css_class("destructive-action");
            remove.set_sensitive(!self.busy && self.draft.is_none());
            let id = record.id;
            let input = sender.input_sender().clone();
            remove.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyDeleteRequest(id)));
            row.add_suffix(&remove);
            widgets.proxy_list.append(&row);
            if self.confirming_delete == Some(record.id) {
                widgets
                    .proxy_list
                    .append(&self.proxy_delete_confirmation(&sender));
            }
        }

        if let Some(draft) = &self.draft {
            let selected = PROXY_KINDS
                .iter()
                .position(|kind| *kind == draft.kind)
                .unwrap_or(0);
            widgets.kind.set_selected(selected as u32);
            if widgets.host.text().as_str() != draft.host {
                widgets.host.set_text(&draft.host);
            }
            if widgets.port.text().as_str() != draft.port_text {
                widgets.port.set_text(&draft.port_text);
            }
            widgets.auth_row.set_visible(draft.kind.uses_auth());
            if draft.kind.uses_auth() {
                if widgets.username.text().as_str() != draft.username {
                    widgets.username.set_text(&draft.username);
                }
                if widgets.password.text().as_str() != draft.password {
                    widgets.password.set_text(&draft.password);
                }
            }
            widgets.secret_row.set_visible(draft.kind.uses_secret());
            if draft.kind.uses_secret() && widgets.secret.text().as_str() != draft.secret {
                widgets.secret.set_text(&draft.secret);
            }
        }

        clear(&widgets.exceptions_list);
        if self.exceptions.is_empty() {
            widgets.exceptions_list.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_notifications_exceptions_empty"),
            )));
        }
        for exception in &self.exceptions {
            let row = adw::SwitchRow::builder()
                .title(self.exception_title(exception))
                .active(exception.muted)
                .sensitive(!self.busy)
                .build();
            let chat_id = exception.chat_id;
            let input = sender.input_sender().clone();
            let rendering = self.rendering.clone();
            row.connect_active_notify(move |row| {
                if rendering.get() {
                    return;
                }
                let message = if row.is_active() {
                    ConnectivityInput::NotificationMute { chat_id }
                } else {
                    ConnectivityInput::NotificationUnmute { chat_id }
                };
                input.emit(message);
            });
            widgets.exceptions_list.append(&row);
        }

        clear(&widgets.devices_list);
        if self.devices.is_empty() {
            widgets.devices_list.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_devices_empty"),
            )));
        }
        for device in &self.devices {
            let title = if device.is_current {
                self.text("feature-settings-settings_devices_current_title")
            } else {
                device.name.clone()
            };
            let row = adw::ActionRow::builder().title(title).build();
            if let Some(last_active) = &device.last_active {
                row.set_subtitle(
                    &self.text_arg("feature-settings-settings_devices_last_active", last_active),
                );
            }
            if !device.is_current {
                let revoke = gtk::Button::with_label(
                    &self.text("feature-settings-settings_devices_terminate"),
                );
                revoke.add_css_class("destructive-action");
                revoke.set_sensitive(!self.busy);
                let hash = device.hash.clone();
                let input = sender.input_sender().clone();
                revoke.connect_clicked(move |_| {
                    input.emit(ConnectivityInput::DeviceRevokeRequest(hash.clone()))
                });
                row.add_suffix(&revoke);
            }
            widgets.devices_list.append(&row);
            if self.confirming_revoke.as_deref() == Some(device.hash.as_str()) {
                widgets
                    .devices_list
                    .append(&self.revoke_confirmation(&sender));
            }
        }

        clear(&widgets.qr_box);
        match &self.qr {
            QrLink::Idle => {}
            QrLink::Pending { token, .. } => {
                if let Some(texture) = qr_texture(token) {
                    let picture = gtk::Picture::new();
                    picture.set_paintable(Some(&texture));
                    picture.set_can_shrink(true);
                    picture.set_halign(gtk::Align::Center);
                    picture.set_size_request(240, 240);
                    widgets.qr_box.append(&picture);
                }
            }
            QrLink::PasswordPending => widgets.qr_box.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_devices_password_pending"),
            ))),
            QrLink::Expired => widgets.qr_box.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_devices_expired"),
            ))),
            QrLink::Invalid => widgets.qr_box.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_devices_invalid"),
            ))),
            QrLink::Linked => widgets.qr_box.append(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_devices_linked"),
            ))),
        }
        self.rendering.set(false);
    }
}

impl ConnectivityPage {
    fn proxy_delete_confirmation(&self, sender: &ComponentSender<Self>) -> gtk::Box {
        let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let body = gtk::Label::new(Some(
            &self.text("feature-settings-settings_proxy_remove_title"),
        ));
        body.set_wrap(true);
        confirmation.append(&body);
        let cancel = gtk::Button::with_label(&self.text("desktop-cancel"));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyDeleteCancel));
        confirmation.append(&cancel);
        let confirm = gtk::Button::with_label(&self.text("feature-settings-settings_proxy_remove"));
        confirm.add_css_class("destructive-action");
        let input = sender.input_sender().clone();
        confirm.connect_clicked(move |_| input.emit(ConnectivityInput::ProxyDeleteConfirm));
        confirmation.append(&confirm);
        confirmation
    }

    fn revoke_confirmation(&self, sender: &ComponentSender<Self>) -> gtk::Box {
        let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let body = gtk::Label::new(Some(
            &self.text("feature-settings-settings_devices_terminate_confirm"),
        ));
        body.set_wrap(true);
        confirmation.append(&body);
        let cancel = gtk::Button::with_label(&self.text("desktop-cancel"));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(ConnectivityInput::DeviceRevokeCancel));
        confirmation.append(&cancel);
        let confirm = gtk::Button::with_label(
            &self.text("feature-settings-settings_devices_terminate_action"),
        );
        confirm.add_css_class("destructive-action");
        let input = sender.input_sender().clone();
        confirm.connect_clicked(move |_| input.emit(ConnectivityInput::DeviceRevokeConfirm));
        confirmation.append(&confirm);
        confirmation
    }
}

#[cfg(test)]
#[path = "../../tests/unit/connectivity_ui.rs"]
mod tests;
