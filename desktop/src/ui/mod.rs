use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use mono_proto_client::{ChatDto, MessageDto, OutgoingMessageResult, ProfileDto, UpdateEventDto};
use relm4::{adw, adw::prelude::*, gtk, gtk::glib, prelude::*};

use crate::{
    client::{Worker, WorkerCommand, WorkerEvent, WorkerResponse},
    domain::{
        ApiConfig, AuthState, ClientError, ClientState, ConnectionState, NetworkClass, PendingSend,
        StartupState, TextSend,
    },
    i18n::I18n,
    platform::{
        DeepLink, GeoClueSampler, LaunchAllowlist, Location, NotificationAction,
        NotificationAdapter, NotificationCapabilities, NotificationContent, NotificationLabels,
        NotificationTarget, store_api_credentials,
    },
    ui::connectivity::{LocalNotificationPolicy, local_notification_policy_path},
};

pub mod appearance;
pub mod auth;
pub mod chats;
pub mod connectivity;
pub mod dialog;
pub mod folders;
pub mod interactions;
pub mod maintenance;
pub mod markup;
pub mod media;
pub mod passkeys;
pub mod profile;
pub mod settings;
pub mod sticker;
pub mod web;

use appearance::WallpaperConfig;
use auth::{AuthInput, AuthOutput, AuthPage};
use chats::{ChatsInput, ChatsOutput, ChatsPage, SearchSection};
use connectivity::{ConnectivityInput, NotificationException, ProxyProbe, ProxyRecord};
use dialog::{DialogInput, DialogOutput, DialogPage};
use folders::{FoldersInput, FoldersOutput, FoldersPage};
use maintenance::MaintenanceInput;
use passkeys::{PasskeyInput, PasskeyOutput, PasskeyPage, PasskeyRecord};
use profile::{Profile, ProfileInput, ProfileOutput};
use settings::{Settings, SettingsInput, SettingsOutput};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Chats,
    Dialog,
    Settings,
    Passkeys,
    Profile,
    Folders,
}

enum RequestKind {
    Configure,
    Connect,
    Authenticate,
    ExportQr,
    PollQr,
    Chats {
        cached: bool,
    },
    History {
        chat_id: i64,
        append: bool,
        cached: bool,
    },
    Pending,
    Send {
        message: TextSend,
        random_id: Option<i64>,
    },
    Retry {
        chat_id: i64,
        random_id: i64,
    },
    Mutation {
        chat_id: i64,
        deleted: Option<i32>,
    },
    ClearActive,
    Logout,
    Extra(ExtraRequest),
}

#[derive(Clone)]
pub struct ForwardIntent {
    from_chat_id: i64,
    message_ids: Vec<i32>,
    to_chat_id: i64,
    drop_author: bool,
}

enum PeerIntent {
    OpenChat,
    OpenMessage(i32),
    Recipient,
    Forward {
        from_chat_id: i64,
        message_ids: Vec<i32>,
        drop_author: bool,
    },
}

enum ExtraRequest {
    Folders {
        cached: bool,
    },
    FolderMutation,
    Search {
        generation: u64,
        epoch: u64,
        section: SearchSection,
    },
    Resolve {
        chat_id: i64,
        navigation: u64,
        intent: PeerIntent,
    },
    Profile {
        chat_id: i64,
        request_id: u64,
        epoch: u64,
        section: ProfileRequest,
    },
    Dialog {
        token: u64,
        epoch: u64,
        transfer: bool,
    },
    MediaCancel,
    DraftRefresh,
    Connectivity,
    Appearance,
    Maintenance,
    Passkey,
    Preference(crate::domain::PreferenceNamespace),
    GeoUpdate,
    DeepLink {
        message_id: Option<i32>,
    },
    Settings(SettingsRequest),
}

#[derive(Clone, Copy)]
enum SettingsRequest {
    Usage,
    Settings,
}

enum ProfileRequest {
    Info,
    Members,
    CommonChats,
    Media,
}

impl RequestKind {
    fn auth(&self) -> bool {
        matches!(
            self,
            Self::Configure | Self::Connect | Self::Authenticate | Self::ExportQr | Self::PollQr
        )
    }

    fn chat_id(&self) -> Option<i64> {
        match self {
            Self::History { chat_id, .. }
            | Self::Retry { chat_id, .. }
            | Self::Mutation { chat_id, .. } => Some(*chat_id),
            Self::Send { message, .. } => Some(message.chat_id),
            Self::Extra(
                ExtraRequest::Resolve { chat_id, .. } | ExtraRequest::Profile { chat_id, .. },
            ) => Some(*chat_id),
            Self::Extra(
                ExtraRequest::Folders { .. }
                | ExtraRequest::FolderMutation
                | ExtraRequest::Search { .. }
                | ExtraRequest::Dialog { .. }
                | ExtraRequest::MediaCancel
                | ExtraRequest::DraftRefresh
                | ExtraRequest::Connectivity
                | ExtraRequest::Appearance
                | ExtraRequest::Maintenance
                | ExtraRequest::Passkey
                | ExtraRequest::Preference(_)
                | ExtraRequest::GeoUpdate
                | ExtraRequest::DeepLink { .. }
                | ExtraRequest::Settings(_),
            ) => None,
            Self::Configure
            | Self::Connect
            | Self::Authenticate
            | Self::ExportQr
            | Self::PollQr
            | Self::Chats { .. }
            | Self::Pending
            | Self::ClearActive
            | Self::Logout => None,
        }
    }
}

enum SecretEvent {
    CredentialsStored {
        config: ApiConfig,
        result: Result<(), ClientError>,
    },
}

pub enum AppMessage {
    Worker(Box<WorkerEvent>),
    ReceiverClosed,
    Network(NetworkClass),
    Location(Location),
    DeepLink(String),
    Notification(NotificationAction),
    NotificationsReady(NotificationAdapter),
    CredentialsStored {
        config: ApiConfig,
        result: Result<(), ClientError>,
    },
    Auth(AuthOutput),
    Chats(ChatsOutput),
    Dialog(DialogOutput),
    Settings(SettingsOutput),
    Passkey(PasskeyOutput),
    Profile(ProfileOutput),
    Folders(FoldersOutput),
    ForwardChoice {
        intent: ForwardIntent,
        search: bool,
    },
    OpenSettings,
    OpenPasskeys,
    RetryStartup,
}

impl fmt::Debug for AppMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AppMessage(..)")
    }
}

struct Shell {
    root: adw::ToolbarView,
    pages: gtk::Stack,
    split: adw::NavigationSplitView,
    content: gtk::Stack,
    status: gtk::Label,
    startup: adw::StatusPage,
    startup_spinner: gtk::Spinner,
    auth: Controller<AuthPage>,
    chats: Controller<ChatsPage>,
    dialog: Controller<DialogPage>,
    settings: Controller<Settings>,
    passkeys: Controller<PasskeyPage>,
    profile: Controller<Profile>,
    folders: Controller<FoldersPage>,
}

impl Shell {
    fn new(i18n: Rc<I18n>, sender: &ComponentSender<App>) -> Self {
        let tr = |key: &str| i18n.text(key, &[]).unwrap_or_default();
        let auth = AuthPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Auth);
        let chats = ChatsPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Chats);
        let dialog = DialogPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Dialog);
        let settings = Settings::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Settings);
        let passkeys = PasskeyPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Passkey);
        let profile = Profile::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Profile);
        let root = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(&tr("app-app_name")))));
        let settings_button = gtk::Button::with_label(&tr("desktop-settings"));
        let input = sender.input_sender().clone();
        settings_button.connect_clicked(move |_| input.emit(AppMessage::OpenSettings));
        header.pack_end(&settings_button);
        let passkeys_button = gtk::Button::with_label(&tr("feature-settings-settings_passkeys"));
        let input = sender.input_sender().clone();
        passkeys_button.connect_clicked(move |_| input.emit(AppMessage::OpenPasskeys));
        header.pack_end(&passkeys_button);
        root.add_top_bar(&header);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_visible(false);
        status.add_css_class("error");
        body.append(&status);
        let pages = gtk::Stack::new();
        pages.set_vexpand(true);
        pages.set_hexpand(true);
        body.append(&pages);
        root.set_content(Some(&body));
        let startup = adw::StatusPage::builder()
            .title(tr("desktop-loading"))
            .build();
        let progress = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let startup_spinner = gtk::Spinner::builder()
            .spinning(true)
            .halign(gtk::Align::Center)
            .build();
        progress.append(&startup_spinner);
        let retry = gtk::Button::with_label(&tr("desktop-retry"));
        let input = sender.input_sender().clone();
        retry.connect_clicked(move |_| input.emit(AppMessage::RetryStartup));
        progress.append(&retry);
        startup.set_child(Some(&progress));
        pages.add_named(&startup, Some("startup"));
        pages.add_named(auth.widget(), Some("auth"));
        pages.add_named(settings.widget(), Some("settings"));
        pages.add_named(passkeys.widget(), Some("passkeys"));
        pages.add_named(profile.widget(), Some("profile"));
        let content = gtk::Stack::new();
        let empty = adw::StatusPage::builder()
            .title(tr("app-select_chat"))
            .icon_name("chat-symbolic")
            .build();
        content.add_named(&empty, Some("empty"));
        content.add_named(dialog.widget(), Some("dialog"));
        let sidebar = adw::NavigationPage::new(chats.widget(), &tr("desktop-chats"));
        let detail = adw::NavigationPage::new(&content, &tr("desktop-message"));
        let split = adw::NavigationSplitView::new();
        split.set_sidebar(Some(&sidebar));
        split.set_content(Some(&detail));
        split.set_min_sidebar_width(260.0);
        split.set_max_sidebar_width(360.0);
        let adaptive = adw::BreakpointBin::new();
        adaptive.set_size_request(320, 360);
        adaptive.set_child(Some(&split));
        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            720.0,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        adaptive.add_breakpoint(breakpoint);
        pages.add_named(&adaptive, Some("main"));
        let folders = FoldersPage::builder()
            .launch(i18n.clone())
            .forward(sender.input_sender(), AppMessage::Folders);
        pages.add_named(folders.widget(), Some("folders"));
        Self {
            root,
            pages,
            split,
            content,
            status,
            startup,
            startup_spinner,
            auth,
            chats,
            dialog,
            settings,
            passkeys,
            profile,
            folders,
        }
    }

    fn update_chat(&self, selected: Option<i64>, chat: ChatDto) {
        if selected == Some(chat.id) {
            self.dialog.emit(DialogInput::SetChat(chat.clone()));
        }
        self.chats.emit(ChatsInput::UpdateChat(chat));
    }
}

#[derive(Default)]
struct ExtendedState {
    folders: Vec<mono_proto_client::FolderDto>,
    profiles: BTreeMap<i64, ProfileDto>,
    pending_forward: Option<ForwardIntent>,
    conservative_chats: BTreeSet<i64>,
    search_epoch: u64,
    search_generation: Option<u64>,
    navigation: u64,
}

pub struct App {
    root: adw::ApplicationWindow,
    shell: Shell,
    i18n: Rc<I18n>,
    startup: StartupState,
    state: ClientState,
    screen: Screen,
    return_screen: Screen,
    worker: Option<Worker>,
    event_source: Option<glib::SourceId>,
    network_subscription: Option<crate::network::NetworkSubscription>,
    notifications: Option<NotificationAdapter>,
    notification_policy: LocalNotificationPolicy,
    wallpaper: WallpaperConfig,
    secret_sender: std::sync::mpsc::Sender<SecretEvent>,
    secret_event_source: Option<glib::SourceId>,
    geoclue: GeoClueSampler,
    credentials_busy: bool,
    next_request: u64,
    requests: BTreeMap<u64, RequestKind>,
    chats: BTreeMap<i64, ChatDto>,
    histories: BTreeMap<i64, BTreeMap<i32, MessageDto>>,
    live: BTreeSet<(i64, i32)>,
    deleted: BTreeSet<(i64, i32)>,
    selected: Option<i64>,
    profile_id: Option<i64>,
    extended: ExtendedState,
    pending_sends: BTreeMap<i64, PendingSend>,
    error: Option<ClientError>,
    unrepresented_updates: u64,
}

