use std::{
    collections::BTreeMap,
    rc::Rc,
    time::{Duration, Instant},
};

use mono_proto_client::{
    ChatDto, ContactsSearchDto, FolderDto, GlobalMessageSearchDto, MessageDto, SearchPeerDto,
};
use relm4::{
    ComponentParts, ComponentSender, Sender, SimpleComponent, adw, adw::prelude::*, gtk, gtk::glib,
};

use crate::i18n::I18n;

pub struct ChatsPage {
    i18n: Rc<I18n>,
    chats: BTreeMap<i64, ChatDto>,
    folders: [FolderState; 2],
    folder: usize,
    custom_folders: Vec<FolderDto>,
    selected_filter: Option<i32>,
    query: String,
    remote: SearchState,
    debounce: Option<glib::SourceId>,
    recipient_mode: bool,
    current_user: Option<i64>,
    loading: bool,
    syncing: bool,
    from_cache: bool,
    offline: bool,
    error: bool,
    pending_folder: Option<usize>,
    replacing: bool,
    typing: BTreeMap<i64, Instant>,
    images: BTreeMap<String, gtk::gdk::Texture>,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSection {
    Contacts,
    Messages,
}

pub enum ChatsInput {
    SetChats(Vec<ChatDto>),
    UpdateChat(ChatDto),
    SetFolders(Vec<FolderDto>),
    SetSearchContacts {
        generation: u64,
        result: ContactsSearchDto,
    },
    SetSearchMessages {
        generation: u64,
        result: GlobalMessageSearchDto,
    },
    SearchFailed {
        generation: u64,
        section: SearchSection,
    },
    SetRecipientMode(bool),
    SearchDebounced(u64),
    RetrySearch,
    LoadMoreSearch,
    OpenSearchPeer(i64),
    OpenMessage {
        chat_id: i64,
        message_id: i32,
    },
    OpenFolders,
    SetLoading(bool),
    SetSyncing(bool),
    SetFromCache(bool),
    SetOffline(bool),
    SetError(bool),
    SetCurrentUser(i64),
    SetTyping {
        chat_id: i64,
        active: bool,
    },
    SetImage {
        cache_key: String,
        texture: gtk::gdk::Texture,
    },
    Clear,
    Refresh,
    LoadMore,
    OpenChat(i64),
    OpenProfile(i64),
    OpenSettings,
    SelectFolder(usize),
    Filter(String),
    Tick,
}

impl std::fmt::Debug for ChatsInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::SetChats(_) => "SetChats",
            Self::UpdateChat(_) => "UpdateChat",
            Self::SetFolders(_) => "SetFolders",
            Self::SetSearchContacts { .. } => "SetSearchContacts",
            Self::SetSearchMessages { .. } => "SetSearchMessages",
            Self::SearchFailed { .. } => "SearchFailed",
            Self::SetRecipientMode(_) => "SetRecipientMode",
            Self::SearchDebounced(_) => "SearchDebounced",
            Self::RetrySearch => "RetrySearch",
            Self::LoadMoreSearch => "LoadMoreSearch",
            Self::OpenSearchPeer(_) => "OpenSearchPeer",
            Self::OpenMessage { .. } => "OpenMessage",
            Self::OpenFolders => "OpenFolders",
            Self::SetLoading(_) => "SetLoading",
            Self::SetSyncing(_) => "SetSyncing",
            Self::SetFromCache(_) => "SetFromCache",
            Self::SetOffline(_) => "SetOffline",
            Self::SetError(_) => "SetError",
            Self::SetCurrentUser(_) => "SetCurrentUser",
            Self::SetTyping { .. } => "SetTyping",
            Self::SetImage { .. } => "SetImage",
            Self::Clear => "Clear",
            Self::Refresh => "Refresh",
            Self::LoadMore => "LoadMore",
            Self::OpenChat(_) => "OpenChat",
            Self::OpenProfile(_) => "OpenProfile",
            Self::OpenSettings => "OpenSettings",
            Self::SelectFolder(_) => "SelectFolder",
            Self::Filter(_) => "Filter",
            Self::Tick => "Tick",
        };
        formatter.write_str(name)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ChatsOutput {
    Refresh,
    LoadMore {
        offset_date: i32,
        offset_id: i32,
        offset_peer_id: i64,
        folder_id: i32,
    },
    SearchContacts {
        generation: u64,
        query: String,
        limit: i32,
    },
    SearchMessages {
        generation: u64,
        query: String,
        offset_rate: i32,
        offset_peer_id: i64,
        offset_id: i32,
        limit: i32,
        folder_id: i32,
    },
    CancelSearch {
        generation: u64,
    },
    OpenMessage {
        chat_id: i64,
        message_id: i32,
    },
    SelectRecipient(i64),
    OpenFolders,
    OpenChat(i64),
    OpenProfile(i64),
    OpenSettings,
}

impl std::fmt::Debug for ChatsOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SearchContacts { generation, .. } => f
                .debug_struct("SearchContacts")
                .field("generation", generation)
                .finish_non_exhaustive(),
            Self::SearchMessages { generation, .. } => f
                .debug_struct("SearchMessages")
                .field("generation", generation)
                .finish_non_exhaustive(),
            Self::CancelSearch { generation } => f
                .debug_struct("CancelSearch")
                .field("generation", generation)
                .finish(),
            Self::Refresh => f.write_str("Refresh"),
            Self::LoadMore { .. } => f.write_str("LoadMore"),
            Self::OpenMessage { .. } => f.write_str("OpenMessage"),
            Self::SelectRecipient(_) => f.write_str("SelectRecipient"),
            Self::OpenFolders => f.write_str("OpenFolders"),
            Self::OpenChat(_) => f.write_str("OpenChat"),
            Self::OpenProfile(_) => f.write_str("OpenProfile"),
            Self::OpenSettings => f.write_str("OpenSettings"),
        }
    }
}

