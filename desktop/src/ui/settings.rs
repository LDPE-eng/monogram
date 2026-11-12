use std::{cell::Cell, collections::BTreeMap, fmt, rc::Rc};

use mono_proto_client::adapter::socket::ProxyConfig;
use relm4::{adw, adw::prelude::*, gtk, prelude::*};
use zeroize::Zeroizing;

use super::{
    appearance::{
        Appearance, AppearanceCommand, AppearanceInput, AppearanceOutput, AppearanceState,
        WallpaperConfig, WallpaperFile as UiWallpaperFile, WallpaperMode,
    },
    connectivity::{
        ConnectivityInput, ConnectivityOutput, ConnectivityPage, LocalNotificationPolicy,
        NotificationSettings, ProxyKind as UiProxyKind, ProxySubmission,
        local_notification_policy_path,
    },
    maintenance::{Maintenance, MaintenanceInput, MaintenanceOutput},
};
use crate::{
    client::{
        WorkerCommand,
        appearance::{AppearancePreferences, WallpaperSource},
        connectivity::{ConnectivityCommand, NotifyScope, NotifyUpdate, ProxySpec},
        maintenance::MaintenanceCommand,
    },
    domain::{
        APP_PREFERENCES_VERSION, AppPreferences, AutoDownloadRule, AutoDownloadSettings,
        CacheLimit, ClearMediaScope, MediaCategory, MediaSettings, MediaUsage, NetworkClass,
        NetworkPolicy, PreferenceDocument, PreferenceNamespace,
    },
    i18n::{I18n, SUPPORTED_LOCALES},
};

const NETWORK_CLASSES: [NetworkClass; 3] = [
    NetworkClass::Wifi,
    NetworkClass::Mobile,
    NetworkClass::Roaming,
];
const DOWNLOAD_CATEGORIES: [MediaCategory; 3] = [
    MediaCategory::Photo,
    MediaCategory::Video,
    MediaCategory::Document,
];
const STORAGE_CATEGORIES: [MediaCategory; 5] = [
    MediaCategory::Photo,
    MediaCategory::Video,
    MediaCategory::Document,
    MediaCategory::Avatar,
    MediaCategory::Other,
];
const DOWNLOAD_SIZE_STEPS: [u64; 12] = [
    512_000,
    1024 * 1024,
    2 * 1024 * 1024,
    3 * 1024 * 1024,
    5 * 1024 * 1024,
    10 * 1024 * 1024,
    15 * 1024 * 1024,
    25 * 1024 * 1024,
    50 * 1024 * 1024,
    100 * 1024 * 1024,
    512 * 1024 * 1024,
    2 * 1024 * 1024 * 1024,
];
const CACHE_LIMITS: [CacheLimit; 6] = [
    CacheLimit::Limited(500 * 1024 * 1024),
    CacheLimit::Limited(1024 * 1024 * 1024),
    CacheLimit::Limited(2 * 1024 * 1024 * 1024),
    CacheLimit::Limited(5 * 1024 * 1024 * 1024),
    CacheLimit::Limited(10 * 1024 * 1024 * 1024),
    CacheLimit::Unlimited,
];
const CACHE_LIMIT_KEYS: [&str; 6] = [
    "feature-settings-settings_cache_limit_500",
    "feature-settings-settings_cache_limit_1",
    "feature-settings-settings_cache_limit_2",
    "feature-settings-settings_cache_limit_5",
    "feature-settings-settings_cache_limit_10",
    "feature-settings-settings_cache_unlimited",
];

fn with_auto_download_rule(
    mut settings: AutoDownloadSettings,
    network: NetworkClass,
    category: MediaCategory,
    rule: AutoDownloadRule,
) -> Option<AutoDownloadSettings> {
    if matches!(category, MediaCategory::Avatar | MediaCategory::Other) {
        return None;
    }
    let mut policy = settings.policy(network);
    match category {
        MediaCategory::Photo => policy.photo = rule,
        MediaCategory::Video => policy.video = rule,
        MediaCategory::Document => policy.document = rule,
        MediaCategory::Avatar | MediaCategory::Other => return None,
    }
    match network {
        NetworkClass::Wifi => settings.wifi = policy,
        NetworkClass::Mobile => settings.mobile = policy,
        NetworkClass::Roaming => settings.roaming = policy,
    }
    Some(settings)
}

fn with_network_policy(
    mut policy: NetworkPolicy,
    network: NetworkClass,
    enabled: bool,
) -> NetworkPolicy {
    match network {
        NetworkClass::Wifi => policy.wifi = enabled,
        NetworkClass::Mobile => policy.mobile = enabled,
        NetworkClass::Roaming => policy.roaming = enabled,
    }
    policy
}

fn size_steps(current: u64) -> Vec<u64> {
    let mut steps = DOWNLOAD_SIZE_STEPS.to_vec();
    if let Err(index) = steps.binary_search(&current) {
        steps.insert(index, current);
    }
    steps
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearTarget {
    All,
    Category(MediaCategory),
    Chat(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Overview,
    Data,
    Connectivity,
    Appearance,
    Maintenance,
}

pub struct Settings {
    i18n: Rc<I18n>,
    theme: u32,
    locale: u32,
    page: SettingsPage,
    logout_enabled: bool,
    confirming_logout: bool,
    media_settings: Option<MediaSettings>,
    usage: Option<MediaUsage>,
    media_loading: bool,
    media_error: Option<String>,
    selected_network: NetworkClass,
    confirming_clear: Option<ClearTarget>,
    connectivity: Option<Controller<ConnectivityPage>>,
    appearance: Option<Controller<Appearance>>,
    maintenance: Option<Controller<Maintenance>>,
    appearance_state: AppearanceState,
    appearance_backend: AppearancePreferences,
    appearance_restored: bool,
    preference_versions: BTreeMap<PreferenceNamespace, u32>,
    notification_settings: NotificationSettings,
    local_notification_policy: LocalNotificationPolicy,
    proxy_ids: Vec<u64>,
    restoring_connectivity: u8,
    rendering: Rc<Cell<bool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }

    fn apply(self) {
        adw::StyleManager::default().set_color_scheme(match self {
            Self::System => adw::ColorScheme::Default,
            Self::Light => adw::ColorScheme::ForceLight,
            Self::Dark => adw::ColorScheme::ForceDark,
        });
    }
}

pub enum SettingsInput {
    RestorePreferences {
        theme: ThemePreference,
        locale: String,
    },
    SetI18n(Rc<I18n>),
    SetLogoutEnabled(bool),
    SetMediaSettings(MediaSettings),
    SetMediaUsage(MediaUsage),
    SetMediaLoading(bool),
    SetMediaError(Option<String>),
    RefreshMedia,
    CacheLimit(u32),
    SelectNetwork(u32),
    DownloadEnabled(usize, bool),
    DownloadLimit(usize, u32),
    Autoplay(bool),
    Preload(bool),
    RequestClear,
    RequestClearCategory(MediaCategory),
    RequestClearChat(i64),
    CancelClear,
    ConfirmClear,
    Open(SettingsPage),
    RefreshPreferences,
    SetPreferences(AppPreferences),
    PreferenceChanged {
        namespace: PreferenceNamespace,
        changed: bool,
    },
    Connectivity(ConnectivityInput),
    ConnectivityOutput(ConnectivityOutput),
    Appearance(AppearanceInput),
    AppearanceOutput(AppearanceOutput),
    WallpaperCatalog(Box<mono_proto_client::adapter::media::wallpaper::WallpaperCatalogDto>),
    WallpaperLoaded(crate::client::appearance::WallpaperFile),
    AppearanceFailed(Option<String>),
    Maintenance(MaintenanceInput),
    MaintenanceOutput(MaintenanceOutput),
    Back,
    Theme(u32),
    Locale(u32),
    RequestLogout,
    CancelLogout,
    ConfirmLogout,
}

impl fmt::Debug for SettingsInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RestorePreferences { .. } => "RestorePreferences",
            Self::SetI18n(_) => "SetI18n",
            Self::SetLogoutEnabled(_) => "SetLogoutEnabled",
            Self::SetMediaSettings(_) => "SetMediaSettings",
            Self::SetMediaUsage(_) => "SetMediaUsage",
            Self::SetMediaLoading(_) => "SetMediaLoading",
            Self::SetMediaError(_) => "SetMediaError",
            Self::RefreshMedia => "RefreshMedia",
            Self::CacheLimit(_) => "CacheLimit",
            Self::SelectNetwork(_) => "SelectNetwork",
            Self::DownloadEnabled(_, _) => "DownloadEnabled",
            Self::DownloadLimit(_, _) => "DownloadLimit",
            Self::Autoplay(_) => "Autoplay",
            Self::Preload(_) => "Preload",
            Self::RequestClear => "RequestClear",
            Self::RequestClearCategory(_) => "RequestClearCategory",
            Self::RequestClearChat(_) => "RequestClearChat",
            Self::CancelClear => "CancelClear",
            Self::ConfirmClear => "ConfirmClear",
            Self::Open(_) => "Open",
            Self::RefreshPreferences => "RefreshPreferences",
            Self::SetPreferences(_) => "SetPreferences",
            Self::PreferenceChanged { .. } => "PreferenceChanged",
            Self::Connectivity(_) => "Connectivity",
            Self::ConnectivityOutput(_) => "ConnectivityOutput",
            Self::Appearance(_) => "Appearance",
            Self::AppearanceOutput(_) => "AppearanceOutput",
            Self::WallpaperCatalog(_) => "WallpaperCatalog",
            Self::WallpaperLoaded(_) => "WallpaperLoaded",
            Self::AppearanceFailed(_) => "AppearanceFailed",
            Self::Maintenance(_) => "Maintenance",
            Self::MaintenanceOutput(_) => "MaintenanceOutput",
            Self::Back => "Back",
            Self::Theme(_) => "Theme",
            Self::Locale(_) => "Locale",
            Self::RequestLogout => "RequestLogout",
            Self::CancelLogout => "CancelLogout",
            Self::ConfirmLogout => "ConfirmLogout",
        })
    }
}

