use std::{collections::BTreeMap, fmt, rc::Rc};

use mono_proto_client::{MessageDto, MessagesPageDto, ProfileDto};
use relm4::{adw, adw::prelude::*, gtk, prelude::*};
use serde::Deserialize;

use crate::i18n::I18n;

const PAGE_SIZE: i32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaTab {
    PhotoVideo,
    Document,
    Url,
    Gif,
    Voice,
    Music,
}

impl MediaTab {
    pub const ALL: [Self; 6] = [
        Self::PhotoVideo,
        Self::Document,
        Self::Url,
        Self::Gif,
        Self::Voice,
        Self::Music,
    ];

    pub fn filter(self) -> &'static str {
        match self {
            Self::PhotoVideo => "photo_video",
            Self::Document => "document",
            Self::Url => "url",
            Self::Gif => "gif",
            Self::Voice => "voice",
            Self::Music => "music",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::PhotoVideo => "feature-profile-profile_tab_media",
            Self::Document => "feature-profile-profile_tab_files",
            Self::Url => "feature-profile-profile_tab_links",
            Self::Gif => "feature-profile-profile_tab_gifs",
            Self::Voice => "feature-profile-profile_tab_voice",
            Self::Music => "feature-profile-profile_tab_music",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSection {
    Info,
    Members,
    CommonChats,
    Media(MediaTab),
}

#[derive(Clone, Deserialize)]
pub struct ProfileMember {
    pub id: i64,
    pub peer: i64,
    pub title: String,
    pub username: Option<String>,
    pub role: Option<String>,
    pub rank: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct MembersPage {
    pub count: i32,
    pub members: Vec<ProfileMember>,
}

#[derive(Clone, Deserialize)]
pub struct CommonChat {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Deserialize)]
pub struct CommonChatsPage {
    pub chats: Vec<CommonChat>,
}

#[derive(Clone, Copy)]
struct Request {
    id: u64,
    count_only: bool,
}

struct Page<T> {
    items: Vec<T>,
    count: Option<usize>,
    offset: i32,
    cursor: i64,
    pending: Option<Request>,
    initialized: bool,
    more: bool,
    error: bool,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            count: None,
            offset: 0,
            cursor: 0,
            pending: None,
            initialized: false,
            more: true,
            error: false,
        }
    }
}

impl<T> Page<T> {
    fn matches(&self, id: u64) -> bool {
        self.pending.is_some_and(|request| request.id == id)
    }

    fn fail(&mut self, id: u64) {
        if self.matches(id) {
            self.pending = None;
            self.error = true;
        }
    }
}

pub struct Profile {
    i18n: Rc<I18n>,
    profile: Option<ProfileDto>,
    target: Option<i64>,
    loading: bool,
    error: bool,
    serial: u64,
    pending_profile: Option<u64>,
    selected: ProfileSection,
    members: Page<ProfileMember>,
    common: Page<CommonChat>,
    media: BTreeMap<MediaTab, Page<MessageDto>>,
}

pub enum ProfileInput {
    SetI18n(Rc<I18n>),
    Open(i64),
    Loading,
    SetProfile(Option<ProfileDto>),
    ProfileLoaded {
        request_id: u64,
        profile: ProfileDto,
    },
    MembersLoaded {
        request_id: u64,
        page: MembersPage,
    },
    CommonChatsLoaded {
        request_id: u64,
        page: CommonChatsPage,
    },
    SearchMessagesPage {
        request_id: u64,
        page: MessagesPageDto,
    },
    Failed {
        request_id: u64,
    },
    SelectSection(ProfileSection),
    Retry,
    LoadMore,
    Back,
    OpenChat,
    OpenPeer(i64),
    OpenMessage {
        chat_id: i64,
        message_id: i32,
    },
}