#[derive(Default)]
struct FolderState {
    cursor: Option<Cursor>,
    exhausted: bool,
    initialized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cursor {
    date: i32,
    message_id: i32,
    peer_id: i64,
}

#[derive(Default)]
struct SearchState {
    generation: u64,
    started: bool,
    contacts_loading: bool,
    messages_loading: bool,
    contacts_error: bool,
    messages_error: bool,
    people: Vec<SearchPeerDto>,
    chats: Vec<SearchPeerDto>,
    messages: Vec<MessageDto>,
    cursor: SearchCursor,
    exhausted: bool,
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct SearchCursor {
    rate: i32,
    peer_id: i64,
    message_id: i32,
}

pub struct ChatsWidgets {
    stack: gtk::Stack,
    lists: [gtk::ListBox; 2],
    rows: [BTreeMap<i64, gtk::ListBoxRow>; 2],
    empty: [gtk::Label; 2],
    remote: gtk::Box,
    custom_tabs: gtk::Box,
    status: gtk::Label,
    spinner: gtk::Spinner,
    retry: gtk::Button,
    refresh: gtk::Button,
    more: gtk::Button,
    search: gtk::SearchEntry,
    all: gtk::ToggleButton,
    archive: gtk::ToggleButton,
    revision: u64,
    tick: Option<glib::SourceId>,
}

impl ChatsPage {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            chats: BTreeMap::new(),
            folders: Default::default(),
            folder: 0,
            query: String::new(),
            custom_folders: Vec::new(),
            selected_filter: None,
            remote: SearchState::default(),
            debounce: None,
            recipient_mode: false,
            current_user: None,
            loading: false,
            syncing: false,
            from_cache: false,
            offline: false,
            error: false,
            pending_folder: None,
            replacing: false,
            typing: BTreeMap::new(),
            images: BTreeMap::new(),
            revision: 1,
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_default()
    }

    fn title(&self, chat: &ChatDto) -> String {
        if self.current_user == Some(chat.id) {
            self.text("feature-chats-chats_saved_messages")
        } else if chat.title.trim().is_empty() {
            self.text("feature-chats-chats_untitled")
        } else {
            chat.title.clone()
        }
    }

    fn selected_filter(&self) -> Option<&FolderDto> {
        self.custom_folders
            .iter()
            .find(|filter| Some(filter.id) == self.selected_filter)
    }

    fn matches_filter(chat: &ChatDto, filter: &FolderDto) -> bool {
        if filter.exclude_chat_ids.contains(&chat.id) {
            return false;
        }
        if filter.chat_ids.contains(&chat.id) || filter.pinned_chat_ids.contains(&chat.id) {
            return true;
        }
        !(filter.exclude_muted && chat.muted
            || filter.exclude_read && chat.unread_count == 0 && !chat.unread_mark
            || filter.exclude_archived && chat.archived)
            && if chat.is_bot {
                filter.include_bots
            } else if chat.is_group {
                filter.include_groups
            } else if chat.is_channel {
                filter.include_channels
            } else if chat.is_contact {
                filter.include_contacts
            } else {
                filter.include_non_contacts
            }
    }

    fn visible(&self, folder: usize) -> Vec<&ChatDto> {
        let query = self.query.trim().to_lowercase();
        let filter = (folder == 0).then(|| self.selected_filter()).flatten();
        let mut rows: Vec<_> = self
            .chats
            .values()
            .filter(|chat| {
                filter.map_or(usize::from(chat.archived) == folder, |filter| {
                    Self::matches_filter(chat, filter)
                }) && (query.is_empty() || self.title(chat).to_lowercase().contains(&query))
            })
            .collect();
        rows.sort_by(|a, b| {
            let pinned = |chat: &ChatDto| {
                filter
                    .and_then(|filter| filter.pinned_chat_ids.iter().position(|id| *id == chat.id))
            };
            pinned(a)
                .unwrap_or(usize::MAX)
                .cmp(&pinned(b).unwrap_or(usize::MAX))
                .then_with(|| {
                    if filter.is_none() {
                        b.pinned.cmp(&a.pinned)
                    } else {
                        std::cmp::Ordering::Equal
                    }
                })
                .then_with(|| b.last_message_date.cmp(&a.last_message_date))
                .then_with(|| a.id.cmp(&b.id))
        });
        rows
    }

    fn invalidate_search(&mut self) -> Option<ChatsOutput> {
        if let Some(debounce) = self.debounce.take()
            && let Some(source) = glib::MainContext::default().find_source_by_id(&debounce)
        {
            source.destroy();
        }
        let generation = self.remote.generation;
        let cancel = self
            .remote
            .started
            .then_some(ChatsOutput::CancelSearch { generation });
        self.remote = SearchState {
            generation: generation + 1,
            ..Default::default()
        };
        self.revision += 1;
        cancel
    }

    fn restart_search(&mut self, sender: &ComponentSender<Self>) {
        if let Some(cancel) = self.invalidate_search() {
            let _ = sender.output(cancel);
        }
        if !self.query.trim().is_empty() {
            let generation = self.remote.generation;
            let input = sender.input_sender().clone();
            self.debounce = Some(glib::timeout_add_local_once(
                Duration::from_millis(300),
                move || {
                    input.emit(ChatsInput::SearchDebounced(generation));
                },
            ));
        }
    }