pub enum SettingsOutput {
    Back,
    ChangeTheme(ThemePreference),
    ChangeLocale(String),
    Command(WorkerCommand),
    Wallpaper(WallpaperConfig),
    Logout,
}

impl fmt::Debug for SettingsOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Back => "Back",
            Self::ChangeTheme(_) => "ChangeTheme",
            Self::ChangeLocale(_) => "ChangeLocale",
            Self::Command(_) => "Command(..)",
            Self::Wallpaper(_) => "Wallpaper",
            Self::Logout => "Logout",
        })
    }
}

impl Settings {
    fn new(i18n: Rc<I18n>) -> Self {
        let theme = match adw::StyleManager::default().color_scheme() {
            adw::ColorScheme::ForceLight | adw::ColorScheme::PreferLight => 1,
            adw::ColorScheme::ForceDark | adw::ColorScheme::PreferDark => 2,
            _ => 0,
        };
        let locale = locale_index(&i18n);
        let appearance_state = AppearanceState {
            theme: match theme {
                1 => super::appearance::ThemePreference::Light,
                2 => super::appearance::ThemePreference::Dark,
                _ => super::appearance::ThemePreference::System,
            },
            ..AppearanceState::default()
        };
        Self {
            i18n,
            theme,
            locale,
            page: SettingsPage::Overview,
            logout_enabled: false,
            confirming_logout: false,
            media_settings: None,
            usage: None,
            media_loading: false,
            media_error: None,
            selected_network: NetworkClass::Wifi,
            confirming_clear: None,
            connectivity: None,
            appearance: None,
            maintenance: None,
            appearance_state,
            appearance_backend: AppearancePreferences::default(),
            appearance_restored: false,
            preference_versions: BTreeMap::new(),
            notification_settings: NotificationSettings::default(),
            local_notification_policy: LocalNotificationPolicy::default(),
            proxy_ids: Vec::new(),
            restoring_connectivity: 0,
            rendering: Rc::new(Cell::new(false)),
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }

    fn text_arg(&self, key: &str, value: &str) -> String {
        self.i18n
            .text(key, &[("arg1", value)])
            .unwrap_or_else(|_| key.into())
    }

    fn total_usage(&self) -> u64 {
        self.usage.as_ref().map_or(0, |usage| {
            usage.complete_bytes.saturating_add(usage.partial_bytes)
        })
    }

    fn category_usage(&self, category: MediaCategory) -> u64 {
        self.usage.as_ref().map_or(0, |usage| {
            usage
                .by_category
                .iter()
                .filter(|row| row.category == category)
                .fold(0_u64, |total, row| {
                    total.saturating_add(row.complete_bytes.saturating_add(row.partial_bytes))
                })
        })
    }

    fn chat_usage(&self, chat_id: i64) -> u64 {
        self.usage.as_ref().map_or(0, |usage| {
            usage
                .by_chat
                .iter()
                .filter(|row| row.chat_id == chat_id)
                .fold(0_u64, |total, row| {
                    total.saturating_add(row.complete_bytes.saturating_add(row.partial_bytes))
                })
        })
    }

    fn clear_bytes(&self, target: ClearTarget) -> u64 {
        match target {
            ClearTarget::All => self.total_usage(),
            ClearTarget::Category(category) => self.category_usage(category),
            ClearTarget::Chat(chat_id) => self.chat_usage(chat_id),
        }
    }

    fn clear_scope(&self, target: ClearTarget) -> Option<ClearMediaScope> {
        if self.clear_bytes(target) == 0 {
            return None;
        }
        match target {
            ClearTarget::All => Some(ClearMediaScope::All),
            ClearTarget::Category(category) => Some(ClearMediaScope::Category(category)),
            ClearTarget::Chat(0) => None,
            ClearTarget::Chat(chat_id) => Some(ClearMediaScope::Chat(chat_id)),
        }
    }

    fn command(&mut self, command: WorkerCommand) -> Option<SettingsOutput> {
        if self.media_loading {
            return None;
        }
        self.media_loading = true;
        self.media_error = None;
        Some(SettingsOutput::Command(command))
    }

    fn refresh_media(&mut self) -> Vec<SettingsOutput> {
        if self.media_loading {
            return Vec::new();
        }
        self.media_loading = true;
        self.media_error = None;
        vec![
            SettingsOutput::Command(WorkerCommand::MediaUsage),
            SettingsOutput::Command(WorkerCommand::GetMediaSettings),
        ]
    }

    fn connectivity_refresh() -> Vec<SettingsOutput> {
        [
            ConnectivityCommand::ListProxies,
            ConnectivityCommand::GetNotifyDefaults,
            ConnectivityCommand::GetNotifyExceptions,
            ConnectivityCommand::GetDeviceSessions,
        ]
        .into_iter()
        .map(|command| SettingsOutput::Command(WorkerCommand::Connectivity(command)))
        .collect()
    }

    fn open(&mut self, page: SettingsPage) -> Vec<SettingsOutput> {
        self.page = page;
        match page {
            SettingsPage::Overview => Vec::new(),
            SettingsPage::Data => self.refresh_media(),
            SettingsPage::Connectivity => Self::connectivity_refresh(),
            SettingsPage::Appearance => {
                if self
                    .preference_versions
                    .contains_key(&PreferenceNamespace::Appearance)
                {
                    Vec::new()
                } else {
                    vec![SettingsOutput::Command(WorkerCommand::GetPreferences)]
                }
            }
            SettingsPage::Maintenance => vec![
                SettingsOutput::Command(WorkerCommand::Maintenance(MaintenanceCommand::Logs {
                    level: None,
                    query: String::new(),
                })),
                SettingsOutput::Command(WorkerCommand::Maintenance(MaintenanceCommand::Stats)),
            ],
        }
    }

    fn back(&mut self) -> Option<SettingsOutput> {
        self.confirming_logout = false;
        self.confirming_clear = None;
        if self.page == SettingsPage::Overview {
            Some(SettingsOutput::Back)
        } else {
            self.page = SettingsPage::Overview;
            None
        }
    }

    fn restore_appearance(&self) {
        if let Some(appearance) = &self.appearance {
            appearance.emit(AppearanceInput::Restore(self.appearance_state.clone()));
        }
    }

    fn accept_connectivity_input(&mut self, input: ConnectivityInput) {
        match &input {
            ConnectivityInput::SetProxies(proxies) => {
                self.proxy_ids = proxies
                    .iter()
                    .filter_map(|proxy| u64::try_from(proxy.id).ok())
                    .collect();
            }
            ConnectivityInput::SetNotificationSettings(settings) => {
                self.notification_settings = *settings;
            }
            _ => {}
        }
        if let Some(connectivity) = &self.connectivity {
            connectivity.emit(input);
        }
    }

    fn handle(&mut self, message: SettingsInput) -> Vec<SettingsOutput> {
        let output = match message {
            SettingsInput::RestorePreferences { theme, locale } => {
                self.theme = theme.index();
                theme.apply();
                self.appearance_state.theme =
                    super::appearance::ThemePreference::from_setting(theme.as_str())
                        .unwrap_or(super::appearance::ThemePreference::System);
                self.appearance_restored = true;
                if SUPPORTED_LOCALES.contains(&locale.as_str())
                    && let Ok(i18n) = I18n::new(&locale)
                {
                    self.locale = locale_index(&i18n);
                    self.i18n = Rc::new(i18n);
                }
                self.restore_appearance();
                None
            }
            SettingsInput::SetI18n(i18n) => {
                self.locale = locale_index(&i18n);
                self.i18n = i18n.clone();
                if let Some(connectivity) = &self.connectivity {
                    connectivity.emit(ConnectivityInput::SetI18n(i18n.clone()));
                }
                if let Some(appearance) = &self.appearance {
                    appearance.emit(AppearanceInput::SetI18n(i18n.clone()));
                }
                if let Some(maintenance) = &self.maintenance {
                    maintenance.emit(MaintenanceInput::SetI18n(i18n));
                }
                None
            }
            SettingsInput::SetLogoutEnabled(enabled) => {
                self.logout_enabled = enabled;
                self.confirming_logout = false;
                None
            }
            SettingsInput::SetMediaSettings(settings) => {
                if self.media_settings.is_none() {
                    self.selected_network = settings.network;
                }
                self.media_settings = Some(settings);
                None
            }
            SettingsInput::SetMediaUsage(usage) => {
                self.usage = Some(usage);
                self.confirming_clear = None;
                None
            }
            SettingsInput::SetMediaLoading(loading) => {
                self.media_loading = loading;
                None
            }
            SettingsInput::SetMediaError(error) => {
                self.media_loading = false;
                self.media_error = error;
                None
            }
            SettingsInput::RefreshMedia if !self.media_loading => {
                return self.refresh_media();
            }
            SettingsInput::CacheLimit(index) => {
                let Some(settings) = self.media_settings else {
                    return Vec::new();
                };
                if let Some(limit) = CACHE_LIMITS.get(index as usize)
                    && *limit != settings.cache_limit
                    && !self.media_loading
                {
                    self.media_loading = true;
                    self.media_error = None;
                    return vec![
                        SettingsOutput::Command(WorkerCommand::SetCacheLimit { limit: *limit }),
                        SettingsOutput::Command(WorkerCommand::GetMediaSettings),
                    ];
                }
                None
            }
            SettingsInput::SelectNetwork(index) => {
                if let Some(network) = NETWORK_CLASSES.get(index as usize) {
                    self.selected_network = *network;
                }
                None
            }
            SettingsInput::DownloadEnabled(category, enabled) => {
                self.update_rule(category, |rule| AutoDownloadRule { enabled, ..rule })
            }
            SettingsInput::DownloadLimit(category, index) => {
                self.update_rule(category, |rule| AutoDownloadRule {
                    max_bytes: size_steps(rule.max_bytes)
                        .get(index as usize)
                        .copied()
                        .unwrap_or(rule.max_bytes),
                    ..rule
                })
            }
            SettingsInput::Autoplay(enabled) => {
                let Some(settings) = self.media_settings else {
                    return Vec::new();
                };
                let updated =
                    with_network_policy(settings.autoplay, self.selected_network, enabled);
                (updated != settings.autoplay)
                    .then(|| self.command(WorkerCommand::SetAutoplay(updated)))
                    .flatten()
            }
            SettingsInput::Preload(enabled) => {
                let Some(settings) = self.media_settings else {
                    return Vec::new();
                };
                let updated = with_network_policy(settings.preload, self.selected_network, enabled);
                (updated != settings.preload)
                    .then(|| self.command(WorkerCommand::SetPreload(updated)))
                    .flatten()
            }
            SettingsInput::RequestClear
                if self.clear_scope(ClearTarget::All).is_some() && !self.media_loading =>
            {
                self.confirming_clear = Some(ClearTarget::All);
                None
            }
            SettingsInput::RequestClearCategory(category)
                if self.category_usage(category) > 0 && !self.media_loading =>
            {
                self.confirming_clear = Some(ClearTarget::Category(category));
                None
            }
            SettingsInput::RequestClearChat(chat_id)
                if chat_id != 0 && self.chat_usage(chat_id) > 0 && !self.media_loading =>
            {
                self.confirming_clear = Some(ClearTarget::Chat(chat_id));
                None
            }
            SettingsInput::CancelClear => {
                self.confirming_clear = None;
                None
            }
            SettingsInput::ConfirmClear => self
                .confirming_clear
                .and_then(|target| self.clear_scope(target))
                .and_then(|scope| self.command(WorkerCommand::ClearMedia(scope))),
            SettingsInput::Open(page) => return self.open(page),
            SettingsInput::RefreshPreferences if !self.media_loading => {
                Some(SettingsOutput::Command(WorkerCommand::GetPreferences))
            }
            SettingsInput::SetPreferences(preferences) => {
                self.media_loading = false;
                let mut outputs = self.apply_preferences(preferences);
                outputs.extend(self.refresh_media());
                return outputs;
            }
            SettingsInput::PreferenceChanged { namespace, changed } => {
                if changed {
                    None
                } else {
                    self.preference_versions.remove(&namespace);
                    Some(SettingsOutput::Command(WorkerCommand::GetPreferences))
                }
            }
            SettingsInput::Connectivity(input) => {
                self.accept_connectivity_input(input);
                None
            }
            SettingsInput::ConnectivityOutput(output) => return self.connectivity_output(output),
            SettingsInput::Appearance(input) => {
                if let Some(appearance) = &self.appearance {
                    appearance.emit(input);
                }
                None
            }
            SettingsInput::AppearanceOutput(output) => return self.appearance_output(output),
            SettingsInput::WallpaperCatalog(catalog) => {
                self.appearance_backend.hash = catalog.hash;
                if let Some(appearance) = &self.appearance {
                    appearance.emit(AppearanceInput::WallpaperCatalog(catalog));
                }
                return self.appearance_preference_output().into_iter().collect();
            }
            SettingsInput::WallpaperLoaded(file) => {
                if let Some(appearance) = &self.appearance {
                    appearance.emit(AppearanceInput::WallpaperLoaded(UiWallpaperFile {
                        path: file.path,
                        pattern: file.pattern,
                        colors: file.colors,
                        intensity: file.intensity,
                        rotation: file.rotation,
                        blur: file.blur,
                    }));
                }
                None
            }
            SettingsInput::AppearanceFailed(error) => {
                if let Some(appearance) = &self.appearance {
                    appearance.emit(AppearanceInput::WallpaperFailed(error));
                }
                None
            }
            SettingsInput::Maintenance(input) => {
                if let Some(maintenance) = &self.maintenance {
                    maintenance.emit(input);
                }
                None
            }
            SettingsInput::MaintenanceOutput(output) => return self.maintenance_output(output),
            SettingsInput::Back => self.back(),
            SettingsInput::Theme(theme) if theme < 3 && theme != self.theme => {
                self.theme = theme;
                let preference = match theme {
                    1 => ThemePreference::Light,
                    2 => ThemePreference::Dark,
                    _ => ThemePreference::System,
                };
                preference.apply();
                self.appearance_state.theme =
                    super::appearance::ThemePreference::from_setting(preference.as_str())
                        .unwrap_or(super::appearance::ThemePreference::System);
                let mut outputs = vec![SettingsOutput::ChangeTheme(preference)];
                outputs.extend(self.appearance_preference_output());
                return outputs;
            }
            SettingsInput::Locale(index) if index != self.locale => {
                if let Some(locale) = SUPPORTED_LOCALES.get(index as usize)
                    && let Ok(i18n) = I18n::new(locale)
                {
                    self.locale = index;
                    self.i18n = Rc::new(i18n);
                    let mut outputs = vec![SettingsOutput::ChangeLocale((*locale).into())];
                    outputs.extend(self.appearance_preference_output());
                    return outputs;
                }
                None
            }
            SettingsInput::RequestLogout if self.logout_enabled => {
                self.confirming_logout = true;
                None
            }
            SettingsInput::CancelLogout => {
                self.confirming_logout = false;
                None
            }
            SettingsInput::ConfirmLogout if self.logout_enabled && self.confirming_logout => {
                self.confirming_logout = false;
                self.logout_enabled = false;
                Some(SettingsOutput::Logout)
            }
            _ => None,
        };
        output.into_iter().collect()
    }

    fn update_rule(
        &mut self,
        category: usize,
        update: impl FnOnce(AutoDownloadRule) -> AutoDownloadRule,
    ) -> Option<SettingsOutput> {
        let settings = self.media_settings?;
        let category = *DOWNLOAD_CATEGORIES.get(category)?;
        let rule = settings
            .auto_download
            .policy(self.selected_network)
            .rule(category)?;
        let updated = with_auto_download_rule(
            settings.auto_download,
            self.selected_network,
            category,
            update(rule),
        )?;
        (updated != settings.auto_download)
            .then(|| self.command(WorkerCommand::SetAutoDownload(updated)))
            .flatten()
    }

    fn apply_preferences(&mut self, preferences: AppPreferences) -> Vec<SettingsOutput> {
        let previous = self.appearance_state.wallpaper.clone();
        for (namespace, document) in preferences.documents {
            self.preference_versions.insert(namespace, document.version);
            match namespace {
                PreferenceNamespace::Appearance => {
                    self.restore_appearance_document(&document.payload_json);
                }
                PreferenceNamespace::Notifications => {
                    self.restore_notification_document(&document.payload_json);
                }
                _ => {}
            }
        }
        (previous != self.appearance_state.wallpaper)
            .then(|| SettingsOutput::Wallpaper(self.appearance_state.wallpaper.clone()))
            .into_iter()
            .collect()
    }

    fn restore_appearance_document(&mut self, payload: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            return;
        };
        let backend =
            serde_json::from_value::<AppearancePreferences>(value.clone()).unwrap_or_default();
        if backend.validate().is_err() {
            return;
        }
        let empty = serde_json::json!({});
        let ui = value.get("ui").unwrap_or(&empty);
        let mut state = self.appearance_state.clone();
        let locale = ui
            .get("locale")
            .and_then(serde_json::Value::as_str)
            .filter(|locale| SUPPORTED_LOCALES.contains(locale));
        if !self.appearance_restored {
            if let Some(theme) = ui
                .get("theme")
                .and_then(serde_json::Value::as_str)
                .and_then(super::appearance::ThemePreference::from_setting)
            {
                state.theme = theme;
                self.theme = ThemePreference::from_setting(theme.as_str())
                    .unwrap_or(ThemePreference::System)
                    .index();
            }
            if let Some(locale) = locale
                && let Ok(i18n) = I18n::new(locale)
            {
                self.i18n = Rc::new(i18n);
                self.locale = locale_index(&self.i18n);
            }
        }
        if let Some(accent) = ui
            .get("accent")
            .and_then(serde_json::Value::as_str)
            .and_then(super::appearance::AccentPreset::from_setting)
        {
            state.accent = accent;
        }
        state.message_text_size = ui
            .get("message_text_size")
            .and_then(serde_json::Value::as_u64)
            .map_or(state.message_text_size, |value| {
                super::appearance::clamp_message_text_size(value as u32)
            });
        state.line_spacing = ui
            .get("line_spacing")
            .and_then(serde_json::Value::as_f64)
            .map_or(state.line_spacing, |value| {
                super::appearance::clamp_line_spacing(value as f32)
            });
        state.letter_spacing = ui
            .get("letter_spacing")
            .and_then(serde_json::Value::as_f64)
            .map_or(state.letter_spacing, |value| {
                super::appearance::clamp_letter_spacing(value as f32)
            });
        state.preview_lines = ui
            .get("preview_lines")
            .and_then(serde_json::Value::as_u64)
            .map_or(state.preview_lines, |value| {
                super::appearance::clamp_preview_lines(value as u32)
            });
        state.show_chat_avatars = ui
            .get("show_chat_avatars")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.show_chat_avatars);
        state.open_profile_on_avatar_tap = ui
            .get("open_profile_on_avatar_tap")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.open_profile_on_avatar_tap);
        state.show_read_status = ui
            .get("show_read_status")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.show_read_status);
        state.show_muted_counter = ui
            .get("show_muted_counter")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.show_muted_counter);
        state.folders_at_bottom = ui
            .get("folders_at_bottom")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.folders_at_bottom);
        if let Some(style) = ui
            .get("composer_style")
            .and_then(serde_json::Value::as_str)
            .and_then(super::appearance::ComposerStyle::from_setting)
        {
            state.composer_style = style;
        }
        if let Some(action) = ui
            .get("left_composer_action")
            .and_then(serde_json::Value::as_str)
            .and_then(super::appearance::ComposerSlotAction::from_setting)
        {
            state.left_composer_action = action;
        }
        if let Some(action) = ui
            .get("right_composer_action")
            .and_then(serde_json::Value::as_str)
            .and_then(super::appearance::ComposerSlotAction::from_setting)
        {
            state.right_composer_action = action;
        }
        state.send_by_enter = ui
            .get("send_by_enter")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.send_by_enter);
        state.fix_link_previews = ui
            .get("fix_link_previews")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(state.fix_link_previews);
        state.wallpaper = if let Some(wallpaper) = ui.get("wallpaper") {
            let path = wallpaper
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let telegram = wallpaper
                .get("telegram")
                .and_then(|telegram| {
                    Some(super::appearance::TelegramWallpaper {
                        id: telegram.get("id")?.as_i64()?,
                        access_hash: telegram.get("access_hash")?.as_i64()?,
                        slug: telegram
                            .get("slug")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .into(),
                    })
                })
                .filter(|telegram| telegram.id != 0);
            let mode = wallpaper
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(WallpaperMode::from_setting)
                .unwrap_or(WallpaperMode::Monogram);
            WallpaperConfig {
                mode: if mode == WallpaperMode::Image && path.is_none() && telegram.is_none() {
                    WallpaperMode::Monogram
                } else {
                    mode
                },
                path,
                telegram,
                pattern: wallpaper
                    .get("pattern")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(state.wallpaper.pattern),
                colors: wallpaper
                    .get("colors")
                    .and_then(serde_json::Value::as_array)
                    .map(|colors| {
                        colors
                            .iter()
                            .filter_map(serde_json::Value::as_i64)
                            .filter_map(|color| i32::try_from(color).ok())
                            .take(4)
                            .collect()
                    })
                    .unwrap_or_else(|| state.wallpaper.colors.clone()),
                dim: wallpaper
                    .get("dim")
                    .and_then(serde_json::Value::as_f64)
                    .map_or(state.wallpaper.dim, |value| {
                        super::appearance::clamp_wallpaper_dim(value as f32)
                    }),
                blur: wallpaper
                    .get("blur")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(state.wallpaper.blur),
                intensity: wallpaper
                    .get("intensity")
                    .and_then(serde_json::Value::as_i64)
                    .map_or(state.wallpaper.intensity, |value| {
                        super::appearance::clamp_wallpaper_intensity(value as i32)
                    }),
                rotation: wallpaper
                    .get("rotation")
                    .and_then(serde_json::Value::as_i64)
                    .map_or(state.wallpaper.rotation, |value| {
                        super::appearance::clamp_wallpaper_rotation(value as i32)
                    }),
            }
        } else {
            let dim = if backend.dim {
                state.wallpaper.dim
            } else {
                0.0
            };
            match &backend.source {
                WallpaperSource::Default => WallpaperConfig {
                    mode: WallpaperMode::Monogram,
                    path: None,
                    telegram: None,
                    dim,
                    blur: backend.blur,
                    intensity: backend.intensity,
                    rotation: backend.rotation,
                    ..state.wallpaper.clone()
                },
                WallpaperSource::Telegram { id, access_hash } => WallpaperConfig {
                    mode: WallpaperMode::Image,
                    path: None,
                    telegram: Some(super::appearance::TelegramWallpaper {
                        id: *id,
                        access_hash: *access_hash,
                        slug: String::new(),
                    }),
                    dim,
                    blur: backend.blur,
                    intensity: backend.intensity,
                    rotation: backend.rotation,
                    ..state.wallpaper.clone()
                },
                WallpaperSource::None => WallpaperConfig {
                    mode: WallpaperMode::None,
                    path: None,
                    telegram: None,
                    dim,
                    blur: backend.blur,
                    intensity: backend.intensity,
                    rotation: backend.rotation,
                    ..state.wallpaper.clone()
                },
                WallpaperSource::Local { path } => WallpaperConfig {
                    mode: WallpaperMode::Image,
                    path: Some(path.clone()),
                    telegram: None,
                    dim,
                    blur: backend.blur,
                    intensity: backend.intensity,
                    rotation: backend.rotation,
                    ..state.wallpaper.clone()
                },
            }
        };
        self.appearance_backend = backend;
        self.appearance_state = state;
        if let Some(appearance) = &self.appearance {
            appearance.emit(AppearanceInput::SetI18n(self.i18n.clone()));
        }
        self.restore_appearance();
    }

    fn restore_notification_document(&mut self, payload: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            return;
        };
        let settings = NotificationSettings {
            private: value
                .get("private")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.notification_settings.private),
            groups: value
                .get("groups")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.notification_settings.groups),
            channels: value
                .get("channels")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.notification_settings.channels),
            mentions: value
                .get("mentions")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.notification_settings.mentions),
        };
        self.notification_settings = settings;
        let policy = LocalNotificationPolicy {
            enabled: value
                .get("local_enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.local_notification_policy.enabled),
            preview: value
                .get("local_preview")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.local_notification_policy.preview),
        };
        let previous = self.local_notification_policy;
        self.local_notification_policy = policy;
        if let Some(connectivity) = &self.connectivity {
            connectivity.emit(ConnectivityInput::SetNotificationSettings(settings));
            self.restoring_connectivity = u8::from(previous.enabled != policy.enabled)
                + u8::from(previous.preview != policy.preview);
            connectivity.emit(ConnectivityInput::NotificationEnabled(policy.enabled));
            connectivity.emit(ConnectivityInput::NotificationPreview(policy.preview));
        }
    }

    fn notification_preference_output(&self) -> Option<SettingsOutput> {
        self.preference_output(
            PreferenceNamespace::Notifications,
            PreferenceDocument {
                version: APP_PREFERENCES_VERSION,
                payload_json: serde_json::json!({
                    "private": self.notification_settings.private,
                    "groups": self.notification_settings.groups,
                    "channels": self.notification_settings.channels,
                    "mentions": self.notification_settings.mentions,
                    "local_enabled": self.local_notification_policy.enabled,
                    "local_preview": self.local_notification_policy.preview,
                })
                .to_string(),
            },
        )
    }

    fn notification_defaults(settings: NotificationSettings) -> Vec<SettingsOutput> {
        [
            (NotifyScope::Users, settings.private),
            (NotifyScope::Chats, settings.groups),
            (NotifyScope::Broadcasts, settings.channels),
        ]
        .into_iter()
        .map(|(scope, enabled)| {
            SettingsOutput::Command(WorkerCommand::Connectivity(
                ConnectivityCommand::SetNotifyDefault {
                    scope,
                    settings: NotifyUpdate {
                        show_previews: None,
                        silent: !enabled,
                        mute_until: 0,
                        stories_muted: false,
                        sound: "default".into(),
                    },
                },
            ))
        })
        .collect()
    }

    fn connectivity_output(&mut self, output: ConnectivityOutput) -> Vec<SettingsOutput> {
        match output {
            ConnectivityOutput::Back => self.back().into_iter().collect(),
            ConnectivityOutput::Refresh => Self::connectivity_refresh(),
            ConnectivityOutput::ProxySave(submission) => match proxy_save_command(submission) {
                Ok(command) => vec![SettingsOutput::Command(command)],
                Err(key) => {
                    let error = self.text(key);
                    if let Some(connectivity) = &self.connectivity {
                        connectivity.emit(ConnectivityInput::SetError(Some(error)));
                    }
                    Vec::new()
                }
            },
            ConnectivityOutput::ProxyDelete(id) => u64::try_from(id)
                .ok()
                .map(|id| {
                    SettingsOutput::Command(WorkerCommand::Connectivity(
                        ConnectivityCommand::DeleteProxy { id },
                    ))
                })
                .into_iter()
                .collect(),
            ConnectivityOutput::ProxyActivate(id) => u64::try_from(id)
                .ok()
                .map(|id| {
                    SettingsOutput::Command(WorkerCommand::Connectivity(
                        ConnectivityCommand::ActivateProxy { id },
                    ))
                })
                .into_iter()
                .collect(),
            ConnectivityOutput::ProxyDirect => {
                vec![SettingsOutput::Command(WorkerCommand::Connectivity(
                    ConnectivityCommand::SetDirect,
                ))]
            }
            ConnectivityOutput::ProxyProbe(id) => u64::try_from(id)
                .ok()
                .map(|id| {
                    SettingsOutput::Command(WorkerCommand::Connectivity(
                        ConnectivityCommand::ProbeProxies { ids: vec![id] },
                    ))
                })
                .into_iter()
                .collect(),
            ConnectivityOutput::ProxyProbeAll => {
                if self.proxy_ids.is_empty() {
                    Vec::new()
                } else {
                    vec![SettingsOutput::Command(WorkerCommand::Connectivity(
                        ConnectivityCommand::ProbeProxies {
                            ids: self.proxy_ids.clone(),
                        },
                    ))]
                }
            }
            ConnectivityOutput::NotificationSave(settings) => {
                self.notification_settings = settings;
                let mut outputs = Self::notification_defaults(settings);
                outputs.extend(self.notification_preference_output());
                outputs
            }
            ConnectivityOutput::NotificationMute { chat_id } => {
                vec![SettingsOutput::Command(WorkerCommand::Connectivity(
                    ConnectivityCommand::SetPeerNotify {
                        chat_id,
                        settings: NotifyUpdate {
                            show_previews: None,
                            silent: true,
                            mute_until: 0,
                            stories_muted: false,
                            sound: "default".into(),
                        },
                    },
                ))]
            }
            ConnectivityOutput::NotificationUnmute { chat_id } => {
                vec![SettingsOutput::Command(WorkerCommand::Connectivity(
                    ConnectivityCommand::SetPeerNotify {
                        chat_id,
                        settings: NotifyUpdate {
                            show_previews: None,
                            silent: false,
                            mute_until: 0,
                            stories_muted: false,
                            sound: "default".into(),
                        },
                    },
                ))]
            }
            ConnectivityOutput::NotificationReset => {
                self.notification_settings = NotificationSettings::default();
                let mut outputs = vec![SettingsOutput::Command(WorkerCommand::Connectivity(
                    ConnectivityCommand::ResetNotify,
                ))];
                outputs.extend(self.notification_preference_output());
                outputs
            }
            ConnectivityOutput::LocalPolicyChanged(policy) => {
                if self.restoring_connectivity > 0 {
                    self.restoring_connectivity -= 1;
                    return Vec::new();
                }
                self.local_notification_policy = policy;
                self.notification_preference_output().into_iter().collect()
            }
            ConnectivityOutput::DeviceRevoke(hash) => hash
                .parse::<i64>()
                .ok()
                .map(|hash| {
                    SettingsOutput::Command(WorkerCommand::Connectivity(
                        ConnectivityCommand::RevokeDeviceSession { hash },
                    ))
                })
                .into_iter()
                .collect(),
            ConnectivityOutput::DeviceQr => {
                vec![SettingsOutput::Command(WorkerCommand::ExportQr)]
            }
            ConnectivityOutput::DeviceQrPoll => {
                vec![SettingsOutput::Command(WorkerCommand::PollQr)]
            }
        }
    }

    fn maintenance_output(&mut self, output: MaintenanceOutput) -> Vec<SettingsOutput> {
        match output {
            MaintenanceOutput::Back => self.back().into_iter().collect(),
            MaintenanceOutput::Command(command) => {
                vec![SettingsOutput::Command(WorkerCommand::Maintenance(command))]
            }
        }
    }

    fn appearance_output(&mut self, output: AppearanceOutput) -> Vec<SettingsOutput> {
        match output {
            AppearanceOutput::Back => self.back().into_iter().collect(),
            AppearanceOutput::Theme(theme) => {
                let preference = ThemePreference::from_setting(theme.as_str())
                    .unwrap_or(ThemePreference::System);
                self.theme = preference.index();
                self.appearance_state.theme = theme;
                let mut outputs = vec![SettingsOutput::ChangeTheme(preference)];
                outputs.extend(self.appearance_preference_output());
                outputs
            }
            AppearanceOutput::BrowseTelegramWallpapers => {
                vec![SettingsOutput::Command(WorkerCommand::Appearance(
                    crate::client::appearance::AppearanceCommand::GetWallpapers {
                        hash: self.appearance_backend.hash,
                    },
                ))]
            }
            AppearanceOutput::ImportLocalWallpaper { path } => {
                vec![SettingsOutput::Command(WorkerCommand::Appearance(
                    crate::client::appearance::AppearanceCommand::ImportLocalWallpaper {
                        path,
                        pattern: false,
                    },
                ))]
            }
            AppearanceOutput::DownloadTelegramWallpaper {
                id,
                access_hash,
                pattern,
            } => {
                vec![SettingsOutput::Command(WorkerCommand::Appearance(
                    crate::client::appearance::AppearanceCommand::DownloadWallpaper {
                        id,
                        access_hash,
                        pattern,
                    },
                ))]
            }
            AppearanceOutput::Command(command) => {
                let wallpaper = match &command {
                    AppearanceCommand::SetWallpaper(config) => Some(config.clone()),
                    _ => None,
                };
                self.apply_appearance_command(command);
                let mut outputs = self
                    .appearance_preference_output()
                    .into_iter()
                    .collect::<Vec<_>>();
                outputs.extend(wallpaper.map(SettingsOutput::Wallpaper));
                outputs
            }
        }
    }

    fn apply_appearance_command(&mut self, command: AppearanceCommand) {
        match command {
            AppearanceCommand::SetAccent(accent) => self.appearance_state.accent = accent,
            AppearanceCommand::SetMessageTextSize(size) => {
                self.appearance_state.message_text_size = size;
            }
            AppearanceCommand::SetLineSpacing(value) => {
                self.appearance_state.line_spacing = value;
            }
            AppearanceCommand::SetLetterSpacing(value) => {
                self.appearance_state.letter_spacing = value;
            }
            AppearanceCommand::SetPreviewLines(lines) => {
                self.appearance_state.preview_lines = lines;
            }
            AppearanceCommand::SetShowChatAvatars(enabled) => {
                self.appearance_state.show_chat_avatars = enabled;
            }
            AppearanceCommand::SetOpenProfileOnAvatarTap(enabled) => {
                self.appearance_state.open_profile_on_avatar_tap = enabled;
            }
            AppearanceCommand::SetShowReadStatus(enabled) => {
                self.appearance_state.show_read_status = enabled;
            }
            AppearanceCommand::SetShowMutedCounter(enabled) => {
                self.appearance_state.show_muted_counter = enabled;
            }
            AppearanceCommand::SetFoldersAtBottom(enabled) => {
                self.appearance_state.folders_at_bottom = enabled;
            }
            AppearanceCommand::SetComposerStyle(style) => {
                self.appearance_state.composer_style = style;
            }
            AppearanceCommand::SetLeftComposerAction(action) => {
                self.appearance_state.left_composer_action = action;
            }
            AppearanceCommand::SetRightComposerAction(action) => {
                self.appearance_state.right_composer_action = action;
            }
            AppearanceCommand::SetSendByEnter(enabled) => {
                self.appearance_state.send_by_enter = enabled;
            }
            AppearanceCommand::SetFixLinkPreviews(enabled) => {
                self.appearance_state.fix_link_previews = enabled;
            }
            AppearanceCommand::SetWallpaper(config) => {
                self.appearance_state.wallpaper = config.clone();
                self.appearance_backend.source = match config.mode {
                    WallpaperMode::Monogram => WallpaperSource::Default,
                    WallpaperMode::None => WallpaperSource::None,
                    WallpaperMode::Image => match config.telegram {
                        Some(telegram) => WallpaperSource::Telegram {
                            id: telegram.id,
                            access_hash: telegram.access_hash,
                        },
                        None => WallpaperSource::Local {
                            path: config.path.unwrap_or_default(),
                        },
                    },
                };
                self.appearance_backend.dim = config.dim > 0.0;
                self.appearance_backend.blur = config.blur;
                self.appearance_backend.intensity = config.intensity;
                self.appearance_backend.rotation = config.rotation;
            }
        }
    }

    fn preference_output(
        &self,
        namespace: PreferenceNamespace,
        document: PreferenceDocument,
    ) -> Option<SettingsOutput> {
        document.validate().ok()?;
        Some(SettingsOutput::Command(WorkerCommand::SetPreference {
            namespace,
            expected_version: self.preference_versions.get(&namespace).copied(),
            document,
        }))
    }

    fn appearance_preference_output(&self) -> Option<SettingsOutput> {
        let mut payload = serde_json::to_value(&self.appearance_backend).ok()?;
        payload["ui"] = serde_json::json!({
            "theme": self.appearance_state.theme.as_str(),
            "locale": self.i18n.locale(),
            "accent": self.appearance_state.accent.as_str(),
            "message_text_size": self.appearance_state.message_text_size,
            "line_spacing": self.appearance_state.line_spacing,
            "letter_spacing": self.appearance_state.letter_spacing,
            "preview_lines": self.appearance_state.preview_lines,
            "show_chat_avatars": self.appearance_state.show_chat_avatars,
            "open_profile_on_avatar_tap": self.appearance_state.open_profile_on_avatar_tap,
            "show_read_status": self.appearance_state.show_read_status,
            "show_muted_counter": self.appearance_state.show_muted_counter,
            "folders_at_bottom": self.appearance_state.folders_at_bottom,
            "composer_style": self.appearance_state.composer_style.as_str(),
            "left_composer_action": self.appearance_state.left_composer_action.as_str(),
            "right_composer_action": self.appearance_state.right_composer_action.as_str(),
            "send_by_enter": self.appearance_state.send_by_enter,
            "fix_link_previews": self.appearance_state.fix_link_previews,
            "wallpaper": {
                "mode": self.appearance_state.wallpaper.mode.as_str(),
                "path": self.appearance_state.wallpaper.path,
                "telegram": self.appearance_state.wallpaper.telegram.as_ref().map(|telegram| serde_json::json!({
                    "id": telegram.id,
                    "access_hash": telegram.access_hash,
                    "slug": telegram.slug,
                })),
                "pattern": self.appearance_state.wallpaper.pattern,
                "colors": self.appearance_state.wallpaper.colors,
                "dim": self.appearance_state.wallpaper.dim,
                "blur": self.appearance_state.wallpaper.blur,
                "intensity": self.appearance_state.wallpaper.intensity,
                "rotation": self.appearance_state.wallpaper.rotation,
            }
        });
        self.preference_output(
            PreferenceNamespace::Appearance,
            PreferenceDocument {
                version: APP_PREFERENCES_VERSION,
                payload_json: payload.to_string(),
            },
        )
    }

    fn render(&self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        self.rendering.set(true);
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        match self.page {
            SettingsPage::Connectivity => {
                if let Some(connectivity) = &self.connectivity {
                    root.append(connectivity.widget());
                }
                self.rendering.set(false);
                return;
            }
            SettingsPage::Appearance => {
                if let Some(appearance) = &self.appearance {
                    root.append(appearance.widget());
                }
                self.rendering.set(false);
                return;
            }
            SettingsPage::Maintenance => {
                if let Some(maintenance) = &self.maintenance {
                    root.append(maintenance.widget());
                }
                self.rendering.set(false);
                return;
            }
            SettingsPage::Overview | SettingsPage::Data => {}
        }

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(&self.text(
            if self.page == SettingsPage::Data {
                "feature-settings-settings_data"
            } else {
                "desktop-settings"
            },
        )))));
        let back = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(SettingsInput::Back));
        header.pack_start(&back);
        root.append(&header);

        let page = adw::PreferencesPage::builder().vexpand(true).build();
        if self.page == SettingsPage::Data {
            page.add(&self.media_group(sender));
            page.add(&self.category_group(sender));
            page.add(&self.chat_group(sender));
            page.add(&self.download_group(sender));
        } else {
            page.add(&self.navigation_group(sender));
            page.add(&self.appearance_group(sender));
            let about = adw::PreferencesGroup::builder()
                .title(self.text("desktop-about"))
                .build();
            about.add(
                &adw::ActionRow::builder()
                    .title("Monogram")
                    .subtitle(env!("CARGO_PKG_VERSION"))
                    .build(),
            );
            page.add(&about);
            page.add(&self.logout_group(sender));
        }
        root.append(&page);
        self.rendering.set(false);
    }

    fn navigation_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        for (page, title, subtitle) in [
            (
                SettingsPage::Data,
                "feature-settings-settings_data",
                "feature-settings-settings_data_sub",
            ),
            (
                SettingsPage::Connectivity,
                "feature-auth-auth_connection",
                "feature-settings-settings_proxy_sub",
            ),
            (
                SettingsPage::Appearance,
                "desktop-appearance",
                "feature-settings-settings_theme",
            ),
            (
                SettingsPage::Maintenance,
                "desktop-maintenance",
                "feature-settings-settings_logs_sub",
            ),
        ] {
            let row = adw::ActionRow::builder()
                .title(self.text(title))
                .subtitle(self.text(subtitle))
                .activatable(true)
                .build();
            let input = sender.input_sender().clone();
            row.connect_activated(move |_| input.emit(SettingsInput::Open(page)));
            group.add(&row);
        }
        group
    }

    fn media_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_data"))
            .description(self.text("feature-settings-settings_cache_sub"))
            .build();
        let usage = format_bytes(self.total_usage());
        let summary = adw::ActionRow::builder()
            .title(self.text("feature-settings-settings_cache"))
            .subtitle(self.text_arg("feature-settings-settings_cache_size", &usage))
            .build();
        let spinner = gtk::Spinner::builder()
            .spinning(self.media_loading)
            .visible(self.media_loading)
            .build();
        summary.add_suffix(&spinner);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(&self.text("feature-settings-settings_refresh")));
        refresh.set_sensitive(!self.media_loading);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        refresh.connect_clicked(move |_| {
            if !rendering.get() {
                input.emit(SettingsInput::RefreshMedia);
            }
        });
        summary.add_suffix(&refresh);
        group.add(&summary);

        if let Some(error) = &self.media_error {
            let row = adw::ActionRow::builder()
                .title(self.text("desktop-error-generic"))
                .subtitle(error)
                .build();
            let retry = gtk::Button::with_label(&self.text("desktop-retry"));
            let input = sender.input_sender().clone();
            retry.connect_clicked(move |_| input.emit(SettingsInput::RefreshMedia));
            row.add_suffix(&retry);
            group.add(&row);
        }

        if let Some(settings) = self.media_settings {
            let limits = gtk::StringList::new(
                &CACHE_LIMIT_KEYS
                    .map(|key| self.text(key))
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            );
            let row = adw::ComboRow::builder()
                .title(self.text("feature-settings-settings_cache_limit"))
                .model(&limits)
                .selected(
                    CACHE_LIMITS
                        .iter()
                        .position(|limit| *limit == settings.cache_limit)
                        .unwrap_or(2) as u32,
                )
                .sensitive(!self.media_loading)
                .build();
            let input = sender.input_sender().clone();
            let rendering = self.rendering.clone();
            row.connect_selected_notify(move |row| {
                if !rendering.get() {
                    input.emit(SettingsInput::CacheLimit(row.selected()));
                }
            });
            group.add(&row);
        }

        let clear = gtk::Button::with_label(&self.text("feature-settings-settings_clear_cache"));
        clear.add_css_class("destructive-action");
        clear.set_sensitive(self.total_usage() > 0 && !self.media_loading);
        clear.set_visible(self.confirming_clear != Some(ClearTarget::All));
        let input = sender.input_sender().clone();
        clear.connect_clicked(move |_| input.emit(SettingsInput::RequestClear));
        group.add(&clear);
        if self.confirming_clear == Some(ClearTarget::All) {
            group.add(&self.clear_confirmation(sender, None, self.total_usage()));
        }
        group
    }

    fn category_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-storage"))
            .visible(self.usage.is_some())
            .build();
        for category in STORAGE_CATEGORIES {
            let bytes = self.category_usage(category);
            let title = match category {
                MediaCategory::Photo => "feature-settings-settings_storage_photos",
                MediaCategory::Video => "feature-settings-settings_storage_videos",
                MediaCategory::Document => "feature-settings-settings_storage_files",
                MediaCategory::Avatar => "feature-settings-settings_show_avatars",
                MediaCategory::Other => "feature-settings-settings_storage_other",
            };
            let target = ClearTarget::Category(category);
            let row = adw::ActionRow::builder()
                .title(self.text(title))
                .subtitle(format_bytes(bytes))
                .build();
            let clear =
                gtk::Button::with_label(&self.text("feature-settings-settings_clear_cache_action"));
            clear.add_css_class("destructive-action");
            clear.set_sensitive(bytes > 0 && !self.media_loading);
            clear.set_visible(self.confirming_clear != Some(target));
            let input = sender.input_sender().clone();
            clear.connect_clicked(move |_| {
                input.emit(SettingsInput::RequestClearCategory(category));
            });
            row.add_suffix(&clear);
            group.add(&row);
            if self.confirming_clear == Some(target) {
                group.add(&self.clear_confirmation(sender, None, bytes));
            }
        }
        group
    }

    fn chat_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_storage_chats"))
            .visible(self.usage.is_some())
            .build();
        let Some(usage) = &self.usage else {
            return group;
        };
        if usage.by_chat.is_empty() {
            group.add(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_storage_empty"),
            )));
            return group;
        }
        let mut chats = usage.by_chat.iter().collect::<Vec<_>>();
        chats.sort_by(|left, right| {
            let left_bytes = left.complete_bytes.saturating_add(left.partial_bytes);
            let right_bytes = right.complete_bytes.saturating_add(right.partial_bytes);
            right_bytes
                .cmp(&left_bytes)
                .then(left.chat_id.cmp(&right.chat_id))
        });
        for chat in chats {
            let bytes = chat.complete_bytes.saturating_add(chat.partial_bytes);
            let target = ClearTarget::Chat(chat.chat_id);
            let row = adw::ActionRow::builder()
                .title(chat.chat_id.to_string())
                .subtitle(format_bytes(bytes))
                .build();
            let clear =
                gtk::Button::with_label(&self.text("feature-settings-settings_clear_cache_action"));
            clear.add_css_class("destructive-action");
            clear.set_sensitive(chat.chat_id != 0 && bytes > 0 && !self.media_loading);
            clear.set_visible(self.confirming_clear != Some(target));
            let input = sender.input_sender().clone();
            let chat_id = chat.chat_id;
            clear.connect_clicked(move |_| {
                input.emit(SettingsInput::RequestClearChat(chat_id));
            });
            row.add_suffix(&clear);
            group.add(&row);
            if self.confirming_clear == Some(target) {
                group.add(&self.clear_confirmation(
                    sender,
                    Some("feature-settings-settings_storage_clear_chat"),
                    bytes,
                ));
            }
        }
        group
    }

    fn clear_confirmation(
        &self,
        sender: &ComponentSender<Self>,
        heading: Option<&str>,
        bytes: u64,
    ) -> gtk::Box {
        let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if let Some(heading) = heading {
            confirmation.append(&gtk::Label::new(Some(&self.text(heading))));
        }
        let body = gtk::Label::new(Some(&self.text_arg(
            "feature-settings-settings_clear_cache_confirm",
            &format_bytes(bytes),
        )));
        body.set_wrap(true);
        confirmation.append(&body);
        let cancel = gtk::Button::with_label(&self.text("desktop-cancel"));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(SettingsInput::CancelClear));
        confirmation.append(&cancel);
        let confirm =
            gtk::Button::with_label(&self.text("feature-settings-settings_clear_cache_action"));
        confirm.add_css_class("destructive-action");
        confirm.set_sensitive(!self.media_loading);
        let input = sender.input_sender().clone();
        confirm.connect_clicked(move |_| input.emit(SettingsInput::ConfirmClear));
        confirmation.append(&confirm);
        confirmation
    }

    fn download_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_autodownload"))
            .visible(self.media_settings.is_some())
            .build();
        let Some(settings) = self.media_settings else {
            return group;
        };
        let network_names = gtk::StringList::new(&[
            &self.text("feature-settings-settings_autodownload_wifi"),
            &self.text("feature-settings-settings_autodownload_mobile"),
            &self.text("feature-settings-settings_autodownload_roaming"),
        ]);
        let selected = NETWORK_CLASSES
            .iter()
            .position(|network| *network == self.selected_network)
            .unwrap_or(0) as u32;
        let networks = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_autodownload"))
            .model(&network_names)
            .selected(selected)
            .sensitive(!self.media_loading)
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        networks.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(SettingsInput::SelectNetwork(row.selected()));
            }
        });
        group.add(&networks);

        let policy = settings.auto_download.policy(self.selected_network);
        for (index, category) in DOWNLOAD_CATEGORIES.into_iter().enumerate() {
            let Some(rule) = policy.rule(category) else {
                continue;
            };
            let title = match category {
                MediaCategory::Photo => "feature-settings-settings_autodownload_photos",
                MediaCategory::Video => "feature-settings-settings_autodownload_videos",
                MediaCategory::Document => "feature-settings-settings_autodownload_files",
                MediaCategory::Avatar | MediaCategory::Other => unreachable!(),
            };
            let toggle = adw::SwitchRow::builder()
                .title(self.text(title))
                .active(rule.enabled)
                .sensitive(!self.media_loading)
                .build();
            let input = sender.input_sender().clone();
            let rendering = self.rendering.clone();
            toggle.connect_active_notify(move |_| {
                if !rendering.get() {
                    input.emit(SettingsInput::DownloadEnabled(index, !rule.enabled));
                }
            });
            group.add(&toggle);

            if rule.enabled {
                let steps = size_steps(rule.max_bytes);
                let labels = steps
                    .iter()
                    .map(|bytes| format_bytes(*bytes))
                    .collect::<Vec<_>>();
                let list =
                    gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
                let selected = steps
                    .iter()
                    .position(|bytes| *bytes == rule.max_bytes)
                    .unwrap_or(0) as u32;
                let limit = adw::ComboRow::builder()
                    .title(self.text(title))
                    .subtitle(self.text_arg(
                        "feature-settings-settings_autodownload_limit",
                        &format_bytes(rule.max_bytes),
                    ))
                    .model(&list)
                    .selected(selected)
                    .sensitive(!self.media_loading)
                    .build();
                let input = sender.input_sender().clone();
                let rendering = self.rendering.clone();
                limit.connect_selected_notify(move |row| {
                    if !rendering.get() {
                        input.emit(SettingsInput::DownloadLimit(index, row.selected()));
                    }
                });
                group.add(&limit);
            }
        }
        for (title, subtitle, active, message) in [
            (
                "feature-settings-settings_autoplay",
                "feature-settings-settings_autoplay_videos_sub",
                settings.autoplay.allows(self.selected_network),
                SettingsInput::Autoplay as fn(bool) -> SettingsInput,
            ),
            (
                "feature-settings-settings_autodownload_preload_video",
                "feature-settings-settings_autodownload_preload_video_sub",
                settings.preload.allows(self.selected_network),
                SettingsInput::Preload as fn(bool) -> SettingsInput,
            ),
        ] {
            let toggle = adw::SwitchRow::builder()
                .title(self.text(title))
                .subtitle(self.text(subtitle))
                .active(active)
                .sensitive(!self.media_loading)
                .build();
            let input = sender.input_sender().clone();
            let rendering = self.rendering.clone();
            toggle.connect_active_notify(move |_| {
                if !rendering.get() {
                    input.emit(message(!active));
                }
            });
            group.add(&toggle);
        }
        group
    }

    fn appearance_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-appearance"))
            .build();
        let locale = adw::ComboRow::builder()
            .title(self.text("feature-dialog-dialog_editor_language"))
            .model(&gtk::StringList::new(&SUPPORTED_LOCALES))
            .selected(self.locale)
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        locale.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(SettingsInput::Locale(row.selected()));
            }
        });
        group.add(&locale);
        group
    }

    fn logout_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        group.set_visible(self.logout_enabled);
        let logout = gtk::Button::with_label(&self.text("desktop-logout"));
        logout.add_css_class("destructive-action");
        logout.set_visible(!self.confirming_logout);
        let input = sender.input_sender().clone();
        logout.connect_clicked(move |_| input.emit(SettingsInput::RequestLogout));
        group.add(&logout);
        if self.confirming_logout {
            let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 12);
            confirmation.append(&gtk::Label::new(Some(&self.text("desktop-logout-confirm"))));
            let cancel = gtk::Button::with_label(&self.text("desktop-cancel"));
            let input = sender.input_sender().clone();
            cancel.connect_clicked(move |_| input.emit(SettingsInput::CancelLogout));
            confirmation.append(&cancel);
            let confirm = gtk::Button::with_label(&self.text("desktop-logout"));
            confirm.add_css_class("destructive-action");
            let input = sender.input_sender().clone();
            confirm.connect_clicked(move |_| input.emit(SettingsInput::ConfirmLogout));
            confirmation.append(&confirm);
            group.add(&confirmation);
        }
        group
    }
}

