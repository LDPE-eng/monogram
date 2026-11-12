use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use mono_proto_client::{
    self as proto, ChatDto, MessageDto, OutgoingMessageResult, SendOptions, UpdateBatchDto,
    UpdateEventDto,
    adapter::{
        runtime::{api_invoke, request_control, scheduler},
        types::{auth_types::QrLoginDto, init_connection::InitConnectionInfo},
    },
};
use zeroize::Zeroizing;

use crate::{
    domain::{
        ApiConfig, AuthState, ClientError, ClientState, ConnectionState, PendingSend, TextSend,
        page_limit,
    },
    platform::AppPaths,
    storage::Storage,
};

pub enum WorkerCommand {
    Configure(ApiConfig),
    ConfigureFromSecrets,
    GetPreferences,
    SetPreference {
        namespace: crate::domain::PreferenceNamespace,
        expected_version: Option<u32>,
        document: crate::domain::PreferenceDocument,
    },
    DeletePreference {
        namespace: crate::domain::PreferenceNamespace,
    },
    Connect,
    SendCode {
        phone: String,
    },
    ResendCode,
    SignIn {
        code: String,
    },
    CheckPassword {
        password: String,
    },
    ExportQr,
    PollQr,
    LoadChats,
    LoadMoreChats {
        offset_date: i32,
        offset_id: i32,
        offset_peer_id: i64,
        folder_id: i32,
    },
    LoadHistory {
        chat_id: i64,
        limit: i32,
        offset_id: i32,
        offset_date: i32,
        add_offset: i32,
    },
    LoadCachedChats,
    LoadCachedHistory {
        chat_id: i64,
        limit: i32,
    },
    PendingSends,
    SendText(TextSend),
    RetryText {
        random_id: i64,
    },
    DownloadMessageMedia {
        chat_id: i64,
        message_id: i32,
        kind: crate::domain::MediaDownloadKind,
        mode: crate::domain::MediaDownloadMode,
    },
    DownloadPeerAvatar {
        peer_id: i64,
        kind: crate::domain::MediaDownloadKind,
        mode: crate::domain::MediaDownloadMode,
    },
    DownloadCustomEmoji {
        document_id: i64,
        mode: crate::domain::MediaDownloadMode,
    },
    DownloadMediaRange {
        chat_id: i64,
        message_id: i32,
        offset: u64,
        mode: crate::domain::MediaDownloadMode,
    },
    CancelMedia {
        transfer_id: u64,
    },
    MediaUsage,
    SetCacheLimit {
        limit: crate::domain::CacheLimit,
    },
    ClearMedia(crate::domain::ClearMediaScope),
    GetMediaSettings,
    SetNetworkClass(crate::domain::NetworkClass),
    SetAutoDownload(crate::domain::AutoDownloadSettings),
    SetAutoplay(crate::domain::NetworkPolicy),
    SetPreload(crate::domain::NetworkPolicy),
    MediaDecision {
        category: crate::domain::MediaCategory,
    },
    PendingMedia,
    SendMedia(crate::domain::MediaSend),
    RetryMedia {
        album_id: i64,
    },
    PendingSpecial,
    Interaction(interactions::InteractionCommand),
    Web(web::WebCommand),
    Connectivity(connectivity::ConnectivityCommand),
    Appearance(appearance::AppearanceCommand),
    Maintenance(maintenance::MaintenanceCommand),
    Passkey(passkeys::PasskeyCommand),
    SendSpecial(crate::domain::SpecialSend),
    RetrySpecial {
        random_id: i64,
    },
    VotePoll {
        chat_id: i64,
        message_id: i32,
        options: Vec<Vec<u8>>,
    },
    RetractPollVote {
        chat_id: i64,
        message_id: i32,
    },
    ClosePoll {
        chat_id: i64,
        message_id: i32,
    },
    PollVoters {
        chat_id: i64,
        message_id: i32,
        offset: Option<String>,
    },
    AppendChecklist {
        chat_id: i64,
        message_id: i32,
        first_id: i32,
        titles: Vec<String>,
    },
    ToggleChecklistCompleted {
        chat_id: i64,
        message_id: i32,
        completed: Vec<i32>,
        incompleted: Vec<i32>,
    },
    EditChecklistTitle {
        chat_id: i64,
        message_id: i32,
        edit: crate::domain::ChecklistTextEdit,
    },
    EditChecklistItem {
        chat_id: i64,
        message_id: i32,
        edit: crate::domain::ChecklistTextEdit,
    },
    DeleteChecklistItem {
        chat_id: i64,
        message_id: i32,
        edit: crate::domain::ChecklistTextEdit,
    },
    ReplaceMedia {
        chat_id: i64,
        message_id: i32,
        expected_key: String,
        item: crate::domain::MediaUpload,
    },
    SendSavedGif {
        chat_id: i64,
        document_id: i64,
        reply_to_msg_id: i32,
        top_msg_id: i32,
    },
    SendSticker {
        chat_id: i64,
        document_id: i64,
        reply_to_msg_id: i32,
        top_msg_id: i32,
    },
    SendCustomEmoji {
        chat_id: i64,
        document_id: i64,
        reply_to_msg_id: i32,
        top_msg_id: i32,
    },
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
    ContactsSearch {
        query: String,
        limit: i32,
    },
    SearchGlobal {
        query: String,
        offset_rate: i32,
        offset_peer_id: i64,
        offset_id: i32,
        limit: i32,
        folder_id: i32,
    },
    SearchMessagesPage {
        chat_id: i64,
        query: String,
        filter: String,
        offset_id: i32,
        add_offset: i32,
        limit: i32,
    },
    GetProfile {
        peer_id: i64,
    },
    GetCachedProfile {
        peer_id: i64,
    },
    GetMembers {
        chat_id: i64,
        filter: String,
        query: String,
        offset: i32,
    },
    GetCommonChats {
        user_id: i64,
        max_id: i64,
        limit: i32,
    },
    GetFolders,
    GetCachedFolders,
    SaveFolder(proto::FolderDto),
    DeleteFolder {
        id: i32,
    },
    ReorderFolders {
        order: Vec<i32>,
    },
    GetReplies {
        chat_id: i64,
        message_id: i32,
        limit: i32,
        offset_id: i32,
        add_offset: i32,
    },
    GetPinnedMessages {
        chat_id: i64,
        limit: i32,
    },
    ReadHistory {
        chat_id: i64,
        max_id: i32,
    },
    MarkDialogUnread {
        chat_id: i64,
        unread: bool,
    },
    GetCloudDrafts,
    GetCachedCloudDrafts,
    SaveCloudDraft {
        draft: proto::CloudDraft,
        expected_revision: i64,
    },
    DiscardCloudDraft {
        chat_id: i64,
        thread_id: i32,
        expected_revision: i64,
    },
    GetScheduledHistory {
        chat_id: i64,
    },
    GetCachedScheduledHistory {
        chat_id: i64,
    },
    SendScheduledNow {
        chat_id: i64,
        message_ids: Vec<i32>,
    },
    DeleteScheduled {
        chat_id: i64,
        message_ids: Vec<i32>,
    },
    Reschedule {
        chat_id: i64,
        message_id: i32,
        schedule_date: i32,
    },
    EditScheduled {
        chat_id: i64,
        message_id: i32,
        schedule_date: i32,
        text: String,
        entities_json: Option<String>,
        no_webpage: bool,
    },
    ClearActiveDialog,
    RefreshUpdates,
    Logout,
    Destroy,
    Shutdown,
}

