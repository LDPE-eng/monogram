use std::{cell::Cell, fmt, path::PathBuf, rc::Rc};

use relm4::{adw, adw::prelude::*, gtk, gtk::gio, prelude::*};

use crate::{
    client::maintenance::{MaintenanceCommand, MaintenanceResponse},
    i18n::I18n,
    platform::{
        diagnostics::{DebugStats, LogEntry},
        tdlib::{ImportSummary, ImportToken},
        update::UpdateInfo,
    },
};

pub struct Maintenance {
    i18n: Rc<I18n>,
    logs: Vec<LogEntry>,
    stats: Option<DebugStats>,
    update: Option<UpdateInfo>,
    import_summary: Option<ImportSummary>,
    exported: Option<PathBuf>,
    error: Option<String>,
    busy: bool,
    import_token: Option<ImportToken>,
    rendering: Rc<Cell<bool>>,
}

pub enum MaintenanceInput {
    Back,
    SetI18n(Rc<I18n>),
    Response(MaintenanceResponse),
    Failed(String),
    RefreshLogs,
    RefreshStats,
    ClearLogs,
    ExportLogs,
    CheckUpdates,
    OpenInstaller,
    ImportTdlib { source: PathBuf },
    CancelImport,
}

pub enum MaintenanceOutput {
    Back,
    Command(MaintenanceCommand),
}

impl fmt::Debug for MaintenanceInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Back => "Back",
            Self::SetI18n(_) => "SetI18n",
            Self::Response(_) => "Response",
            Self::Failed(_) => "Failed",
            Self::RefreshLogs => "RefreshLogs",
            Self::RefreshStats => "RefreshStats",
            Self::ClearLogs => "ClearLogs",
            Self::ExportLogs => "ExportLogs",
            Self::CheckUpdates => "CheckUpdates",
            Self::OpenInstaller => "OpenInstaller",
            Self::ImportTdlib { .. } => "ImportTdlib",
            Self::CancelImport => "CancelImport",
        })
    }
}

impl fmt::Debug for MaintenanceOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Back => "Back",
            Self::Command(_) => "Command(..)",
        })
    }
}

