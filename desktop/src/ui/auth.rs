use std::{
    fmt,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use qrcode::{Color, QrCode};
use relm4::{adw, adw::prelude::*, gtk, gtk::glib, prelude::*};

use crate::{
    domain::{AuthState, ClientError, ClientState, ConnectionState},
    i18n::I18n,
};

pub enum AuthInput {
    State(ClientState),
    Busy(bool),
    Error(Option<ClientError>),
    QrToken { token: Vec<u8>, expires: i32 },
    Submit,
    Resend,
    ShowQr,
    UsePhone,
    Retry,
    Tick,
}

impl fmt::Debug for AuthInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::State(_) => "AuthInput::State(..)",
            Self::Busy(_) => "AuthInput::Busy(..)",
            Self::Error(_) => "AuthInput::Error(..)",
            Self::QrToken { .. } => "AuthInput::QrToken(..)",
            Self::Submit => "AuthInput::Submit",
            Self::Resend => "AuthInput::Resend",
            Self::ShowQr => "AuthInput::ShowQr",
            Self::UsePhone => "AuthInput::UsePhone",
            Self::Retry => "AuthInput::Retry",
            Self::Tick => "AuthInput::Tick",
        })
    }
}

pub enum AuthOutput {
    Configure { api_id: i32, api_hash: String },
    Connect,
    SendCode(String),
    ResendCode,
    SignIn(String),
    CheckPassword(String),
    ExportQr,
    PollQr,
}

impl fmt::Debug for AuthOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configure { .. } => "Configure(..)",
            Self::Connect => "Connect",
            Self::SendCode(_) => "SendCode(..)",
            Self::ResendCode => "ResendCode",
            Self::SignIn(_) => "SignIn(..)",
            Self::CheckPassword(_) => "CheckPassword(..)",
            Self::ExportQr => "ExportQr",
            Self::PollQr => "PollQr",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Setup,
    Connect,
    Phone,
    Code,
    Password,
    Qr,
    Authorized,
}

pub struct AuthPage {
    i18n: Rc<I18n>,
    state: ClientState,
    busy: bool,
    error: Option<ClientError>,
    phone: String,
    use_phone: bool,
    qr: Option<(gtk::gdk::Texture, i32)>,
    timer: Option<glib::SourceId>,
}

pub struct AuthWidgets {
    title: gtk::Label,
    description: gtk::Label,
    setup: gtk::Box,
    api_id: gtk::Entry,
    api_hash: gtk::PasswordEntry,
    phone: gtk::Entry,
    code: gtk::Entry,
    password: gtk::PasswordEntry,
    qr: gtk::Box,
    picture: gtk::Picture,
    qr_status: gtk::Label,
    submit: gtk::Button,
    resend: gtk::Button,
    show_qr: gtk::Button,
    use_phone: gtk::Button,
    spinner: gtk::Spinner,
    error: gtk::Label,
    retry: gtk::Button,
    form: gtk::Box,
}