pub enum WorkerResponse {
    Done,
    Preferences(crate::domain::AppPreferences),
    PreferenceChanged(bool),
    Chats(Vec<ChatDto>),
    History {
        chat_id: i64,
        messages: Vec<MessageDto>,
    },
    QrToken {
        token: Vec<u8>,
        expires: i32,
    },
    QrChanged(bool),
    Outgoing {
        random_id: i64,
        result: Box<OutgoingMessageResult>,
    },
    Message(Box<MessageDto>),
    Forwarded(Vec<MessageDto>),
    Pending(Vec<PendingSend>),
    MediaReady {
        transfer_id: u64,
        cache_key: String,
        lease: Box<crate::media::cache::CacheLease>,
    },
    MediaUsage(crate::domain::MediaUsage),
    MediaSettings(crate::domain::MediaSettings),
    MediaDecision(crate::domain::MediaDecision),
    MediaChunk {
        transfer_id: u64,
        cache_key: String,
        offset: u64,
        bytes: Vec<u8>,
        done: bool,
        total: Option<u64>,
    },
    MediaOutgoing {
        album_id: i64,
        random_ids: Vec<i64>,
        results: Vec<OutgoingMessageResult>,
    },
    PendingMedia(Vec<crate::domain::PendingMediaSend>),
    SpecialOutgoing {
        random_id: i64,
        result: Box<OutgoingMessageResult>,
    },
    PendingSpecial(Vec<crate::domain::PendingSpecialSend>),
    Interaction(interactions::InteractionResponse),
    Web(web::WebResponse),
    Connectivity(connectivity::ConnectivityResponse),
    Appearance(appearance::AppearanceResponse),
    Maintenance(maintenance::MaintenanceResponse),
    Passkey(passkeys::PasskeyResponse),
    PollVoters {
        chat_id: i64,
        message_id: i32,
        page: proto::adapter::messages::extras_rpc::PollVotersDto,
    },
    ContactsSearch(proto::ContactsSearchDto),
    GlobalSearch(proto::GlobalMessageSearchDto),
    MessagesPage {
        chat_id: i64,
        page: proto::MessagesPageDto,
    },
    Profile(Option<Box<proto::ProfileDto>>),
    Members {
        chat_id: i64,
        json: String,
    },
    CommonChats {
        user_id: i64,
        json: String,
    },
    Folders(Vec<proto::FolderDto>),
    Drafts(Vec<crate::domain::DraftRevision>),
    Draft(crate::domain::DraftRevision),
    Scheduled {
        chat_id: i64,
        messages: Vec<proto::ScheduledMessageDto>,
    },
}

pub enum WorkerEvent {
    Ready,
    StartupFailed,
    State(ClientState),
    Response {
        request_id: u64,
        response: WorkerResponse,
    },
    Failed {
        request_id: Option<u64>,
        error: ClientError,
    },
    OutgoingQueued {
        request_id: u64,
        random_id: i64,
    },
    SpecialQueued {
        request_id: u64,
        random_id: i64,
    },
    MediaQueued {
        request_id: u64,
        album_id: i64,
        random_ids: Vec<i64>,
    },
    MediaProgress {
        transfer_id: u64,
        downloaded: u64,
        total: Option<u64>,
    },
    UpdatesCommitted(UpdateBatchDto),
    LiveUpdates(Vec<UpdateEventDto>),
    Stopped,
}