fn proxy_save_command(submission: ProxySubmission) -> Result<WorkerCommand, &'static str> {
    if submission.host.trim().is_empty() {
        return Err("feature-settings-settings_proxy_host_required");
    }
    if submission.port == 0 {
        return Err("feature-settings-settings_proxy_port_invalid");
    }
    let id =
        u64::try_from(submission.id).map_err(|_| "feature-settings-settings_proxy_test_failed")?;
    let kind = match submission.kind {
        UiProxyKind::Socks5 => mono_proto_client::adapter::socket::ProxyKind::Socks5,
        UiProxyKind::Http => mono_proto_client::adapter::socket::ProxyKind::Http,
        UiProxyKind::Https => mono_proto_client::adapter::socket::ProxyKind::Https,
        UiProxyKind::Mtproto => mono_proto_client::adapter::socket::ProxyKind::Mtproto,
    };
    let mut password = None;
    let mut mtproto_secret = None;
    let mut fake_tls_domain = None;
    if kind == mono_proto_client::adapter::socket::ProxyKind::Mtproto {
        let encoded = submission
            .secret
            .ok_or("feature-settings-settings_proxy_secret_invalid")?;
        let bytes = decode_hex(&encoded).ok_or("feature-settings-settings_proxy_secret_invalid")?;
        let (secret, domain) = ProxyConfig::decode_mtproto_secret(&bytes)
            .map_err(|_| "feature-settings-settings_proxy_secret_invalid")?
            .ok_or("feature-settings-settings_proxy_secret_invalid")?;
        mtproto_secret = Some(Zeroizing::new(secret));
        fake_tls_domain = domain;
    } else {
        password = submission.password.map(Zeroizing::new);
    }
    let spec = ProxySpec {
        kind,
        host: submission.host,
        port: submission.port,
        username: (!submission.username.is_empty()).then_some(submission.username),
        password,
        mtproto_secret,
        fake_tls_domain,
    };
    Ok(if id == 0 {
        WorkerCommand::Connectivity(ConnectivityCommand::AddProxy { spec })
    } else {
        WorkerCommand::Connectivity(ConnectivityCommand::EditProxy { id, spec })
    })
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some((high * 16 + low) as u8)
        })
        .collect()
}

