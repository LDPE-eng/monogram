use mono_proto_client::{ChecklistSendSpec, ContactSendSpec, LocationSendSpec, PollSendSpec};
use serde::{Deserialize, Serialize};

use crate::domain::ClientError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SpecialContent {
    Poll(PollSendSpec),
    Checklist(ChecklistSendSpec),
    Contact(ContactSendSpec),
    Dice { emoticon: String },
    Location(LocationSendSpec),
    InlineBotResult { query_id: i64, result_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpecialSend {
    pub chat_id: i64,
    pub reply_to_msg_id: i32,
    pub top_msg_id: i32,
    pub silent: bool,
    pub schedule_date: Option<i32>,
    pub content: SpecialContent,
}

impl SpecialSend {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.chat_id == 0
            || self.reply_to_msg_id < 0
            || self.top_msg_id < 0
            || self
                .schedule_date
                .is_some_and(|date| date <= 0 || date == i32::MAX)
        {
            return Err(ClientError::InvalidRequest);
        }
        match &self.content {
            SpecialContent::Poll(spec) => validate_poll(spec),
            SpecialContent::Checklist(spec) => validate_checklist(spec),
            SpecialContent::Contact(spec) => validate_contact(spec),
            SpecialContent::Dice { emoticon } => validate_dice(emoticon),
            SpecialContent::Location(spec) => {
                validate_location(spec)?;
                if spec.live_period > 0 && self.schedule_date.is_some() {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(())
            }
            SpecialContent::InlineBotResult {
                query_id,
                result_id,
            } => validate_inline_bot_result(*query_id, result_id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PendingSpecialSend {
    pub random_id: i64,
    pub send: SpecialSend,
    pub uncertain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistTextEdit {
    pub item_id: Option<i32>,
    pub text: String,
    pub entities_json: Option<String>,
    pub expected_text: String,
    pub expected_entities_json: Option<String>,
}

impl ChecklistTextEdit {
    pub fn validate(&self, remove: bool) -> Result<(), ClientError> {
        if self.item_id.is_some_and(|id| id <= 0)
            || (remove && self.item_id.is_none())
            || (!remove
                && (self.text.trim().is_empty()
                    || self.text.encode_utf16().count()
                        > if self.item_id.is_some() { 64 } else { 32 }))
        {
            return Err(ClientError::InvalidRequest);
        }
        validate_entities(self.entities_json.as_deref())?;
        validate_entities(self.expected_entities_json.as_deref())
    }
}

pub fn validate_append_checklist(first_id: i32, titles: &[String]) -> Result<(), ClientError> {
    if first_id <= 0
        || titles.is_empty()
        || titles.len() > 30
        || titles
            .iter()
            .any(|title| title.trim().is_empty() || title.encode_utf16().count() > 64)
        || first_id
            .checked_add(i32::try_from(titles.len() - 1).map_err(|_| ClientError::InvalidRequest)?)
            .is_none()
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

pub fn validate_checklist_completion(
    completed: &[i32],
    incompleted: &[i32],
) -> Result<(), ClientError> {
    let unique = completed
        .iter()
        .chain(incompleted)
        .copied()
        .collect::<std::collections::HashSet<_>>();
    if (completed.is_empty() && incompleted.is_empty())
        || unique.len() != completed.len() + incompleted.len()
        || unique.contains(&0)
        || unique.iter().any(|id| *id < 0)
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

pub fn validate_poll_vote(options: &[Vec<u8>]) -> Result<(), ClientError> {
    if options.is_empty()
        || options.len() > 10
        || options.iter().any(Vec::is_empty)
        || options
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != options.len()
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_poll(spec: &PollSendSpec) -> Result<(), ClientError> {
    if spec.question.trim().is_empty()
        || spec.question.encode_utf16().count() > 255
        || !(2..=10).contains(&spec.answers.len())
        || spec
            .answers
            .iter()
            .any(|answer| answer.trim().is_empty() || answer.encode_utf16().count() > 100)
        || spec.quiz && (spec.multiple_choice || spec.correct_answer.is_none())
        || !spec.quiz && (spec.correct_answer.is_some() || spec.solution.is_some())
        || spec
            .correct_answer
            .is_some_and(|index| index as usize >= spec.answers.len())
        || spec.solution.as_ref().is_some_and(|solution| {
            solution.trim().is_empty() || solution.encode_utf16().count() > 200
        })
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_checklist(spec: &ChecklistSendSpec) -> Result<(), ClientError> {
    if spec.title.trim().is_empty()
        || spec.title.encode_utf16().count() > 32
        || !(1..=30).contains(&spec.items.len())
        || spec
            .items
            .iter()
            .any(|item| item.trim().is_empty() || item.encode_utf16().count() > 64)
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_contact(spec: &ContactSendSpec) -> Result<(), ClientError> {
    let digits = spec.phone.strip_prefix('+').unwrap_or(&spec.phone);
    if !(3..=15).contains(&digits.len())
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || spec.first_name.trim().is_empty()
        || spec.first_name.encode_utf16().count() > 256
        || spec.last_name.encode_utf16().count() > 256
        || spec.vcard.len() > 65_536
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_dice(emoticon: &str) -> Result<(), ClientError> {
    if !matches!(emoticon, "🎲" | "🎯" | "🏀" | "⚽" | "🎳" | "🎰") {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_location(spec: &LocationSendSpec) -> Result<(), ClientError> {
    if !spec.latitude.is_finite()
        || !spec.longitude.is_finite()
        || !(-90.0..=90.0).contains(&spec.latitude)
        || !(-180.0..=180.0).contains(&spec.longitude)
        || spec.live_period < 0
        || (spec.live_period > 0
            && spec.live_period != i32::MAX
            && !(60..=86_400).contains(&spec.live_period))
        || spec
            .heading
            .is_some_and(|heading| !(1..=360).contains(&heading))
        || (spec.live_period == 0 && spec.heading.is_some())
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_inline_bot_result(query_id: i64, result_id: &str) -> Result<(), ClientError> {
    if query_id == 0 || result_id.is_empty() {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}

fn validate_entities(value: Option<&str>) -> Result<(), ClientError> {
    if let Some(value) = value
        && (value.len() > 65_536
            || !serde_json::from_str::<serde_json::Value>(value)
                .map_err(|_| ClientError::InvalidRequest)?
                .is_array())
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}
