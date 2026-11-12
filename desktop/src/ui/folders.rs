use std::{collections::BTreeMap, rc::Rc};

use mono_proto_client::{ChatDto, FolderDto};
use relm4::{adw, adw::prelude::*, gtk, prelude::*};

use crate::i18n::I18n;

pub struct FoldersPage {
    i18n: Rc<I18n>,
    folders: Vec<FolderDto>,
    chats: BTreeMap<i64, ChatDto>,
    draft: Option<FolderDto>,
    query: String,
    busy: bool,
    loaded: bool,
    from_cache: bool,
    error: Option<String>,
    revision: u64,
}

pub enum FoldersInput {
    SetFolders(Vec<FolderDto>),
    SetChats(Vec<ChatDto>),
    SetBusy(bool),
    SetFromCache(bool),
    SetI18n(Rc<I18n>),
    Error(String),
    Back,
    Refresh,
    New,
    Edit(i32),
    Cancel,
    Title(String),
    Emoticon(String),
    Flag(usize, bool),
    Query(String),
    Include(i64),
    Exclude(i64),
    RemoveInclude(i64),
    RemoveExclude(i64),
    Save,
    Delete,
    Move(usize, usize),
}

impl std::fmt::Debug for FoldersInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FoldersInput(..)")
    }
}

pub enum FoldersOutput {
    Save(FolderDto),
    Delete(i32),
    Reorder(Vec<i32>),
    Back,
    Refresh,
}

impl std::fmt::Debug for FoldersOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Save(_) => "Save(..)",
            Self::Delete(_) => "Delete(..)",
            Self::Reorder(_) => "Reorder(..)",
            Self::Back => "Back",
            Self::Refresh => "Refresh",
        })
    }
}

const FLAGS: [&str; 8] = [
    "feature-folders-folders_include_contacts",
    "feature-folders-folders_include_non_contacts",
    "feature-folders-folders_include_groups",
    "feature-folders-folders_include_channels",
    "feature-folders-folders_include_bots",
    "feature-folders-folders_exclude_muted",
    "feature-folders-folders_exclude_read",
    "feature-folders-folders_exclude_archived",
];

fn empty_folder(id: i32) -> FolderDto {
    FolderDto {
        id,
        title: String::new(),
        emoticon: String::new(),
        chat_ids: Vec::new(),
        exclude_chat_ids: Vec::new(),
        pinned_chat_ids: Vec::new(),
        include_contacts: false,
        include_non_contacts: false,
        include_groups: false,
        include_channels: false,
        include_bots: false,
        exclude_muted: false,
        exclude_read: false,
        exclude_archived: false,
    }
}

fn flags(folder: &FolderDto) -> [bool; 8] {
    [
        folder.include_contacts,
        folder.include_non_contacts,
        folder.include_groups,
        folder.include_channels,
        folder.include_bots,
        folder.exclude_muted,
        folder.exclude_read,
        folder.exclude_archived,
    ]
}

fn contains(folder: &FolderDto, chat: &ChatDto) -> bool {
    if folder.id == 0 {
        return !chat.archived;
    }
    if folder.exclude_chat_ids.contains(&chat.id) {
        return false;
    }
    if folder.chat_ids.contains(&chat.id) {
        return true;
    }
    let attention = chat.unread_mentions_count > 0 || chat.unread_reactions_count > 0;
    if (folder.exclude_archived && chat.archived)
        || (folder.exclude_muted && chat.muted && !attention)
        || (folder.exclude_read && chat.unread_count <= 0 && !chat.unread_mark && !attention)
    {
        return false;
    }
    (chat.is_channel && folder.include_channels)
        || (chat.is_group && folder.include_groups)
        || (chat.is_bot && folder.include_bots)
        || (!chat.is_channel && !chat.is_group && chat.is_contact && folder.include_contacts)
        || (!chat.is_channel
            && !chat.is_group
            && !chat.is_contact
            && !chat.is_bot
            && folder.include_non_contacts)
}