    fn search_messages(&mut self) -> Option<ChatsOutput> {
        if self.offline
            || self.recipient_mode
            || self.query.trim().is_empty()
            || !self.remote.started
            || self.remote.messages_loading
            || self.remote.exhausted
        {
            return None;
        }
        self.remote.messages_loading = true;
        self.remote.messages_error = false;
        self.revision += 1;
        let cursor = self.remote.cursor;
        Some(ChatsOutput::SearchMessages {
            generation: self.remote.generation,
            query: self.query.trim().to_owned(),
            offset_rate: cursor.rate,
            offset_peer_id: cursor.peer_id,
            offset_id: cursor.message_id,
            limit: 50,
            folder_id: self.folder as i32,
        })
    }

    fn start_search(&mut self, generation: u64) -> Vec<ChatsOutput> {
        if generation != self.remote.generation
            || self.remote.started
            || self.offline
            || self.query.trim().is_empty()
        {
            return Vec::new();
        }
        self.remote.started = true;
        self.remote.contacts_loading = true;
        let mut outputs = vec![ChatsOutput::SearchContacts {
            generation,
            query: self.query.trim().to_owned(),
            limit: 50,
        }];
        outputs.extend(self.search_messages());
        self.revision += 1;
        outputs
    }

    fn accept_contacts(&mut self, generation: u64, result: ContactsSearchDto) {
        if generation != self.remote.generation || !self.remote.contacts_loading {
            return;
        }
        let mut seen = std::collections::BTreeSet::new();
        self.remote.people = result
            .people
            .into_iter()
            .filter(|peer| peer.peer_id != 0 && seen.insert(peer.peer_id))
            .collect();
        self.remote.chats = result
            .chats
            .into_iter()
            .filter(|peer| peer.peer_id != 0 && seen.insert(peer.peer_id))
            .collect();
        self.remote.contacts_loading = false;
        self.remote.contacts_error = false;
        self.revision += 1;
    }

    fn accept_messages(&mut self, generation: u64, result: GlobalMessageSearchDto) {
        if generation != self.remote.generation || !self.remote.messages_loading {
            return;
        }
        let next = SearchCursor {
            rate: result.next_rate,
            peer_id: result.next_peer_id,
            message_id: result.next_offset_id,
        };
        self.remote.exhausted = result.messages.is_empty()
            || next.message_id <= 0
            || next.peer_id == 0
            || next == self.remote.cursor;
        self.remote.cursor = next;
        let mut seen: std::collections::BTreeSet<_> = self
            .remote
            .messages
            .iter()
            .map(|message| (message.chat_id, message.id))
            .collect();
        self.remote
            .messages
            .extend(result.messages.into_iter().filter(|message| {
                message.chat_id != 0 && message.id > 0 && seen.insert((message.chat_id, message.id))
            }));
        self.remote.messages_loading = false;
        self.remote.messages_error = false;
        self.revision += 1;
    }

    fn open_peer(&self, id: i64, remote: bool) -> Option<ChatsOutput> {
        if let Some(chat) = self.chats.get(&id) {
            if !chat.can_view
                || self.recipient_mode && (!chat.can_send_plain || !chat.can_forward || chat.left)
            {
                return None;
            }
        } else if !remote {
            return None;
        }
        Some(if self.recipient_mode {
            ChatsOutput::SelectRecipient(id)
        } else {
            ChatsOutput::OpenChat(id)
        })
    }

    fn merge_chat(existing: &ChatDto, mut next: ChatDto) -> ChatDto {
        let existing_stamp = (
            existing.last_message_date.unwrap_or(0),
            existing.last_message_id,
        );
        let next_stamp = (next.last_message_date.unwrap_or(0), next.last_message_id);
        if next.photo_cache_key.is_none() {
            next.photo_cache_key = existing.photo_cache_key.clone();
        }
        if next.last_media_thumb_cache_key.is_none() {
            next.last_media_thumb_cache_key = existing.last_media_thumb_cache_key.clone();
        }
        if next_stamp < existing_stamp {
            next.last_message_id = existing.last_message_id;
            next.last_message_date = existing.last_message_date;
            next.last_message_preview = existing.last_message_preview.clone();
            next.last_media_thumb_cache_key = existing.last_media_thumb_cache_key.clone();
            next.last_message_outgoing = existing.last_message_outgoing;
        } else if next_stamp == existing_stamp {
            if next
                .last_message_preview
                .as_deref()
                .is_none_or(str::is_empty)
            {
                next.last_message_preview = existing.last_message_preview.clone();
            }
            if !next.last_message_outgoing {
                next.last_message_outgoing = existing.last_message_outgoing;
            }
        }
        next
    }

    fn upsert_chat(&mut self, chat: ChatDto, from_cache: bool) {
        if chat.id == 0 {
            return;
        }
        match self.chats.get(&chat.id) {
            Some(_) if from_cache => {}
            Some(existing) => {
                self.chats.insert(chat.id, Self::merge_chat(existing, chat));
            }
            None => {
                self.chats.insert(chat.id, chat);
            }
        }
    }

    fn accept_chats(&mut self, chats: Vec<ChatDto>) {
        let folder = self.pending_folder.unwrap_or(self.folder);
        if !self.from_cache {
            let last = chats.iter().rev().find(|chat| {
                usize::from(chat.archived) == folder
                    && !chat.pinned
                    && chat.id != 0
                    && chat.last_message_id > 0
            });
            let cursor = last.and_then(|chat| {
                Some(Cursor {
                    date: i32::try_from(chat.last_message_date?).ok()?,
                    message_id: chat.last_message_id,
                    peer_id: chat.id,
                })
            });
            let state = &mut self.folders[folder];
            state.exhausted = cursor.is_none()
                || (state.initialized && cursor == state.cursor && !self.replacing);
            state.cursor = cursor.or(state.cursor);
            state.initialized = true;
            if self.replacing {
                self.chats
                    .retain(|_, chat| usize::from(chat.archived) != folder);
            }
            self.pending_folder = None;
            self.replacing = false;
            self.loading = false;
            self.syncing = false;
            self.error = false;
        }
        for chat in chats {
            self.upsert_chat(chat, self.from_cache);
        }
        self.revision += 1;
    }