fn locale_index(i18n: &I18n) -> u32 {
    SUPPORTED_LOCALES
        .iter()
        .position(|locale| *locale == i18n.locale())
        .unwrap_or(0) as u32
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    let Some((divisor, suffix)) = [(TIB, "TB"), (GIB, "GB"), (MIB, "MB"), (KIB, "KB")]
        .into_iter()
        .find(|(divisor, _)| bytes >= *divisor)
    else {
        return format!("{bytes} B");
    };
    if bytes.is_multiple_of(divisor) {
        format!("{} {suffix}", bytes / divisor)
    } else {
        format!("{:.1} {suffix}", bytes as f64 / divisor as f64)
    }
}

impl SimpleComponent for Settings {
    type Init = Rc<I18n>;
    type Input = SettingsInput;
    type Output = SettingsOutput;
    type Root = gtk::Box;
    type Widgets = gtk::Box;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let connectivity = ConnectivityPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), SettingsInput::ConnectivityOutput);
        let appearance = Appearance::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), SettingsInput::AppearanceOutput);
        let maintenance = Maintenance::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), SettingsInput::MaintenanceOutput);
        if let Some(path) = local_notification_policy_path() {
            connectivity.emit(ConnectivityInput::SetLocalPolicyPath(path));
        }
        let mut model = Self::new(i18n);
        model.connectivity = Some(connectivity);
        model.appearance = Some(appearance);
        model.maintenance = Some(maintenance);
        model.render(&root, &sender);
        sender.input(SettingsInput::RefreshPreferences);
        ComponentParts {
            model,
            widgets: root,
        }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        for output in self.handle(message) {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, root: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.render(root, &sender);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/settings_ui.rs"]
mod tests;