impl std::fmt::Debug for WorkerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready => f.write_str("Ready"),
            Self::StartupFailed => f.write_str("StartupFailed"),
            Self::State(state) => f.debug_tuple("State").field(state).finish(),
            Self::Response { request_id, .. } => f
                .debug_struct("Response")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Self::Failed { request_id, error } => f
                .debug_struct("Failed")
                .field("request_id", request_id)
                .field("error", error)
                .finish(),
            Self::OutgoingQueued {
                request_id,
                random_id,
            } => f
                .debug_struct("OutgoingQueued")
                .field("request_id", request_id)
                .field("random_id", random_id)
                .finish(),
            Self::SpecialQueued {
                request_id,
                random_id,
            } => f
                .debug_struct("SpecialQueued")
                .field("request_id", request_id)
                .field("random_id", random_id)
                .finish(),
            Self::MediaQueued {
                request_id,
                album_id,
                ..
            } => f
                .debug_struct("MediaQueued")
                .field("request_id", request_id)
                .field("album_id", album_id)
                .finish_non_exhaustive(),
            Self::MediaProgress {
                transfer_id,
                downloaded,
                total,
            } => f
                .debug_struct("MediaProgress")
                .field("transfer_id", transfer_id)
                .field("downloaded", downloaded)
                .field("total", total)
                .finish(),
            Self::UpdatesCommitted(_) => f.write_str("UpdatesCommitted(..)"),
            Self::LiveUpdates(_) => f.write_str("LiveUpdates(..)"),
            Self::Stopped => f.write_str("Stopped"),
        }
    }
}

pub mod appearance;
pub mod connectivity;
mod features;
mod guards;
pub mod interactions;
pub mod maintenance;
mod media;
pub mod passkeys;
pub mod web;
use guards::{RequestGuard, RequestGuards};

struct Request {
    id: u64,
    command: WorkerCommand,
    guard: RequestGuard,
}

pub struct Worker {
    commands: Sender<Request>,
    thread: Option<JoinHandle<()>>,
    active_request: Arc<AtomicU64>,
    active_media: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    guards: Arc<RequestGuards>,
}

struct RuntimeResources {
    storage: Storage,
    session_path: PathBuf,
    media_cache: crate::media::cache::MediaCache,
    media_staging: PathBuf,
}

impl Worker {
    pub fn start() -> io::Result<(Self, Receiver<WorkerEvent>)> {
        Self::spawn(|| {
            let paths = AppPaths::discover().map_err(|_| ClientError::Storage)?;
            let storage = Storage::open(&paths.database).map_err(|_| ClientError::Storage)?;
            let media_cache = crate::media::cache::MediaCache::open(
                paths.cache.join("media"),
                crate::media::cache::CacheLimits {
                    total_bytes: u64::MAX,
                    file_bytes: 512 * 1024 * 1024,
                    ..crate::media::cache::CacheLimits::default()
                },
            )
            .map_err(|_| ClientError::Storage)?;
            let media_staging = paths.cache.join("media-staging");
            std::fs::create_dir_all(&media_staging).map_err(|_| ClientError::Storage)?;
            Ok(RuntimeResources {
                storage,
                session_path: paths.database.with_file_name("account-1.session"),
                media_cache,
                media_staging,
            })
        })
    }

    fn spawn(
        initialize: impl FnOnce() -> Result<RuntimeResources, ClientError> + Send + 'static,
    ) -> io::Result<(Self, Receiver<WorkerEvent>)> {
        let (commands, command_rx) = mpsc::channel();
        let (events, event_rx) = mpsc::channel();
        let active_request = Arc::new(AtomicU64::new(0));
        let active_media = Arc::new(AtomicU64::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let guards = Arc::new(RequestGuards::default());
        let runtime_guards = guards.clone();
        let active = active_request.clone();
        let media = active_media.clone();
        let stop = stopping.clone();
        let thread = thread::Builder::new()
            .name("monogram-client".into())
            .spawn(move || {
                let RuntimeResources {
                    storage,
                    session_path,
                    media_cache,
                    media_staging,
                } = match initialize() {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = events.send(WorkerEvent::StartupFailed);
                        return;
                    }
                };
                let mut state = ClientState::default();
                match storage.credentials_metadata() {
                    Ok(Some(meta)) => state.api_id = Some(meta.api_id),
                    Ok(None) => {}
                    Err(_) => {
                        let _ = events.send(WorkerEvent::StartupFailed);
                        return;
                    }
                }
                if events.send(WorkerEvent::Ready).is_err() {
                    return;
                }
                let mut runtime = Runtime {
                    storage,
                    session_path,
                    media_cache,
                    media_staging,
                    client: None,
                    state,
                    phone: None,
                    events,
                    next_update: Instant::now(),
                    active,
                    active_media: media,
                    stopping: stop,
                    guards: runtime_guards,
                };
                runtime.emit(WorkerEvent::State(runtime.state.clone()));
                runtime.run(command_rx);
            })?;
        Ok((
            Self {
                commands,
                thread: Some(thread),
                active_request,
                active_media,
                stopping,
                guards,
            },
            event_rx,
        ))
    }

    pub fn submit(&self, request_id: u64, command: WorkerCommand) -> Result<(), ClientError> {
        if matches!(command, WorkerCommand::Shutdown) {
            self.stopping.store(true, Ordering::Release);
            self.cancel_current();
        } else if self.stopping.load(Ordering::Acquire) {
            return Err(ClientError::Stopped);
        }
        if let WorkerCommand::CancelMedia { transfer_id } = &command {
            self.cancel_media(*transfer_id);
        }
        let guard = self
            .guards
            .register(request_id, None, None, guards::lifecycle(&command))?;
        self.commands
            .send(Request {
                id: request_id,
                command,
                guard,
            })
            .map_err(|_| ClientError::Stopped)
    }

    pub fn cancel_current(&self) {
        let id = self.active_request.load(Ordering::Acquire);
        if id != 0 {
            request_control::cancel(id);
        }
    }