impl SimpleComponent for App {
    type Init = ();
    type Input = AppMessage;
    type Output = ();
    type Root = adw::ApplicationWindow;
    type Widgets = ();

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .default_width(1040)
            .default_height(760)
            .build()
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let locale = std::env::var("LC_ALL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                std::env::var("LC_MESSAGES")
                    .ok()
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| std::env::var("LANG").ok())
            .unwrap_or_else(|| "en".into());
        let i18n = Rc::new(I18n::new(&locale).expect("valid embedded localization resources"));
        let shell = Shell::new(i18n.clone(), &sender);
        root.set_content(Some(&shell.root));
        let (secret_sender, secret_events) = std::sync::mpsc::channel();
        let secret_input = sender.input_sender().clone();
        let secret_event_source = glib::timeout_add_local(Duration::from_millis(50), move || {
            for _ in 0..32 {
                let Ok(SecretEvent::CredentialsStored { config, result }) =
                    secret_events.try_recv()
                else {
                    break;
                };
                secret_input.emit(AppMessage::CredentialsStored { config, result });
            }
            glib::ControlFlow::Continue
        });
        let location_input = sender.input_sender().clone();
        let geoclue =
            GeoClueSampler::new(glib::MainContext::ref_thread_default(), move |location| {
                location_input.emit(AppMessage::Location(location))
            });
        let mut model = Self {
            root,
            shell,
            i18n,
            startup: StartupState::Starting,
            state: ClientState::default(),
            screen: Screen::Chats,
            return_screen: Screen::Chats,
            worker: None,
            event_source: None,
            network_subscription: None,
            notifications: None,
            notification_policy: local_notification_policy_path()
                .as_ref()
                .map(|path| LocalNotificationPolicy::load(path))
                .unwrap_or_default(),
            wallpaper: WallpaperConfig::default(),
            secret_sender,
            secret_event_source: Some(secret_event_source),
            geoclue,
            credentials_busy: false,
            next_request: 1,
            requests: BTreeMap::new(),
            chats: BTreeMap::new(),
            histories: BTreeMap::new(),
            live: BTreeSet::new(),
            deleted: BTreeSet::new(),
            selected: None,
            profile_id: None,
            extended: ExtendedState::default(),
            pending_sends: BTreeMap::new(),
            error: None,
            unrepresented_updates: 0,
        };
        model.start_worker(&sender);
        model.start_uri_handlers(&sender);
        model.start_notifications(&sender);
        model.geoclue.start();
        model.geoclue.pause();
        model.refresh();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        let was_authorized = self.authorized();
        match message {
            AppMessage::Worker(event) => self.worker_event(*event),
            AppMessage::ReceiverClosed => {
                if self.startup != StartupState::Failed
                    && self.state.connection != ConnectionState::Stopped
                {
                    self.worker_event(WorkerEvent::Stopped);
                }
            }
            AppMessage::Network(network) => {
                self.settings_command(WorkerCommand::SetNetworkClass(network));
            }
            AppMessage::Location(location) => self.location_update(location),
            AppMessage::DeepLink(uri) => self.open_uri(&uri),
            AppMessage::Notification(action) => self.notification_action(action),
            AppMessage::NotificationsReady(adapter) => self.notifications = Some(adapter),
            AppMessage::CredentialsStored { config, result } => {
                self.credentials_busy = false;
                match result {
                    Ok(()) => self.submit(WorkerCommand::Configure(config), RequestKind::Configure),
                    Err(error) => self.failed(RequestKind::Configure, error),
                }
            }
            AppMessage::Auth(output) => self.auth_output(output),
            AppMessage::Chats(output) => self.chats_output(output),
            AppMessage::Dialog(DialogOutput::Forward {
                from_chat_id,
                message_ids,
                to_chat_id,
                drop_author,
            }) => {
                self.confirm_forward(
                    ForwardIntent {
                        from_chat_id,
                        message_ids,
                        to_chat_id,
                        drop_author,
                    },
                    &sender,
                );
            }
            AppMessage::Dialog(output) => self.dialog_output(output),
            AppMessage::ForwardChoice { intent, search } => {
                if !self.authorized() {
                    self.error = Some(ClientError::Unauthorized);
                } else if search {
                    self.extended.pending_forward = Some(intent);
                    self.shell.chats.emit(ChatsInput::SetRecipientMode(true));
                    self.screen = Screen::Chats;
                } else {
                    self.resolve_peer(
                        intent.to_chat_id,
                        PeerIntent::Forward {
                            from_chat_id: intent.from_chat_id,
                            message_ids: intent.message_ids,
                            drop_author: intent.drop_author,
                        },
                    );
                }
            }
            AppMessage::Settings(output) => match output {
                SettingsOutput::Back => {
                    self.screen = self.return_screen;
                    if let Some(path) = local_notification_policy_path() {
                        self.notification_policy = LocalNotificationPolicy::load(&path);
                    }
                }
                SettingsOutput::ChangeTheme(theme) => adw::StyleManager::default()
                    .set_color_scheme(match theme {
                        settings::ThemePreference::System => adw::ColorScheme::Default,
                        settings::ThemePreference::Light => adw::ColorScheme::ForceLight,
                        settings::ThemePreference::Dark => adw::ColorScheme::ForceDark,
                    }),
                SettingsOutput::ChangeLocale(locale) => self.change_locale(&locale, &sender),
                SettingsOutput::Command(command) => self.settings_command(command),
                SettingsOutput::Wallpaper(config) => {
                    self.wallpaper = config.clone();
                    self.shell.dialog.emit(DialogInput::SetWallpaper(config));
                }
                SettingsOutput::Logout => {
                    self.stop_dialog_media();
                    self.shell
                        .settings
                        .emit(SettingsInput::SetLogoutEnabled(false));
                    self.submit(WorkerCommand::Logout, RequestKind::Logout);
                }
            },
            AppMessage::Passkey(output) => match output {
                PasskeyOutput::Back => self.screen = self.return_screen,
                PasskeyOutput::Refresh => self.submit(
                    WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::List),
                    RequestKind::Extra(ExtraRequest::Passkey),
                ),
                PasskeyOutput::Create => self.submit(
                    WorkerCommand::Passkey(
                        crate::client::passkeys::PasskeyCommand::InitRegistration,
                    ),
                    RequestKind::Extra(ExtraRequest::Passkey),
                ),
                PasskeyOutput::Delete(id) => self.submit(
                    WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::Delete { id }),
                    RequestKind::Extra(ExtraRequest::Passkey),
                ),
            },
            AppMessage::Profile(output) => self.profile_output(output),
            AppMessage::Folders(output) => self.folders_output(output),
            AppMessage::OpenSettings => self.open_settings(),
            AppMessage::OpenPasskeys => self.open_passkeys(),
            AppMessage::RetryStartup => {
                if self.startup == StartupState::Failed
                    || self.state.connection == ConnectionState::Stopped
                {
                    self.stop_worker();
                    self.start_worker(&sender);
                }
            }
        }
        if was_authorized && !self.authorized() {
            self.clear_account_view();
            self.rebuild_ui(&sender);
        }
        self.refresh();
    }

    fn shutdown(&mut self, _: &mut Self::Widgets, _: relm4::Sender<Self::Output>) {
        self.geoclue.stop();
        self.stop_dialog_media();
        self.stop_worker();
    }
}

impl App {
    fn start_uri_handlers(&mut self, sender: &ComponentSender<Self>) {
        let Some(application) = self.root.application() else {
            return;
        };
        let action = gtk::gio::SimpleAction::new("open-uri", Some(glib::VariantTy::STRING));
        let input = sender.input_sender().clone();
        action.connect_activate(move |_, parameter| {
            if let Some(uri) = parameter.and_then(|value| value.str()) {
                input.emit(AppMessage::DeepLink(uri.to_owned()));
            }
        });
        application.add_action(&action);
    }

    fn open_uri(&mut self, uri: &str) {
        if let Some(link) = crate::platform::parse(uri) {
            match link {
                DeepLink::Username {
                    username,
                    message_id,
                    start: _,
                } => {
                    let message_id = message_id
                        .and_then(|id| i32::try_from(id).ok())
                        .filter(|id| *id > 0);
                    self.submit(
                        WorkerCommand::Interaction(
                            crate::client::interactions::InteractionCommand::ResolveInlineBot {
                                username,
                            },
                        ),
                        RequestKind::Extra(ExtraRequest::DeepLink { message_id }),
                    );
                }
                DeepLink::PrivateChannel { message_id, .. } => {
                    let Some(chat_id) = link.chat_id() else {
                        self.error = Some(ClientError::InvalidRequest);
                        return;
                    };
                    let Some(message_id) = i32::try_from(message_id).ok().filter(|id| *id > 0)
                    else {
                        self.error = Some(ClientError::InvalidRequest);
                        return;
                    };
                    self.resolve_peer(chat_id, PeerIntent::OpenMessage(message_id));
                }
                DeepLink::Invite { .. } | DeepLink::Share { .. } => {
                    self.error = Some(ClientError::UnsupportedContent);
                }
            }
            return;
        }
        let allowed = LaunchAllowlist::new()
            .allow_scheme("https")
            .allow_scheme("http")
            .allow_scheme("tg")
            .allow_scheme("mailto")
            .allow_scheme("tel")
            .allow_scheme("geo");
        if !(uri.starts_with("https://") || uri.starts_with("http://") || allowed.can_launch(uri)) {
            self.error = Some(ClientError::InvalidRequest);
            return;
        }
        gtk::UriLauncher::new(uri).launch(Some(&self.root), gtk::gio::Cancellable::NONE, |_| {});
    }

    fn start_notifications(&mut self, sender: &ComponentSender<Self>) {
        let Some(application) = self.root.application() else {
            return;
        };
        let input = sender.input_sender().clone();
        glib::MainContext::ref_thread_default().spawn_local(async move {
            let Ok(capabilities) = NotificationCapabilities::detect().await else {
                return;
            };
            let actions = input.clone();
            let adapter = NotificationAdapter::new(&application, capabilities, move |action| {
                actions.emit(AppMessage::Notification(action));
            });
            input.emit(AppMessage::NotificationsReady(adapter));
        });
    }