impl AuthPage {
    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.to_owned())
    }

    fn page(&self) -> Page {
        if self.state.connection == ConnectionState::Unconfigured {
            return Page::Setup;
        }
        if self.state.connection != ConnectionState::Connected {
            return Page::Connect;
        }
        match self.state.auth {
            AuthState::SignedOut => Page::Phone,
            AuthState::CodeRequired { .. } => Page::Code,
            AuthState::PasswordRequired => Page::Password,
            AuthState::QrPending { .. } if self.use_phone => Page::Phone,
            AuthState::QrPending { .. } => Page::Qr,
            AuthState::Authorized { .. } => Page::Authorized,
        }
    }

    fn send(&mut self, output: AuthOutput, sender: &ComponentSender<Self>) {
        self.error = None;
        if sender.output(output).is_ok() {
            self.busy = true;
        }
    }

    fn submit(&mut self, widgets: &AuthWidgets, sender: &ComponentSender<Self>) {
        let output = match self.page() {
            Page::Setup => {
                let id = widgets.api_id.text().trim().parse::<i32>().ok();
                let hash = widgets.api_hash.text().to_string();
                if let Some(api_id) = id.filter(|id| valid_api(*id, &hash)) {
                    widgets.api_hash.set_text("");
                    AuthOutput::Configure {
                        api_id,
                        api_hash: hash,
                    }
                } else {
                    self.error = Some(ClientError::InvalidConfiguration);
                    return;
                }
            }
            Page::Connect => AuthOutput::Connect,
            Page::Phone => {
                let Some(phone) = normalize_phone(&widgets.phone.text()) else {
                    self.error = Some(ClientError::InvalidPhone);
                    return;
                };
                widgets.phone.set_text(&phone);
                self.phone = phone.clone();
                self.use_phone = false;
                AuthOutput::SendCode(phone)
            }
            Page::Code => {
                let AuthState::CodeRequired {
                    ref code_type,
                    code_length,
                } = self.state.auth
                else {
                    return;
                };
                let code = widgets.code.text().trim().to_owned();
                if !valid_code(&code, code_type, code_length) {
                    self.error = Some(ClientError::InvalidCode);
                    return;
                }
                widgets.code.set_text("");
                AuthOutput::SignIn(code)
            }
            Page::Password => {
                let password = widgets.password.text().to_string();
                if password.is_empty() {
                    self.error = Some(ClientError::PasswordRequired);
                    return;
                }
                widgets.password.set_text("");
                AuthOutput::CheckPassword(password)
            }
            Page::Qr | Page::Authorized => return,
        };
        self.send(output, sender);
    }

    fn refresh(&self, w: &AuthWidgets) {
        let page = self.page();
        let title = match page {
            Page::Setup => "desktop-api-setup",
            Page::Code => "feature-auth-auth_code_title",
            Page::Password => "feature-auth-auth_password_title",
            Page::Qr => "feature-auth-auth_qr_login",
            _ => "feature-auth-auth_title",
        };
        w.title.set_label(&self.text(title));
        let description = match page {
            Page::Setup => self.text("desktop-api-help"),
            Page::Phone => self.text("feature-auth-auth_subtitle"),
            Page::Password => self.text("feature-auth-auth_password_prompt"),
            Page::Qr => self.text("feature-auth-auth_qr_instructions"),
            Page::Code => {
                let AuthState::CodeRequired {
                    ref code_type,
                    code_length,
                } = self.state.auth
                else {
                    return;
                };
                let key = match code_type.as_str() {
                    "app" => "feature-auth-auth_code_sent_telegram",
                    "sms" | "firebase" | "fragment" => "feature-auth-auth_code_sent_sms",
                    "call" | "flash_call" | "missed_call" => "feature-auth-auth_code_sent_call",
                    _ => "feature-auth-auth_code_sent_to",
                };
                let sent = self
                    .i18n
                    .text(key, &[("arg1", &self.phone)])
                    .unwrap_or_default();
                format!(
                    "{sent}\n{} · {code_type} · {code_length}",
                    self.text("feature-auth-auth_code_label")
                )
            }
            _ => String::new(),
        };
        w.description.set_label(&description);
        w.description.set_visible(!description.is_empty());
        w.setup.set_visible(page == Page::Setup);
        w.phone.set_visible(page == Page::Phone);
        w.code.set_visible(page == Page::Code);
        w.password.set_visible(page == Page::Password);
        w.qr.set_visible(page == Page::Qr);
        w.submit
            .set_visible(!matches!(page, Page::Qr | Page::Authorized));
        w.submit.set_label(&self.text(match page {
            Page::Setup => "desktop-api-save",
            Page::Connect => "desktop-connect",
            Page::Code => "feature-auth-auth_sign_in",
            Page::Password => "feature-auth-auth_submit_password",
            _ => "feature-auth-auth_send_code",
        }));
        w.resend.set_visible(page == Page::Code);
        w.show_qr
            .set_visible(matches!(page, Page::Phone | Page::Qr));
        w.use_phone.set_visible(page == Page::Qr);
        let qr_valid = self
            .qr
            .as_ref()
            .filter(|(_, expires)| i64::from(*expires) > unix_time());
        w.picture
            .set_paintable(qr_valid.map(|(texture, _)| texture));
        w.picture.set_visible(qr_valid.is_some());
        w.qr_status.set_visible(qr_valid.is_none());
        w.qr_status.set_label(&self.text(if self.qr.is_some() {
            "feature-settings-settings_devices_expired"
        } else {
            "desktop-loading"
        }));
        w.form.set_sensitive(!self.busy);
        w.spinner.set_spinning(self.busy);
        w.spinner.set_visible(self.busy);
        w.error.set_visible(self.error.is_some());
        w.retry.set_visible(self.error.is_some());
        w.retry.set_sensitive(!self.busy);
        if let Some(error) = &self.error {
            let key = match error {
                ClientError::InvalidConfiguration | ClientError::NotConfigured => {
                    "desktop-api-help"
                }
                ClientError::InvalidPhone => "feature-auth-auth_error_phone_required",
                ClientError::InvalidCode => "feature-auth-auth_error_restart",
                ClientError::InvalidPassword => "feature-auth-auth_error_bad_password",
                ClientError::PasswordRequired => "feature-auth-auth_error_password_required",
                ClientError::RegistrationRequired => "feature-auth-auth_error_no_account",
                ClientError::Network => "feature-auth-auth_error_network",
                _ => "desktop-error-generic",
            };
            let message = if let ClientError::FloodWait { seconds } = error {
                self.i18n
                    .text(
                        "core-ui-status_flood_wait",
                        &[("arg1", &seconds.to_string())],
                    )
                    .unwrap_or_else(|_| self.text("desktop-error-generic"))
            } else {
                self.text(key)
            };
            w.error.set_label(&message);
        }
    }
}