    pub fn cancel_media(&self, transfer_id: u64) {
        if transfer_id != 0 && self.active_media.load(Ordering::Acquire) == transfer_id {
            self.cancel_current();
        }
    }

    pub fn shutdown(mut self) -> Result<(), ClientError> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> Result<(), ClientError> {
        self.stopping.store(true, Ordering::Release);
        self.cancel_current();
        if let Ok(guard) = self.guards.register(0, None, None, false) {
            let _ = self.commands.send(Request {
                id: 0,
                command: WorkerCommand::Shutdown,
                guard,
            });
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| ClientError::Stopped)?;
        }
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

struct ClientHandle(u64);
impl Drop for ClientHandle {
    fn drop(&mut self) {
        proto::registry::destroy_client(self.0);
    }
}

struct RequestScope {
    id: u64,
    previous: u64,
    active: Arc<AtomicU64>,
}

impl RequestScope {
    fn enter(active: &Arc<AtomicU64>, stopping: &AtomicBool) -> Result<Self, ClientError> {
        let id = request_control::create();
        let previous = request_control::bind(id);
        active.store(id, Ordering::Release);
        let scope = Self {
            id,
            previous,
            active: active.clone(),
        };
        if stopping.load(Ordering::Acquire) {
            request_control::cancel(id);
            return Err(ClientError::Cancelled);
        }
        Ok(scope)
    }
}

impl Drop for RequestScope {
    fn drop(&mut self) {
        self.active.store(0, Ordering::Release);
        request_control::bind(self.previous);
        request_control::release(self.id);
    }
}

struct PhoneLogin {
    phone: Zeroizing<String>,
    hash: Zeroizing<String>,
}

struct Runtime {
    storage: Storage,
    session_path: PathBuf,
    media_cache: crate::media::cache::MediaCache,
    media_staging: PathBuf,
    client: Option<ClientHandle>,
    state: ClientState,
    phone: Option<PhoneLogin>,
    events: Sender<WorkerEvent>,
    next_update: Instant,
    active: Arc<AtomicU64>,
    active_media: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    guards: Arc<RequestGuards>,
}

impl Runtime {
    fn emit(&self, event: WorkerEvent) {
        if self.events.send(event).is_err() {
            self.stopping.store(true, Ordering::Release);
        }
    }

