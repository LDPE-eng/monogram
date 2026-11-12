use std::{cell::Cell, fmt, rc::Rc};

use relm4::{adw, adw::prelude::*, gtk, prelude::*};

use crate::i18n::I18n;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    pub id: String,
    pub label: String,
    pub created_at: Option<String>,
}

pub enum PasskeyInput {
    SetI18n(Rc<I18n>),
    SetPasskeys(Vec<PasskeyRecord>),
    SetSupported(bool),
    SetBusy(bool),
    SetError(Option<String>),
    Back,
    Refresh,
    Create,
    DeleteRequest(String),
    DeleteCancel,
    DeleteConfirm,
}

impl fmt::Debug for PasskeyInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SetI18n(_) => "SetI18n",
            Self::SetPasskeys(_) => "SetPasskeys",
            Self::SetSupported(_) => "SetSupported",
            Self::SetBusy(_) => "SetBusy",
            Self::SetError(_) => "SetError",
            Self::Back => "Back",
            Self::Refresh => "Refresh",
            Self::Create => "Create",
            Self::DeleteRequest(_) => "DeleteRequest",
            Self::DeleteCancel => "DeleteCancel",
            Self::DeleteConfirm => "DeleteConfirm",
        })
    }
}

pub enum PasskeyOutput {
    Back,
    Refresh,
    Create,
    Delete(String),
}

impl fmt::Debug for PasskeyOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Back => "Back",
            Self::Refresh => "Refresh",
            Self::Create => "Create",
            Self::Delete(_) => "Delete(..)",
        })
    }
}

pub struct PasskeyPage {
    i18n: Rc<I18n>,
    passkeys: Vec<PasskeyRecord>,
    busy: bool,
    supported: bool,
    error: Option<String>,
    confirming_delete: Option<String>,
    rendering: Rc<Cell<bool>>,
}

impl PasskeyPage {
    fn new(i18n: Rc<I18n>) -> Self {
        Self {
            i18n,
            passkeys: Vec::new(),
            busy: false,
            supported: true,
            error: None,
            confirming_delete: None,
            rendering: Rc::new(Cell::new(false)),
        }
    }

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_default()
    }

    fn handle(&mut self, input: PasskeyInput) -> Option<PasskeyOutput> {
        match input {
            PasskeyInput::SetI18n(i18n) => self.i18n = i18n,
            PasskeyInput::SetPasskeys(passkeys) => {
                self.passkeys = passkeys;
                self.busy = false;
                self.error = None;
                self.confirming_delete = None;
            }
            PasskeyInput::SetSupported(supported) => self.supported = supported,
            PasskeyInput::SetBusy(busy) => self.busy = busy,
            PasskeyInput::SetError(error) => {
                self.error = error;
                self.busy = false;
            }
            PasskeyInput::Back => return Some(PasskeyOutput::Back),
            PasskeyInput::Refresh if !self.busy => {
                self.busy = true;
                self.error = None;
                return Some(PasskeyOutput::Refresh);
            }
            PasskeyInput::Create if !self.busy && self.supported => {
                self.busy = true;
                self.error = None;
                return Some(PasskeyOutput::Create);
            }
            PasskeyInput::DeleteRequest(id)
                if !self.busy && self.passkeys.iter().any(|record| record.id == id) =>
            {
                self.confirming_delete = Some(id);
            }
            PasskeyInput::DeleteCancel => self.confirming_delete = None,
            PasskeyInput::DeleteConfirm if !self.busy => {
                if let Some(id) = self.confirming_delete.take() {
                    self.busy = true;
                    self.error = None;
                    return Some(PasskeyOutput::Delete(id));
                }
            }
            _ => {}
        }
        None
    }

    fn render(&self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        self.rendering.set(true);
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(
            &self.text("feature-settings-settings_passkeys"),
        ))));
        let back = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(PasskeyInput::Back));
        header.pack_start(&back);
        root.append(&header);

        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_label(
            &self
                .error
                .clone()
                .or_else(|| self.busy.then(|| self.text("desktop-loading")))
                .unwrap_or_default(),
        );
        status.set_visible(self.error.is_some() || self.busy);
        root.append(&status);

        let page = adw::PreferencesPage::builder().vexpand(true).build();
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_passkeys"))
            .description(self.text("feature-settings-settings_passkeys_hint"))
            .build();

        if self.passkeys.is_empty() {
            group.add(&gtk::Label::new(Some(
                &self.text("feature-settings-settings_passkeys_empty"),
            )));
        }
        for record in &self.passkeys {
            let row = adw::ActionRow::builder()
                .title(record.label.clone())
                .build();
            if let Some(created_at) = &record.created_at {
                row.set_subtitle(created_at);
            }
            let delete =
                gtk::Button::with_label(&self.text("feature-settings-settings_passkeys_delete"));
            delete.add_css_class("destructive-action");
            delete.set_sensitive(!self.busy);
            let id = record.id.clone();
            let input = sender.input_sender().clone();
            delete.connect_clicked(move |_| input.emit(PasskeyInput::DeleteRequest(id.clone())));
            row.add_suffix(&delete);
            group.add(&row);
            if self.confirming_delete.as_deref() == Some(record.id.as_str()) {
                group.add(&self.delete_confirmation(sender));
            }
        }

        let create =
            gtk::Button::with_label(&self.text("feature-settings-settings_passkeys_create"));
        create.set_sensitive(!self.busy && self.supported);
        let input = sender.input_sender().clone();
        create.connect_clicked(move |_| input.emit(PasskeyInput::Create));
        group.add(&create);
        page.add(&group);
        root.append(&page);
        self.rendering.set(false);
    }

    fn delete_confirmation(&self, sender: &ComponentSender<Self>) -> gtk::Box {
        let confirmation = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let cancel = gtk::Button::with_label(&self.text("desktop-cancel"));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(PasskeyInput::DeleteCancel));
        confirmation.append(&cancel);
        let confirm =
            gtk::Button::with_label(&self.text("feature-settings-settings_passkeys_delete"));
        confirm.add_css_class("destructive-action");
        let input = sender.input_sender().clone();
        confirm.connect_clicked(move |_| input.emit(PasskeyInput::DeleteConfirm));
        confirmation.append(&confirm);
        confirmation
    }
}

impl SimpleComponent for PasskeyPage {
    type Init = Rc<I18n>;
    type Input = PasskeyInput;
    type Output = PasskeyOutput;
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

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        if let Some(output) = self.handle(input) {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, root: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.render(root, &sender);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/passkeys_ui.rs"]
mod tests;