    fn notification_action(&mut self, action: NotificationAction) {
        match action {
            NotificationAction::OpenChat(target) => {
                self.resolve_peer(target.chat_id, PeerIntent::OpenChat);
            }
            NotificationAction::OpenMessage(target) => {
                let message_id = target
                    .message_id
                    .and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id > 0);
                if let Some(message_id) = message_id {
                    self.resolve_peer(target.chat_id, PeerIntent::OpenMessage(message_id));
                } else {
                    self.error = Some(ClientError::InvalidRequest);
                }
            }
            NotificationAction::MarkRead(target) => {
                let max_id = target
                    .message_id
                    .and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id > 0)
                    .or_else(|| {
                        self.chats
                            .get(&target.chat_id)
                            .map(|chat| chat.last_message_id)
                            .filter(|id| *id > 0)
                    });
                if let Some(max_id) = max_id {
                    self.submit(
                        WorkerCommand::ReadHistory {
                            chat_id: target.chat_id,
                            max_id,
                        },
                        RequestKind::Mutation {
                            chat_id: target.chat_id,
                            deleted: None,
                        },
                    );
                } else {
                    self.error = Some(ClientError::InvalidRequest);
                }
            }
            NotificationAction::Mute(target) => self.submit(
                WorkerCommand::Connectivity(
                    crate::client::connectivity::ConnectivityCommand::SetPeerNotify {
                        chat_id: target.chat_id,
                        settings: crate::client::connectivity::NotifyUpdate {
                            silent: true,
                            ..crate::client::connectivity::NotifyUpdate::reset()
                        },
                    },
                ),
                RequestKind::Extra(ExtraRequest::Connectivity),
            ),
            NotificationAction::Settings => self.open_settings(),
        }
    }

    fn secret_task(&mut self, task: impl FnOnce() -> SecretEvent + Send + 'static) {
        let input = self.secret_sender.clone();
        let result = std::thread::Builder::new()
            .name("monogram-secrets".into())
            .spawn(move || {
                let _ = input.send(task());
            });
        if result.is_err() {
            self.credentials_busy = false;
            self.error = Some(ClientError::SessionKey);
        }
    }

    fn start_worker(&mut self, sender: &ComponentSender<Self>) {
        self.startup = StartupState::Starting;
        self.state = ClientState::default();
        self.requests.clear();
        match Worker::start() {
            Ok((worker, events)) => {
                let input = sender.input_sender().clone();
                let mut disconnected = false;
                self.event_source = Some(glib::timeout_add_local(
                    Duration::from_millis(50),
                    move || {
                        for _ in 0..256 {
                            match events.try_recv() {
                                Ok(event) => input.emit(AppMessage::Worker(Box::new(event))),
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    if !disconnected {
                                        input.emit(AppMessage::ReceiverClosed);
                                        disconnected = true;
                                    }
                                    break;
                                }
                            }
                        }
                        glib::ControlFlow::Continue
                    },
                ));
                self.worker = Some(worker);
            }
            Err(_) => {
                self.startup = StartupState::Failed;
                self.error = Some(ClientError::Storage);
            }
        }
        if self.worker.is_some() {
            let input = sender.input_sender().clone();
            self.network_subscription = Some(
                crate::network::NetworkAdapter::new()
                    .subscribe(move |network| input.emit(AppMessage::Network(network))),
            );
            self.submit(WorkerCommand::ConfigureFromSecrets, RequestKind::Configure);
        }
    }

    fn stop_worker(&mut self) {
        self.geoclue.pause();
        self.network_subscription.take();
        if let Some(source) = self.event_source.take() {
            source.remove();
        }
        if let Some(worker) = self.worker.take()
            && let Err(error) = worker.shutdown()
        {
            self.error = Some(error);
        }
    }

    fn submit(&mut self, command: WorkerCommand, kind: RequestKind) {
        if self
            .requests
            .values()
            .any(|pending| matches!(pending, RequestKind::Logout))
        {
            self.failed(kind, ClientError::Cancelled);
            return;
        }
        let Some(next) = self.next_request.checked_add(1) else {
            self.failed(kind, ClientError::InvalidRequest);
            return;
        };
        let id = self.next_request;
        self.next_request = next;
        let result = self
            .worker
            .as_ref()
            .ok_or(ClientError::Stopped)
            .and_then(|worker| worker.submit(id, command));
        match result {
            Ok(()) => {
                self.requests.insert(id, kind);
                self.error = None;
            }
            Err(error) => self.failed(kind, error),
        }
    }

    fn authorized(&self) -> bool {
        matches!(self.state.auth, AuthState::Authorized { .. })
    }

    fn refresh(&self) {
        self.shell.dialog.emit(DialogInput::SetActive(
            self.authorized() && self.screen == Screen::Dialog && self.selected.is_some(),
        ));
        self.root.set_title(Some(&self.text("app-app_name")));
        let failed = self.startup == StartupState::Failed
            || self.state.connection == ConnectionState::Stopped;
        self.shell.startup.set_title(&self.text(if failed {
            "desktop-error-generic"
        } else {
            "desktop-loading"
        }));
        self.shell.startup_spinner.set_spinning(!failed);
        self.shell.startup_spinner.set_visible(!failed);
        let page = if self.screen == Screen::Settings {
            "settings"
        } else if self.screen == Screen::Passkeys {
            "passkeys"
        } else if self.startup != StartupState::Ready || failed {
            "startup"
        } else if !self.authorized() {
            "auth"
        } else if self.screen == Screen::Profile {
            "profile"
        } else if self.screen == Screen::Folders {
            "folders"
        } else {
            "main"
        };
        self.shell.pages.set_visible_child_name(page);
        self.shell
            .content
            .set_visible_child_name(if self.selected.is_some() {
                "dialog"
            } else {
                "empty"
            });
        self.shell
            .split
            .set_show_content(self.screen == Screen::Dialog && self.selected.is_some());
        let error = self.error.as_ref().map(|error| self.error_text(error));
        self.shell
            .status
            .set_label(error.as_deref().unwrap_or_default());
        self.shell.status.set_visible(error.is_some());
        let auth_busy = self.credentials_busy || self.requests.values().any(RequestKind::auth);
        self.shell.auth.emit(AuthInput::Busy(auth_busy));
        self.shell.dialog.emit(DialogInput::SetBusy(
            self.requests
                .values()
                .any(|request| request.chat_id() == self.selected && self.selected.is_some()),
        ));
        self.refresh_geoclue();
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_default()
    }

    fn error_text(&self, error: &ClientError) -> String {
        let key = match error {
            ClientError::Network => "core-ui-status_network",
            ClientError::Unauthorized => "core-ui-status_session",
            ClientError::Forbidden => "core-ui-status_privacy",
            ClientError::NotConfigured | ClientError::InvalidConfiguration => "desktop-api-help",
            ClientError::InvalidPhone => "feature-auth-auth_error_phone_required",
            ClientError::InvalidPassword => "feature-auth-auth_error_bad_password",
            ClientError::PasswordRequired => "feature-auth-auth_error_password_required",
            ClientError::RegistrationRequired => "feature-auth-auth_error_no_account",
            ClientError::InvalidCode => "feature-auth-auth_error_restart",
            ClientError::FloodWait { seconds } => {
                return self
                    .i18n
                    .text(
                        "core-ui-status_flood_wait",
                        &[("arg1", &seconds.to_string())],
                    )
                    .unwrap_or_default();
            }
            ClientError::RevisionConflict | ClientError::UnsupportedContent => {
                "desktop-error-generic"
            }
            ClientError::Cancelled
            | ClientError::InvalidRequest
            | ClientError::Storage
            | ClientError::SessionKey
            | ClientError::Session
            | ClientError::Stopped
            | ClientError::Protocol => "desktop-error-generic",
        };
        self.text(key)
    }

    fn auth_output(&mut self, output: AuthOutput) {
        self.shell.auth.emit(AuthInput::Error(None));
        let (command, kind) = match output {
            AuthOutput::Configure { api_id, api_hash } => match ApiConfig::new(api_id, api_hash) {
                Ok(config) => {
                    self.credentials_busy = true;
                    let api_id = config.api_id;
                    let api_hash = config.api_hash.to_string();
                    self.secret_task(move || {
                        let result = store_api_credentials(api_id, &api_hash)
                            .map_err(|_| ClientError::SessionKey);
                        SecretEvent::CredentialsStored { config, result }
                    });
                    return;
                }
                Err(error) => {
                    self.failed(RequestKind::Configure, error);
                    return;
                }
            },
            AuthOutput::Connect => (WorkerCommand::Connect, RequestKind::Connect),
            AuthOutput::SendCode(phone) => {
                (WorkerCommand::SendCode { phone }, RequestKind::Authenticate)
            }
            AuthOutput::ResendCode => (WorkerCommand::ResendCode, RequestKind::Authenticate),
            AuthOutput::SignIn(code) => (WorkerCommand::SignIn { code }, RequestKind::Authenticate),
            AuthOutput::CheckPassword(password) => (
                WorkerCommand::CheckPassword { password },
                RequestKind::Authenticate,
            ),
            AuthOutput::ExportQr => (WorkerCommand::ExportQr, RequestKind::ExportQr),
            AuthOutput::PollQr => (WorkerCommand::PollQr, RequestKind::PollQr),
        };
        self.submit(command, kind);
    }

    fn chats_output(&mut self, output: ChatsOutput) {
        match output {
            ChatsOutput::Refresh => self.load_chats(false),
            ChatsOutput::LoadMore {
                offset_date,
                offset_id,
                offset_peer_id,
                folder_id,
            } => {
                self.submit(
                    WorkerCommand::LoadMoreChats {
                        offset_date,
                        offset_id,
                        offset_peer_id,
                        folder_id,
                    },
                    RequestKind::Chats { cached: false },
                );
            }
            ChatsOutput::SearchContacts {
                generation,
                query,
                limit,
            } => {
                self.extended.search_generation = Some(generation);
                self.submit(
                    WorkerCommand::ContactsSearch { query, limit },
                    RequestKind::Extra(ExtraRequest::Search {
                        generation,
                        epoch: self.extended.search_epoch,
                        section: SearchSection::Contacts,
                    }),
                );
            }
            ChatsOutput::SearchMessages {
                generation,
                query,
                offset_rate,
                offset_peer_id,
                offset_id,
                limit,
                folder_id,
            } => {
                self.extended.search_generation = Some(generation);
                self.submit(
                    WorkerCommand::SearchGlobal {
                        query,
                        offset_rate,
                        offset_peer_id,
                        offset_id,
                        limit,
                        folder_id,
                    },
                    RequestKind::Extra(ExtraRequest::Search {
                        generation,
                        epoch: self.extended.search_epoch,
                        section: SearchSection::Messages,
                    }),
                );
            }
            ChatsOutput::CancelSearch { generation } => {
                if self.extended.search_generation == Some(generation) {
                    self.extended.search_generation = None;
                }
            }
            ChatsOutput::OpenMessage {
                chat_id,
                message_id,
            } => self.resolve_peer(chat_id, PeerIntent::OpenMessage(message_id)),
            ChatsOutput::SelectRecipient(id) => self.resolve_peer(id, PeerIntent::Recipient),
            ChatsOutput::OpenFolders => {
                self.screen = Screen::Folders;
                self.load_folders(true);
            }
            ChatsOutput::OpenChat(id) => self.resolve_peer(id, PeerIntent::OpenChat),
            ChatsOutput::OpenProfile(id) => self.open_profile(id),
            ChatsOutput::OpenSettings => self.open_settings(),
        }
    }

    fn folders_output(&mut self, output: FoldersOutput) {
        match output {
            FoldersOutput::Back => self.screen = Screen::Chats,
            FoldersOutput::Refresh => self.load_folders(false),
            FoldersOutput::Save(folder) => self.submit(
                WorkerCommand::SaveFolder(folder),
                RequestKind::Extra(ExtraRequest::FolderMutation),
            ),
            FoldersOutput::Delete(id) => self.submit(
                WorkerCommand::DeleteFolder { id },
                RequestKind::Extra(ExtraRequest::FolderMutation),
            ),
            FoldersOutput::Reorder(order) => self.submit(
                WorkerCommand::ReorderFolders { order },
                RequestKind::Extra(ExtraRequest::FolderMutation),
            ),
        }
    }

    fn profile_output(&mut self, output: ProfileOutput) {
        let (request_id, chat_id, section, command) = match output {
            ProfileOutput::Back => {
                self.screen = Screen::Chats;
                return;
            }
            ProfileOutput::OpenChat(id) => {
                self.resolve_peer(id, PeerIntent::OpenChat);
                return;
            }
            ProfileOutput::OpenMessage {
                chat_id,
                message_id,
            } => {
                self.resolve_peer(chat_id, PeerIntent::OpenMessage(message_id));
                return;
            }
            ProfileOutput::LoadProfile {
                request_id,
                chat_id,
            } => (
                request_id,
                chat_id,
                ProfileRequest::Info,
                WorkerCommand::GetProfile { peer_id: chat_id },
            ),
            ProfileOutput::LoadMembers {
                request_id,
                chat_id,
                offset,
                limit: _,
            } => (
                request_id,
                chat_id,
                ProfileRequest::Members,
                WorkerCommand::GetMembers {
                    chat_id,
                    filter: "recent".into(),
                    query: String::new(),
                    offset,
                },
            ),
            ProfileOutput::LoadCommonChats {
                request_id,
                user_id,
                max_id,
                limit,
            } => (
                request_id,
                user_id,
                ProfileRequest::CommonChats,
                WorkerCommand::GetCommonChats {
                    user_id,
                    max_id,
                    limit,
                },
            ),
            ProfileOutput::SearchMessagesPage {
                request_id,
                chat_id,
                filter,
                offset_id,
                limit,
            } => (
                request_id,
                chat_id,
                ProfileRequest::Media,
                WorkerCommand::SearchMessagesPage {
                    chat_id,
                    query: String::new(),
                    filter: filter.into(),
                    offset_id,
                    add_offset: 0,
                    limit,
                },
            ),
        };
        if self.profile_id != Some(chat_id) {
            return;
        }
        self.submit(
            command,
            RequestKind::Extra(ExtraRequest::Profile {
                chat_id,
                request_id,
                epoch: self.extended.search_epoch,
                section,
            }),
        );
    }

    fn open_profile(&mut self, id: i64) {
        self.extended.navigation = self.extended.navigation.saturating_add(1);
        self.profile_id = Some(id);
        self.return_screen = self.screen;
        self.screen = Screen::Profile;
        self.shell.profile.emit(ProfileInput::Open(id));
    }

    fn resolve_peer(&mut self, chat_id: i64, intent: PeerIntent) {
        let Some(navigation) = self.extended.navigation.checked_add(1) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        self.extended.navigation = navigation;
        if matches!(&intent, PeerIntent::OpenChat | PeerIntent::OpenMessage(_))
            && self.chats.contains_key(&chat_id)
        {
            match intent {
                PeerIntent::OpenChat => self.open_chat(chat_id),
                PeerIntent::OpenMessage(message_id) => {
                    self.open_chat(chat_id);
                    self.shell.dialog.emit(DialogInput::JumpTo(message_id));
                }
                PeerIntent::Recipient | PeerIntent::Forward { .. } => unreachable!(),
            }
            return;
        }
        self.submit(
            WorkerCommand::GetProfile { peer_id: chat_id },
            RequestKind::Extra(ExtraRequest::Resolve {
                chat_id,
                navigation,
                intent,
            }),
        );
    }

    fn resolved_profile(&mut self, profile: ProfileDto, intent: PeerIntent) {
        let chat_id = profile.id;
        let extra = profile
            .extra_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok());
        let blocked = extra
            .as_ref()
            .and_then(|json| json.get("blocked"))
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        let forwarding = matches!(&intent, PeerIntent::Recipient | PeerIntent::Forward { .. });
        let restricted = !self.extended.conservative_chats.contains(&chat_id)
            && self.chats.get(&chat_id).is_some_and(|chat| {
                !chat.can_view || chat.left || !chat.can_send_plain || !chat.can_forward
            });
        if chat_id == 0 || forwarding && (blocked || restricted) {
            self.error = Some(ClientError::Forbidden);
            self.shell
                .dialog
                .emit(DialogInput::SetError(Some(ClientError::Forbidden)));
            return;
        }
        self.extended.profiles.insert(chat_id, profile.clone());
        match intent {
            PeerIntent::Forward {
                from_chat_id,
                message_ids,
                drop_author,
            } => {
                self.submit(
                    WorkerCommand::Forward {
                        from_chat_id,
                        message_ids,
                        to_chat_id: chat_id,
                        drop_author,
                    },
                    RequestKind::Mutation {
                        chat_id,
                        deleted: None,
                    },
                );
            }
            PeerIntent::Recipient => {
                let Some(pending) = self.extended.pending_forward.take() else {
                    self.error = Some(ClientError::InvalidRequest);
                    return;
                };
                self.shell.chats.emit(ChatsInput::SetRecipientMode(false));
                self.screen = Screen::Dialog;
                self.submit(
                    WorkerCommand::Forward {
                        from_chat_id: pending.from_chat_id,
                        message_ids: pending.message_ids,
                        to_chat_id: chat_id,
                        drop_author: pending.drop_author,
                    },
                    RequestKind::Mutation {
                        chat_id,
                        deleted: None,
                    },
                );
            }
            PeerIntent::OpenChat | PeerIntent::OpenMessage(_) => {
                if !self.chats.contains_key(&chat_id) {
                    self.extended.conservative_chats.insert(chat_id);
                }
                let chat = self
                    .chats
                    .entry(chat_id)
                    .or_insert_with(|| ChatDto {
                        id: chat_id,
                        title: profile.title.clone(),
                        is_channel: profile.kind == "channel",
                        is_group: matches!(profile.kind.as_str(), "chat" | "group" | "supergroup"),
                        is_forum: false,
                        left: false,
                        unread_count: 0,
                        last_message_preview: None,
                        last_message_date: None,
                        archived: false,
                        muted: false,
                        is_contact: false,
                        is_bot: profile.kind == "bot",
                        is_verified: false,
                        photo_cache_key: profile.avatar_cache_key.clone(),
                        pinned: false,
                        read_inbox_max_id: 0,
                        read_outbox_max_id: 0,
                        peer_status: profile.status.clone(),
                        peer_status_at: profile.status_at,
                        last_media_thumb_cache_key: None,
                        last_message_id: 0,
                        can_view: true,
                        can_send_plain: false,
                        can_send_photos: false,
                        can_send_polls: false,
                        can_forward: false,
                        can_delete_others: false,
                        emoji_status_document_id: None,
                        last_message_outgoing: false,
                        mute_override: false,
                        unread_mark: false,
                        unread_mentions_count: 0,
                        unread_reactions_count: 0,
                        can_manage_topics: false,
                    })
                    .clone();
                self.shell.chats.emit(ChatsInput::UpdateChat(chat));
                self.open_chat(chat_id);
                if let PeerIntent::OpenMessage(message_id) = intent {
                    self.shell.dialog.emit(DialogInput::JumpTo(message_id));
                }
            }
        }
    }

    fn load_folders(&mut self, cached: bool) {
        if self.requests.values().any(|kind| matches!(kind, RequestKind::Extra(ExtraRequest::Folders { cached: pending }) if *pending == cached)) { return; }
        self.shell.folders.emit(FoldersInput::SetBusy(true));
        self.submit(
            if cached {
                WorkerCommand::GetCachedFolders
            } else {
                WorkerCommand::GetFolders
            },
            RequestKind::Extra(ExtraRequest::Folders { cached }),
        );
    }

    fn confirm_forward(&self, intent: ForwardIntent, sender: &ComponentSender<Self>) {
        let target = self
            .chats
            .get(&intent.to_chat_id)
            .map(|chat| chat.title.clone())
            .unwrap_or_else(|| intent.to_chat_id.to_string());
        let dialog = adw::AlertDialog::builder()
            .heading(self.text("feature-dialog-dialog_forward"))
            .body(format!(
                "{}: {target}",
                self.text("feature-dialog-dialog_forward_to")
            ))
            .build();
        dialog.add_response("cancel", &self.text("desktop-cancel"));
        dialog.add_response("send", &self.text("feature-dialog-dialog_forward"));
        dialog.add_response("search", &self.text("feature-dialog-dialog_forward_search"));
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let input = sender.input_sender().clone();
        dialog.connect_response(None, move |_, response| {
            if matches!(response, "send" | "search") {
                input.emit(AppMessage::ForwardChoice {
                    intent: intent.clone(),
                    search: response == "search",
                });
            }
        });
        dialog.present(Some(&self.root));
    }

    fn dialog_output(&mut self, output: DialogOutput) {
        match output {
            DialogOutput::Command { token, command } => {
                let transfer = matches!(
                    command,
                    WorkerCommand::DownloadMessageMedia { .. }
                        | WorkerCommand::DownloadPeerAvatar { .. }
                        | WorkerCommand::DownloadCustomEmoji { .. }
                        | WorkerCommand::DownloadMediaRange { .. }
                );
                self.submit(
                    command,
                    RequestKind::Extra(ExtraRequest::Dialog {
                        token,
                        epoch: self.extended.search_epoch,
                        transfer,
                    }),
                );
            }
            DialogOutput::Back => {
                self.stop_dialog_media();
                self.screen = Screen::Chats;
                self.clear_active();
            }
            DialogOutput::ClearActiveDialog => self.clear_active(),
            DialogOutput::LoadHistory {
                chat_id,
                limit,
                offset_id,
                offset_date,
                add_offset,
            } => {
                if !self.requests.values().any(|kind| matches!(kind, RequestKind::History { chat_id: id, cached: true, .. } if *id == chat_id)) && !self.histories.contains_key(&chat_id) {
                    self.submit(WorkerCommand::LoadCachedHistory { chat_id, limit }, RequestKind::History { chat_id, append: false, cached: true });
                }
                self.submit(
                    WorkerCommand::LoadHistory {
                        chat_id,
                        limit,
                        offset_id,
                        offset_date,
                        add_offset,
                    },
                    RequestKind::History {
                        chat_id,
                        append: offset_id != 0 || offset_date != 0 || add_offset != 0,
                        cached: false,
                    },
                );
            }
            DialogOutput::SendText(message) => self.submit(
                WorkerCommand::SendText(message.clone()),
                RequestKind::Send {
                    message,
                    random_id: None,
                },
            ),
            DialogOutput::RetryText(random_id) => {
                if let Some(pending) = self.pending_sends.get(&random_id) {
                    self.submit(
                        WorkerCommand::RetryText { random_id },
                        RequestKind::Retry {
                            chat_id: pending.message.chat_id,
                            random_id,
                        },
                    );
                } else {
                    self.error = Some(ClientError::InvalidRequest);
                }
            }
            DialogOutput::EditText {
                chat_id,
                message_id,
                text,
                entities_json,
            } => self.submit(
                WorkerCommand::EditText {
                    chat_id,
                    message_id,
                    text,
                    entities_json,
                },
                RequestKind::Mutation {
                    chat_id,
                    deleted: None,
                },
            ),
            DialogOutput::DeleteMessage {
                chat_id,
                message_id,
                revoke,
            } => self.submit(
                WorkerCommand::DeleteMessage {
                    chat_id,
                    message_id,
                    revoke,
                },
                RequestKind::Mutation {
                    chat_id,
                    deleted: Some(message_id),
                },
            ),
            DialogOutput::Forward {
                from_chat_id,
                message_ids,
                to_chat_id,
                drop_author,
            } => self.resolve_peer(
                to_chat_id,
                PeerIntent::Forward {
                    from_chat_id,
                    message_ids,
                    drop_author,
                },
            ),
            DialogOutput::React {
                chat_id,
                message_id,
                emoticon,
                document_id,
            } => self.submit(
                WorkerCommand::React {
                    chat_id,
                    message_id,
                    emoticon,
                    document_id,
                },
                RequestKind::Mutation {
                    chat_id,
                    deleted: None,
                },
            ),
        }
    }

    fn settings_command(&mut self, command: WorkerCommand) {
        let request = match &command {
            WorkerCommand::MediaUsage
            | WorkerCommand::SetCacheLimit { .. }
            | WorkerCommand::ClearMedia(_) => ExtraRequest::Settings(SettingsRequest::Usage),
            WorkerCommand::GetMediaSettings
            | WorkerCommand::SetNetworkClass(_)
            | WorkerCommand::SetAutoDownload(_)
            | WorkerCommand::SetAutoplay(_)
            | WorkerCommand::SetPreload(_)
            | WorkerCommand::GetPreferences => ExtraRequest::Settings(SettingsRequest::Settings),
            WorkerCommand::SetPreference { namespace, .. } => ExtraRequest::Preference(*namespace),
            WorkerCommand::Connectivity(_) | WorkerCommand::ExportQr | WorkerCommand::PollQr => {
                ExtraRequest::Connectivity
            }
            WorkerCommand::Appearance(_) => ExtraRequest::Appearance,
            WorkerCommand::Maintenance(_) => ExtraRequest::Maintenance,
            WorkerCommand::Passkey(_) => ExtraRequest::Passkey,
            _ => {
                self.failed(
                    RequestKind::Extra(ExtraRequest::Settings(SettingsRequest::Settings)),
                    ClientError::InvalidRequest,
                );
                return;
            }
        };
        self.shell
            .settings
            .emit(SettingsInput::SetMediaLoading(true));
        self.submit(command, RequestKind::Extra(request));
    }

    fn stop_dialog_media(&mut self) {
        self.shell.dialog.emit(DialogInput::StopPlayback);
        let transfers: Vec<u64> = self
            .requests
            .iter()
            .filter_map(|(id, kind)| {
                matches!(
                    kind,
                    RequestKind::Extra(ExtraRequest::Dialog { transfer: true, .. })
                )
                .then_some(*id)
            })
            .collect();
        if self.worker.is_none() {
            return;
        }
        for transfer_id in transfers {
            if let Some(worker) = &self.worker {
                worker.cancel_media(transfer_id);
            }
            self.submit(
                WorkerCommand::CancelMedia { transfer_id },
                RequestKind::Extra(ExtraRequest::MediaCancel),
            );
        }
    }

    fn open_settings(&mut self) {
        if self.screen != Screen::Settings {
            self.return_screen = self.screen;
            self.screen = Screen::Settings;
            self.shell.settings.emit(SettingsInput::RefreshMedia);
            self.shell.settings.emit(SettingsInput::RefreshPreferences);
        }
    }

    fn open_passkeys(&mut self) {
        if self.screen != Screen::Passkeys {
            self.return_screen = self.screen;
            self.screen = Screen::Passkeys;
        }
        self.submit(
            WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::Enabled),
            RequestKind::Extra(ExtraRequest::Passkey),
        );
        self.submit(
            WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::List),
            RequestKind::Extra(ExtraRequest::Passkey),
        );
    }

    fn clear_active(&mut self) {
        if self.authorized() {
            self.submit(WorkerCommand::ClearActiveDialog, RequestKind::ClearActive);
        }
    }

    fn active_live_location(&self) -> Option<(i64, i32)> {
        let chat_id = self.selected?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        self.histories
            .get(&chat_id)?
            .values()
            .rev()
            .find_map(|message| {
                if !message.outgoing || message.media_kind.as_deref() != Some("geo") {
                    return None;
                }
                let card = serde_json::from_str::<serde_json::Value>(message.file_name.as_deref()?)
                    .ok()?;
                let live = card
                    .get("live")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);
                let period = card
                    .get("p")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0);
                let active = period == i64::from(i32::MAX)
                    || (period > 0 && message.date.saturating_add(period) > now);
                (live != 0 && active).then_some((chat_id, message.id))
            })
    }

    fn location_update(&mut self, location: Location) {
        let Some((chat_id, message_id)) = self.active_live_location() else {
            return;
        };
        self.submit(
            WorkerCommand::Interaction(
                crate::client::interactions::InteractionCommand::UpdateLiveLocation {
                    chat_id,
                    message_id,
                    point: crate::client::interactions::GeoPoint {
                        latitude: location.latitude,
                        longitude: location.longitude,
                        heading: None,
                    },
                },
            ),
            RequestKind::Extra(ExtraRequest::GeoUpdate),
        );
    }

    fn refresh_geoclue(&self) {
        if self.root.is_active() && self.authorized() && self.active_live_location().is_some() {
            self.geoclue.resume();
        } else {
            self.geoclue.pause();
        }
    }

    fn open_chat(&mut self, id: i64) {
        let Some(chat) = self.chats.get(&id).filter(|chat| chat.can_view).cloned() else {
            self.error = Some(ClientError::Forbidden);
            return;
        };
        let reopening = self.selected == Some(id);
        if !reopening {
            self.stop_dialog_media();
            self.clear_active();
        }
        self.selected = Some(id);
        self.screen = Screen::Dialog;
        self.shell.dialog.emit(DialogInput::SetChat(chat));
        self.restore_history();
        self.restore_pending();
        if reopening {
            self.dialog_output(DialogOutput::LoadHistory {
                chat_id: id,
                limit: 100,
                offset_id: 0,
                offset_date: 0,
                add_offset: 0,
            });
        }
    }

    fn restore_history(&self) {
        if let Some(chat_id) = self.selected
            && let Some(messages) = self.histories.get(&chat_id)
        {
            self.shell.dialog.emit(DialogInput::SetHistory {
                chat_id,
                messages: messages.values().cloned().collect(),
                append: false,
            });
        }
    }

    fn restore_pending(&self) {
        self.shell.dialog.emit(DialogInput::SetPending(
            self.pending_sends.values().cloned().collect(),
        ));
    }

    fn show_profile(&self) {
        if let Some(id) = self.profile_id {
            if let Some(profile) = self.extended.profiles.get(&id) {
                self.shell
                    .profile
                    .emit(ProfileInput::SetProfile(Some(profile.clone())));
            } else {
                self.shell.profile.emit(ProfileInput::Open(id));
            }
        }
    }

    fn change_locale(&mut self, locale: &str, sender: &ComponentSender<Self>) {
        let Ok(i18n) = I18n::new(locale) else {
            self.error = Some(ClientError::InvalidRequest);
            return;
        };
        if i18n.locale() == self.i18n.locale() {
            return;
        }
        self.i18n = Rc::new(i18n);
        self.rebuild_ui(sender);
        if matches!(self.state.auth, AuthState::QrPending { .. }) {
            self.submit(WorkerCommand::ExportQr, RequestKind::ExportQr);
        }
    }

    fn rebuild_ui(&mut self, sender: &ComponentSender<Self>) {
        self.extended.search_epoch = self.extended.search_epoch.saturating_add(1);
        self.extended.search_generation = None;
        self.shell = Shell::new(self.i18n.clone(), sender);
        self.root.set_content(Some(&self.shell.root));
        self.shell.auth.emit(AuthInput::State(self.state.clone()));
        self.shell.chats.emit(ChatsInput::SetFromCache(true));
        self.shell
            .chats
            .emit(ChatsInput::SetFolders(self.extended.folders.clone()));
        self.shell
            .folders
            .emit(FoldersInput::SetFolders(self.extended.folders.clone()));
        self.shell.folders.emit(FoldersInput::SetChats(
            self.chats.values().cloned().collect(),
        ));
        self.shell
            .chats
            .emit(ChatsInput::SetChats(self.chats.values().cloned().collect()));
        self.update_connection();
        self.shell
            .dialog
            .emit(DialogInput::SetWallpaper(self.wallpaper.clone()));
        if let Some(chat) = self.selected.and_then(|id| self.chats.get(&id)).cloned() {
            self.shell.dialog.emit(DialogInput::SetChat(chat));
            self.restore_history();
        }
        self.restore_pending();
        self.show_profile();
        if self.screen == Screen::Passkeys {
            self.open_passkeys();
        }
    }

    fn update_connection(&self) {
        let online = self.state.connection == ConnectionState::Connected;
        self.shell.chats.emit(ChatsInput::SetOffline(!online));
        self.shell.dialog.emit(DialogInput::SetOnline(online));
        self.shell.settings.emit(SettingsInput::SetLogoutEnabled(
            self.authorized()
                && !self
                    .requests
                    .values()
                    .any(|kind| matches!(kind, RequestKind::Logout)),
        ));
        if let AuthState::Authorized { user_id } = self.state.auth {
            self.shell.chats.emit(ChatsInput::SetCurrentUser(user_id));
        }
    }

    fn load_chats(&mut self, cached: bool) {
        if !self
            .requests
            .values()
            .any(|kind| matches!(kind, RequestKind::Chats { cached: old } if *old == cached))
        {
            self.shell.chats.emit(ChatsInput::SetLoading(true));
            self.submit(
                if cached {
                    WorkerCommand::LoadCachedChats
                } else {
                    WorkerCommand::LoadChats
                },
                RequestKind::Chats { cached },
            );
        }
    }

    fn worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Ready => {
                self.startup = StartupState::Ready;
                self.error = None;
            }
            WorkerEvent::StartupFailed => {
                self.startup = StartupState::Failed;
                self.error = Some(ClientError::Storage);
            }
            WorkerEvent::State(state) => {
                let was_authorized = self.authorized();
                self.state = state;
                self.shell.auth.emit(AuthInput::State(self.state.clone()));
                self.update_connection();
                if self.authorized() && !was_authorized {
                    self.error = None;
                    self.load_chats(true);
                    self.load_folders(true);
                    self.submit(WorkerCommand::PendingSends, RequestKind::Pending);
                } else if !self.authorized() && was_authorized {
                    self.clear_account_view();
                }
            }
            WorkerEvent::Response {
                request_id,
                response,
            } => {
                if let Some(kind) = self.requests.remove(&request_id) {
                    self.response(kind, response);
                } else {
                    self.error = Some(ClientError::Protocol);
                }
            }
            WorkerEvent::Failed { request_id, error } => {
                match request_id.and_then(|id| self.requests.remove(&id)) {
                    Some(kind) => self.failed(kind, error),
                    None => {
                        self.error = Some(error.clone());
                        self.shell.chats.emit(ChatsInput::SetError(true));
                        self.shell.dialog.emit(DialogInput::SetError(Some(error)));
                    }
                }
            }
            WorkerEvent::OutgoingQueued {
                request_id,
                random_id,
            } => {
                if let Some(RequestKind::Send {
                    message,
                    random_id: id,
                }) = self.requests.get_mut(&request_id)
                {
                    *id = Some(random_id);
                    self.pending_sends.insert(
                        random_id,
                        PendingSend {
                            random_id,
                            message: message.clone(),
                            uncertain: false,
                        },
                    );
                    if self.selected == Some(message.chat_id) {
                        self.shell
                            .dialog
                            .emit(DialogInput::OutgoingQueued { random_id });
                    }
                } else {
                    self.error = Some(ClientError::Protocol);
                }
            }
            WorkerEvent::SpecialQueued { request_id, .. } => {
                if !matches!(
                    self.requests.get(&request_id),
                    Some(RequestKind::Extra(ExtraRequest::Dialog { .. }))
                ) {
                    self.error = Some(ClientError::Protocol);
                }
            }
            WorkerEvent::MediaQueued {
                request_id,
                album_id,
                random_ids,
            } => {
                let token = match self.requests.get(&request_id) {
                    Some(RequestKind::Extra(ExtraRequest::Dialog { token, .. })) => Some(*token),
                    _ => None,
                };
                if let Some(token) = token {
                    self.shell.dialog.emit(DialogInput::MediaQueued {
                        token,
                        album_id,
                        random_ids,
                    });
                } else {
                    self.error = Some(ClientError::Protocol);
                }
            }
            WorkerEvent::MediaProgress {
                transfer_id,
                downloaded,
                total,
            } => {
                let token = match self.requests.get(&transfer_id) {
                    Some(RequestKind::Extra(ExtraRequest::Dialog {
                        token,
                        transfer: true,
                        ..
                    })) => Some(*token),
                    _ => None,
                };
                if let Some(token) = token {
                    self.shell.dialog.emit(DialogInput::MediaProgress {
                        token,
                        transfer_id,
                        downloaded,
                        total,
                    });
                } else {
                    self.error = Some(ClientError::Protocol);
                }
            }
            WorkerEvent::UpdatesCommitted(batch) => self.apply_updates(batch.events),
            WorkerEvent::LiveUpdates(events) => self.apply_updates(events),
            WorkerEvent::Stopped => {
                self.state.connection = ConnectionState::Stopped;
                self.error = Some(ClientError::Stopped);
                let pending = std::mem::take(&mut self.requests);
                for (_, kind) in pending {
                    self.failed(kind, ClientError::Stopped);
                }
                self.update_connection();
            }
        }
    }

    fn extra_response(&mut self, extra: ExtraRequest, response: WorkerResponse) {
        match (extra, response) {
            (ExtraRequest::Dialog { token, .. }, response) => self
                .shell
                .dialog
                .emit(DialogInput::Response { token, response }),
            (ExtraRequest::Folders { cached }, WorkerResponse::Folders(folders)) => {
                self.accept_folders(folders, cached);
                if cached {
                    self.load_folders(false);
                }
            }
            (ExtraRequest::FolderMutation, WorkerResponse::Folders(folders)) => {
                self.accept_folders(folders, false)
            }
            (
                ExtraRequest::Search {
                    generation,
                    section: SearchSection::Contacts,
                    ..
                },
                WorkerResponse::ContactsSearch(result),
            ) => self
                .shell
                .chats
                .emit(ChatsInput::SetSearchContacts { generation, result }),
            (
                ExtraRequest::Search {
                    generation,
                    section: SearchSection::Messages,
                    ..
                },
                WorkerResponse::GlobalSearch(result),
            ) => self
                .shell
                .chats
                .emit(ChatsInput::SetSearchMessages { generation, result }),
            (
                ExtraRequest::Resolve {
                    chat_id, intent, ..
                },
                WorkerResponse::Profile(Some(profile)),
            ) if profile.id == chat_id => self.resolved_profile(*profile, intent),
            (
                ExtraRequest::Profile {
                    chat_id,
                    request_id,
                    section: ProfileRequest::Info,
                    ..
                },
                WorkerResponse::Profile(Some(profile)),
            ) if profile.id == chat_id => {
                self.extended.profiles.insert(chat_id, (*profile).clone());
                self.shell.profile.emit(ProfileInput::ProfileLoaded {
                    request_id,
                    profile: *profile,
                });
            }
            (
                ExtraRequest::Profile {
                    chat_id: expected,
                    request_id,
                    section: ProfileRequest::Members,
                    ..
                },
                WorkerResponse::Members { chat_id, json },
            ) if expected == chat_id => match serde_json::from_str(&json) {
                Ok(page) => self
                    .shell
                    .profile
                    .emit(ProfileInput::MembersLoaded { request_id, page }),
                Err(_) => self.shell.profile.emit(ProfileInput::Failed { request_id }),
            },
            (
                ExtraRequest::Profile {
                    chat_id: expected,
                    request_id,
                    section: ProfileRequest::CommonChats,
                    ..
                },
                WorkerResponse::CommonChats { user_id, json },
            ) if expected == user_id => match serde_json::from_str(&json) {
                Ok(page) => self
                    .shell
                    .profile
                    .emit(ProfileInput::CommonChatsLoaded { request_id, page }),
                Err(_) => self.shell.profile.emit(ProfileInput::Failed { request_id }),
            },
            (
                ExtraRequest::Profile {
                    chat_id: expected,
                    request_id,
                    section: ProfileRequest::Media,
                    ..
                },
                WorkerResponse::MessagesPage { chat_id, page },
            ) if expected == chat_id => self
                .shell
                .profile
                .emit(ProfileInput::SearchMessagesPage { request_id, page }),
            (ExtraRequest::DraftRefresh, WorkerResponse::Drafts(drafts)) => {
                for draft in drafts {
                    self.shell.dialog.emit(DialogInput::DraftChanged(draft));
                }
            }
            (ExtraRequest::MediaCancel, WorkerResponse::Done) => {}
            (ExtraRequest::Settings(SettingsRequest::Usage), WorkerResponse::MediaUsage(usage)) => {
                self.shell
                    .settings
                    .emit(SettingsInput::SetMediaUsage(usage));
                self.shell.settings.emit(SettingsInput::SetMediaLoading(
                    self.requests
                        .values()
                        .any(|kind| matches!(kind, RequestKind::Extra(ExtraRequest::Settings(_)))),
                ));
            }
            (
                ExtraRequest::Settings(SettingsRequest::Settings),
                WorkerResponse::MediaSettings(settings),
            ) => {
                self.shell
                    .settings
                    .emit(SettingsInput::SetMediaSettings(settings));
                self.shell.settings.emit(SettingsInput::SetMediaLoading(
                    self.requests
                        .values()
                        .any(|kind| matches!(kind, RequestKind::Extra(ExtraRequest::Settings(_)))),
                ));
            }
            (ExtraRequest::Connectivity, WorkerResponse::Connectivity(response)) => {
                self.connectivity_response(response);
            }
            (ExtraRequest::Connectivity, WorkerResponse::QrToken { token, expires }) => {
                self.shell
                    .settings
                    .emit(SettingsInput::Connectivity(ConnectivityInput::SetQr(
                        connectivity::QrLink::Pending {
                            token,
                            expires: i64::from(expires),
                        },
                    )));
            }
            (ExtraRequest::Connectivity, WorkerResponse::QrChanged(changed)) => {
                if changed {
                    self.submit(
                        WorkerCommand::ExportQr,
                        RequestKind::Extra(ExtraRequest::Connectivity),
                    );
                }
            }
            (ExtraRequest::Appearance, WorkerResponse::Appearance(response)) => {
                self.appearance_response(response);
            }
            (ExtraRequest::Maintenance, WorkerResponse::Maintenance(response)) => {
                self.shell
                    .settings
                    .emit(SettingsInput::Maintenance(MaintenanceInput::Response(
                        response,
                    )));
            }
            (ExtraRequest::Passkey, WorkerResponse::Passkey(response)) => {
                self.passkey_response(response);
            }
            (
                ExtraRequest::GeoUpdate,
                WorkerResponse::Interaction(crate::client::interactions::InteractionResponse::Done),
            ) => {}
            (
                ExtraRequest::GeoUpdate,
                WorkerResponse::Interaction(
                    crate::client::interactions::InteractionResponse::Message(Some(message)),
                ),
            ) => self.upsert_message(*message),
            (
                ExtraRequest::DeepLink { message_id },
                WorkerResponse::Interaction(
                    crate::client::interactions::InteractionResponse::ResolvedPeer(peer),
                ),
            ) => {
                let intent = message_id.map_or(PeerIntent::OpenChat, PeerIntent::OpenMessage);
                self.resolve_peer(peer.peer_id, intent);
            }
            (
                ExtraRequest::Settings(SettingsRequest::Settings),
                WorkerResponse::Preferences(preferences),
            ) => {
                self.shell
                    .settings
                    .emit(SettingsInput::SetPreferences(preferences));
            }
            (ExtraRequest::Preference(namespace), WorkerResponse::PreferenceChanged(changed)) => {
                self.shell
                    .settings
                    .emit(SettingsInput::PreferenceChanged { namespace, changed });
            }
            (extra, _) => self.failed(RequestKind::Extra(extra), ClientError::Protocol),
        }
    }

    fn connectivity_response(
        &mut self,
        response: crate::client::connectivity::ConnectivityResponse,
    ) {
        use crate::client::connectivity::ConnectivityResponse;
        match response {
            ConnectivityResponse::Done => {
                self.shell
                    .settings
                    .emit(SettingsInput::Connectivity(ConnectivityInput::SetError(
                        None,
                    )))
            }
            ConnectivityResponse::Proxy(_) | ConnectivityResponse::ActiveProxy(_) => self.submit(
                WorkerCommand::Connectivity(
                    crate::client::connectivity::ConnectivityCommand::ListProxies,
                ),
                RequestKind::Extra(ExtraRequest::Connectivity),
            ),
            ConnectivityResponse::Proxies(proxies) => {
                let records = proxies
                    .into_iter()
                    .map(|proxy| {
                        let has_password = proxy.username.is_some();
                        let username = proxy.username.unwrap_or_default();
                        ProxyRecord {
                            id: i64::try_from(proxy.id).unwrap_or_default(),
                            kind: match proxy.kind {
                                mono_proto_client::adapter::socket::ProxyKind::Socks5 => {
                                    connectivity::ProxyKind::Socks5
                                }
                                mono_proto_client::adapter::socket::ProxyKind::Http => {
                                    connectivity::ProxyKind::Http
                                }
                                mono_proto_client::adapter::socket::ProxyKind::Https => {
                                    connectivity::ProxyKind::Https
                                }
                                mono_proto_client::adapter::socket::ProxyKind::Mtproto => {
                                    connectivity::ProxyKind::Mtproto
                                }
                            },
                            host: proxy.host,
                            port: proxy.port,
                            username,
                            has_password,
                            has_secret: proxy.has_secret,
                            active: proxy.active,
                            checked: false,
                            latency_ms: None,
                        }
                    })
                    .collect();
                self.shell.settings.emit(SettingsInput::Connectivity(
                    ConnectivityInput::SetProxies(records),
                ));
            }
            ConnectivityResponse::TransportMode(_) => {}
            ConnectivityResponse::Probes(probes) => {
                for probe in probes {
                    self.shell.settings.emit(SettingsInput::Connectivity(
                        ConnectivityInput::SetProxyProbe(ProxyProbe {
                            id: i64::try_from(probe.id).unwrap_or_default(),
                            ok: probe.error.is_none(),
                            latency_ms: probe
                                .latency_ms
                                .and_then(|value| u32::try_from(value).ok()),
                        }),
                    ));
                }
            }
            ConnectivityResponse::NotifyDefaults(defaults) => {
                self.shell.settings.emit(SettingsInput::Connectivity(
                    ConnectivityInput::SetNotificationSettings(
                        connectivity::NotificationSettings {
                            private: !defaults.users.silent,
                            groups: !defaults.chats.silent,
                            channels: !defaults.broadcasts.silent,
                            mentions: true,
                        },
                    ),
                ));
            }
            ConnectivityResponse::NotifySettings(_) => self.submit(
                WorkerCommand::Connectivity(
                    crate::client::connectivity::ConnectivityCommand::GetNotifyDefaults,
                ),
                RequestKind::Extra(ExtraRequest::Connectivity),
            ),
            ConnectivityResponse::NotifyExceptions(exceptions) => {
                let exceptions = exceptions
                    .into_iter()
                    .map(|exception| NotificationException {
                        chat_id: exception.chat_id,
                        title: self
                            .chats
                            .get(&exception.chat_id)
                            .map(|chat| chat.title.clone()),
                        muted: exception.silent,
                    })
                    .collect();
                self.shell.settings.emit(SettingsInput::Connectivity(
                    ConnectivityInput::SetNotificationExceptions(exceptions),
                ));
            }
            ConnectivityResponse::DeviceSessions(sessions) => {
                let devices = sessions
                    .into_iter()
                    .map(|session| connectivity::DeviceRecord {
                        hash: session.hash.to_string(),
                        name: session.device_model,
                        is_current: session.current,
                        last_active: Some(session.date_active.to_string()),
                    })
                    .collect();
                self.shell.settings.emit(SettingsInput::Connectivity(
                    ConnectivityInput::SetDevices(devices),
                ));
            }
            ConnectivityResponse::DeviceSession(_) => self.submit(
                WorkerCommand::Connectivity(
                    crate::client::connectivity::ConnectivityCommand::GetDeviceSessions,
                ),
                RequestKind::Extra(ExtraRequest::Connectivity),
            ),
            ConnectivityResponse::Pending2fa(sessions) => {
                let devices = sessions
                    .into_iter()
                    .map(|session| connectivity::DeviceRecord {
                        hash: session.hash.to_string(),
                        name: session.device_model,
                        is_current: session.current,
                        last_active: Some(session.date_active.to_string()),
                    })
                    .collect();
                self.shell.settings.emit(SettingsInput::Connectivity(
                    ConnectivityInput::SetDevices(devices),
                ));
            }
        }
    }

    fn appearance_response(&mut self, response: crate::client::appearance::AppearanceResponse) {
        use crate::client::appearance::AppearanceResponse;
        match response {
            AppearanceResponse::Preferences(preferences) => {
                match serde_json::to_string(&preferences) {
                    Ok(payload_json) => self.shell.settings.emit(SettingsInput::SetPreferences(
                        crate::domain::AppPreferences {
                            documents: vec![(
                                crate::domain::PreferenceNamespace::Appearance,
                                crate::domain::PreferenceDocument {
                                    version: crate::domain::APP_PREFERENCES_VERSION,
                                    payload_json,
                                },
                            )],
                        },
                    )),
                    Err(_) => self.error = Some(ClientError::Storage),
                }
            }
            AppearanceResponse::Wallpapers(catalog) => {
                self.shell
                    .settings
                    .emit(SettingsInput::WallpaperCatalog(catalog));
            }
            AppearanceResponse::Wallpaper(file) => {
                self.shell
                    .settings
                    .emit(SettingsInput::WallpaperLoaded(file));
            }
        }
    }

    fn passkey_response(&mut self, response: crate::client::passkeys::PasskeyResponse) {
        use crate::client::passkeys::PasskeyResponse;
        match response {
            PasskeyResponse::Enabled(supported) => self
                .shell
                .passkeys
                .emit(PasskeyInput::SetSupported(supported)),
            PasskeyResponse::LoginOptions(_) => {}
            PasskeyResponse::RegistrationOptions(options_json) => self.submit(
                WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::Register {
                    options_json,
                }),
                RequestKind::Extra(ExtraRequest::Passkey),
            ),
            PasskeyResponse::SignedIn { .. } | PasskeyResponse::Done => {}
            PasskeyResponse::Registered(_) | PasskeyResponse::Deleted(_) => self.submit(
                WorkerCommand::Passkey(crate::client::passkeys::PasskeyCommand::List),
                RequestKind::Extra(ExtraRequest::Passkey),
            ),
            PasskeyResponse::Keys(keys) => {
                let records = keys
                    .into_iter()
                    .map(|key| PasskeyRecord {
                        id: key.id,
                        label: key.name,
                        created_at: Some(key.date.to_string()),
                    })
                    .collect();
                self.shell.passkeys.emit(PasskeyInput::SetPasskeys(records));
            }
        }
    }

    fn accept_folders(&mut self, folders: Vec<mono_proto_client::FolderDto>, cached: bool) {
        self.extended.folders = folders.clone();
        self.shell
            .chats
            .emit(ChatsInput::SetFolders(folders.clone()));
        self.shell.folders.emit(FoldersInput::SetFromCache(cached));
        self.shell.folders.emit(FoldersInput::SetFolders(folders));
    }

    fn response(&mut self, kind: RequestKind, response: WorkerResponse) {
        let kind = match kind {
            RequestKind::Extra(extra) => {
                if self.extra_current(&extra) {
                    self.extra_response(extra, response);
                }
                return;
            }
            kind => kind,
        };
        match response {
            WorkerResponse::ContactsSearch(_)
            | WorkerResponse::GlobalSearch(_)
            | WorkerResponse::MessagesPage { .. }
            | WorkerResponse::Profile(_)
            | WorkerResponse::Members { .. }
            | WorkerResponse::CommonChats { .. }
            | WorkerResponse::Folders(_)
            | WorkerResponse::Drafts(_)
            | WorkerResponse::Draft(_)
            | WorkerResponse::Scheduled { .. }
            | WorkerResponse::MediaReady { .. }
            | WorkerResponse::MediaUsage(_)
            | WorkerResponse::MediaSettings(_)
            | WorkerResponse::MediaDecision(_)
            | WorkerResponse::MediaChunk { .. }
            | WorkerResponse::MediaOutgoing { .. }
            | WorkerResponse::PendingMedia(_)
            | WorkerResponse::Interaction(_)
            | WorkerResponse::Web(_)
            | WorkerResponse::SpecialOutgoing { .. }
            | WorkerResponse::PendingSpecial(_)
            | WorkerResponse::Preferences(_)
            | WorkerResponse::PreferenceChanged(_)
            | WorkerResponse::Connectivity(_)
            | WorkerResponse::Appearance(_)
            | WorkerResponse::Maintenance(_)
            | WorkerResponse::Passkey(_)
            | WorkerResponse::PollVoters { .. } => self.failed(kind, ClientError::Protocol),
            WorkerResponse::Done => match kind {
                RequestKind::Configure => self.submit(WorkerCommand::Connect, RequestKind::Connect),
                RequestKind::Mutation {
                    chat_id,
                    deleted: Some(id),
                } => self.remove_message(chat_id, id),
                RequestKind::Logout => self.clear_account_view(),
                RequestKind::Extra(_) => unreachable!(),
                RequestKind::Connect
                | RequestKind::Authenticate
                | RequestKind::ExportQr
                | RequestKind::PollQr
                | RequestKind::Mutation { deleted: None, .. }
                | RequestKind::ClearActive => {}
                RequestKind::Chats { .. }
                | RequestKind::History { .. }
                | RequestKind::Pending
                | RequestKind::Send { .. }
                | RequestKind::Retry { .. } => self.error = Some(ClientError::Protocol),
            },
            WorkerResponse::Chats(chats) => {
                let RequestKind::Chats { cached } = kind else {
                    self.error = Some(ClientError::Protocol);
                    return;
                };
                for chat in &chats {
                    if cached {
                        self.chats.entry(chat.id).or_insert_with(|| chat.clone());
                    } else {
                        self.chats.insert(chat.id, chat.clone());
                    }
                }
                self.shell
                    .folders
                    .emit(FoldersInput::SetChats(chats.clone()));
                self.shell.chats.emit(ChatsInput::SetFromCache(cached));
                self.shell.chats.emit(ChatsInput::SetChats(chats));
                self.shell.chats.emit(ChatsInput::SetLoading(false));
                if cached {
                    self.load_chats(false);
                }
            }
            WorkerResponse::History { chat_id, messages } => {
                let RequestKind::History {
                    chat_id: expected,
                    append,
                    cached,
                } = kind
                else {
                    self.error = Some(ClientError::Protocol);
                    return;
                };
                if expected != chat_id {
                    self.error = Some(ClientError::Protocol);
                    return;
                }
                let history = self.histories.entry(chat_id).or_default();
                for message in &messages {
                    if message.chat_id == chat_id
                        && !self.live.contains(&(chat_id, message.id))
                        && !self.deleted.contains(&(chat_id, message.id))
                        && history.get(&message.id).is_none_or(|old| {
                            old.edit_date.unwrap_or(0) <= message.edit_date.unwrap_or(0)
                        })
                    {
                        history.insert(message.id, message.clone());
                    }
                }
                if self.selected == Some(chat_id) {
                    let messages = messages
                        .into_iter()
                        .filter_map(|message| history.get(&message.id).cloned())
                        .collect();
                    self.shell.dialog.emit(DialogInput::SetHistory {
                        chat_id,
                        messages,
                        append: append && !cached,
                    });
                }
            }
            WorkerResponse::QrToken { token, expires } => {
                self.shell.auth.emit(AuthInput::QrToken { token, expires })
            }
            WorkerResponse::QrChanged(changed) => {
                if changed && matches!(self.state.auth, AuthState::QrPending { .. }) {
                    self.submit(WorkerCommand::ExportQr, RequestKind::ExportQr);
                }
            }
            WorkerResponse::Outgoing { random_id, result } => self.outgoing(random_id, *result),
            WorkerResponse::Message(message) => self.upsert_message(*message),
            WorkerResponse::Forwarded(messages) => {
                for message in messages {
                    self.upsert_message(message);
                }
            }
            WorkerResponse::Pending(pending) => {
                self.pending_sends = pending
                    .into_iter()
                    .map(|pending| (pending.random_id, pending))
                    .collect();
                self.restore_pending();
            }
        }
    }

    fn extra_current(&self, kind: &ExtraRequest) -> bool {
        if matches!(
            kind,
            ExtraRequest::MediaCancel
                | ExtraRequest::Connectivity
                | ExtraRequest::Appearance
                | ExtraRequest::Maintenance
                | ExtraRequest::Passkey
                | ExtraRequest::Preference(_)
                | ExtraRequest::GeoUpdate
                | ExtraRequest::DeepLink { .. }
                | ExtraRequest::Settings(_)
        ) {
            return true;
        }
        if !self.authorized() {
            return false;
        }
        match kind {
            ExtraRequest::Folders { .. }
            | ExtraRequest::FolderMutation
            | ExtraRequest::DraftRefresh => true,
            ExtraRequest::MediaCancel
            | ExtraRequest::Connectivity
            | ExtraRequest::Appearance
            | ExtraRequest::Maintenance
            | ExtraRequest::Passkey
            | ExtraRequest::Preference(_)
            | ExtraRequest::GeoUpdate
            | ExtraRequest::DeepLink { .. }
            | ExtraRequest::Settings(_) => unreachable!(),
            ExtraRequest::Dialog { epoch, .. } => *epoch == self.extended.search_epoch,
            ExtraRequest::Search {
                generation, epoch, ..
            } => {
                *epoch == self.extended.search_epoch
                    && self.extended.search_generation == Some(*generation)
            }
            ExtraRequest::Resolve { navigation, .. } => *navigation == self.extended.navigation,
            ExtraRequest::Profile { chat_id, epoch, .. } => {
                *epoch == self.extended.search_epoch && self.profile_id == Some(*chat_id)
            }
        }
    }

    fn extra_failed(&mut self, kind: ExtraRequest, error: ClientError) {
        match kind {
            ExtraRequest::Folders { cached } => {
                self.shell
                    .folders
                    .emit(FoldersInput::Error(self.error_text(&error)));
                if cached {
                    self.load_folders(false);
                }
            }
            ExtraRequest::FolderMutation => {
                self.shell
                    .folders
                    .emit(FoldersInput::Error(self.error_text(&error)));
                self.load_folders(false);
            }
            ExtraRequest::Search {
                generation,
                section,
                ..
            } => self.shell.chats.emit(ChatsInput::SearchFailed {
                generation,
                section,
            }),
            ExtraRequest::Resolve { .. } => {
                self.shell.dialog.emit(DialogInput::SetError(Some(error)))
            }
            ExtraRequest::Profile { request_id, .. } => {
                self.shell.profile.emit(ProfileInput::Failed { request_id })
            }
            ExtraRequest::Dialog { token, .. } => {
                self.shell.dialog.emit(DialogInput::Failed { token, error })
            }
            ExtraRequest::DraftRefresh => {
                self.shell.dialog.emit(DialogInput::SetError(Some(error)))
            }
            ExtraRequest::MediaCancel => {}
            ExtraRequest::Connectivity => {
                let message = self.error_text(&error);
                self.shell
                    .settings
                    .emit(SettingsInput::Connectivity(ConnectivityInput::SetError(
                        Some(message),
                    )));
            }
            ExtraRequest::Appearance => {
                let message = self.error_text(&error);
                self.shell
                    .settings
                    .emit(SettingsInput::AppearanceFailed(Some(message)));
            }
            ExtraRequest::Maintenance => {
                let message = self.error_text(&error);
                self.shell
                    .settings
                    .emit(SettingsInput::Maintenance(MaintenanceInput::Failed(
                        message,
                    )));
            }
            ExtraRequest::Passkey => {}
            ExtraRequest::Preference(namespace) => {
                self.shell.settings.emit(SettingsInput::PreferenceChanged {
                    namespace,
                    changed: false,
                });
            }
            ExtraRequest::GeoUpdate | ExtraRequest::DeepLink { .. } => {}
            ExtraRequest::Settings(_) => {
                let message = self.error_text(&error);
                self.shell
                    .settings
                    .emit(SettingsInput::SetMediaError(Some(message)));
            }
        }
    }

    fn failed(&mut self, kind: RequestKind, error: ClientError) {
        if let RequestKind::Extra(extra) = &kind
            && !self.extra_current(extra)
        {
            return;
        }
        self.error = Some(error.clone());
        if kind.auth() {
            self.shell.auth.emit(AuthInput::Error(Some(error)));
            return;
        }
        match kind {
            RequestKind::Chats { cached } => {
                self.shell.chats.emit(ChatsInput::SetLoading(false));
                self.shell.chats.emit(ChatsInput::SetError(true));
                if cached {
                    self.load_chats(false);
                }
            }
            RequestKind::Send { message, random_id } => {
                if let Some(random_id) = random_id {
                    if let Some(pending) = self.pending_sends.get_mut(&random_id) {
                        pending.uncertain = true;
                    }
                    if self.selected == Some(message.chat_id) {
                        self.shell.dialog.emit(DialogInput::OutgoingResult {
                            random_id,
                            result: Err(error),
                        });
                    }
                } else if self.selected == Some(message.chat_id) {
                    self.shell.dialog.emit(DialogInput::SetError(Some(error)));
                }
            }
            RequestKind::Retry { chat_id, random_id } => {
                if self.selected == Some(chat_id) {
                    self.shell.dialog.emit(DialogInput::OutgoingResult {
                        random_id,
                        result: Err(error),
                    });
                }
            }
            RequestKind::History { chat_id, .. } | RequestKind::Mutation { chat_id, .. } => {
                if self.selected == Some(chat_id) {
                    self.shell.dialog.emit(DialogInput::SetError(Some(error)));
                }
            }
            RequestKind::Extra(extra) => self.extra_failed(extra, error),
            RequestKind::Pending | RequestKind::ClearActive | RequestKind::Logout => {}
            RequestKind::Configure
            | RequestKind::Connect
            | RequestKind::Authenticate
            | RequestKind::ExportQr
            | RequestKind::PollQr => unreachable!(),
        }
    }

    fn clear_account_view(&mut self) {
        self.stop_dialog_media();
        self.extended = ExtendedState {
            search_epoch: self.extended.search_epoch.saturating_add(1),
            navigation: self.extended.navigation.saturating_add(1),
            ..Default::default()
        };
        self.shell
            .folders
            .emit(FoldersInput::SetFolders(Vec::new()));
        self.chats.clear();
        self.histories.clear();
        self.live.clear();
        self.deleted.clear();
        self.pending_sends.clear();
        self.selected = None;
        self.profile_id = None;
        self.screen = Screen::Chats;
        self.shell.chats.emit(ChatsInput::Clear);
        self.shell.profile.emit(ProfileInput::SetProfile(None));
        self.shell.dialog.emit(DialogInput::SetPending(Vec::new()));
        self.shell
            .dialog
            .emit(DialogInput::SetPendingMedia(Vec::new()));
        self.shell.auth.emit(AuthInput::State(self.state.clone()));
    }

    fn outgoing(&mut self, random_id: i64, result: OutgoingMessageResult) {
        self.pending_sends.remove(&random_id);
        if let OutgoingMessageResult::Sent { message } = &result {
            self.upsert_message(message.clone());
        }
        self.shell.dialog.emit(DialogInput::OutgoingResult {
            random_id,
            result: Ok(result),
        });
    }

    fn upsert_message(&mut self, message: MessageDto) {
        let key = (message.chat_id, message.id);
        if self.deleted.contains(&key) {
            return;
        }
        let history = self.histories.entry(message.chat_id).or_default();
        if history
            .get(&message.id)
            .is_some_and(|old| old.edit_date.unwrap_or(0) > message.edit_date.unwrap_or(0))
        {
            return;
        }
        self.live.insert(key);
        history.insert(message.id, message.clone());
        if let Some(chat) = self.chats.get_mut(&message.chat_id) {
            if chat.last_message_id <= message.id {
                chat.last_message_id = message.id;
                chat.last_message_date = Some(message.date);
                chat.last_message_preview = message.text.clone();
                chat.last_message_outgoing = message.outgoing;
            }
            self.shell.update_chat(self.selected, chat.clone());
        }
        self.shell.dialog.emit(DialogInput::UpsertMessage(message));
    }

    fn publish_notification(&self, message: &MessageDto) {
        let Some(adapter) = &self.notifications else {
            return;
        };
        if !self.notification_policy.enabled
            || message.outgoing
            || (self.screen == Screen::Dialog && self.selected == Some(message.chat_id))
            || self
                .chats
                .get(&message.chat_id)
                .is_some_and(|chat| chat.muted)
        {
            return;
        }
        let AuthState::Authorized { user_id } = self.state.auth else {
            return;
        };
        let id = format!("monogram-message-{}-{}", message.chat_id, message.id);
        let title = self
            .chats
            .get(&message.chat_id)
            .map(|chat| chat.title.clone())
            .or_else(|| message.sender_name.clone())
            .unwrap_or_else(|| self.text("app-app_name"));
        let target = NotificationTarget {
            account_id: user_id.to_string(),
            chat_id: message.chat_id,
            message_id: Some(i64::from(message.id)),
        };
        let mark_read = self.text("app-push_mark_read");
        let mute = self.text("app-push_mute_hour");
        let settings = self.text("desktop-settings");
        let labels = NotificationLabels {
            mark_read: &mark_read,
            mute: &mute,
            settings: &settings,
        };
        let content = NotificationContent {
            id: &id,
            title: &title,
            body: self
                .notification_policy
                .preview
                .then_some(message.text.as_deref())
                .flatten(),
            target,
        };
        let _ = adapter.send(&content, &labels);
    }

    fn remove_message(&mut self, chat_id: i64, message_id: i32) {
        self.deleted.insert((chat_id, message_id));
        self.live.remove(&(chat_id, message_id));
        if let Some(history) = self.histories.get_mut(&chat_id) {
            history.remove(&message_id);
        }
        self.shell.dialog.emit(DialogInput::RemoveMessage {
            chat_id,
            message_id,
        });
    }

    fn apply_updates(&mut self, events: Vec<UpdateEventDto>) {
        for event in events {
            match event {
                UpdateEventDto::NewMessage { message } => {
                    self.upsert_message(message.clone());
                    self.publish_notification(&message);
                }
                UpdateEventDto::MessageEdited { message } => self.upsert_message(message),
                UpdateEventDto::OutgoingMessageAcknowledged { random_id, result } => {
                    self.outgoing(random_id, result)
                }
                UpdateEventDto::MessagesDeleted {
                    chat_id,
                    message_ids,
                } => {
                    let ids: Vec<_> = chat_id
                        .map(|id| vec![id])
                        .unwrap_or_else(|| self.histories.keys().copied().collect());
                    for chat in ids {
                        for id in &message_ids {
                            self.remove_message(chat, *id);
                        }
                    }
                }
                UpdateEventDto::DialogsPatched { chats } => {
                    for chat in chats {
                        self.chats.insert(chat.id, chat.clone());
                        if self.selected == Some(chat.id) {
                            self.shell.dialog.emit(DialogInput::SetChat(chat.clone()));
                        }
                        self.shell.chats.emit(ChatsInput::UpdateChat(chat));
                    }
                }
                UpdateEventDto::ChatsChanged | UpdateEventDto::FoldersChanged => {
                    self.load_chats(false)
                }
                UpdateEventDto::PeerTyping {
                    chat_id, typing, ..
                } => self.shell.chats.emit(ChatsInput::SetTyping {
                    chat_id,
                    active: typing,
                }),
                UpdateEventDto::PeerStatus {
                    user_id,
                    status,
                    status_at,
                } => {
                    if let Some(chat) = self.chats.get_mut(&user_id) {
                        chat.peer_status = status;
                        chat.peer_status_at = status_at;
                        self.shell.update_chat(self.selected, chat.clone());
                    }
                }
                UpdateEventDto::PeerEmojiStatus {
                    user_id,
                    document_id,
                } => {
                    if let Some(chat) = self.chats.get_mut(&user_id) {
                        chat.emoji_status_document_id = document_id;
                        self.shell.update_chat(self.selected, chat.clone());
                    }
                }
                UpdateEventDto::ReadInbox {
                    chat_id,
                    max_id,
                    still_unread,
                } => {
                    if let Some(chat) = self.chats.get_mut(&chat_id) {
                        chat.read_inbox_max_id = chat.read_inbox_max_id.max(max_id);
                        chat.unread_count = still_unread;
                        self.shell.update_chat(self.selected, chat.clone());
                    }
                }
                UpdateEventDto::ReadOutbox { chat_id, max_id } => {
                    if let Some(chat) = self.chats.get_mut(&chat_id) {
                        chat.read_outbox_max_id = chat.read_outbox_max_id.max(max_id);
                        self.shell.update_chat(self.selected, chat.clone());
                    }
                }
                UpdateEventDto::MessageReactions {
                    chat_id,
                    message_id,
                    reactions_json,
                } => {
                    if let Some(message) = self
                        .histories
                        .get_mut(&chat_id)
                        .and_then(|history| history.get_mut(&message_id))
                    {
                        message.reactions_json = Some(reactions_json);
                        let message = message.clone();
                        self.upsert_message(message);
                    }
                }
                UpdateEventDto::SyncState { is_syncing } => {
                    self.shell.chats.emit(ChatsInput::SetSyncing(is_syncing))
                }
                UpdateEventDto::DraftChanged { .. } => {
                    if !self
                        .requests
                        .values()
                        .any(|kind| matches!(kind, RequestKind::Extra(ExtraRequest::DraftRefresh)))
                    {
                        self.submit(
                            WorkerCommand::GetCachedCloudDrafts,
                            RequestKind::Extra(ExtraRequest::DraftRefresh),
                        );
                    }
                }
                UpdateEventDto::ScheduledMessageChanged { message } => self
                    .shell
                    .dialog
                    .emit(DialogInput::ScheduledChanged(message)),
                UpdateEventDto::ScheduledMessagesDeleted {
                    chat_id,
                    message_ids,
                    sent_message_ids: _,
                } => self.shell.dialog.emit(DialogInput::ScheduledDeleted {
                    chat_id,
                    message_ids,
                }),
                UpdateEventDto::PollChanged { .. }
                | UpdateEventDto::SavedGifsChanged
                | UpdateEventDto::DiscussionInbox { .. }
                | UpdateEventDto::Ignored { .. } => {
                    self.unrepresented_updates = self.unrepresented_updates.saturating_add(1);
                }
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.geoclue.stop();
        if let Some(source) = self.secret_event_source.take() {
            source.remove();
        }
        self.stop_worker();
    }
}