impl FoldersPage {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            folders: Vec::new(),
            chats: BTreeMap::new(),
            draft: None,
            query: String::new(),
            busy: false,
            loaded: false,
            from_cache: false,
            error: None,
            revision: 1,
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_default()
    }

    fn accept_folders(&mut self, folders: Vec<FolderDto>) {
        let mut seen = std::collections::BTreeSet::new();
        self.folders = folders
            .into_iter()
            .filter(|f| (f.id == 0 || f.id > 1) && seen.insert(f.id))
            .collect();
        if !seen.contains(&0) {
            self.folders.insert(0, empty_folder(0));
        }
        self.loaded = true;
        self.busy = false;
        self.error = None;
        self.revision += 1;
    }

    fn picker(&self) -> Vec<&ChatDto> {
        let query = self.query.trim().to_lowercase();
        self.chats
            .values()
            .filter(|chat| {
                chat.title.to_lowercase().contains(&query) || chat.id.to_string() == query
            })
            .take(24)
            .collect()
    }

    fn valid(&self) -> bool {
        self.draft.as_ref().is_some_and(|f| {
            f.id > 1
                && !f.title.trim().is_empty()
                && f.title.chars().count() <= 8
                && f.emoticon.chars().count() <= 8
        })
    }

    fn handle(&mut self, input: FoldersInput) -> Option<FoldersOutput> {
        match input {
            FoldersInput::SetFolders(folders) => self.accept_folders(folders),
            FoldersInput::SetChats(chats) => {
                for chat in chats {
                    if chat.id != 0 {
                        self.chats.insert(chat.id, chat);
                    }
                }
                self.revision += 1;
            }
            FoldersInput::SetBusy(value) => self.busy = value,
            FoldersInput::SetFromCache(value) => self.from_cache = value,
            FoldersInput::SetI18n(i18n) => {
                self.i18n = i18n;
                self.revision += 1;
            }
            FoldersInput::Error(error) => {
                self.error = Some(error);
                self.busy = false;
            }
            FoldersInput::Back => return Some(FoldersOutput::Back),
            FoldersInput::Refresh if !self.busy => {
                self.busy = true;
                self.error = None;
                return Some(FoldersOutput::Refresh);
            }
            _ if self.busy => {}
            FoldersInput::New => {
                if let Some(id) = self
                    .folders
                    .iter()
                    .map(|f| f.id)
                    .max()
                    .unwrap_or(1)
                    .max(1)
                    .checked_add(1)
                {
                    self.draft = Some(empty_folder(id));
                    self.query.clear();
                    self.revision += 1;
                }
            }
            FoldersInput::Edit(id) if id > 1 => {
                self.draft = self.folders.iter().find(|f| f.id == id).cloned();
                self.query.clear();
                self.revision += 1;
            }
            FoldersInput::Cancel => {
                self.draft = None;
                self.revision += 1;
            }
            FoldersInput::Title(title) => {
                if let Some(f) = &mut self.draft {
                    f.title = title;
                }
            }
            FoldersInput::Emoticon(emoticon) => {
                if let Some(f) = &mut self.draft {
                    f.emoticon = emoticon;
                }
            }
            FoldersInput::Flag(index, value) => {
                if let Some(f) = &mut self.draft {
                    match index {
                        0 => f.include_contacts = value,
                        1 => f.include_non_contacts = value,
                        2 => f.include_groups = value,
                        3 => f.include_channels = value,
                        4 => f.include_bots = value,
                        5 => f.exclude_muted = value,
                        6 => f.exclude_read = value,
                        7 => f.exclude_archived = value,
                        _ => {}
                    }
                }
            }
            FoldersInput::Query(query) => {
                self.query = query;
                self.revision += 1;
            }
            FoldersInput::Include(id) | FoldersInput::Exclude(id)
                if self.chats.contains_key(&id) =>
            {
                let exclude = matches!(input, FoldersInput::Exclude(_));
                if let Some(f) = &mut self.draft {
                    let ids = if exclude {
                        &mut f.exclude_chat_ids
                    } else {
                        &mut f.chat_ids
                    };
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                    self.revision += 1;
                }
            }
            FoldersInput::RemoveInclude(id) => {
                if let Some(f) = &mut self.draft {
                    f.chat_ids.retain(|peer| *peer != id);
                    self.revision += 1;
                }
            }
            FoldersInput::RemoveExclude(id) => {
                if let Some(f) = &mut self.draft {
                    f.exclude_chat_ids.retain(|peer| *peer != id);
                    self.revision += 1;
                }
            }
            FoldersInput::Save if self.valid() => {
                self.busy = true;
                self.error = None;
                return self.draft.clone().map(FoldersOutput::Save);
            }
            FoldersInput::Delete => {
                if let Some(f) = &self.draft
                    && self.folders.iter().any(|saved| saved.id == f.id)
                {
                    self.busy = true;
                    self.error = None;
                    return Some(FoldersOutput::Delete(f.id));
                }
            }
            FoldersInput::Move(from, to)
                if from < self.folders.len() && to < self.folders.len() && from != to =>
            {
                let folder = self.folders.remove(from);
                self.folders.insert(to, folder);
                self.revision += 1;
                self.busy = true;
                self.error = None;
                return Some(FoldersOutput::Reorder(
                    self.folders.iter().map(|f| f.id).collect(),
                ));
            }
            _ => {}
        }
        None
    }
}

