use gio::prelude::*;
use relm4::gtk::{gio, glib};
use serde::{Deserialize, Serialize};

use super::PlatformError;

const ACTION: &str = "notification";
const DETAILED_ACTION: &str = "app.notification";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationTarget {
    pub account_id: String,
    pub chat_id: i64,
    pub message_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationAction {
    OpenChat(NotificationTarget),
    OpenMessage(NotificationTarget),
    MarkRead(NotificationTarget),
    Mute(NotificationTarget),
    Settings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotificationCapabilities {
    pub actions: bool,
}

impl NotificationCapabilities {
    pub fn from_names(names: &[String]) -> Self {
        Self {
            actions: names.iter().any(|name| name == "actions"),
        }
    }

    pub async fn detect() -> Result<Self, PlatformError> {
        let bus = gio::bus_get_future(gio::BusType::Session)
            .await
            .map_err(|_| PlatformError::NotificationServerUnavailable)?;
        let reply = bus
            .call_future(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "GetCapabilities",
                None,
                Some(glib::VariantTy::new("(as)").expect("static variant type")),
                gio::DBusCallFlags::NONE,
                3000,
            )
            .await
            .map_err(|_| PlatformError::NotificationServerUnavailable)?;
        let (names,) = reply
            .get::<(Vec<String>,)>()
            .ok_or(PlatformError::NotificationServerUnavailable)?;
        Ok(Self::from_names(&names))
    }

    pub fn actions_for(self, target: &NotificationTarget) -> Vec<NotificationAction> {
        if !self.actions {
            return Vec::new();
        }
        let open = if target.message_id.is_some() {
            NotificationAction::OpenMessage(target.clone())
        } else {
            NotificationAction::OpenChat(target.clone())
        };
        vec![
            open,
            NotificationAction::MarkRead(target.clone()),
            NotificationAction::Mute(target.clone()),
            NotificationAction::Settings,
        ]
    }

    pub fn supports_inline_reply(self) -> bool {
        false
    }
}

pub struct NotificationLabels<'a> {
    pub mark_read: &'a str,
    pub mute: &'a str,
    pub settings: &'a str,
}

pub struct NotificationContent<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub target: NotificationTarget,
}

pub struct NotificationAdapter {
    application: gio::Application,
    capabilities: NotificationCapabilities,
}

impl NotificationAdapter {
    pub fn new(
        application: &impl IsA<gio::Application>,
        capabilities: NotificationCapabilities,
        handler: impl Fn(NotificationAction) + 'static,
    ) -> Self {
        let application = application.as_ref().clone();
        let action = gio::SimpleAction::new(ACTION, Some(glib::VariantTy::STRING));
        action.connect_activate(move |_, parameter| {
            if let Some(action) = parameter
                .and_then(|parameter| parameter.str())
                .and_then(|value| serde_json::from_str(value).ok())
            {
                handler(action);
            }
        });
        application.add_action(&action);
        Self {
            application,
            capabilities,
        }
    }

    /// Queues through GIO; successful return does not acknowledge server delivery.
    pub fn send(
        &self,
        content: &NotificationContent<'_>,
        labels: &NotificationLabels<'_>,
    ) -> Result<(), PlatformError> {
        if !self.application.is_registered() || self.application.is_remote() {
            return Err(PlatformError::ApplicationNotRegistered);
        }
        if [
            content.id,
            content.title,
            content.body.unwrap_or_default(),
            labels.mark_read,
            labels.mute,
            labels.settings,
        ]
        .iter()
        .any(|text| text.contains('\0'))
        {
            return Err(PlatformError::InvalidNotificationContent);
        }
        let notification = gio::Notification::new(content.title);
        notification.set_body(content.body);
        for action in self.capabilities.actions_for(&content.target) {
            let label = match &action {
                NotificationAction::OpenChat(_) | NotificationAction::OpenMessage(_) => None,
                NotificationAction::MarkRead(_) => Some(labels.mark_read),
                NotificationAction::Mute(_) => Some(labels.mute),
                NotificationAction::Settings => Some(labels.settings),
            };
            let target = serde_json::to_string(&action)
                .map_err(|_| PlatformError::InvalidNotificationContent)?
                .to_variant();
            if let Some(label) = label {
                notification.add_button_with_target_value(label, DETAILED_ACTION, Some(&target));
            } else {
                notification.set_default_action_and_target_value(DETAILED_ACTION, Some(&target));
            }
        }
        self.application
            .send_notification(Some(content.id), &notification);
        Ok(())
    }

    pub fn withdraw(&self, id: &str) -> Result<(), PlatformError> {
        if !self.application.is_registered() || self.application.is_remote() {
            return Err(PlatformError::ApplicationNotRegistered);
        }
        if id.contains('\0') {
            return Err(PlatformError::InvalidNotificationContent);
        }
        self.application.withdraw_notification(id);
        Ok(())
    }
}