    fn load_more(&mut self) -> Option<ChatsOutput> {
        if self.loading
            || self.offline
            || self.pending_folder.is_some()
            || self.folders[self.folder].exhausted
        {
            return None;
        }
        let cursor = self.folders[self.folder].cursor.unwrap_or(Cursor {
            date: 0,
            message_id: 0,
            peer_id: 0,
        });
        self.loading = true;
        self.error = false;
        self.from_cache = false;
        self.pending_folder = Some(self.folder);
        Some(ChatsOutput::LoadMore {
            offset_date: cursor.date,
            offset_id: cursor.message_id,
            offset_peer_id: cursor.peer_id,
            folder_id: self.folder as i32,
        })
    }

    fn row_content(&self, chat: &ChatDto, sender: &ComponentSender<Self>) -> gtk::Box {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let title = self.title(chat);
        let avatar = adw::Avatar::new(44, Some(&title), true);
        if let Some(texture) = chat
            .photo_cache_key
            .as_ref()
            .and_then(|key| self.images.get(key))
        {
            avatar.set_custom_image(Some(texture));
        }
        let profile = gtk::Button::new();
        profile.add_css_class("flat");
        profile.set_child(Some(&avatar));
        profile.set_tooltip_text(Some(&self.text("desktop-profile")));
        let id = chat.id;
        let input = sender.input_sender().clone();
        profile.connect_clicked(move |_| input.emit(ChatsInput::OpenProfile(id)));
        content.append(&profile);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 3);
        body.set_hexpand(true);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = ellipsized(&title);
        name.add_css_class("heading");
        name.set_hexpand(true);
        heading.append(&name);
        for (present, icon, key) in [
            (
                chat.is_verified,
                "emblem-ok-symbolic",
                "feature-chats-chats_verified",
            ),
            (
                chat.pinned,
                "view-pin-symbolic",
                "feature-chats-chats_pinned",
            ),
            (
                chat.muted,
                "notifications-disabled-symbolic",
                "feature-chats-chats_muted",
            ),
        ] {
            if present {
                let image = gtk::Image::from_icon_name(icon);
                image.set_tooltip_text(Some(&self.text(key)));
                heading.append(&image);
            }
        }
        let date = chat
            .last_message_date
            .and_then(format_date)
            .unwrap_or_default();
        let date = gtk::Label::new(Some(&date));
        date.add_css_class("dim-label");
        heading.append(&date);
        body.append(&heading);

        let preview = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if let Some(texture) = chat
            .last_media_thumb_cache_key
            .as_ref()
            .and_then(|key| self.images.get(key))
        {
            let image = gtk::Image::from_paintable(Some(texture));
            image.set_pixel_size(32);
            preview.append(&image);
        }
        let typing = self
            .typing
            .get(&id)
            .is_some_and(|until| *until > Instant::now());
        let text = if typing {
            self.text("core-ui-status_typing")
        } else {
            chat.last_message_preview.clone().unwrap_or_default()
        };
        let label = ellipsized(&text);
        label.set_hexpand(true);
        label.add_css_class(if typing { "accent" } else { "dim-label" });
        preview.append(&label);
        if chat.last_message_outgoing && chat.last_message_id > 0 {
            let read = chat.read_outbox_max_id >= chat.last_message_id;
            let mark = gtk::Label::new(Some(if read { "✓✓" } else { "✓" }));
            mark.set_tooltip_text(Some(&self.text(if read {
                "feature-chats-chats_status_read"
            } else {
                "feature-chats-chats_status_sent"
            })));
            preview.append(&mark);
        }
        if chat.unread_count > 0 {
            let count = chat.unread_count.to_string();
            let tooltip = self
                .i18n
                .plural(
                    "feature-chats-chats_unread_count",
                    chat.unread_count as u64,
                    &[("arg1", &count)],
                )
                .unwrap_or_default();
            append_badge(&preview, &count, &tooltip, chat.muted);
        } else if chat.unread_mark {
            append_badge(
                &preview,
                "●",
                &self.text("feature-chats-chats_marked_unread"),
                chat.muted,
            );
        }
        if chat.unread_mentions_count > 0 {
            append_badge(
                &preview,
                &format!("@ {}", chat.unread_mentions_count),
                &self.text("feature-chats-chats_unread_mentions"),
                false,
            );
        }
        if chat.unread_reactions_count > 0 {
            append_badge(
                &preview,
                &format!("♡ {}", chat.unread_reactions_count),
                &self.text("feature-chats-chats_unread_reactions"),
                false,
            );
        }
        body.append(&preview);
        let status = self.peer_status(chat);
        if !status.is_empty() {
            let status = ellipsized(&status);
            status.add_css_class("dim-label");
            body.append(&status);
        }
        content.append(&body);
        content
    }