pub struct FoldersWidgets {
    heading: gtk::Label,
    status: gtk::Label,
    spinner: gtk::Spinner,
    list: gtk::Box,
    editor: gtk::Box,
    title: gtk::Entry,
    emoticon: gtk::Entry,
    flags: Vec<gtk::CheckButton>,
    search: gtk::SearchEntry,
    peers: gtk::Box,
    save: gtk::Button,
    delete: gtk::Button,
    add: gtk::Button,
    refresh: gtk::Button,
    revision: u64,
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn button(label: &str, input: FoldersInput, sender: &ComponentSender<FoldersPage>) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    let input = Rc::new(input);
    let sender = sender.input_sender().clone();
    button.connect_clicked(move |_| {
        let message = match input.as_ref() {
            FoldersInput::Back => FoldersInput::Back,
            FoldersInput::Refresh => FoldersInput::Refresh,
            FoldersInput::New => FoldersInput::New,
            FoldersInput::Cancel => FoldersInput::Cancel,
            FoldersInput::Save => FoldersInput::Save,
            FoldersInput::Delete => FoldersInput::Delete,
            FoldersInput::Edit(id) => FoldersInput::Edit(*id),
            FoldersInput::Move(a, b) => FoldersInput::Move(*a, *b),
            FoldersInput::Include(id) => FoldersInput::Include(*id),
            FoldersInput::Exclude(id) => FoldersInput::Exclude(*id),
            FoldersInput::RemoveInclude(id) => FoldersInput::RemoveInclude(*id),
            FoldersInput::RemoveExclude(id) => FoldersInput::RemoveExclude(*id),
            _ => return,
        };
        sender.emit(message);
    });
    button
}