    fn run(&mut self, commands: Receiver<Request>) {
        loop {
            if self.stopping.load(Ordering::Acquire) {
                break;
            }
            let timeout = self
                .next_update
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250));
            match commands.recv_timeout(timeout) {
                Ok(Request {
                    command: WorkerCommand::Shutdown,
                    ..
                })
                | Err(RecvTimeoutError::Disconnected) => break,
                Ok(request) => {
                    let class = command_class(&request.command);
                    let result = if self.guards.current(request.guard, request.id) {
                        RequestScope::enter(&self.active, &self.stopping).and_then(|_scope| {
                            scheduler::with_class(class, || {
                                self.execute(request.id, request.command)
                            })
                        })
                    } else {
                        Err(ClientError::Cancelled)
                    };
                    let result = if self.guards.current(request.guard, request.id) {
                        result
                    } else {
                        Err(ClientError::Cancelled)
                    };
                    match result {
                        Ok(response) => self.emit(WorkerEvent::Response {
                            request_id: request.id,
                            response,
                        }),
                        Err(error) => {
                            self.failed_state(&error);
                            self.emit(WorkerEvent::Failed {
                                request_id: Some(request.id),
                                error,
                            });
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= self.next_update {
                self.next_update = Instant::now() + Duration::from_millis(250);
                if matches!(self.state.auth, AuthState::Authorized { .. })
                    && self.state.connection == ConnectionState::Connected
                {
                    let result =
                        RequestScope::enter(&self.active, &self.stopping).and_then(|_scope| {
                            scheduler::with_class(scheduler::RequestClass::BackgroundRead, || {
                                self.drain()
                            })
                        });
                    if let Err(error) = result {
                        let delay = match error {
                            ClientError::FloodWait { seconds } => seconds.max(1),
                            _ => 5,
                        };
                        self.next_update = Instant::now() + Duration::from_secs(delay.min(86_400));
                        self.failed_state(&error);
                        self.emit(WorkerEvent::Failed {
                            request_id: None,
                            error,
                        });
                    }
                }
            }
        }
        self.phone = None;
        self.client = None;
        self.state.connection = ConnectionState::Stopped;
        self.emit(WorkerEvent::State(self.state.clone()));
        self.emit(WorkerEvent::Stopped);
    }

    fn handle(&self) -> Result<u64, ClientError> {
        self.client
            .as_ref()
            .map(|client| client.0)
            .ok_or(ClientError::NotConfigured)
    }

    fn authorized(&self) -> Result<u64, ClientError> {
        let handle = self.handle()?;
        if !matches!(self.state.auth, AuthState::Authorized { .. }) {
            return Err(ClientError::Unauthorized);
        }
        Ok(handle)
    }

    fn failed_state(&mut self, error: &ClientError) {
        if *error == ClientError::PasswordRequired {
            self.state.auth = AuthState::PasswordRequired;
        }
        if *error == ClientError::Unauthorized {
            self.state.auth = AuthState::SignedOut;
        }
        if self.state.connection == ConnectionState::Connecting {
            self.state.connection = ConnectionState::Disconnected;
        }
        self.emit(WorkerEvent::State(self.state.clone()));
    }

    fn signed_in(&mut self, user_id: i64) -> Result<(), ClientError> {
        self.storage
            .set_user(user_id)
            .map_err(|_| ClientError::Storage)?;
        self.phone = None;
        self.state.auth = AuthState::Authorized { user_id };
        self.state.connection = ConnectionState::Connected;
        proto::updates::start_updates(self.handle()?).map_err(ClientError::from)?;
        self.replay()?;
        self.emit(WorkerEvent::State(self.state.clone()));
        self.next_update = Instant::now();
        Ok(())
    }

    fn replay(&mut self) -> Result<(), ClientError> {
        let handle = self.handle()?;
        for batch in proto::updates::pending_update_batches(handle).map_err(ClientError::from)? {
            self.commit_batch(&batch)?;
        }
        Ok(())
    }

    fn commit_batch(&mut self, batch: &UpdateBatchDto) -> Result<(), ClientError> {
        let applied = self
            .storage
            .apply_batch(batch)
            .map_err(|_| ClientError::Storage)?;
        if applied {
            self.emit(WorkerEvent::UpdatesCommitted(batch.clone()));
        }
        proto::updates::acknowledge_update_batch(
            self.handle()?,
            batch.account_generation,
            batch.batch_id,
        )
        .map_err(ClientError::from)
    }

    fn drain(&mut self) -> Result<(), ClientError> {
        self.replay()?;
        let drained = proto::updates::drain_updates(self.handle()?).map_err(ClientError::from)?;
        if drained.replay_required {
            self.replay()?;
        }
        for batch in &drained.batches {
            self.commit_batch(batch)?;
        }
        if !drained.live_events.is_empty() {
            self.emit(WorkerEvent::LiveUpdates(drained.live_events));
        }
        Ok(())
    }

    fn send_pending(&mut self, random_id: i64) -> Result<WorkerResponse, ClientError> {
        let handle = self.authorized()?;
        let pending = self
            .storage
            .pending_send(random_id)
            .map_err(|_| ClientError::Storage)?
            .ok_or(ClientError::InvalidRequest)?;
        pending.message.validate()?;
        self.storage
            .mark_attempted(random_id)
            .map_err(|_| ClientError::Storage)?;
        let message = pending.message;
        let result = proto::dispatch::send_text_message_with_options(
            handle,
            message.chat_id,
            message.text,
            message.reply_to_msg_id,
            message.entities_json,
            message.top_msg_id,
            message.webpage_url,
            random_id,
            SendOptions {
                silent: message.silent,
                no_webpage: message.no_webpage,
                schedule_date: message.schedule_date,
            },
        )
        .map_err(ClientError::from)?;
        self.replay()?;
        Ok(WorkerResponse::Outgoing {
            random_id,
            result: Box::new(result),
        })
    }

    fn execute(
        &mut self,
        request_id: u64,
        command: WorkerCommand,
    ) -> Result<WorkerResponse, ClientError> {
        if matches!(self.state.auth, AuthState::Authorized { .. })
            && matches!(
                &command,
                WorkerCommand::SendCode { .. }
                    | WorkerCommand::ResendCode
                    | WorkerCommand::SignIn { .. }
                    | WorkerCommand::CheckPassword { .. }
            )
        {
            return Err(ClientError::InvalidRequest);
        }
        match command {
            WorkerCommand::ConfigureFromSecrets => {
                return self.execute(
                    request_id,
                    WorkerCommand::Configure(load_api_credentials()?),
                );
            }
            WorkerCommand::GetPreferences => {
                return Ok(WorkerResponse::Preferences(
                    self.storage
                        .preferences()
                        .map_err(|_| ClientError::Storage)?,
                ));
            }
            WorkerCommand::SetPreference {
                namespace,
                expected_version,
                document,
            } => {
                document.validate()?;
                let changed = if let Some(expected_version) = expected_version {
                    self.storage
                        .compare_and_swap_preference(namespace, Some(expected_version), &document)
                        .map_err(|_| ClientError::Storage)?
                } else {
                    self.storage
                        .set_preference(namespace, &document)
                        .map_err(|_| ClientError::Storage)?;
                    true
                };
                return Ok(WorkerResponse::PreferenceChanged(changed));
            }
            WorkerCommand::DeletePreference { namespace } => {
                return Ok(WorkerResponse::PreferenceChanged(
                    self.storage
                        .delete_preference(namespace)
                        .map_err(|_| ClientError::Storage)?,
                ));
            }
            WorkerCommand::Configure(config) => {
                let config = ApiConfig::new(config.api_id, config.api_hash.to_string())?;
                if self
                    .storage
                    .credentials_metadata()
                    .map_err(|_| ClientError::Storage)?
                    .is_some_and(|meta| meta.api_id != config.api_id)
                {
                    return Err(ClientError::InvalidConfiguration);
                }
                let key = Zeroizing::new(
                    crate::platform::session_key().map_err(|_| ClientError::SessionKey)?,
                );
                let path = self
                    .session_path
                    .to_str()
                    .ok_or(ClientError::Session)?
                    .to_owned();
                self.client = None;
                self.phone = None;
                self.state.connection = ConnectionState::Disconnected;
                self.state.auth = AuthState::SignedOut;
                api_invoke::set_init_connection_info(InitConnectionInfo {
                    device_model: "Monogram Desktop".into(),
                    system_version: "Linux / Fedora".into(),
                    app_version: env!("CARGO_PKG_VERSION").into(),
                    system_lang_code: "en".into(),
                    lang_pack: "tdesktop".into(),
                    lang_code: "en".into(),
                });
                let handle = ClientHandle(
                    proto::create_encrypted_client(
                        config.api_id,
                        config.api_hash.to_string(),
                        path,
                        key.to_vec(),
                    )
                    .map_err(|_| ClientError::Session)?,
                );
                self.storage
                    .configure(config.api_id)
                    .map_err(|_| ClientError::Storage)?;
                self.install_media_progress(handle.0)?;
                self.client = Some(handle);
                self.state.api_id = Some(config.api_id);
                self.state.connection = ConnectionState::Disconnected;
                self.state.auth = AuthState::SignedOut;
                self.emit(WorkerEvent::State(self.state.clone()));
            }
            WorkerCommand::Connect => {
                let handle = self.handle()?;
                self.state.connection = ConnectionState::Connecting;
                self.emit(WorkerEvent::State(self.state.clone()));
                proto::connect(handle).map_err(ClientError::from)?;
                self.state.connection = ConnectionState::Connected;
                if proto::is_authorized(handle).map_err(ClientError::from)? {
                    let profile =
                        proto::dispatch::get_profile(handle, 0).map_err(ClientError::from)?;
                    self.signed_in(profile.id)?;
                } else {
                    self.state.auth = AuthState::SignedOut;
                    self.emit(WorkerEvent::State(self.state.clone()));
                }
            }
            WorkerCommand::SendCode { phone } => {
                let phone = Zeroizing::new(phone);
                let sent = proto::send_auth_code(self.handle()?, phone.to_string())
                    .map_err(ClientError::from)?;
                self.phone = Some(PhoneLogin {
                    phone: Zeroizing::new(sent.phone),
                    hash: Zeroizing::new(sent.phone_code_hash),
                });
                self.state.auth = AuthState::CodeRequired {
                    code_type: sent.code_type,
                    code_length: sent.code_length,
                };
                self.emit(WorkerEvent::State(self.state.clone()));
            }
            WorkerCommand::ResendCode => {
                let phone = self.phone.as_ref().ok_or(ClientError::InvalidRequest)?;
                let sent = proto::auth::resend_auth_code(
                    self.handle()?,
                    phone.phone.to_string(),
                    phone.hash.to_string(),
                )
                .map_err(ClientError::from)?;
                self.phone = Some(PhoneLogin {
                    phone: Zeroizing::new(sent.phone),
                    hash: Zeroizing::new(sent.phone_code_hash),
                });
                self.state.auth = AuthState::CodeRequired {
                    code_type: sent.code_type,
                    code_length: sent.code_length,
                };
                self.emit(WorkerEvent::State(self.state.clone()));
            }
            WorkerCommand::SignIn { code } => {
                let code = Zeroizing::new(code);
                let phone = self.phone.as_ref().ok_or(ClientError::InvalidRequest)?;
                let signed = proto::sign_in(
                    self.handle()?,
                    phone.phone.to_string(),
                    phone.hash.to_string(),
                    code.to_string(),
                )
                .map_err(ClientError::from)?;
                self.signed_in(signed.user_id)?;
            }
            WorkerCommand::CheckPassword { password } => {
                let password = Zeroizing::new(password);
                let signed = proto::auth::check_password(self.handle()?, password.to_string())
                    .map_err(ClientError::from)?;
                self.signed_in(signed.user_id)?;
            }
            WorkerCommand::ExportQr => {
                self.phone = None;
                match proto::auth::export_login_token(self.handle()?).map_err(ClientError::from)? {
                    QrLoginDto::Token { token, expires } => {
                        self.state.auth = AuthState::QrPending { expires };
                        self.emit(WorkerEvent::State(self.state.clone()));
                        return Ok(WorkerResponse::QrToken { token, expires });
                    }
                    QrLoginDto::Authorized { session } => self.signed_in(session.user_id)?,
                }
            }
            WorkerCommand::PollQr => {
                if !matches!(self.state.auth, AuthState::QrPending { .. }) {
                    return Err(ClientError::InvalidRequest);
                }
                let changed =
                    proto::auth::poll_login_token(self.handle()?).map_err(ClientError::from)?;
                return Ok(WorkerResponse::QrChanged(changed));
            }
            WorkerCommand::LoadChats => {
                let chats =
                    proto::dispatch::get_chats(self.authorized()?).map_err(ClientError::from)?;
                self.storage
                    .cache_chats(&chats)
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::Chats(chats));
            }
            WorkerCommand::LoadMoreChats {
                offset_date,
                offset_id,
                offset_peer_id,
                folder_id,
            } => {
                let chats = proto::dispatch::load_more_chats(
                    self.authorized()?,
                    offset_date,
                    offset_id,
                    offset_peer_id,
                    folder_id,
                )
                .map_err(ClientError::from)?;
                self.storage
                    .cache_chats(&chats)
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::Chats(chats));
            }
            WorkerCommand::LoadHistory {
                chat_id,
                limit,
                offset_id,
                offset_date,
                add_offset,
            } => {
                let messages = proto::dispatch::get_history(
                    self.authorized()?,
                    chat_id,
                    page_limit(limit)?,
                    offset_id,
                    offset_date,
                    add_offset,
                )
                .map_err(ClientError::from)?;
                self.storage
                    .cache_messages(&messages)
                    .map_err(|_| ClientError::Storage)?;
                let messages = self
                    .storage
                    .project_messages(&messages)
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::History { chat_id, messages });
            }
            WorkerCommand::LoadCachedChats => {
                self.authorized()?;
                return Ok(WorkerResponse::Chats(
                    self.storage.chats().map_err(|_| ClientError::Storage)?,
                ));
            }
            WorkerCommand::LoadCachedHistory { chat_id, limit } => {
                self.authorized()?;
                return Ok(WorkerResponse::History {
                    chat_id,
                    messages: self
                        .storage
                        .messages(chat_id, page_limit(limit)?)
                        .map_err(|_| ClientError::Storage)?,
                });
            }
            WorkerCommand::PendingSends => {
                self.authorized()?;
                return Ok(WorkerResponse::Pending(
                    self.storage.pending().map_err(|_| ClientError::Storage)?,
                ));
            }
            WorkerCommand::SendText(message) => {
                self.authorized()?;
                message.validate()?;
                let random_id = self
                    .storage
                    .enqueue(&message)
                    .map_err(|_| ClientError::Storage)?;
                self.emit(WorkerEvent::OutgoingQueued {
                    request_id,
                    random_id,
                });
                return self.send_pending(random_id);
            }
            WorkerCommand::RetryText { random_id } => return self.send_pending(random_id),
            WorkerCommand::SendSpecial(send) => return self.send_special(request_id, send),
            WorkerCommand::EditText {
                chat_id,
                message_id,
                text,
                entities_json,
            } => {
                valid_message(chat_id, message_id)?;
                let message = proto::dispatch::edit_text_message(
                    self.authorized()?,
                    chat_id,
                    message_id,
                    text,
                    entities_json,
                )
                .map_err(ClientError::from)?;
                self.storage
                    .cache_messages(std::slice::from_ref(&message))
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::Message(Box::new(message)));
            }
            WorkerCommand::DeleteMessage {
                chat_id,
                message_id,
                revoke,
            } => {
                valid_message(chat_id, message_id)?;
                proto::dispatch::delete_message(self.authorized()?, chat_id, message_id, revoke)
                    .map_err(ClientError::from)?;
                self.storage
                    .delete_message(chat_id, message_id)
                    .map_err(|_| ClientError::Storage)?;
            }
            WorkerCommand::Forward {
                from_chat_id,
                message_ids,
                to_chat_id,
                drop_author,
            } => {
                if from_chat_id == 0
                    || to_chat_id == 0
                    || message_ids.is_empty()
                    || message_ids.len() > 100
                    || message_ids.iter().any(|id| *id <= 0)
                {
                    return Err(ClientError::InvalidRequest);
                }
                let messages = proto::dispatch::forward_messages(
                    self.authorized()?,
                    from_chat_id,
                    message_ids,
                    to_chat_id,
                    drop_author,
                )
                .map_err(ClientError::from)?;
                self.storage
                    .cache_messages(&messages)
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::Forwarded(messages));
            }
            WorkerCommand::React {
                chat_id,
                message_id,
                emoticon,
                document_id,
            } => {
                valid_message(chat_id, message_id)?;
                proto::extras::send_reaction(
                    self.authorized()?,
                    chat_id,
                    message_id,
                    emoticon,
                    document_id,
                )
                .map_err(ClientError::from)?;
            }
            WorkerCommand::ClearActiveDialog => {
                proto::updates::clear_active_dialog(self.authorized()?)
                    .map_err(ClientError::from)?
            }
            WorkerCommand::RefreshUpdates => {
                self.authorized()?;
                self.drain()?;
            }
            WorkerCommand::Logout => {
                let remote = proto::auth::logout(self.handle()?).map_err(ClientError::from);
                self.phone = None;
                self.client = None;
                self.state.auth = AuthState::SignedOut;
                self.state.connection = ConnectionState::Unconfigured;
                let session = proto::session_store::FileSessionStore::new(&self.session_path)
                    .clear()
                    .map_err(|_| ClientError::Session);
                let cache = self
                    .storage
                    .clear_account()
                    .map_err(|_| ClientError::Storage);
                let secret = clear_session_secret();
                self.emit(WorkerEvent::State(self.state.clone()));
                secret?;
                session?;
                cache?;
                remote?;
            }
            WorkerCommand::Destroy => {
                self.client = None;
                self.phone = None;
                self.state.auth = AuthState::SignedOut;
                self.state.connection = ConnectionState::Unconfigured;
                self.emit(WorkerEvent::State(self.state.clone()));
            }
            WorkerCommand::Connectivity(command) => {
                return self
                    .execute_connectivity(command)
                    .map(WorkerResponse::Connectivity);
            }
            WorkerCommand::Appearance(command) => {
                return self
                    .execute_appearance(command)
                    .map(WorkerResponse::Appearance);
            }
            WorkerCommand::Maintenance(command) => {
                return self
                    .execute_maintenance(command)
                    .map(WorkerResponse::Maintenance);
            }
            WorkerCommand::Passkey(command) => {
                return self.execute_passkey(command).map(WorkerResponse::Passkey);
            }
            WorkerCommand::Shutdown => return Err(ClientError::Stopped),
            command => return self.execute_media(request_id, command),
        }
        Ok(WorkerResponse::Done)
    }
}