impl Component for AuthPage {
    type Init = Rc<I18n>;
    type Input = AuthInput;
    type Output = AuthOutput;
    type CommandOutput = ();
    type Root = adw::Clamp;
    type Widgets = AuthWidgets;

    fn init_root() -> Self::Root {
        adw::Clamp::builder()
            .maximum_size(480)
            .tightening_threshold(360)
            .build()
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut model = Self {
            i18n,
            state: ClientState::default(),
            busy: false,
            error: None,
            phone: String::new(),
            use_phone: false,
            qr: None,
            timer: None,
        };
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&content)
            .build();
        root.set_child(Some(&scroll));
        let title = gtk::Label::new(None);
        title.add_css_class("title-1");
        title.set_wrap(true);
        content.append(&title);
        let description = gtk::Label::new(None);
        description.set_wrap(true);
        content.append(&description);
        let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.append(&form);
        let setup = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let api_id = entry(&model.text("desktop-api-id"));
        api_id.set_input_purpose(gtk::InputPurpose::Digits);
        let api_hash = password_entry(&model.text("desktop-api-hash"));
        setup.append(&api_id);
        setup.append(&api_hash);
        form.append(&setup);
        let phone = entry(&model.text("feature-auth-auth_phone_label"));
        phone.set_input_purpose(gtk::InputPurpose::Phone);
        form.append(&phone);
        let code = entry(&model.text("feature-auth-auth_code_label"));
        code.set_input_purpose(gtk::InputPurpose::FreeForm);
        form.append(&code);
        let password = password_entry(&model.text("feature-auth-auth_password_label"));
        form.append(&password);
        let qr = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let picture = gtk::Picture::new();
        picture.set_size_request(240, 240);
        picture.set_can_shrink(true);
        picture.set_alternative_text(Some(&model.text("feature-auth-auth_qr_code")));
        qr.append(&picture);
        let qr_status = gtk::Label::new(None);
        qr_status.set_wrap(true);
        qr.append(&qr_status);
        form.append(&qr);
        let submit = button(
            &model,
            "feature-auth-auth_send_code",
            AuthInput::Submit,
            &sender,
        );
        submit.add_css_class("suggested-action");
        form.append(&submit);
        let resend = button(
            &model,
            "feature-auth-auth_code_help",
            AuthInput::Resend,
            &sender,
        );
        form.append(&resend);
        let show_qr = button(
            &model,
            "feature-auth-auth_qr_login",
            AuthInput::ShowQr,
            &sender,
        );
        form.append(&show_qr);
        let use_phone = button(
            &model,
            "feature-auth-auth_qr_use_phone",
            AuthInput::UsePhone,
            &sender,
        );
        form.append(&use_phone);
        let spinner = gtk::Spinner::new();
        spinner.set_tooltip_text(Some(&model.text("desktop-loading")));
        content.append(&spinner);
        let error = gtk::Label::new(None);
        error.set_wrap(true);
        error.add_css_class("error");
        error.set_selectable(true);
        content.append(&error);
        let retry = button(&model, "desktop-retry", AuthInput::Retry, &sender);
        content.append(&retry);
        for entry in [&api_id, &phone, &code] {
            let input = sender.input_sender().clone();
            entry.connect_activate(move |_| {
                input.emit(AuthInput::Submit);
            });
        }
        for entry in [&api_hash, &password] {
            let input = sender.input_sender().clone();
            entry.connect_activate(move |_| {
                input.emit(AuthInput::Submit);
            });
        }
        let input = sender.input_sender().clone();
        model.timer = Some(glib::timeout_add_local(Duration::from_secs(2), move || {
            input.emit(AuthInput::Tick);
            glib::ControlFlow::Continue
        }));
        let widgets = AuthWidgets {
            title,
            description,
            setup,
            api_id,
            api_hash,
            phone,
            code,
            password,
            qr,
            picture,
            qr_status,
            submit,
            resend,
            show_qr,
            use_phone,
            spinner,
            error,
            retry,
            form,
        };
        model.refresh(&widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        w: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _: &Self::Root,
    ) {
        match message {
            AuthInput::State(state) => {
                let old_page = self.page();
                self.state = state;
                if self.page() != old_page {
                    w.code.set_text("");
                    w.password.set_text("");
                    w.api_hash.set_text("");
                    self.error = None;
                }
                if !matches!(self.state.auth, AuthState::QrPending { .. }) {
                    self.qr = None;
                }
                if matches!(self.state.auth, AuthState::Authorized { .. }) {
                    self.phone.clear();
                    w.phone.set_text("");
                }
            }
            AuthInput::Busy(busy) => self.busy = busy,
            AuthInput::Error(error) => {
                self.busy = false;
                self.error = error;
            }
            AuthInput::QrToken { token, expires } => {
                self.busy = false;
                self.use_phone = false;
                self.qr = qr_texture(&token).map(|texture| (texture, expires));
                if self.qr.is_none() {
                    self.error = Some(ClientError::Protocol);
                }
            }
            AuthInput::Tick => {
                if self.page() == Page::Qr
                    && !self.busy
                    && self.error.is_none()
                    && self
                        .qr
                        .as_ref()
                        .is_some_and(|(_, expires)| i64::from(*expires) > unix_time())
                {
                    self.send(AuthOutput::PollQr, &sender);
                }
            }
            AuthInput::UsePhone if !self.busy => {
                self.use_phone = true;
                self.qr = None;
                self.error = None;
            }
            AuthInput::Submit if !self.busy => self.submit(w, &sender),
            AuthInput::Resend if !self.busy && self.page() == Page::Code => {
                self.send(AuthOutput::ResendCode, &sender)
            }
            AuthInput::ShowQr if !self.busy && matches!(self.page(), Page::Phone | Page::Qr) => {
                self.use_phone = false;
                self.qr = None;
                self.send(AuthOutput::ExportQr, &sender);
            }
            AuthInput::Retry if !self.busy => match self.page() {
                Page::Qr => self.send(AuthOutput::ExportQr, &sender),
                Page::Code => self.send(AuthOutput::ResendCode, &sender),
                _ => self.submit(w, &sender),
            },
            _ => {}
        }
        self.refresh(w);
    }
}