impl SimpleComponent for FoldersPage {
    type Init = Rc<I18n>;
    type Input = FoldersInput;
    type Output = FoldersOutput;
    type Root = gtk::Box;
    type Widgets = FoldersWidgets;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 8)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Self::new(i18n);
        let header = adw::HeaderBar::new();
        let heading = gtk::Label::new(Some(&model.text("feature-folders-folders_title")));
        header.set_title_widget(Some(&heading));
        header.pack_start(&button(
            &model.text("desktop-back"),
            FoldersInput::Back,
            &sender,
        ));
        root.append(&header);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        root.append(&status);
        let spinner = gtk::Spinner::new();
        root.append(&spinner);
        let refresh = button(
            &model.text("feature-folders-folders_refresh"),
            FoldersInput::Refresh,
            &sender,
        );
        root.append(&refresh);
        let add = button(
            &model.text("feature-folders-folders_add"),
            FoldersInput::New,
            &sender,
        );
        root.append(&add);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        root.append(&scroll);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        scroll.set_child(Some(&body));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.append(&list);
        let editor = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&editor);
        let title = gtk::Entry::new();
        title.set_max_length(8);
        editor.append(&title);
        let input = sender.input_sender().clone();
        title.connect_changed(move |entry| {
            input.emit(FoldersInput::Title(entry.text().to_string()))
        });
        let emoticon = gtk::Entry::new();
        emoticon.set_max_length(8);
        editor.append(&emoticon);
        let input = sender.input_sender().clone();
        emoticon.connect_changed(move |entry| {
            input.emit(FoldersInput::Emoticon(entry.text().to_string()))
        });
        let mut checks = Vec::new();
        for (index, key) in FLAGS.iter().enumerate() {
            let check = gtk::CheckButton::with_label(&model.text(key));
            let input = sender.input_sender().clone();
            check.connect_toggled(move |check| {
                input.emit(FoldersInput::Flag(index, check.is_active()))
            });
            editor.append(&check);
            checks.push(check);
        }
        let search = gtk::SearchEntry::new();
        editor.append(&search);
        let input = sender.input_sender().clone();
        search.connect_search_changed(move |entry| {
            input.emit(FoldersInput::Query(entry.text().to_string()))
        });
        let peers = gtk::Box::new(gtk::Orientation::Vertical, 6);
        editor.append(&peers);
        let save = button(
            &model.text("feature-folders-folders_save"),
            FoldersInput::Save,
            &sender,
        );
        editor.append(&save);
        let delete = button(
            &model.text("feature-folders-folders_delete"),
            FoldersInput::Delete,
            &sender,
        );
        editor.append(&delete);
        editor.append(&button(
            &model.text("desktop-cancel"),
            FoldersInput::Cancel,
            &sender,
        ));
        let mut widgets = FoldersWidgets {
            heading,
            status,
            spinner,
            list,
            editor,
            title,
            emoticon,
            flags: checks,
            search,
            peers,
            save,
            delete,
            add,
            refresh,
            revision: 0,
        };
        SimpleComponent::update_view(&model, &mut widgets, sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        if let Some(output) = self.handle(input) {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, widgets: &mut Self::Widgets, sender: ComponentSender<Self>) {
        widgets
            .heading
            .set_label(&self.text("feature-folders-folders_title"));
        widgets
            .status
            .set_label(&self.error.clone().unwrap_or_else(|| {
                self.text(if self.busy {
                    "feature-folders-folders_loading"
                } else if self.from_cache {
                    "core-ui-status_from_cache"
                } else if self.loaded && self.folders.len() <= 1 {
                    "feature-folders-folders_empty"
                } else if !self.loaded {
                    "feature-folders-folders_loading"
                } else {
                    return String::new();
                })
            }));
        widgets
            .spinner
            .set_spinning(self.busy || (!self.loaded && self.error.is_none()));
        widgets.editor.set_visible(self.draft.is_some());
        widgets.editor.set_sensitive(!self.busy);
        widgets.add.set_sensitive(!self.busy && self.loaded);
        widgets.refresh.set_sensitive(!self.busy);
        widgets.save.set_sensitive(!self.busy && self.valid());
        widgets.delete.set_sensitive(
            !self.busy
                && self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| self.folders.iter().any(|f| f.id == draft.id)),
        );
        widgets.list.set_sensitive(!self.busy);
        if widgets.revision == self.revision {
            return;
        }
        widgets.revision = self.revision;
        clear(&widgets.list);
        for (index, folder) in self.folders.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let title = if folder.id == 0 {
                self.text("feature-folders-folders_all_chats")
            } else {
                format!("{} {}", folder.emoticon, folder.title)
            };
            let count = self
                .chats
                .values()
                .filter(|chat| contains(folder, chat))
                .count()
                .to_string();
            let count = self
                .i18n
                .text("feature-folders-folders_chat_count", &[("arg1", &count)])
                .unwrap_or_default();
            let label = gtk::Label::new(Some(&format!("{title} · {count}")));
            label.set_hexpand(true);
            label.set_wrap(true);
            row.append(&label);
            if folder.id > 1 {
                row.append(&button(
                    &self.text("feature-folders-folders_name"),
                    FoldersInput::Edit(folder.id),
                    &sender,
                ));
            }
            for (icon, to, enabled) in [
                ("go-up-symbolic", index.saturating_sub(1), index > 0),
                (
                    "go-down-symbolic",
                    index + 1,
                    index + 1 < self.folders.len(),
                ),
            ] {
                let move_button = button("", FoldersInput::Move(index, to), &sender);
                move_button.set_icon_name(icon);
                move_button.set_tooltip_text(Some(&self.text("feature-folders-folders_reorder")));
                move_button.set_sensitive(enabled);
                row.append(&move_button);
            }
            widgets.list.append(&row);
        }
        if let Some(draft) = &self.draft {
            widgets
                .title
                .set_placeholder_text(Some(&self.text("feature-folders-folders_name")));
            widgets
                .emoticon
                .set_placeholder_text(Some(&self.text("feature-folders-folders_emoji")));
            if widgets.title.text().as_str() != draft.title {
                widgets.title.set_text(&draft.title);
            }
            if widgets.emoticon.text().as_str() != draft.emoticon {
                widgets.emoticon.set_text(&draft.emoticon);
            }
            for (index, value) in flags(draft).into_iter().enumerate() {
                widgets.flags[index].set_active(value);
            }
            widgets
                .search
                .set_placeholder_text(Some(&self.text("feature-folders-folders_search_chats")));
            if widgets.search.text().as_str() != self.query {
                widgets.search.set_text(&self.query);
            }
            clear(&widgets.peers);
            for (key, ids, exclude) in [
                (
                    "feature-folders-folders_include_chats",
                    &draft.chat_ids,
                    false,
                ),
                (
                    "feature-folders-folders_exclude_chats",
                    &draft.exclude_chat_ids,
                    true,
                ),
            ] {
                widgets
                    .peers
                    .append(&gtk::Label::new(Some(&self.text(key))));
                for id in ids {
                    let name = self
                        .chats
                        .get(id)
                        .map(|chat| chat.title.clone())
                        .unwrap_or_else(|| id.to_string());
                    let action = if exclude {
                        FoldersInput::RemoveExclude(*id)
                    } else {
                        FoldersInput::RemoveInclude(*id)
                    };
                    let remove = button(&name, action, &sender);
                    remove.set_tooltip_text(Some(&self.text("desktop-cancel")));
                    widgets.peers.append(&remove);
                }
            }
            for chat in self.picker() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let label = gtk::Label::new(Some(&chat.title));
                label.set_hexpand(true);
                label.set_wrap(true);
                row.append(&label);
                let include = button(
                    &self.text("feature-folders-folders_include_chats"),
                    FoldersInput::Include(chat.id),
                    &sender,
                );
                include.set_sensitive(!draft.chat_ids.contains(&chat.id));
                row.append(&include);
                let exclude = button(
                    &self.text("feature-folders-folders_exclude"),
                    FoldersInput::Exclude(chat.id),
                    &sender,
                );
                exclude.set_sensitive(!draft.exclude_chat_ids.contains(&chat.id));
                row.append(&exclude);
                widgets.peers.append(&row);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/folders_ui.rs"]
mod tests;