    fn render_search(&self, parent: &gtk::Box, sender: &ComponentSender<Self>) {
        while let Some(child) = parent.first_child() {
            parent.remove(&child);
        }
        for (key, peers) in [
            ("feature-chats-chats_search_people", &self.remote.people),
            (
                "feature-chats-chats_search_global_chats",
                &self.remote.chats,
            ),
        ] {
            let heading = ellipsized(&self.text(key));
            heading.add_css_class("heading");
            parent.append(&heading);
            for peer in peers {
                let text = peer
                    .username
                    .as_ref()
                    .map(|name| format!("{} · @{}", peer.title, name))
                    .unwrap_or_else(|| peer.title.clone());
                let button = gtk::Button::new();
                button.set_child(Some(&ellipsized(&text)));
                button.set_sensitive(self.open_peer(peer.peer_id, true).is_some());
                let id = peer.peer_id;
                let input = sender.input_sender().clone();
                button.connect_clicked(move |_| input.emit(ChatsInput::OpenSearchPeer(id)));
                parent.append(&button);
            }
            if peers.is_empty() {
                parent.append(&ellipsized(&self.text(
                    if self.remote.contacts_loading || !self.remote.started {
                        "feature-chats-chats_loading"
                    } else if self.remote.contacts_error {
                        "desktop-error-generic"
                    } else {
                        "feature-chats-chats_search_empty"
                    },
                )));
            }
        }
        if !self.recipient_mode {
            let heading = ellipsized(&self.text("feature-chats-chats_search_messages"));
            heading.add_css_class("heading");
            parent.append(&heading);
            for message in &self.remote.messages {
                let body = gtk::Box::new(gtk::Orientation::Vertical, 3);
                if let Some(chat) = self.chats.get(&message.chat_id) {
                    body.append(&ellipsized(&self.title(chat)));
                }
                body.append(&ellipsized(message.text.as_deref().unwrap_or_default()));
                if let Some(date) = format_date(message.date) {
                    body.append(&ellipsized(&date));
                }
                let button = gtk::Button::new();
                button.set_child(Some(&body));
                let chat_id = message.chat_id;
                let message_id = message.id;
                let input = sender.input_sender().clone();
                button.connect_clicked(move |_| {
                    input.emit(ChatsInput::OpenMessage {
                        chat_id,
                        message_id,
                    })
                });
                parent.append(&button);
            }
            if self.remote.messages_loading
                || !self.remote.started
                || self.remote.messages_error
                || self.remote.messages.is_empty()
            {
                parent.append(&ellipsized(&self.text(
                    if self.remote.messages_loading || !self.remote.started {
                        "feature-chats-chats_loading"
                    } else if self.remote.messages_error {
                        "desktop-error-generic"
                    } else {
                        "feature-chats-chats_search_empty"
                    },
                )));
            }
            if self.remote.started && !self.remote.exhausted && !self.remote.messages_error {
                let more = gtk::Button::with_label(
                    &self.text("feature-dialog-dialog_peer_list_load_more"),
                );
                more.set_sensitive(!self.remote.messages_loading && !self.offline);
                let input = sender.input_sender().clone();
                more.connect_clicked(move |_| input.emit(ChatsInput::LoadMoreSearch));
                parent.append(&more);
            }
        }
        if self.remote.contacts_error
            || self.remote.messages_error
            || self.offline
            || !self.remote.started
        {
            let retry = gtk::Button::with_label(&self.text("feature-chats-chats_search_retry"));
            retry.set_sensitive(!self.offline);
            let input = sender.input_sender().clone();
            retry.connect_clicked(move |_| input.emit(ChatsInput::RetrySearch));
            parent.append(&retry);
        }
    }

    fn render_folders(&self, parent: &gtk::Box, sender: &ComponentSender<Self>) {
        while let Some(child) = parent.first_child() {
            parent.remove(&child);
        }
        for folder in &self.custom_folders {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&ellipsized(&folder.title));
            let unread: u64 = self
                .chats
                .values()
                .filter(|chat| Self::matches_filter(chat, folder))
                .map(|chat| u64::try_from(chat.unread_count).unwrap_or(0))
                .sum();
            if unread > 0 {
                let count = unread.to_string();
                let tooltip = self
                    .i18n
                    .plural(
                        "feature-chats-chats_unread_count",
                        unread,
                        &[("arg1", &count)],
                    )
                    .unwrap_or_default();
                append_badge(&content, &count, &tooltip, false);
            }
            let button = gtk::Button::new();
            button.set_child(Some(&content));
            if self.selected_filter == Some(folder.id) {
                button.add_css_class("suggested-action");
            }
            let id = folder.id as usize;
            let input = sender.input_sender().clone();
            button.connect_clicked(move |_| input.emit(ChatsInput::SelectFolder(id)));
            parent.append(&button);
        }
    }

    fn peer_status(&self, chat: &ChatDto) -> String {
        let key = match chat.peer_status.as_deref() {
            Some("online")
                if chat.peer_status_at.is_some_and(|until| {
                    until
                        > glib::DateTime::now_utc()
                            .map(|now| now.to_unix())
                            .unwrap_or(0)
                }) =>
            {
                "core-ui-status_online"
            }
            Some("recently") => "core-ui-status_last_seen_recently",
            Some("last_week") => "core-ui-status_last_seen_week",
            Some("last_month") => "core-ui-status_last_seen_month",
            Some("offline") => {
                return chat
                    .peer_status_at
                    .and_then(format_date)
                    .and_then(|date| {
                        self.i18n
                            .text("core-ui-status_last_seen_at", &[("arg1", &date)])
                            .ok()
                    })
                    .unwrap_or_default();
            }
            _ => return String::new(),
        };
        self.text(key)
    }
}