impl fmt::Debug for ProfileInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SetI18n(_) => "SetI18n",
            Self::Open(_) => "Open",
            Self::Loading => "Loading",
            Self::SetProfile(_) => "SetProfile",
            Self::ProfileLoaded { .. } => "ProfileLoaded",
            Self::MembersLoaded { .. } => "MembersLoaded",
            Self::CommonChatsLoaded { .. } => "CommonChatsLoaded",
            Self::SearchMessagesPage { .. } => "SearchMessagesPage",
            Self::Failed { .. } => "Failed",
            Self::SelectSection(_) => "SelectSection",
            Self::Retry => "Retry",
            Self::LoadMore => "LoadMore",
            Self::Back => "Back",
            Self::OpenChat => "OpenChat",
            Self::OpenPeer(_) => "OpenPeer",
            Self::OpenMessage { .. } => "OpenMessage",
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileOutput {
    Back,
    OpenChat(i64),
    OpenMessage {
        chat_id: i64,
        message_id: i32,
    },
    LoadProfile {
        request_id: u64,
        chat_id: i64,
    },
    LoadMembers {
        request_id: u64,
        chat_id: i64,
        offset: i32,
        limit: i32,
    },
    LoadCommonChats {
        request_id: u64,
        user_id: i64,
        max_id: i64,
        limit: i32,
    },
    SearchMessagesPage {
        request_id: u64,
        chat_id: i64,
        filter: &'static str,
        offset_id: i32,
        limit: i32,
    },
}