impl Maintenance {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            logs: Vec::new(),
            stats: None,
            update: None,
            import_summary: None,
            exported: None,
            error: None,
            busy: false,
            import_token: None,
            rendering: Rc::new(Cell::new(false)),
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }

    fn text_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n.text(key, args).unwrap_or_else(|_| key.into())
    }

    fn crash_text(&self) -> String {
        self.logs
            .iter()
            .map(LogEntry::render)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn handle(&mut self, message: MaintenanceInput) -> Vec<MaintenanceOutput> {
        match message {
            MaintenanceInput::Back => vec![MaintenanceOutput::Back],
            MaintenanceInput::SetI18n(i18n) => {
                self.i18n = i18n;
                Vec::new()
            }
            MaintenanceInput::Response(response) => self.apply_response(response),
            MaintenanceInput::Failed(error) => {
                self.busy = false;
                self.error = Some(error);
                Vec::new()
            }
            MaintenanceInput::RefreshLogs => {
                self.busy = true;
                self.error = None;
                vec![MaintenanceOutput::Command(MaintenanceCommand::Logs {
                    level: None,
                    query: String::new(),
                })]
            }
            MaintenanceInput::RefreshStats => {
                self.busy = true;
                self.error = None;
                vec![MaintenanceOutput::Command(MaintenanceCommand::Stats)]
            }
            MaintenanceInput::ClearLogs => {
                vec![MaintenanceOutput::Command(MaintenanceCommand::ClearLogs)]
            }
            MaintenanceInput::ExportLogs => {
                vec![MaintenanceOutput::Command(MaintenanceCommand::ExportLogs)]
            }
            MaintenanceInput::CheckUpdates => {
                self.busy = true;
                self.error = None;
                vec![MaintenanceOutput::Command(MaintenanceCommand::CheckUpdates)]
            }
            MaintenanceInput::OpenInstaller => {
                vec![MaintenanceOutput::Command(
                    MaintenanceCommand::OpenInstaller,
                )]
            }
            MaintenanceInput::ImportTdlib { source } => {
                let token = ImportToken::new();
                self.import_token = Some(token.clone());
                self.busy = true;
                self.error = None;
                vec![MaintenanceOutput::Command(
                    MaintenanceCommand::ImportTdlib { source, token },
                )]
            }
            MaintenanceInput::CancelImport => {
                if let Some(token) = &self.import_token {
                    token.cancel();
                }
                Vec::new()
            }
        }
    }

    fn apply_response(&mut self, response: MaintenanceResponse) -> Vec<MaintenanceOutput> {
        self.busy = false;
        match response {
            MaintenanceResponse::Logs(logs) => {
                self.logs = logs;
                Vec::new()
            }
            MaintenanceResponse::Stats(stats) => {
                self.stats = Some(stats);
                Vec::new()
            }
            MaintenanceResponse::Cleared => {
                self.logs.clear();
                Vec::new()
            }
            MaintenanceResponse::Exported(path) => {
                self.exported = Some(path);
                Vec::new()
            }
            MaintenanceResponse::CrashReport(_) => Vec::new(),
            MaintenanceResponse::Update(info) => {
                self.update = Some(info);
                Vec::new()
            }
            MaintenanceResponse::Installed => Vec::new(),
            MaintenanceResponse::Imported(summary) => {
                self.import_summary = Some(summary);
                self.import_token = None;
                Vec::new()
            }
        }
    }

    fn render(&self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        self.rendering.set(true);
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(
            &self.text("desktop-maintenance"),
        ))));
        let back = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(MaintenanceInput::Back));
        header.pack_start(&back);
        root.append(&header);

        let page = adw::PreferencesPage::builder().vexpand(true).build();
        page.add(&self.logs_group(sender));
        page.add(&self.stats_group(sender));
        page.add(&self.update_group(sender));
        page.add(&self.import_group(sender));
        page.add(&self.crash_group(sender));
        root.append(&page);
        self.rendering.set(false);
    }

    fn logs_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-maintenance-logs"))
            .build();
        let subtitle = if let Some(error) = &self.error {
            error.clone()
        } else if let Some(exported) = &self.exported {
            self.text_args(
                "desktop-maintenance-exported",
                &[("arg1", &exported.display().to_string())],
            )
        } else {
            self.text_args(
                "desktop-maintenance-log-count",
                &[("arg1", &self.logs.len().to_string())],
            )
        };
        let row = adw::ActionRow::builder().subtitle(subtitle).build();
        let refresh = gtk::Button::with_label(&self.text("desktop-maintenance-view-logs"));
        let clear = gtk::Button::with_label(&self.text("desktop-maintenance-clear-logs"));
        let export = gtk::Button::with_label(&self.text("desktop-maintenance-export-logs"));
        refresh.set_sensitive(!self.busy);
        clear.set_sensitive(!self.busy);
        export.set_sensitive(!self.busy);
        row.add_suffix(&refresh);
        row.add_suffix(&clear);
        row.add_suffix(&export);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        let refresh_input = input.clone();
        let refresh_rendering = rendering.clone();
        refresh.connect_clicked(move |_| {
            if !refresh_rendering.get() {
                refresh_input.emit(MaintenanceInput::RefreshLogs);
            }
        });
        let clear_input = input.clone();
        let clear_rendering = rendering.clone();
        clear.connect_clicked(move |_| {
            if !clear_rendering.get() {
                clear_input.emit(MaintenanceInput::ClearLogs);
            }
        });
        let export_rendering = rendering.clone();
        export.connect_clicked(move |_| {
            if !export_rendering.get() {
                input.emit(MaintenanceInput::ExportLogs);
            }
        });
        group.add(&row);
        group
    }

    fn stats_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-maintenance-debug-stats"))
            .build();
        let subtitle = match &self.stats {
            Some(stats) => self.text_args(
                "desktop-maintenance-version",
                &[("arg1", stats.app_version.as_str())],
            ),
            None => self.text("desktop-maintenance-not-checked"),
        };
        let row = adw::ActionRow::builder().subtitle(subtitle).build();
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_sensitive(!self.busy);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        refresh.connect_clicked(move |_| {
            if !rendering.get() {
                input.emit(MaintenanceInput::RefreshStats);
            }
        });
        row.add_suffix(&refresh);
        group.add(&row);
        group
    }

    fn update_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-maintenance-update"))
            .build();
        let subtitle = match &self.update {
            Some(info) => match &info.available_version {
                Some(version) => {
                    self.text_args("desktop-maintenance-update-available", &[("arg1", version)])
                }
                None => self.text("desktop-maintenance-up-to-date"),
            },
            None => self.text("desktop-maintenance-not-checked"),
        };
        let row = adw::ActionRow::builder().subtitle(subtitle).build();
        let check = gtk::Button::with_label(&self.text("desktop-maintenance-check-updates"));
        let install = gtk::Button::with_label(&self.text("desktop-maintenance-open-installer"));
        check.set_sensitive(!self.busy);
        row.add_suffix(&check);
        row.add_suffix(&install);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        check.connect_clicked({
            let input = input.clone();
            let rendering = rendering.clone();
            move |_| {
                if !rendering.get() {
                    input.emit(MaintenanceInput::CheckUpdates);
                }
            }
        });
        install.connect_clicked(move |_| {
            if !rendering.get() {
                input.emit(MaintenanceInput::OpenInstaller);
            }
        });
        group.add(&row);
        group
    }

    fn import_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-maintenance-import"))
            .build();
        let subtitle = match &self.import_summary {
            Some(summary) => self.text_args(
                "desktop-maintenance-import-done",
                &[
                    ("arg1", &summary.files.to_string()),
                    ("arg2", &summary.bytes.to_string()),
                ],
            ),
            None => self.text("desktop-maintenance-not-checked"),
        };
        let row = adw::ActionRow::builder().subtitle(subtitle).build();
        let import = gtk::Button::with_label(&self.text("desktop-maintenance-import-tdlib"));
        let cancel = gtk::Button::with_label(&self.text("desktop-maintenance-import-cancel"));
        import.set_sensitive(!self.busy);
        cancel.set_sensitive(self.import_token.is_some());
        row.add_suffix(&import);
        row.add_suffix(&cancel);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        import.connect_clicked(move |_| {
            if rendering.get() {
                return;
            }
            let dialog = gtk::FileDialog::builder()
                .title("Import TDLib copy")
                .build();
            let input = input.clone();
            dialog.select_folder(
                None::<&gtk::Window>,
                None::<&gio::Cancellable>,
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(source) = file.path() else {
                        return;
                    };
                    input.emit(MaintenanceInput::ImportTdlib { source });
                },
            );
        });
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        cancel.connect_clicked(move |_| {
            if !rendering.get() {
                input.emit(MaintenanceInput::CancelImport);
            }
        });
        group.add(&row);
        group
    }

    fn crash_group(&self, _sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("desktop-maintenance-crash-report"))
            .build();
        let label = gtk::Label::new(Some(&self.crash_text()));
        label.set_selectable(true);
        label.set_wrap(true);
        label.set_xalign(0.0);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(160)
            .child(&label)
            .build();
        group.add(&scroll);

        let copy = gtk::Button::with_label(&self.text("desktop-maintenance-copy"));
        let restart = gtk::Button::with_label(&self.text("desktop-maintenance-restart"));
        let text = self.crash_text();
        copy.connect_clicked(move |_| copy_text_to_clipboard(&text));
        restart.connect_clicked(|_| restart_application());
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.append(&copy);
        actions.append(&restart);
        group.add(&actions);
        group
    }
}

/// Copies crash-report text into the default clipboard. GTK must be initialized.
pub fn copy_text_to_clipboard(text: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

/// Starts a fresh process from the current executable and exits this one.
pub fn restart_application() -> ! {
    if let Ok(current) = std::env::current_exe() {
        let _ = std::process::Command::new(current).spawn();
    }
    std::process::exit(0);
}

impl SimpleComponent for Maintenance {
    type Init = Rc<I18n>;
    type Input = MaintenanceInput;
    type Output = MaintenanceOutput;
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
        model.render(&root, &sender);
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
#[path = "../../tests/unit/maintenance_ui.rs"]
mod tests;