fn load_api_credentials() -> Result<ApiConfig, ClientError> {
    let Ok(source) = std::env::var("SECRETS") else {
        return crate::platform::load_api_credentials();
    };
    let contents = if source.contains("API_HASH=") {
        Zeroizing::new(source)
    } else {
        Zeroizing::new(
            std::fs::read_to_string(source).map_err(|_| ClientError::InvalidConfiguration)?,
        )
    };
    ApiConfig::from_secrets(&contents)
}

fn clear_session_secret() -> Result<(), ClientError> {
    crate::platform::clear_session_secret()
}

fn command_class(command: &WorkerCommand) -> scheduler::RequestClass {
    match command {
        WorkerCommand::ConfigureFromSecrets
        | WorkerCommand::SetPreference { .. }
        | WorkerCommand::DeletePreference { .. }
        | WorkerCommand::SendText(_)
        | WorkerCommand::RetryText { .. }
        | WorkerCommand::SendMedia(_)
        | WorkerCommand::RetryMedia { .. }
        | WorkerCommand::SendSpecial(_)
        | WorkerCommand::RetrySpecial { .. }
        | WorkerCommand::VotePoll { .. }
        | WorkerCommand::RetractPollVote { .. }
        | WorkerCommand::ClosePoll { .. }
        | WorkerCommand::AppendChecklist { .. }
        | WorkerCommand::ToggleChecklistCompleted { .. }
        | WorkerCommand::EditChecklistTitle { .. }
        | WorkerCommand::EditChecklistItem { .. }
        | WorkerCommand::DeleteChecklistItem { .. }
        | WorkerCommand::ReplaceMedia { .. }
        | WorkerCommand::SendSavedGif { .. }
        | WorkerCommand::SendSticker { .. }
        | WorkerCommand::SendCustomEmoji { .. }
        | WorkerCommand::SetCacheLimit { .. }
        | WorkerCommand::ClearMedia(_)
        | WorkerCommand::SetNetworkClass(_)
        | WorkerCommand::SetAutoDownload(_)
        | WorkerCommand::SetAutoplay(_)
        | WorkerCommand::SetPreload(_)
        | WorkerCommand::CancelMedia { .. }
        | WorkerCommand::EditText { .. }
        | WorkerCommand::DeleteMessage { .. }
        | WorkerCommand::Forward { .. }
        | WorkerCommand::React { .. }
        | WorkerCommand::SaveFolder(_)
        | WorkerCommand::DeleteFolder { .. }
        | WorkerCommand::ReorderFolders { .. }
        | WorkerCommand::ReadHistory { .. }
        | WorkerCommand::MarkDialogUnread { .. }
        | WorkerCommand::SaveCloudDraft { .. }
        | WorkerCommand::DiscardCloudDraft { .. }
        | WorkerCommand::SendScheduledNow { .. }
        | WorkerCommand::DeleteScheduled { .. }
        | WorkerCommand::Reschedule { .. }
        | WorkerCommand::EditScheduled { .. }
        | WorkerCommand::Logout => scheduler::RequestClass::InteractiveWrite,
        WorkerCommand::Interaction(command) => interaction_class(command),
        WorkerCommand::Web(_) => scheduler::RequestClass::InteractiveRead,
        WorkerCommand::Connectivity(command) => connectivity::command_class(command),
        WorkerCommand::Appearance(
            appearance::AppearanceCommand::GetWallpapers { .. }
            | appearance::AppearanceCommand::DownloadWallpaper { .. }
            | appearance::AppearanceCommand::GetPreferences,
        ) => scheduler::RequestClass::InteractiveRead,
        WorkerCommand::Appearance(_) => scheduler::RequestClass::InteractiveWrite,
        WorkerCommand::Maintenance(maintenance::MaintenanceCommand::CheckUpdates) => {
            scheduler::RequestClass::BackgroundRead
        }
        WorkerCommand::Maintenance(
            maintenance::MaintenanceCommand::Logs { .. }
            | maintenance::MaintenanceCommand::Stats
            | maintenance::MaintenanceCommand::CrashReport,
        ) => scheduler::RequestClass::InteractiveRead,
        WorkerCommand::Maintenance(_) => scheduler::RequestClass::InteractiveWrite,
        WorkerCommand::Passkey(
            passkeys::PasskeyCommand::Enabled
            | passkeys::PasskeyCommand::InitLogin
            | passkeys::PasskeyCommand::InitRegistration
            | passkeys::PasskeyCommand::List,
        ) => scheduler::RequestClass::InteractiveRead,
        WorkerCommand::Passkey(_) => scheduler::RequestClass::InteractiveWrite,
        WorkerCommand::DownloadMessageMedia {
            mode:
                crate::domain::MediaDownloadMode::Automatic | crate::domain::MediaDownloadMode::Preload,
            ..
        }
        | WorkerCommand::DownloadPeerAvatar {
            mode:
                crate::domain::MediaDownloadMode::Automatic | crate::domain::MediaDownloadMode::Preload,
            ..
        }
        | WorkerCommand::DownloadCustomEmoji {
            mode:
                crate::domain::MediaDownloadMode::Automatic | crate::domain::MediaDownloadMode::Preload,
            ..
        }
        | WorkerCommand::DownloadMediaRange {
            mode:
                crate::domain::MediaDownloadMode::Automatic | crate::domain::MediaDownloadMode::Preload,
            ..
        } => scheduler::RequestClass::BackgroundMedia,
        WorkerCommand::RefreshUpdates => scheduler::RequestClass::BackgroundRead,
        _ => scheduler::RequestClass::InteractiveRead,
    }
}

fn interaction_class(command: &interactions::InteractionCommand) -> scheduler::RequestClass {
    match command {
        interactions::InteractionCommand::SendContact { .. }
        | interactions::InteractionCommand::ImportContacts { .. }
        | interactions::InteractionCommand::SendDice { .. }
        | interactions::InteractionCommand::SendLocation { .. }
        | interactions::InteractionCommand::UpdateLiveLocation { .. }
        | interactions::InteractionCommand::StopLiveLocation { .. }
        | interactions::InteractionCommand::ResumeLiveLocation { .. }
        | interactions::InteractionCommand::SendInlineBotResult { .. }
        | interactions::InteractionCommand::SetGeneralTopicHidden { .. } => {
            scheduler::RequestClass::InteractiveWrite
        }
        _ => scheduler::RequestClass::InteractiveRead,
    }
}

fn valid_message(chat_id: i64, message_id: i32) -> Result<(), ClientError> {
    if chat_id == 0 || message_id <= 0 {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/client.rs"]
mod tests;