impl Drop for AuthPage {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

fn entry(label: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .placeholder_text(label)
        .hexpand(true)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(label)]);
    entry
}

fn password_entry(label: &str) -> gtk::PasswordEntry {
    let entry = gtk::PasswordEntry::builder()
        .placeholder_text(label)
        .show_peek_icon(true)
        .hexpand(true)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(label)]);
    entry
}

fn button(
    model: &AuthPage,
    key: &str,
    message: AuthInput,
    sender: &ComponentSender<AuthPage>,
) -> gtk::Button {
    let button = gtk::Button::with_label(&model.text(key));
    let input = sender.input_sender().clone();
    button.connect_clicked(move |_| {
        input.emit(match message {
            AuthInput::Submit => AuthInput::Submit,
            AuthInput::Resend => AuthInput::Resend,
            AuthInput::ShowQr => AuthInput::ShowQr,
            AuthInput::UsePhone => AuthInput::UsePhone,
            _ => AuthInput::Retry,
        });
    });
    button
}

fn valid_api(id: i32, hash: &str) -> bool {
    id > 0 && hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_phone(input: &str) -> Option<String> {
    let mut digits = String::new();
    for (position, ch) in input.trim().chars().enumerate() {
        match ch {
            '+' if position == 0 => {}
            '0'..='9' => digits.push(ch),
            ' ' | '-' | '(' | ')' | '.' => {}
            _ => return None,
        }
    }
    (5..=15)
        .contains(&digits.len())
        .then(|| format!("+{digits}"))
}

fn valid_code(code: &str, kind: &str, length: i32) -> bool {
    if code.is_empty() {
        return false;
    }
    if matches!(kind, "sms_word" | "sms_phrase") {
        return true;
    }
    code.bytes().all(|byte| byte.is_ascii_digit()) && (length <= 0 || code.len() == length as usize)
}

fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_secs() as i64)
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

#[cfg(test)]
#[path = "../../tests/unit/auth_ui.rs"]
mod tests;