impl SimpleComponent for ChatsPage {
    type Init = Rc<I18n>;
    type Input = ChatsInput;
    type Output = ChatsOutput;
    type Root = gtk::Box;
    type Widgets = ChatsWidgets;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 6)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Self::new(i18n);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.set_margin_start(12);
        toolbar.set_margin_end(12);
        let title = gtk::Label::new(Some(&model.text("desktop-chats")));
        title.add_css_class("title-2");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        toolbar.append(&title);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(&model.text("feature-chats-chats_refresh")));
        let input = sender.input_sender().clone();
        refresh.connect_clicked(move |_| input.emit(ChatsInput::Refresh));
        toolbar.append(&refresh);
        let settings = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings.set_tooltip_text(Some(&model.text("desktop-settings")));
        let input = sender.input_sender().clone();
        settings.connect_clicked(move |_| input.emit(ChatsInput::OpenSettings));
        toolbar.append(&settings);
        root.append(&toolbar);
        let folders = gtk::Button::from_icon_name("folder-symbolic");
        folders.set_tooltip_text(Some(&model.text("feature-chats-chats_folder_manage")));
        let input = sender.input_sender().clone();
        folders.connect_clicked(move |_| input.emit(ChatsInput::OpenFolders));
        toolbar.append(&folders);
        let search = gtk::SearchEntry::new();
        search.set_search_delay(0);
        search.set_placeholder_text(Some(&model.text("feature-chats-chats_search")));
        search.set_margin_start(12);
        search.set_margin_end(12);
        let input = sender.input_sender().clone();
        search.connect_search_changed(move |search| {
            input.emit(ChatsInput::Filter(search.text().to_string()))
        });
        root.append(&search);
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        tabs.set_halign(gtk::Align::Center);
        let all = gtk::ToggleButton::with_label(&model.text("feature-chats-chats_folder_all"));
        let archive =
            gtk::ToggleButton::with_label(&model.text("feature-chats-chats_folder_archive"));
        archive.set_group(Some(&all));
        all.set_active(true);
        for (folder, button) in [(0, &all), (1, &archive)] {
            let input = sender.input_sender().clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    input.emit(ChatsInput::SelectFolder(folder));
                }
            });
            tabs.append(button);
        }
        root.append(&tabs);
        let custom_tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let tabs_scroll = gtk::ScrolledWindow::new();
        tabs_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        tabs_scroll.set_child(Some(&custom_tabs));
        root.append(&tabs_scroll);
        let banner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        banner.set_margin_start(12);
        banner.set_margin_end(12);
        let spinner = gtk::Spinner::new();
        banner.append(&spinner);
        let status = ellipsized("");
        status.set_hexpand(true);
        banner.append(&status);
        let retry = gtk::Button::with_label(&model.text("desktop-retry"));
        let input = sender.input_sender().clone();
        retry.connect_clicked(move |_| input.emit(ChatsInput::Refresh));
        banner.append(&retry);
        root.append(&banner);
        let stack = gtk::Stack::new();
        stack.set_vexpand(true);
        let lists = std::array::from_fn(|_| gtk::ListBox::new());
        let empty = std::array::from_fn(|_| gtk::Label::new(None));
        for folder in 0..2 {
            lists[folder].set_selection_mode(gtk::SelectionMode::None);
            lists[folder].add_css_class("navigation-sidebar");
            lists[folder].set_placeholder(Some(&empty[folder]));
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_child(Some(&lists[folder]));
            stack.add_named(&scroll, Some(if folder == 0 { "all" } else { "archive" }));
        }
        root.append(&stack);
        let remote = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let remote_scroll = gtk::ScrolledWindow::new();
        remote_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        remote_scroll.set_vexpand(true);
        remote_scroll.set_child(Some(&remote));
        root.append(&remote_scroll);
        let more =
            gtk::Button::with_label(&model.text("feature-dialog-dialog_peer_list_load_more"));
        let input = sender.input_sender().clone();
        more.connect_clicked(move |_| input.emit(ChatsInput::LoadMore));
        root.append(&more);
        let input = sender.input_sender().clone();
        let tick = glib::timeout_add_local(Duration::from_secs(1), move || {
            input.emit(ChatsInput::Tick);
            glib::ControlFlow::Continue
        });
        let mut widgets = ChatsWidgets {
            stack,
            lists,
            rows: Default::default(),
            empty,
            remote,
            custom_tabs,
            status,
            spinner,
            retry,
            refresh,
            more,
            search,
            all,
            archive,
            revision: 0,
            tick: Some(tick),
        };
        model.update_view(&mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        let output = match message {
            ChatsInput::SetChats(chats) => {
                let requested_folder = self.pending_folder;
                self.accept_chats(chats);
                if !self.from_cache
                    && requested_folder.is_some_and(|folder| folder != self.folder)
                    && !self.folders[self.folder].initialized
                {
                    self.load_more()
                } else {
                    None
                }
            }
            ChatsInput::UpdateChat(chat) => {
                if chat.id != 0 {
                    self.upsert_chat(chat, false);
                    self.revision += 1;
                }
                None
            }
            ChatsInput::SetFolders(folders) => {
                let mut seen = std::collections::BTreeSet::new();
                self.custom_folders = folders
                    .into_iter()
                    .filter(|folder| folder.id > 1 && seen.insert(folder.id))
                    .collect();
                if self.selected_filter.is_some() && self.selected_filter().is_none() {
                    self.selected_filter = None;
                    self.restart_search(&sender);
                }
                self.revision += 1;
                None
            }
            ChatsInput::SetSearchContacts { generation, result } => {
                self.accept_contacts(generation, result);
                None
            }
            ChatsInput::SetSearchMessages { generation, result } => {
                self.accept_messages(generation, result);
                None
            }
            ChatsInput::SearchFailed {
                generation,
                section,
            } => {
                if generation == self.remote.generation {
                    match section {
                        SearchSection::Contacts if self.remote.contacts_loading => {
                            self.remote.contacts_loading = false;
                            self.remote.contacts_error = true;
                        }
                        SearchSection::Messages if self.remote.messages_loading => {
                            self.remote.messages_loading = false;
                            self.remote.messages_error = true;
                        }
                        _ => {}
                    }
                    self.revision += 1;
                }
                None
            }
            ChatsInput::SetRecipientMode(value) => {
                if self.recipient_mode != value {
                    self.recipient_mode = value;
                    self.restart_search(&sender);
                }
                None
            }
            ChatsInput::SearchDebounced(generation) => {
                if generation == self.remote.generation {
                    self.debounce = None;
                }
                for output in self.start_search(generation) {
                    let _ = sender.output(output);
                }
                None
            }
            ChatsInput::RetrySearch => {
                if self.offline {
                    return;
                }
                if !self.remote.started {
                    for output in self.start_search(self.remote.generation) {
                        let _ = sender.output(output);
                    }
                } else {
                    if self.remote.contacts_error && !self.remote.contacts_loading {
                        self.remote.contacts_error = false;
                        self.remote.contacts_loading = true;
                        let _ = sender.output(ChatsOutput::SearchContacts {
                            generation: self.remote.generation,
                            query: self.query.trim().to_owned(),
                            limit: 50,
                        });
                    }
                    if self.remote.messages_error
                        && let Some(output) = self.search_messages()
                    {
                        let _ = sender.output(output);
                    }
                }
                self.revision += 1;
                None
            }
            ChatsInput::LoadMoreSearch => self.search_messages(),
            ChatsInput::OpenSearchPeer(id) => self
                .remote
                .people
                .iter()
                .chain(&self.remote.chats)
                .any(|peer| peer.peer_id == id)
                .then(|| self.open_peer(id, true))
                .flatten(),
            ChatsInput::OpenMessage {
                chat_id,
                message_id,
            } => (!self.recipient_mode
                && self
                    .remote
                    .messages
                    .iter()
                    .any(|message| message.chat_id == chat_id && message.id == message_id))
            .then_some(ChatsOutput::OpenMessage {
                chat_id,
                message_id,
            }),
            ChatsInput::OpenFolders => Some(ChatsOutput::OpenFolders),
            ChatsInput::SetLoading(value) => {
                self.loading = value;
                None
            }
            ChatsInput::SetSyncing(value) => {
                self.syncing = value;
                None
            }
            ChatsInput::SetFromCache(value) => {
                self.from_cache = value;
                None
            }
            ChatsInput::SetOffline(value) => {
                if self.offline != value {
                    self.offline = value;
                    self.revision += 1;
                    if !value && !self.remote.started && !self.query.trim().is_empty() {
                        self.restart_search(&sender);
                    }
                }
                None
            }
            ChatsInput::SetError(value) => {
                self.error = value;
                if value {
                    self.loading = false;
                    self.syncing = false;
                    self.pending_folder = None;
                    self.replacing = false;
                }
                None
            }
            ChatsInput::SetCurrentUser(id) => {
                self.current_user = Some(id);
                self.revision += 1;
                None
            }
            ChatsInput::SetTyping { chat_id, active } => {
                if active {
                    self.typing
                        .insert(chat_id, Instant::now() + Duration::from_secs(6));
                } else {
                    self.typing.remove(&chat_id);
                }
                self.revision += 1;
                None
            }
            ChatsInput::SetImage { cache_key, texture } => {
                self.images.insert(cache_key, texture);
                self.revision += 1;
                None
            }
            ChatsInput::Clear => {
                let cancel = self.invalidate_search();
                let generation = self.remote.generation;
                let revision = self.revision;
                *self = Self::new(self.i18n.clone());
                self.remote.generation = generation;
                self.revision = revision;
                cancel
            }
            ChatsInput::Refresh => {
                if !self.query.trim().is_empty() {
                    sender.input(ChatsInput::RetrySearch);
                    return;
                }
                if self.loading || self.pending_folder.is_some() {
                    return;
                }
                self.error = false;
                self.loading = true;
                self.from_cache = false;
                self.replacing = true;
                self.pending_folder = Some(self.folder);
                self.folders[self.folder] = FolderState::default();
                if self.folder == 0 {
                    Some(ChatsOutput::Refresh)
                } else {
                    Some(ChatsOutput::LoadMore {
                        offset_date: 0,
                        offset_id: 0,
                        offset_peer_id: 0,
                        folder_id: 1,
                    })
                }
            }
            ChatsInput::LoadMore => self.load_more(),
            ChatsInput::OpenChat(id) => self.open_peer(id, false),
            ChatsInput::OpenProfile(id) => self
                .chats
                .contains_key(&id)
                .then_some(ChatsOutput::OpenProfile(id)),
            ChatsInput::OpenSettings => Some(ChatsOutput::OpenSettings),
            ChatsInput::SelectFolder(folder) => {
                let filter = if folder > 1 {
                    let Some(filter) = self
                        .custom_folders
                        .iter()
                        .find(|filter| usize::try_from(filter.id).ok() == Some(folder))
                    else {
                        return;
                    };
                    Some(filter.id)
                } else {
                    None
                };
                let cache_folder = if folder == 1 { 1 } else { 0 };
                if self.folder == cache_folder && self.selected_filter == filter {
                    return;
                }
                self.folder = cache_folder;
                self.selected_filter = filter;
                self.restart_search(&sender);
                if !self.folders[cache_folder].initialized {
                    self.load_more()
                } else {
                    None
                }
            }
            ChatsInput::Filter(query) => {
                if self.query != query {
                    self.query = query;
                    self.restart_search(&sender);
                }
                None
            }
            ChatsInput::Tick => {
                let now = Instant::now();
                let before = self.typing.len();
                self.typing.retain(|_, until| *until > now);
                if self.typing.len() != before
                    || self
                        .chats
                        .values()
                        .any(|chat| chat.peer_status.as_deref() == Some("online"))
                {
                    self.revision += 1;
                }
                None
            }
        };
        if let Some(output) = output {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, widgets: &mut Self::Widgets, sender: ComponentSender<Self>) {
        widgets
            .stack
            .set_visible_child_name(if self.folder == 0 { "all" } else { "archive" });
        widgets
            .all
            .set_active(self.folder == 0 && self.selected_filter.is_none());
        widgets.archive.set_active(self.folder == 1);
        if widgets.search.text().as_str() != self.query {
            widgets.search.set_text(&self.query);
        }
        let searching = !self.query.trim().is_empty();
        if let Some(scroll) = widgets
            .remote
            .parent()
            .and_then(|viewport| viewport.parent())
        {
            scroll.set_visible(searching);
        }
        let loading = if searching {
            self.remote.contacts_loading || self.remote.messages_loading
        } else {
            self.loading || self.syncing
        };
        let error = if searching {
            self.remote.contacts_error || self.remote.messages_error
        } else {
            self.error
        };
        widgets.spinner.set_spinning(loading);
        widgets.spinner.set_visible(loading);
        let status = if error {
            "desktop-error-generic"
        } else if self.offline {
            "core-ui-status_network"
        } else if !searching && self.syncing {
            "core-ui-status_syncing"
        } else if loading {
            "feature-chats-chats_loading"
        } else if !searching && self.from_cache {
            "core-ui-status_from_cache"
        } else {
            ""
        };
        widgets.status.set_label(&self.text(status));
        widgets.retry.set_visible(error || self.offline);
        widgets
            .retry
            .set_sensitive(!self.offline && (!loading || searching));
        widgets.refresh.set_sensitive(if searching {
            !loading && !self.offline
        } else {
            !self.loading && self.pending_folder.is_none()
        });
        widgets
            .more
            .set_visible(!self.folders[self.folder].exhausted && !searching);
        widgets
            .more
            .set_sensitive(!self.loading && !self.offline && self.pending_folder.is_none());
        for folder in 0..2 {
            widgets.empty[folder].set_label(&self.text(if self.loading {
                "feature-chats-chats_loading"
            } else if searching {
                "feature-chats-chats_search_empty"
            } else if folder == 1 {
                "feature-chats-chats_archive_empty"
            } else if self.selected_filter.is_some() {
                "feature-chats-chats_folder_empty"
            } else {
                "desktop-empty-chats"
            }));
        }
        if widgets.revision == self.revision {
            return;
        }
        self.render_folders(&widgets.custom_tabs, &sender);
        if searching {
            self.render_search(&widgets.remote, &sender);
        }
        for folder in 0..2 {
            let chats = self.visible(folder);
            let ids: Vec<_> = chats.iter().map(|chat| chat.id).collect();
            let list = &widgets.lists[folder];
            let scroll = list
                .parent()
                .and_downcast::<gtk::Viewport>()
                .and_then(|viewport| viewport.parent())
                .and_downcast::<gtk::ScrolledWindow>();
            let adjustment = scroll.as_ref().map(|scroll| scroll.vadjustment());
            let position = adjustment
                .as_ref()
                .map(|adjustment| adjustment.value())
                .unwrap_or(0.0);
            widgets.rows[folder].retain(|id, row| {
                if ids.contains(id) {
                    true
                } else {
                    list.remove(row);
                    false
                }
            });
            for (index, chat) in chats.iter().enumerate() {
                let row = widgets.rows[folder].entry(chat.id).or_insert_with(|| {
                    let row = gtk::ListBoxRow::new();
                    let id = chat.id;
                    let input = sender.input_sender().clone();
                    row.connect_activate(move |_| input.emit(ChatsInput::OpenChat(id)));
                    row
                });
                row.set_activatable(self.open_peer(chat.id, false).is_some());
                row.set_child(Some(&self.row_content(chat, &sender)));
                if row.index() != index as i32 || row.parent().is_none() {
                    if row.parent().is_some() {
                        list.remove(&*row);
                    }
                    list.insert(&*row, index as i32);
                }
            }
            if let Some(adjustment) = adjustment {
                glib::idle_add_local_once(move || {
                    adjustment.set_value(
                        position.min((adjustment.upper() - adjustment.page_size()).max(0.0)),
                    )
                });
            }
        }
        widgets.revision = self.revision;
    }

    fn shutdown(&mut self, widgets: &mut Self::Widgets, output: Sender<Self::Output>) {
        if let Some(cancel) = self.invalidate_search() {
            let _ = output.send(cancel);
        }
        if let Some(tick) = widgets.tick.take() {
            tick.remove();
        }
    }
}

fn ellipsized(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn append_badge(parent: &gtk::Box, value: &str, tooltip: &str, muted: bool) {
    let label = gtk::Label::new(Some(value));
    label.add_css_class(if muted { "dim-label" } else { "accent" });
    label.add_css_class("numeric");
    label.set_tooltip_text(Some(tooltip));
    parent.append(&label);
}

fn format_date(timestamp: i64) -> Option<String> {
    glib::DateTime::from_unix_local(timestamp)
        .ok()?
        .format("%x %H:%M")
        .ok()
        .map(|date| date.to_string())
}

#[cfg(test)]
#[path = "../../tests/unit/chats_ui.rs"]
mod tests;