impl Profile {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            profile: None,
            target: None,
            loading: false,
            error: false,
            serial: 0,
            pending_profile: None,
            selected: ProfileSection::Info,
            members: Page::default(),
            common: Page::default(),
            media: MediaTab::ALL
                .into_iter()
                .map(|tab| (tab, Page::default()))
                .collect(),
        }
    }

    fn reset(&mut self, target: Option<i64>) {
        self.profile = None;
        self.target = target;
        self.loading = false;
        self.error = false;
        self.pending_profile = None;
        self.selected = ProfileSection::Info;
        self.members = Page::default();
        self.common = Page::default();
        self.media = MediaTab::ALL
            .into_iter()
            .map(|tab| (tab, Page::default()))
            .collect();
    }

    fn next_request(&mut self) -> u64 {
        self.serial = self
            .serial
            .checked_add(1)
            .expect("profile request id exhausted");
        self.serial
    }

    fn extra(&self) -> serde_json::Value {
        self.profile
            .as_ref()
            .and_then(|profile| profile.extra_json.as_deref())
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default()
    }

    fn visible(&self, section: ProfileSection) -> bool {
        let Some(profile) = &self.profile else {
            return false;
        };
        let extra = self.extra();
        match section {
            ProfileSection::Info => true,
            ProfileSection::Members => {
                let group = matches!(profile.kind.as_str(), "chat" | "group" | "supergroup");
                let allowed = extra
                    .get("participants")
                    .and_then(serde_json::Value::as_bool);
                (group && allowed != Some(false)
                    || profile.kind == "channel" && allowed == Some(true))
                    && self.members.count.or_else(|| {
                        extra
                            .get("members")
                            .and_then(serde_json::Value::as_u64)
                            .map(|count| count as usize)
                    }) != Some(0)
            }
            ProfileSection::CommonChats => {
                profile.kind == "user"
                    && !profile.is_self
                    && self.common.count.or_else(|| {
                        extra
                            .get("common")
                            .and_then(serde_json::Value::as_u64)
                            .map(|count| count as usize)
                    }) != Some(0)
            }
            ProfileSection::Media(tab) => self
                .media
                .get(&tab)
                .is_some_and(|page| page.count != Some(0)),
        }
    }

    fn load_profile(&mut self) -> Option<ProfileOutput> {
        let chat_id = self.target?;
        if self.pending_profile.is_some() {
            return None;
        }
        let request_id = self.next_request();
        self.pending_profile = Some(request_id);
        self.loading = true;
        self.error = false;
        Some(ProfileOutput::LoadProfile {
            request_id,
            chat_id,
        })
    }

    fn load_section(&mut self, section: ProfileSection, more: bool) -> Option<ProfileOutput> {
        let chat_id = self.profile.as_ref()?.id;
        if !self.visible(section) && self.selected != section {
            return None;
        }
        let request_id = self.next_request();
        let request = Request {
            id: request_id,
            count_only: false,
        };
        match section {
            ProfileSection::Info => None,
            ProfileSection::Members => {
                let page = &mut self.members;
                if page.pending.is_some()
                    || more && !page.more
                    || !more && page.initialized && !page.error
                {
                    return None;
                }
                page.pending = Some(request);
                page.error = false;
                Some(ProfileOutput::LoadMembers {
                    request_id,
                    chat_id,
                    offset: page.offset,
                    limit: PAGE_SIZE,
                })
            }
            ProfileSection::CommonChats => {
                let page = &mut self.common;
                if page.pending.is_some()
                    || more && !page.more
                    || !more && page.initialized && !page.error
                {
                    return None;
                }
                page.pending = Some(request);
                page.error = false;
                Some(ProfileOutput::LoadCommonChats {
                    request_id,
                    user_id: chat_id,
                    max_id: page.cursor,
                    limit: PAGE_SIZE,
                })
            }
            ProfileSection::Media(tab) => {
                let page = self.media.get_mut(&tab)?;
                if page.pending.is_some()
                    || more && !page.more
                    || !more && page.initialized && !page.error
                {
                    return None;
                }
                page.pending = Some(request);
                page.error = false;
                Some(ProfileOutput::SearchMessagesPage {
                    request_id,
                    chat_id,
                    filter: tab.filter(),
                    offset_id: page.cursor as i32,
                    limit: PAGE_SIZE,
                })
            }
        }
    }

    fn accept_profile(&mut self, profile: ProfileDto) -> Vec<ProfileOutput> {
        let id = profile.id;
        self.reset(Some(id));
        self.profile = Some(profile);
        let mut outputs = Vec::new();
        for tab in MediaTab::ALL {
            let request_id = self.next_request();
            self.media.get_mut(&tab).expect("known media tab").pending = Some(Request {
                id: request_id,
                count_only: true,
            });
            outputs.push(ProfileOutput::SearchMessagesPage {
                request_id,
                chat_id: id,
                filter: tab.filter(),
                offset_id: 0,
                limit: 1,
            });
        }
        outputs
    }

    fn accept_members(&mut self, request_id: u64, response: MembersPage) {
        let page = &mut self.members;
        if !page.matches(request_id) {
            return;
        }
        page.pending = None;
        page.initialized = true;
        page.error = false;
        page.count = Some(response.count.max(0) as usize);
        let length = response.members.len();
        let before = page.items.len();
        for member in response.members {
            if member.peer == 0 {
                continue;
            }
            if let Some(old) = page.items.iter_mut().find(|old| old.peer == member.peer) {
                *old = member;
            } else {
                page.items.push(member);
            }
        }
        page.offset = page.offset.saturating_add(length as i32);
        page.more = length >= PAGE_SIZE as usize
            && page.items.len() > before
            && page.offset < response.count;
    }

    fn accept_common(&mut self, request_id: u64, response: CommonChatsPage) {
        if !self.common.matches(request_id) {
            return;
        }
        let expected = self
            .extra()
            .get("common")
            .and_then(serde_json::Value::as_u64)
            .map(|count| count as usize);
        let page = &mut self.common;
        page.pending = None;
        page.initialized = true;
        page.error = false;
        let length = response.chats.len();
        let before = page.items.len();
        let cursor = response.chats.last().map(|chat| {
            if chat.id < -1_000_000_000_000 {
                chat.id.saturating_neg() - 1_000_000_000_000
            } else {
                chat.id.saturating_abs()
            }
        });
        for chat in response.chats {
            if chat.id == 0 {
                continue;
            }
            if let Some(old) = page.items.iter_mut().find(|old| old.id == chat.id) {
                *old = chat;
            } else {
                page.items.push(chat);
            }
        }
        page.more = length >= PAGE_SIZE as usize
            && page.items.len() > before
            && cursor.is_some_and(|cursor| cursor != page.cursor)
            && expected.is_none_or(|count| page.items.len() < count);
        if let Some(cursor) = cursor {
            page.cursor = cursor;
        }
        page.count = if page.more {
            expected
        } else {
            Some(page.items.len())
        };
    }

    fn accept_media(
        &mut self,
        request_id: u64,
        response: MessagesPageDto,
    ) -> Option<ProfileOutput> {
        let tab = *self
            .media
            .iter()
            .find(|(_, page)| page.matches(request_id))?
            .0;
        let chat_id = self.profile.as_ref()?.id;
        let page = self.media.get_mut(&tab)?;
        let request = page.pending.take()?;
        page.count = Some(response.total_count.max(0) as usize);
        page.error = false;
        if request.count_only {
            page.more = response.total_count > 0;
            page.initialized = response.total_count == 0;
            if self.selected == ProfileSection::Media(tab) && page.more {
                return self.load_section(self.selected, false);
            }
            return None;
        }
        let length = response.messages.len();
        let before = page.items.len();
        let cursor = response
            .messages
            .iter()
            .filter(|message| message.chat_id == chat_id && message.id > 0)
            .map(|message| message.id)
            .min();
        for message in response.messages {
            if message.chat_id != chat_id || message.id <= 0 {
                continue;
            }
            if let Some(old) = page.items.iter_mut().find(|old| old.id == message.id) {
                *old = message;
            } else {
                page.items.push(message);
            }
        }
        page.items
            .sort_by_key(|message| std::cmp::Reverse(message.id));
        page.more = length >= PAGE_SIZE as usize
            && page.items.len() > before
            && cursor.is_some_and(|cursor| page.cursor == 0 || i64::from(cursor) < page.cursor)
            && page.items.len() < response.total_count.max(0) as usize;
        if let Some(cursor) = cursor {
            page.cursor = i64::from(cursor);
        }
        page.initialized = true;
        None
    }

    fn handle(&mut self, message: ProfileInput) -> Vec<ProfileOutput> {
        let mut outputs = Vec::new();
        match message {
            ProfileInput::SetI18n(i18n) => self.i18n = i18n,
            ProfileInput::Open(id) if id != 0 => {
                self.reset(Some(id));
                outputs.extend(self.load_profile());
            }
            ProfileInput::Open(_) => {}
            ProfileInput::Loading => {
                self.reset(None);
                self.loading = true;
            }
            ProfileInput::SetProfile(Some(profile)) if profile.id != 0 => {
                outputs.extend(self.accept_profile(profile));
            }
            ProfileInput::SetProfile(_) => self.reset(None),
            ProfileInput::ProfileLoaded {
                request_id,
                profile,
            } => {
                if self.pending_profile == Some(request_id) {
                    if self.target == Some(profile.id) && profile.id != 0 {
                        outputs.extend(self.accept_profile(profile));
                    } else {
                        self.pending_profile = None;
                        self.loading = false;
                        self.error = true;
                    }
                }
            }
            ProfileInput::MembersLoaded { request_id, page } => {
                self.accept_members(request_id, page)
            }
            ProfileInput::CommonChatsLoaded { request_id, page } => {
                self.accept_common(request_id, page)
            }
            ProfileInput::SearchMessagesPage { request_id, page } => {
                outputs.extend(self.accept_media(request_id, page))
            }
            ProfileInput::Failed { request_id } => {
                if self.pending_profile == Some(request_id) {
                    self.pending_profile = None;
                    self.loading = false;
                    self.error = true;
                }
                self.members.fail(request_id);
                self.common.fail(request_id);
                for page in self.media.values_mut() {
                    page.fail(request_id);
                }
            }
            ProfileInput::SelectSection(section) => {
                if self.visible(section) || self.selected == section {
                    self.selected = section;
                    outputs.extend(self.load_section(section, false));
                }
            }
            ProfileInput::Retry => {
                if self.error && self.profile.is_none() {
                    outputs.extend(self.load_profile());
                } else {
                    outputs.extend(self.load_section(self.selected, false));
                }
            }
            ProfileInput::LoadMore => outputs.extend(self.load_section(self.selected, true)),
            ProfileInput::Back => {
                self.reset(None);
                outputs.push(ProfileOutput::Back);
            }
            ProfileInput::OpenChat => {
                if let Some(profile) = &self.profile {
                    outputs.push(ProfileOutput::OpenChat(profile.id));
                }
            }
            ProfileInput::OpenPeer(id) => {
                if id != 0
                    && (self.members.items.iter().any(|member| member.peer == id)
                        || self.common.items.iter().any(|chat| chat.id == id))
                {
                    outputs.push(ProfileOutput::OpenChat(id));
                }
            }
            ProfileInput::OpenMessage {
                chat_id,
                message_id,
            } => {
                if self.media.values().any(|page| {
                    page.items
                        .iter()
                        .any(|message| message.chat_id == chat_id && message.id == message_id)
                }) {
                    outputs.push(ProfileOutput::OpenMessage {
                        chat_id,
                        message_id,
                    });
                }
            }
        }
        outputs
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }

    fn detail(&self, group: &adw::PreferencesGroup, key: &str, value: &str) {
        if value.trim().is_empty() {
            return;
        }
        let row = adw::ActionRow::builder()
            .title(self.text(key))
            .subtitle(value)
            .subtitle_selectable(true)
            .use_markup(false)
            .build();
        group.add(&row);
    }

    fn action(
        &self,
        label: &str,
        sender: &ComponentSender<Self>,
        message: impl Fn() -> ProfileInput + 'static,
    ) -> gtk::Button {
        let button = gtk::Button::new();
        let title = gtk::Label::builder().label(label).wrap(true).build();
        button.set_child(Some(&title));
        let input = sender.input_sender().clone();
        button.connect_clicked(move |_| input.emit(message()));
        button
    }

    fn section_title(&self, section: ProfileSection) -> String {
        let (key, count) = match section {
            ProfileSection::Info => ("feature-profile-profile_info", None),
            ProfileSection::Members => (
                if self
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.kind == "channel")
                {
                    "feature-profile-profile_tab_subscribers"
                } else {
                    "feature-profile-profile_tab_members"
                },
                self.members.count.or_else(|| {
                    self.extra()
                        .get("members")
                        .and_then(serde_json::Value::as_u64)
                        .map(|count| count as usize)
                }),
            ),
            ProfileSection::CommonChats => (
                "feature-profile-profile_tab_common_groups",
                self.common.count.or_else(|| {
                    self.extra()
                        .get("common")
                        .and_then(serde_json::Value::as_u64)
                        .map(|count| count as usize)
                }),
            ),
            ProfileSection::Media(tab) => {
                (tab.key(), self.media.get(&tab).and_then(|page| page.count))
            }
        };
        let title = self.text(key);
        count.map_or(title.clone(), |count| format!("{title} · {count}"))
    }

    fn tabs(&self, page: &adw::PreferencesPage, sender: &ComponentSender<Self>) {
        let group = adw::PreferencesGroup::new();
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for section in [
            ProfileSection::Info,
            ProfileSection::Members,
            ProfileSection::CommonChats,
        ]
        .into_iter()
        .chain(MediaTab::ALL.map(ProfileSection::Media))
        {
            if !self.visible(section) && self.selected != section {
                continue;
            }
            let button = self.action(&self.section_title(section), sender, move || {
                ProfileInput::SelectSection(section)
            });
            if self.selected == section {
                button.add_css_class("suggested-action");
            }
            tabs.append(&button);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&tabs)
            .build();
        group.add(&scroll);
        page.add(&group);
    }

    fn panel_status<T>(
        &self,
        group: &adw::PreferencesGroup,
        page: &Page<T>,
        sender: &ComponentSender<Self>,
    ) {
        if page.pending.is_some() {
            let spinner = gtk::Spinner::builder()
                .spinning(true)
                .halign(gtk::Align::Center)
                .build();
            spinner.set_tooltip_text(Some(&self.text("desktop-loading")));
            group.add(&spinner);
        } else if page.error {
            let error = gtk::Label::builder()
                .label(self.text("feature-profile-profile_panel_error"))
                .wrap(true)
                .build();
            group.add(&error);
            group.add(&self.action(
                &self.text("feature-profile-profile_panel_retry"),
                sender,
                || ProfileInput::Retry,
            ));
        } else if page.items.is_empty() && page.initialized {
            group.add(&gtk::Label::new(Some(
                &self.text("feature-profile-profile_panel_empty"),
            )));
        } else if page.more && page.initialized {
            group.add(&self.action(
                &self.text("feature-profile-profile_show_more"),
                sender,
                || ProfileInput::LoadMore,
            ));
        }
    }

    fn render_panel(&self, page: &adw::PreferencesPage, sender: &ComponentSender<Self>) {
        let group = adw::PreferencesGroup::builder()
            .title(self.section_title(self.selected))
            .build();
        match self.selected {
            ProfileSection::Members => {
                for member in &self.members.items {
                    let id = member.peer;
                    let role = match member.role.as_deref() {
                        Some("owner") => Some(self.text("feature-profile-profile_role_owner")),
                        Some("admin") => Some(self.text("feature-profile-profile_role_admin")),
                        _ => None,
                    };
                    let title = role.map_or_else(
                        || member.title.clone(),
                        |role| format!("{} · {role}", member.title),
                    );
                    group.add(&self.action(&title, sender, move || ProfileInput::OpenPeer(id)));
                }
                self.panel_status(&group, &self.members, sender);
            }
            ProfileSection::CommonChats => {
                for chat in &self.common.items {
                    let id = chat.id;
                    group
                        .add(&self.action(&chat.title, sender, move || ProfileInput::OpenPeer(id)));
                }
                self.panel_status(&group, &self.common, sender);
            }
            ProfileSection::Media(tab) => {
                if let Some(state) = self.media.get(&tab) {
                    for message in &state.items {
                        let chat_id = message.chat_id;
                        let message_id = message.id;
                        let title = message
                            .file_name
                            .as_deref()
                            .filter(|name| !name.trim().is_empty())
                            .or_else(|| {
                                message
                                    .text
                                    .as_deref()
                                    .filter(|text| !text.trim().is_empty())
                            })
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                format!("{} · {}", self.text(tab.key()), message.id)
                            });
                        group.add(&self.action(&title, sender, move || {
                            ProfileInput::OpenMessage {
                                chat_id,
                                message_id,
                            }
                        }));
                    }
                    self.panel_status(&group, state, sender);
                }
            }
            ProfileSection::Info => {}
        }
        page.add(&group);
    }

    fn render(&self, root: &gtk::Box, sender: ComponentSender<Self>) {
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        let header = adw::HeaderBar::new();
        let title = gtk::Label::new(Some(&self.text("desktop-profile")));
        header.set_title_widget(Some(&title));
        let back = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(ProfileInput::Back));
        header.pack_start(&back);
        root.append(&header);

        if self.loading || self.profile.is_none() {
            let status = adw::StatusPage::builder()
                .vexpand(true)
                .title(self.text(if self.loading {
                    "feature-profile-profile_loading"
                } else if self.error {
                    "feature-profile-profile_panel_error"
                } else {
                    "feature-profile-profile_panel_empty"
                }))
                .build();
            if self.loading {
                let spinner = gtk::Spinner::builder()
                    .spinning(true)
                    .halign(gtk::Align::Center)
                    .build();
                status.set_child(Some(&spinner));
            } else if self.error && self.target.is_some() {
                status.set_child(Some(&self.action(
                    &self.text("feature-profile-profile_panel_retry"),
                    &sender,
                    || ProfileInput::Retry,
                )));
            } else {
                status.set_icon_name(Some("avatar-default-symbolic"));
            }
            root.append(&status);
            return;
        }

        let Some(profile) = &self.profile else {
            return;
        };
        let page = adw::PreferencesPage::builder().vexpand(true).build();
        let identity = adw::PreferencesGroup::new();
        let title = gtk::Label::builder()
            .label(&profile.title)
            .wrap(true)
            .selectable(true)
            .margin_top(12)
            .margin_bottom(12)
            .build();
        title.add_css_class("title-1");
        identity.add(&title);
        if profile.is_self {
            let badge = gtk::Label::new(Some(&self.text("feature-profile-profile_self_badge")));
            badge.add_css_class("dim-label");
            identity.add(&badge);
        }
        if profile.id != 0 {
            let open_key = match profile.kind.as_str() {
                "channel" => "feature-profile-profile_open_channel",
                "chat" | "group" | "supergroup" => "feature-profile-profile_open_group",
                _ => "feature-profile-profile_message",
            };
            let open = gtk::Button::with_label(&self.text(open_key));
            open.add_css_class("suggested-action");
            let input = sender.input_sender().clone();
            open.connect_clicked(move |_| input.emit(ProfileInput::OpenChat));
            identity.add(&open);
        }
        page.add(&identity);
        self.tabs(&page, &sender);
        if self.selected != ProfileSection::Info {
            self.render_panel(&page, &sender);
            root.append(&page);
            return;
        }

        let info = adw::PreferencesGroup::builder()
            .title(self.text("feature-profile-profile_info"))
            .build();
        self.detail(
            &info,
            "feature-profile-profile_id_label",
            &profile.id.to_string(),
        );
        let kind_key = match profile.kind.as_str() {
            "bot" => Some("feature-profile-profile_kind_bot"),
            "channel" => Some("feature-profile-profile_kind_channel"),
            "chat" | "group" | "supergroup" => Some("feature-profile-profile_kind_group"),
            "user" => Some("feature-profile-profile_kind_user"),
            _ => None,
        };
        if let Some(key) = kind_key {
            self.detail(&info, "feature-profile-profile_type_label", &self.text(key));
        }
        if let Some(username) = profile.username.as_deref().filter(|name| !name.is_empty()) {
            let value = self
                .i18n
                .text("feature-profile-profile_username", &[("arg1", username)])
                .unwrap_or_else(|_| username.into());
            self.detail(&info, "feature-profile-profile_username_label", &value);
        }
        if let Some(about) = &profile.about {
            self.detail(&info, "feature-profile-profile_about_label", about);
        }
        if let Some(extra) = profile
            .extra_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        {
            if let Some(phone) = extra.get("phone").and_then(serde_json::Value::as_str) {
                self.detail(&info, "feature-profile-profile_phone_label", phone);
            }
            for (field, key) in [
                (
                    "members",
                    if profile.kind == "channel" {
                        "feature-profile-profile_subscribers_label"
                    } else {
                        "feature-profile-profile_members_label"
                    },
                ),
                ("online", "feature-profile-profile_online_label"),
                ("common", "feature-profile-profile_common_chats_label"),
            ] {
                if let Some(value) = extra.get(field).and_then(serde_json::Value::as_u64) {
                    self.detail(&info, key, &value.to_string());
                }
            }
            for (field, key) in [
                ("verified", "feature-profile-profile_verified"),
                ("scam", "feature-profile-profile_scam"),
            ] {
                if extra.get(field).and_then(serde_json::Value::as_bool) == Some(true) {
                    let badge = gtk::Label::new(Some(&self.text(key)));
                    info.add(&badge);
                }
            }
        }
        page.add(&info);
        root.append(&page);
    }
}

impl SimpleComponent for Profile {
    type Init = Rc<I18n>;
    type Input = ProfileInput;
    type Output = ProfileOutput;
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
        let model = Self::new(i18n);
        model.render(&root, sender);
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
        self.render(root, sender);
    }
}
