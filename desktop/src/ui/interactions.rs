use std::{collections::BTreeMap, fmt, rc::Rc};

use mono_proto_client::{
    ChecklistSendSpec, ContactSendSpec, ForumTopicDto, ForumTopicsPageDto, InlineBotResultsDto,
    LocationSendSpec, MessageDto, PollSendSpec, ResolvedPeerDto, SavedGifDto, StickerCatalogDto,
    StickerListDto, StickerPackDto, adapter::messages::extras_rpc::PollVotersDto,
};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk, gtk::prelude::*};
use serde::Deserialize;

use crate::{
    client::{
        WorkerCommand, WorkerResponse,
        interactions::{
            InteractionCommand, InteractionResponse, OutboxRead, ReactionPeersPage,
            ReadParticipants, ReadReceiptConfig, ReadReceiptIssue,
        },
    },
    domain::{
        ChecklistTextEdit, ClientError, MediaDownloadMode, SpecialContent, SpecialSend, TextSend,
    },
    i18n::I18n,
    media::cache::CacheLease,
};

const DICE_EMOTICONS: [&str; 6] = ["🎲", "🎯", "🏀", "⚽", "🎳", "🎰"];

fn poll_spec(
    question: &str,
    answers: &[String],
    anonymous: bool,
    multiple_choice: bool,
    quiz: bool,
    correct_answer: Option<u32>,
    solution: Option<String>,
) -> Option<PollSendSpec> {
    let solution = solution.filter(|value| !value.trim().is_empty());
    if !quiz && (correct_answer.is_some() || solution.is_some()) {
        return None;
    }
    let spec = PollSendSpec {
        question: question.to_owned(),
        answers: answers.to_vec(),
        anonymous,
        multiple_choice: multiple_choice && !quiz,
        quiz,
        correct_answer: quiz.then_some(correct_answer).flatten(),
        solution: quiz.then_some(solution).flatten(),
    };
    let valid = !spec.question.trim().is_empty()
        && spec.question.encode_utf16().count() <= 255
        && (2..=10).contains(&spec.answers.len())
        && spec
            .answers
            .iter()
            .all(|answer| !answer.trim().is_empty() && answer.encode_utf16().count() <= 100)
        && (!spec.quiz
            || !spec.multiple_choice
                && spec
                    .correct_answer
                    .is_some_and(|answer| (answer as usize) < spec.answers.len()))
        && (spec.quiz || spec.correct_answer.is_none() && spec.solution.is_none())
        && spec
            .solution
            .as_ref()
            .is_none_or(|text| text.encode_utf16().count() <= 200);
    valid.then_some(spec)
}

fn checklist_spec(
    title: &str,
    items: &[String],
    others_can_append: bool,
    others_can_complete: bool,
) -> Option<ChecklistSendSpec> {
    let valid = !title.trim().is_empty()
        && title.encode_utf16().count() <= 32
        && (1..=30).contains(&items.len())
        && items
            .iter()
            .all(|item| !item.trim().is_empty() && item.encode_utf16().count() <= 64);
    valid.then(|| ChecklistSendSpec {
        title: title.to_owned(),
        items: items.to_vec(),
        others_can_append,
        others_can_complete,
    })
}

fn contact_spec(
    phone: &str,
    first_name: &str,
    last_name: &str,
    vcard: &str,
) -> Option<ContactSendSpec> {
    let digits = phone.strip_prefix('+').unwrap_or(phone);
    let valid = (3..=15).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && !first_name.trim().is_empty()
        && first_name.encode_utf16().count() <= 256
        && last_name.encode_utf16().count() <= 256
        && vcard.len() <= 65_536;
    valid.then(|| ContactSendSpec {
        phone: phone.to_owned(),
        first_name: first_name.to_owned(),
        last_name: last_name.to_owned(),
        vcard: vcard.to_owned(),
    })
}

fn location_spec(
    latitude: &str,
    longitude: &str,
    live_period: i32,
    heading: Option<i32>,
) -> Option<LocationSendSpec> {
    let latitude = latitude.trim().parse::<f64>().ok()?;
    let longitude = longitude.trim().parse::<f64>().ok()?;
    let valid = latitude.is_finite()
        && longitude.is_finite()
        && (-90.0..=90.0).contains(&latitude)
        && (-180.0..=180.0).contains(&longitude)
        && live_period >= 0
        && (live_period == 0 || live_period == i32::MAX || (60..=86_400).contains(&live_period))
        && heading.is_none_or(|value| (1..=360).contains(&value))
        && (live_period > 0 || heading.is_none());
    valid.then_some(LocationSendSpec {
        latitude,
        longitude,
        live_period,
        heading,
    })
}

#[cfg(test)]
fn dice_emoticon(value: &str) -> Option<&str> {
    DICE_EMOTICONS
        .into_iter()
        .find(|emoticon| *emoticon == value)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineQuery {
    username: String,
    query: String,
}

fn inline_query(draft: &str) -> Option<InlineQuery> {
    let draft = draft.trim_end();
    let token = draft.strip_prefix('@')?;
    let (username, query) = token
        .split_once(char::is_whitespace)
        .map_or((token, ""), |(username, query)| (username, query));
    let mut chars = username.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && (3..=32).contains(&username.len())
        && chars.all(|char| char.is_ascii_alphanumeric() || char == '_');
    valid.then(|| InlineQuery {
        username: username.to_ascii_lowercase(),
        query: query.to_owned(),
    })
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct PollCard {
    #[serde(default)]
    creator: i32,
    #[serde(default)]
    r: i32,
    #[serde(default)]
    q: String,
    #[serde(default)]
    a: Vec<PollAnswer>,
    #[serde(default)]
    n: i32,
    #[serde(default)]
    z: i32,
    #[serde(default)]
    m: i32,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    o: i32,
    #[serde(default)]
    s: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct PollAnswer {
    #[serde(default)]
    t: String,
    #[serde(default)]
    c: i32,
    #[serde(default)]
    k: i32,
    #[serde(default)]
    v: i32,
    #[serde(default)]
    o: String,
}

impl PollAnswer {
    fn option(&self) -> Option<Vec<u8>> {
        let bytes = self.o.as_bytes();
        (!bytes.is_empty() && bytes.len().is_multiple_of(2))
            .then(|| {
                let (pairs, _) = bytes.as_chunks::<2>();
                pairs
                    .iter()
                    .map(|pair| {
                        std::str::from_utf8(pair)
                            .ok()
                            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .flatten()
    }

    fn chosen(&self) -> bool {
        self.c != 0
    }
}

impl PollCard {
    fn answered(&self) -> bool {
        self.a.iter().any(PollAnswer::chosen)
    }

    fn closed(&self) -> bool {
        self.z != 0
    }

    fn quiz(&self) -> bool {
        self.x != 0
    }

    fn revoting_disabled(&self) -> bool {
        self.r != 0
    }

    fn vote_allowed(&self, options: &[Vec<u8>]) -> bool {
        if self.closed()
            || self.answered() && (self.quiz() || self.revoting_disabled())
            || options.is_empty()
            || self.m == 0 && options.len() > 1
        {
            return false;
        }
        let available = self
            .a
            .iter()
            .filter_map(PollAnswer::option)
            .collect::<Vec<_>>();
        let mut unique = options.to_vec();
        unique.sort();
        unique.dedup();
        unique.len() == options.len()
            && options
                .iter()
                .all(|option| !option.is_empty() && available.contains(option))
    }

    fn retract_allowed(&self) -> bool {
        !self.closed() && self.answered() && !self.quiz() && !self.revoting_disabled()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ChecklistCard {
    #[serde(default)]
    t: String,
    #[serde(default)]
    a: i32,
    #[serde(default)]
    c: i32,
    #[serde(default)]
    i: Vec<ChecklistItem>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ChecklistItem {
    #[serde(default)]
    id: i32,
    #[serde(default)]
    t: String,
    #[serde(default)]
    d: i32,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct ReplyMarkup {
    #[serde(default)]
    rows: Vec<Vec<BotButton>>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
struct BotButton {
    #[serde(default)]
    t: String,
    #[serde(default)]
    x: String,
    #[serde(default)]
    u: Option<String>,
    #[serde(default)]
    d: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    same: bool,
    #[serde(default)]
    c: Option<String>,
    #[serde(default)]
    pw: bool,
}

struct InteractionMessage {
    message: MessageDto,
    poll: Option<PollCard>,
    checklist: Option<ChecklistCard>,
    markup: Option<ReplyMarkup>,
}

impl InteractionMessage {
    fn new(message: MessageDto) -> Option<Self> {
        if message.id <= 0 {
            return None;
        }
        let json = message.file_name.as_deref();
        let poll = (message.media_kind.as_deref() == Some("poll"))
            .then(|| json.and_then(|json| serde_json::from_str(json).ok()))
            .flatten();
        let checklist = (message.media_kind.as_deref() == Some("todo"))
            .then(|| json.and_then(|json| serde_json::from_str(json).ok()))
            .flatten();
        let markup = message
            .reply_markup_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());
        (poll.is_some() || checklist.is_some() || markup.is_some()).then_some(Self {
            message,
            poll,
            checklist,
            markup,
        })
    }

    fn current(&self, chat_id: i64, message_id: i32) -> bool {
        self.message.chat_id == chat_id && self.message.id == message_id
    }
}

pub enum InteractionInput {
    SetContext {
        chat_id: i64,
        reply_to_msg_id: i32,
        top_msg_id: i32,
    },
    SetMessage(Box<MessageDto>),
    SetBusy(bool),
    SetError(Option<ClientError>),
    Response(WorkerResponse),
    SendPoll(PollSendSpec),
    SendChecklist(ChecklistSendSpec),
    Vote {
        message_id: i32,
        options: Vec<Vec<u8>>,
    },
    RetractVote {
        message_id: i32,
    },
    ClosePoll {
        message_id: i32,
    },
    LoadVoters {
        message_id: i32,
        offset: Option<String>,
    },
    AppendChecklist {
        message_id: i32,
        titles: Vec<String>,
    },
    ToggleChecklistItem {
        message_id: i32,
        item_id: i32,
    },
    EditChecklistTitle {
        message_id: i32,
        text: String,
    },
    EditChecklistItem {
        message_id: i32,
        item_id: i32,
        text: String,
    },
    DeleteChecklistItem {
        message_id: i32,
        item_id: i32,
    },
    SetCanManageTopics(bool),
    SetInlineDraft(String),
    LoadInlineMore,
    SendInlineResult(String),
    SendContact(ContactSendSpec),
    SendDice(String),
    SendLocation(LocationSendSpec),
    LoadStickerCatalog {
        emoji: bool,
        hash: i64,
    },
    LoadStickerPack {
        set_id: i64,
        access_hash: i64,
    },
    SearchStickers {
        emoticon: String,
        hash: i64,
    },
    LoadSavedGifs,
    SendDocument {
        document_id: i64,
        kind: DocumentKind,
    },
    DownloadPreview {
        document_id: i64,
    },
    LoadReactionPeers {
        message_id: i32,
        offset: Option<String>,
    },
    LoadReadReceipts {
        message_id: i32,
    },
    LoadOutboxReadDate {
        message_id: i32,
    },
    LoadReadReceiptConfig,
    LoadTopics,
    LoadMoreTopics,
    ToggleHiddenGeneral,
    SetGeneralHidden(bool),
    PressBotButton {
        message_id: i32,
        row: usize,
        column: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Sticker,
    Emoji,
    Gif,
}

impl fmt::Debug for InteractionInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InteractionInput(..)")
    }
}

#[derive(Default)]
struct InteractionModel {
    chat_id: i64,
    reply_to_msg_id: i32,
    top_msg_id: i32,
    current: Option<InteractionMessage>,
    voters: Option<PollVotersDto>,
    voters_append: bool,
    inline: Option<InlineQuery>,
    inline_bot: Option<ResolvedPeerDto>,
    inline_results: Option<InlineBotResultsDto>,
    inline_append: bool,
    catalog: Option<StickerCatalogDto>,
    pack: Option<StickerPackDto>,
    stickers: Option<StickerListDto>,
    saved_gifs: Vec<SavedGifDto>,
    previews: BTreeMap<i64, CacheLease>,
    reaction_peers: Option<ReactionPeersPage>,
    reaction_append: bool,
    read_participants: Option<ReadParticipants>,
    outbox_read: Option<OutboxRead>,
    receipt_config: Option<ReadReceiptConfig>,
    topics: Vec<ForumTopicDto>,
    topics_count: i32,
    topics_cursor: Option<(i32, i32, i32)>,
    topics_append: bool,
    can_manage_topics: bool,
    show_hidden_general: bool,
    bot_notice: Option<String>,
    bot_url: Option<String>,
    copy_text: Option<String>,
    busy: bool,
    error: Option<ClientError>,
}

impl InteractionModel {
    fn send_special(&self, content: SpecialContent) -> Option<WorkerCommand> {
        if self.busy || self.chat_id == 0 || self.reply_to_msg_id < 0 || self.top_msg_id < 0 {
            return None;
        }
        let send = SpecialSend {
            chat_id: self.chat_id,
            reply_to_msg_id: self.reply_to_msg_id,
            top_msg_id: self.top_msg_id,
            silent: false,
            schedule_date: None,
            content,
        };
        send.validate()
            .ok()
            .map(|_| WorkerCommand::SendSpecial(send))
    }

    fn current(&self, message_id: i32) -> Option<&InteractionMessage> {
        if self.busy || self.chat_id == 0 {
            return None;
        }
        self.current
            .as_ref()
            .filter(|current| current.current(self.chat_id, message_id))
    }

    fn voters_response(&mut self, chat_id: i64, message_id: i32, page: PollVotersDto) {
        if self.chat_id != chat_id
            || !self
                .current
                .as_ref()
                .is_some_and(|current| current.message.id == message_id && current.poll.is_some())
        {
            return;
        }
        if let Some(current) = &mut self.voters
            && self.voters_append
            && current.next_offset.is_some()
        {
            let mut seen = current
                .voters
                .iter()
                .map(|voter| voter.peer_id)
                .collect::<std::collections::BTreeSet<_>>();
            current.voters.extend(
                page.voters
                    .into_iter()
                    .filter(|voter| seen.insert(voter.peer_id)),
            );
            current.next_offset = page.next_offset;
            current.count = page.count;
        } else {
            self.voters = Some(page);
        }
        self.voters_append = false;
    }

    fn handle(&mut self, input: InteractionInput) -> Vec<WorkerCommand> {
        if matches!(
            input,
            InteractionInput::Response(WorkerResponse::Interaction(_))
                | InteractionInput::Response(WorkerResponse::MediaReady { .. })
                | InteractionInput::SetCanManageTopics(_)
                | InteractionInput::SetInlineDraft(_)
                | InteractionInput::LoadInlineMore
                | InteractionInput::SendInlineResult(_)
                | InteractionInput::SendContact(_)
                | InteractionInput::SendDice(_)
                | InteractionInput::SendLocation(_)
                | InteractionInput::LoadStickerCatalog { .. }
                | InteractionInput::LoadStickerPack { .. }
                | InteractionInput::SearchStickers { .. }
                | InteractionInput::LoadSavedGifs
                | InteractionInput::SendDocument { .. }
                | InteractionInput::DownloadPreview { .. }
                | InteractionInput::LoadReactionPeers { .. }
                | InteractionInput::LoadReadReceipts { .. }
                | InteractionInput::LoadOutboxReadDate { .. }
                | InteractionInput::LoadReadReceiptConfig
                | InteractionInput::LoadTopics
                | InteractionInput::LoadMoreTopics
                | InteractionInput::ToggleHiddenGeneral
                | InteractionInput::SetGeneralHidden(_)
                | InteractionInput::PressBotButton { .. }
        ) {
            return self.handle_extended(input);
        }
        let command = match input {
            InteractionInput::SetContext {
                chat_id,
                reply_to_msg_id,
                top_msg_id,
            } => {
                if self.chat_id != chat_id || self.top_msg_id != top_msg_id {
                    self.current = None;
                    self.voters = None;
                    self.inline = None;
                    self.inline_bot = None;
                    self.inline_results = None;
                    self.inline_append = false;
                    self.reaction_peers = None;
                    self.reaction_append = false;
                    self.read_participants = None;
                    self.outbox_read = None;
                    self.topics.clear();
                    self.topics_count = 0;
                    self.topics_cursor = None;
                    self.topics_append = false;
                    self.show_hidden_general = false;
                    self.bot_notice = None;
                    self.bot_url = None;
                    self.copy_text = None;
                }
                self.chat_id = chat_id;
                self.reply_to_msg_id = reply_to_msg_id;
                self.top_msg_id = top_msg_id;
                self.busy = false;
                self.error = None;
                None
            }
            InteractionInput::SetMessage(message) => {
                let current = InteractionMessage::new(*message);
                if current.as_ref().is_some_and(|current| {
                    current.message.chat_id == self.chat_id
                        && self.current.as_ref().is_none_or(|old| {
                            old.message.id != current.message.id
                                || old.message.edit_date.unwrap_or(0)
                                    <= current.message.edit_date.unwrap_or(0)
                        })
                }) {
                    self.current = current;
                    self.voters = None;
                }
                None
            }
            InteractionInput::SetBusy(busy) => {
                self.busy = busy;
                None
            }
            InteractionInput::SetError(error) => {
                self.error = error;
                self.busy = false;
                None
            }
            InteractionInput::Response(response) => {
                self.busy = false;
                match response {
                    WorkerResponse::PollVoters {
                        chat_id,
                        message_id,
                        page,
                    } => self.voters_response(chat_id, message_id, page),
                    WorkerResponse::Message(message) => {
                        if let Some(current) = InteractionMessage::new(*message)
                            && current.message.chat_id == self.chat_id
                        {
                            self.current = Some(current);
                            self.voters = None;
                        }
                    }
                    _ => {}
                }
                None
            }
            InteractionInput::SendPoll(spec) => self.send_special(SpecialContent::Poll(spec)),
            InteractionInput::SendChecklist(spec) => {
                self.send_special(SpecialContent::Checklist(spec))
            }
            InteractionInput::Vote {
                message_id,
                options,
            } => self
                .current(message_id)
                .and_then(|current| current.poll.as_ref())
                .filter(|poll| poll.vote_allowed(&options))
                .map(|_| WorkerCommand::VotePoll {
                    chat_id: self.chat_id,
                    message_id,
                    options,
                }),
            InteractionInput::RetractVote { message_id } => self
                .current(message_id)
                .and_then(|current| current.poll.as_ref())
                .filter(|poll| poll.retract_allowed())
                .map(|_| WorkerCommand::RetractPollVote {
                    chat_id: self.chat_id,
                    message_id,
                }),
            InteractionInput::ClosePoll { message_id } => self
                .current(message_id)
                .and_then(|current| current.poll.as_ref())
                .filter(|poll| poll.creator != 0 && !poll.closed())
                .map(|_| WorkerCommand::ClosePoll {
                    chat_id: self.chat_id,
                    message_id,
                }),
            InteractionInput::LoadVoters { message_id, offset } => self
                .current(message_id)
                .and_then(|current| current.poll.as_ref())
                .filter(|poll| poll.o != 0)
                .map(|_| WorkerCommand::PollVoters {
                    chat_id: self.chat_id,
                    message_id,
                    offset,
                }),
            InteractionInput::AppendChecklist { message_id, titles } => self
                .current(message_id)
                .and_then(|current| current.checklist.as_ref().map(|list| (current, list)))
                .filter(|(current, list)| current.message.outgoing || list.a != 0)
                .and_then(|(_, list)| {
                    let first_id = list.i.iter().map(|item| item.id).max().unwrap_or(0) + 1;
                    crate::domain::validate_append_checklist(first_id, &titles)
                        .ok()
                        .map(|_| WorkerCommand::AppendChecklist {
                            chat_id: self.chat_id,
                            message_id,
                            first_id,
                            titles,
                        })
                }),
            InteractionInput::ToggleChecklistItem {
                message_id,
                item_id,
            } => self
                .current(message_id)
                .and_then(|current| current.checklist.as_ref().map(|list| (current, list)))
                .filter(|(current, list)| current.message.outgoing || list.c != 0)
                .and_then(|(_, list)| {
                    list.i.iter().find(|item| item.id == item_id).map(|item| {
                        let (completed, incompleted) = if item.d == 0 {
                            (vec![item_id], Vec::new())
                        } else {
                            (Vec::new(), vec![item_id])
                        };
                        WorkerCommand::ToggleChecklistCompleted {
                            chat_id: self.chat_id,
                            message_id,
                            completed,
                            incompleted,
                        }
                    })
                }),
            InteractionInput::EditChecklistTitle { message_id, text } => self
                .current(message_id)
                .filter(|current| current.message.outgoing)
                .and_then(|current| current.checklist.as_ref())
                .and_then(|list| {
                    let edit = ChecklistTextEdit {
                        item_id: None,
                        text,
                        entities_json: None,
                        expected_text: list.t.clone(),
                        expected_entities_json: list.e.clone(),
                    };
                    edit.validate(false)
                        .ok()
                        .map(|_| WorkerCommand::EditChecklistTitle {
                            chat_id: self.chat_id,
                            message_id,
                            edit,
                        })
                }),
            InteractionInput::EditChecklistItem {
                message_id,
                item_id,
                text,
            } => self
                .current(message_id)
                .filter(|current| current.message.outgoing)
                .and_then(|current| current.checklist.as_ref())
                .and_then(|list| list.i.iter().find(|item| item.id == item_id))
                .and_then(|item| {
                    let edit = ChecklistTextEdit {
                        item_id: Some(item_id),
                        text,
                        entities_json: None,
                        expected_text: item.t.clone(),
                        expected_entities_json: item.e.clone(),
                    };
                    edit.validate(false)
                        .ok()
                        .map(|_| WorkerCommand::EditChecklistItem {
                            chat_id: self.chat_id,
                            message_id,
                            edit,
                        })
                }),
            InteractionInput::DeleteChecklistItem {
                message_id,
                item_id,
            } => self
                .current(message_id)
                .filter(|current| current.message.outgoing)
                .and_then(|current| current.checklist.as_ref())
                .filter(|list| list.i.len() > 1)
                .and_then(|list| list.i.iter().find(|item| item.id == item_id))
                .and_then(|item| {
                    let edit = ChecklistTextEdit {
                        item_id: Some(item_id),
                        text: String::new(),
                        entities_json: None,
                        expected_text: item.t.clone(),
                        expected_entities_json: item.e.clone(),
                    };
                    edit.validate(true)
                        .ok()
                        .map(|_| WorkerCommand::DeleteChecklistItem {
                            chat_id: self.chat_id,
                            message_id,
                            edit,
                        })
                }),
            _ => unreachable!(),
        };
        if let Some(WorkerCommand::PollVoters { offset, .. }) = &command {
            self.voters_append = offset.is_some();
        }
        if command.is_some() {
            self.busy = true;
            self.error = None;
        }
        command.into_iter().collect()
    }
}

fn interaction(command: InteractionCommand) -> WorkerCommand {
    WorkerCommand::Interaction(command)
}

fn sort_topics(topics: &mut [ForumTopicDto]) {
    topics.sort_by(|left, right| {
        right
            .hidden
            .cmp(&left.hidden)
            .then_with(|| right.pinned.cmp(&left.pinned))
            .then_with(|| right.date.cmp(&left.date))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn valid_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len().is_multiple_of(2)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl InteractionModel {
    fn finish_extended(&mut self, commands: Vec<WorkerCommand>) -> Vec<WorkerCommand> {
        if commands
            .iter()
            .any(|command| !matches!(command, WorkerCommand::DownloadCustomEmoji { .. }))
        {
            self.busy = true;
            self.error = None;
        }
        commands
    }

    fn inline_results_command(&self, offset: String) -> Option<WorkerCommand> {
        let inline = self.inline.as_ref()?;
        let bot = self
            .inline_bot
            .as_ref()
            .filter(|bot| bot.is_bot && bot.peer_id > 0)?;
        (bot.username.as_deref().is_some_and(|username| {
            username
                .trim_start_matches('@')
                .eq_ignore_ascii_case(&inline.username)
        }))
        .then(|| {
            interaction(InteractionCommand::InlineBotResults {
                chat_id: self.chat_id,
                bot_id: bot.peer_id,
                query: inline.query.clone(),
                offset,
            })
        })
    }

    fn extended_response(&mut self, response: WorkerResponse) -> Vec<WorkerCommand> {
        self.busy = false;
        match response {
            WorkerResponse::MediaReady {
                cache_key, lease, ..
            } => {
                if let Some(document_id) = cache_key
                    .strip_prefix("custom-emoji:")
                    .and_then(|id| id.parse::<i64>().ok())
                    .filter(|id| *id > 0)
                {
                    self.previews.insert(document_id, *lease);
                }
                Vec::new()
            }
            WorkerResponse::Interaction(response) => match response {
                InteractionResponse::ResolvedPeer(peer) => {
                    let matches = self.inline.as_ref().is_some_and(|inline| {
                        peer.username.as_deref().is_some_and(|username| {
                            username
                                .trim_start_matches('@')
                                .eq_ignore_ascii_case(&inline.username)
                        })
                    });
                    if matches && peer.is_bot && peer.peer_id > 0 {
                        self.inline_bot = Some(peer);
                        let commands = self
                            .inline_results_command(String::new())
                            .into_iter()
                            .collect();
                        self.inline_append = false;
                        return self.finish_extended(commands);
                    }
                    Vec::new()
                }
                InteractionResponse::InlineBotResults(page) => {
                    if let Some(current) = &mut self.inline_results
                        && self.inline_append
                        && current
                            .next_offset
                            .as_deref()
                            .is_some_and(|offset| !offset.is_empty())
                    {
                        let mut seen = current
                            .results
                            .iter()
                            .map(|result| result.id.clone())
                            .collect::<std::collections::BTreeSet<_>>();
                        current.results.extend(
                            page.results
                                .into_iter()
                                .filter(|result| seen.insert(result.id.clone())),
                        );
                        current.next_offset = page.next_offset;
                        current.query_id = page.query_id;
                        current.cache_time = page.cache_time;
                        current.gallery = page.gallery;
                    } else {
                        self.inline_results = Some(page);
                    }
                    self.inline_append = false;
                    Vec::new()
                }
                InteractionResponse::StickerCatalog(catalog) => {
                    if !catalog.not_modified || self.catalog.is_none() {
                        self.catalog = Some(catalog);
                    }
                    Vec::new()
                }
                InteractionResponse::StickerPack(pack) => {
                    self.pack = Some(pack);
                    Vec::new()
                }
                InteractionResponse::Stickers(stickers) => {
                    if !stickers.not_modified || self.stickers.is_none() {
                        self.stickers = Some(stickers);
                    }
                    Vec::new()
                }
                InteractionResponse::SavedGifs(gifs) => {
                    self.saved_gifs = gifs;
                    Vec::new()
                }
                InteractionResponse::ReactionPeers(page) => {
                    if let Some(current) = &mut self.reaction_peers
                        && self.reaction_append
                        && current.next_offset.is_some()
                    {
                        let mut seen = current
                            .peers
                            .iter()
                            .map(|peer| (peer.peer_id, peer.emoticon.clone(), peer.document_id))
                            .collect::<std::collections::BTreeSet<_>>();
                        current.peers.extend(page.peers.into_iter().filter(|peer| {
                            seen.insert((peer.peer_id, peer.emoticon.clone(), peer.document_id))
                        }));
                        current.next_offset = page.next_offset;
                        current.count = page.count;
                    } else {
                        self.reaction_peers = Some(page);
                    }
                    self.reaction_append = false;
                    Vec::new()
                }
                InteractionResponse::MessageReadParticipants(participants) => {
                    self.read_participants = Some(participants);
                    Vec::new()
                }
                InteractionResponse::OutboxReadDate(read) => {
                    self.outbox_read = Some(read);
                    Vec::new()
                }
                InteractionResponse::ReadReceiptConfig(config) => {
                    self.receipt_config = Some(config);
                    Vec::new()
                }
                InteractionResponse::ForumTopics(page) => {
                    let ForumTopicsPageDto { count, topics } = page;
                    self.topics_count = count.max(0);
                    self.topics_cursor = topics.last().map(|topic| {
                        (topic.date.max(0), topic.top_message.max(0), topic.id.max(0))
                    });
                    if self.topics_append {
                        let mut seen = self
                            .topics
                            .iter()
                            .map(|topic| topic.id)
                            .collect::<std::collections::BTreeSet<_>>();
                        self.topics.extend(
                            topics
                                .into_iter()
                                .filter(|topic| !topic.deleted && seen.insert(topic.id)),
                        );
                    } else {
                        self.topics = topics.into_iter().filter(|topic| !topic.deleted).collect();
                    }
                    sort_topics(&mut self.topics);
                    self.topics_append = false;
                    Vec::new()
                }
                InteractionResponse::BotCallback(answer) => {
                    self.bot_notice = answer.message;
                    self.bot_url = answer.url;
                    Vec::new()
                }
                InteractionResponse::Message(message) => {
                    if let Some(message) = message
                        && let Some(current) = InteractionMessage::new(*message)
                        && current.message.chat_id == self.chat_id
                    {
                        self.current = Some(current);
                        self.voters = None;
                    }
                    Vec::new()
                }
                InteractionResponse::Done
                | InteractionResponse::Outgoing { .. }
                | InteractionResponse::ContactsImported(_)
                | InteractionResponse::Mentions(_)
                | InteractionResponse::MentionContacts(_)
                | InteractionResponse::RecentReactions(_)
                | InteractionResponse::AnimatedEmojiMax(_)
                | InteractionResponse::CustomEmojiIsFree(_) => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn bot_button(&mut self, message_id: i32, row: usize, column: usize) -> Vec<WorkerCommand> {
        if self.busy {
            return Vec::new();
        }
        let Some(current) = self.current(message_id) else {
            return Vec::new();
        };
        let Some(button) = current
            .markup
            .as_ref()
            .and_then(|markup| markup.rows.get(row))
            .and_then(|row| row.get(column))
            .cloned()
        else {
            return Vec::new();
        };
        match button.t.as_str() {
            "text" => {
                let send = TextSend {
                    chat_id: self.chat_id,
                    text: button.x,
                    reply_to_msg_id: self.reply_to_msg_id,
                    top_msg_id: self.top_msg_id,
                    entities_json: None,
                    webpage_url: None,
                    silent: false,
                    no_webpage: false,
                    schedule_date: None,
                };
                send.validate()
                    .ok()
                    .map(|_| vec![WorkerCommand::SendText(send)])
                    .unwrap_or_default()
            }
            "url" => {
                if button.u.as_deref().is_some_and(|url| {
                    url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("tg://")
                }) {
                    self.bot_url = button.u;
                } else {
                    self.bot_notice = Some("feature-dialog-dialog_unsupported_message".into());
                }
                Vec::new()
            }
            "copy" => {
                self.copy_text = Some(button.c.unwrap_or(button.x));
                Vec::new()
            }
            "cb" if button.pw => {
                self.bot_notice = Some("feature-dialog-dialog_bot_callback_password".into());
                Vec::new()
            }
            "cb" => button
                .d
                .filter(|data| valid_hex(data))
                .map(|data_hex| {
                    vec![interaction(InteractionCommand::BotCallback {
                        chat_id: self.chat_id,
                        message_id,
                        data_hex,
                    })]
                })
                .unwrap_or_default(),
            "sw" if button.same => {
                let Some(username) = current.message.via_bot.as_deref() else {
                    self.bot_notice = Some("feature-dialog-dialog_bot_switch_inline".into());
                    return Vec::new();
                };
                let query = format!("@{username} {}", button.q.unwrap_or_default());
                self.handle_extended(InteractionInput::SetInlineDraft(query))
            }
            "sw" => {
                self.bot_notice = Some("feature-dialog-dialog_bot_switch_inline".into());
                Vec::new()
            }
            "user" => {
                if button.u.as_deref().is_some_and(|id| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                }) {
                    self.bot_url = button.u.map(|id| format!("tg://user?id={id}"));
                }
                Vec::new()
            }
            _ => {
                self.bot_notice = Some("feature-dialog-dialog_unsupported_message".into());
                Vec::new()
            }
        }
    }

    fn handle_extended(&mut self, input: InteractionInput) -> Vec<WorkerCommand> {
        let commands = match input {
            InteractionInput::Response(response) => return self.extended_response(response),
            InteractionInput::SetCanManageTopics(can_manage) => {
                self.can_manage_topics = can_manage;
                Vec::new()
            }
            InteractionInput::SetInlineDraft(draft) => {
                let inline = inline_query(&draft);
                if inline == self.inline {
                    return Vec::new();
                }
                let same_bot = inline.as_ref().is_some_and(|inline| {
                    self.inline_bot.as_ref().is_some_and(|bot| {
                        bot.username.as_deref().is_some_and(|username| {
                            username
                                .trim_start_matches('@')
                                .eq_ignore_ascii_case(&inline.username)
                        })
                    })
                });
                self.inline = inline;
                self.inline_results = None;
                self.inline_append = false;
                if !same_bot {
                    self.inline_bot = None;
                }
                if self.busy || self.chat_id == 0 {
                    return Vec::new();
                }
                if let Some(command) = self.inline_results_command(String::new()) {
                    vec![command]
                } else if let Some(inline) = &self.inline {
                    vec![interaction(InteractionCommand::ResolveInlineBot {
                        username: inline.username.clone(),
                    })]
                } else {
                    Vec::new()
                }
            }
            InteractionInput::LoadInlineMore => {
                if self.busy {
                    Vec::new()
                } else {
                    let offset = self
                        .inline_results
                        .as_ref()
                        .and_then(|page| page.next_offset.clone())
                        .filter(|offset| !offset.is_empty());
                    let command = offset.and_then(|offset| self.inline_results_command(offset));
                    self.inline_append = command.is_some();
                    command.into_iter().collect()
                }
            }
            InteractionInput::SendInlineResult(result_id) => {
                let query_id = self
                    .inline_results
                    .as_ref()
                    .filter(|page| page.results.iter().any(|result| result.id == result_id))
                    .map(|page| page.query_id)
                    .filter(|id| *id != 0);
                query_id
                    .and_then(|query_id| {
                        self.send_special(SpecialContent::InlineBotResult {
                            query_id,
                            result_id,
                        })
                    })
                    .into_iter()
                    .collect()
            }
            InteractionInput::SendContact(contact) => self
                .send_special(SpecialContent::Contact(contact))
                .into_iter()
                .collect(),
            InteractionInput::SendDice(emoticon) => self
                .send_special(SpecialContent::Dice { emoticon })
                .into_iter()
                .collect(),
            InteractionInput::SendLocation(location) => self
                .send_special(SpecialContent::Location(location))
                .into_iter()
                .collect(),
            InteractionInput::LoadStickerCatalog { emoji, hash } => {
                if self.busy {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::StickerCatalog {
                        emoji,
                        hash,
                    })]
                }
            }
            InteractionInput::LoadStickerPack {
                set_id,
                access_hash,
            } => {
                if self.busy || set_id <= 0 {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::StickerPack {
                        set_id,
                        access_hash,
                    })]
                }
            }
            InteractionInput::SearchStickers { emoticon, hash } => {
                if self.busy || emoticon.is_empty() {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::Stickers { emoticon, hash })]
                }
            }
            InteractionInput::LoadSavedGifs => {
                if self.busy {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::SavedGifs)]
                }
            }
            InteractionInput::SendDocument { document_id, kind } => {
                if self.busy || self.chat_id == 0 || document_id <= 0 {
                    Vec::new()
                } else {
                    let command = match kind {
                        DocumentKind::Sticker => WorkerCommand::SendSticker {
                            chat_id: self.chat_id,
                            document_id,
                            reply_to_msg_id: self.reply_to_msg_id,
                            top_msg_id: self.top_msg_id,
                        },
                        DocumentKind::Emoji => WorkerCommand::SendCustomEmoji {
                            chat_id: self.chat_id,
                            document_id,
                            reply_to_msg_id: self.reply_to_msg_id,
                            top_msg_id: self.top_msg_id,
                        },
                        DocumentKind::Gif => WorkerCommand::SendSavedGif {
                            chat_id: self.chat_id,
                            document_id,
                            reply_to_msg_id: self.reply_to_msg_id,
                            top_msg_id: self.top_msg_id,
                        },
                    };
                    vec![command]
                }
            }
            InteractionInput::DownloadPreview { document_id } => {
                if document_id <= 0 || self.previews.contains_key(&document_id) {
                    Vec::new()
                } else {
                    vec![WorkerCommand::DownloadCustomEmoji {
                        document_id,
                        mode: MediaDownloadMode::Automatic,
                    }]
                }
            }
            InteractionInput::LoadReactionPeers { message_id, offset } => {
                if self.busy || self.chat_id == 0 || message_id <= 0 {
                    Vec::new()
                } else {
                    self.reaction_append = offset.is_some();
                    vec![interaction(InteractionCommand::ReactionPeers {
                        chat_id: self.chat_id,
                        message_id,
                        offset,
                    })]
                }
            }
            InteractionInput::LoadReadReceipts { message_id } => {
                if self.busy || self.chat_id == 0 || message_id <= 0 {
                    Vec::new()
                } else {
                    self.read_participants = None;
                    vec![interaction(InteractionCommand::MessageReadParticipants {
                        chat_id: self.chat_id,
                        message_id,
                    })]
                }
            }
            InteractionInput::LoadOutboxReadDate { message_id } => {
                if self.busy || self.chat_id == 0 || message_id <= 0 {
                    Vec::new()
                } else {
                    self.outbox_read = None;
                    vec![interaction(InteractionCommand::OutboxReadDate {
                        chat_id: self.chat_id,
                        message_id,
                    })]
                }
            }
            InteractionInput::LoadReadReceiptConfig => {
                if self.busy {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::ReadReceiptConfig)]
                }
            }
            InteractionInput::LoadTopics => {
                if self.busy || self.chat_id == 0 {
                    Vec::new()
                } else {
                    self.topics_append = false;
                    vec![interaction(InteractionCommand::ForumTopics {
                        chat_id: self.chat_id,
                        offset_date: 0,
                        offset_id: 0,
                        offset_topic: 0,
                        limit: 100,
                    })]
                }
            }
            InteractionInput::LoadMoreTopics => {
                let cursor = self.topics_cursor.filter(|_| {
                    !self.busy && self.chat_id != 0 && self.topics_count > self.topics.len() as i32
                });
                if let Some((offset_date, offset_id, offset_topic)) = cursor {
                    self.topics_append = true;
                    vec![interaction(InteractionCommand::ForumTopics {
                        chat_id: self.chat_id,
                        offset_date,
                        offset_id,
                        offset_topic,
                        limit: 100,
                    })]
                } else {
                    Vec::new()
                }
            }
            InteractionInput::ToggleHiddenGeneral => {
                self.show_hidden_general = !self.show_hidden_general;
                Vec::new()
            }
            InteractionInput::SetGeneralHidden(hidden) => {
                let allowed = self.can_manage_topics
                    && self
                        .topics
                        .iter()
                        .any(|topic| topic.id == 1 && topic.hidden != hidden);
                if self.busy || self.chat_id == 0 || !allowed {
                    Vec::new()
                } else {
                    vec![interaction(InteractionCommand::SetGeneralTopicHidden {
                        chat_id: self.chat_id,
                        hidden,
                    })]
                }
            }
            InteractionInput::PressBotButton {
                message_id,
                row,
                column,
            } => self.bot_button(message_id, row, column),
            _ => unreachable!(),
        };
        self.finish_extended(commands)
    }
}

struct InteractionWidgets {
    status: gtk::Label,
    spinner: gtk::Spinner,
    poll_question: gtk::Entry,
    poll_options: gtk::Box,
    poll_anonymous: gtk::CheckButton,
    poll_multiple: gtk::CheckButton,
    poll_quiz: gtk::CheckButton,
    poll_correct: gtk::SpinButton,
    poll_solution: gtk::Entry,
    checklist_title: gtk::Entry,
    checklist_items: gtk::Box,
    checklist_append: gtk::CheckButton,
    checklist_complete: gtk::CheckButton,
    contact_phone: gtk::Entry,
    contact_first: gtk::Entry,
    contact_last: gtk::Entry,
    contact_vcard: gtk::Entry,
    latitude: gtk::Entry,
    longitude: gtk::Entry,
    live_period: gtk::Entry,
    heading: gtk::Entry,
    poll_current: gtk::Box,
    checklist_current: gtk::Box,
    voters: gtk::Box,
    inline_results: gtk::Box,
    pack_results: gtk::Box,
    topic_results: gtk::Box,
    people_results: gtk::Box,
    receipt_message: gtk::Entry,
    keyboard: gtk::Box,
    command_buttons: Vec<gtk::Button>,
    identity_buttons: Vec<gtk::Button>,
}

pub struct InteractionOutput(pub WorkerCommand);

impl fmt::Debug for InteractionOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InteractionOutput(..)")
    }
}

pub struct InteractionsPanel {
    i18n: Rc<I18n>,
    model: InteractionModel,
    widgets: InteractionWidgets,
    poll_selection: std::collections::BTreeSet<Vec<u8>>,
    notice: Option<String>,
}

impl InteractionsPanel {
    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }
}

impl SimpleComponent for InteractionsPanel {
    type Init = Rc<I18n>;
    type Input = InteractionInput;
    type Output = InteractionOutput;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 6)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tr = |key: &str| i18n.text(key, &[]).unwrap_or_else(|_| key.into());
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.add_css_class("error");
        root.append(&status);
        let spinner = gtk::Spinner::new();
        spinner.update_property(&[gtk::accessible::Property::Label(&tr("desktop-loading"))]);
        root.append(&spinner);
        let notebook = gtk::Notebook::new();
        notebook.set_hexpand(true);
        notebook.set_vexpand(true);
        root.append(&notebook);

        let poll_page = notebook_page(&notebook, &tr("feature-dialog-dialog_media_poll"));
        let poll_question = form_entry(&poll_page, &tr("feature-dialog-dialog_poll_question"));
        let poll_options = gtk::Box::new(gtk::Orientation::Vertical, 4);
        poll_page.append(&poll_options);
        for index in 1..=2 {
            form_entry(
                &poll_options,
                &i18n_arg(
                    &i18n,
                    "feature-dialog-dialog_poll_option",
                    &index.to_string(),
                ),
            );
        }
        let poll_anonymous = check_row(
            &poll_page,
            &tr("feature-dialog-dialog_poll_anonymous"),
            true,
        );
        let poll_multiple = check_row(
            &poll_page,
            &tr("feature-dialog-dialog_poll_multiple"),
            false,
        );
        let poll_quiz = check_row(&poll_page, &tr("feature-dialog-dialog_poll_quiz"), false);
        let multiple = poll_multiple.clone();
        poll_quiz.connect_toggled(move |quiz| {
            if quiz.is_active() {
                multiple.set_active(false);
            }
            multiple.set_sensitive(!quiz.is_active());
        });
        poll_page.append(&gtk::Label::new(Some(&i18n_arg(
            &i18n,
            "feature-dialog-dialog_poll_correct_option",
            "1",
        ))));
        let poll_correct = gtk::SpinButton::with_range(1.0, 10.0, 1.0);
        poll_correct.set_value(1.0);
        poll_page.append(&poll_correct);
        let poll_solution = form_entry(&poll_page, &tr("feature-dialog-dialog_poll_explanation"));
        let poll_create = gtk::Button::with_label(&tr("feature-dialog-dialog_poll_create"));
        poll_page.append(&poll_create);
        let poll_current = gtk::Box::new(gtk::Orientation::Vertical, 5);
        poll_page.append(&poll_current);
        let voters = gtk::Box::new(gtk::Orientation::Vertical, 4);
        poll_page.append(&voters);
        notebook_page_scroller(&notebook, poll_page);

        let checklist_page = notebook_page(&notebook, &tr("feature-dialog-dialog_media_checklist"));
        let checklist_title = form_entry(
            &checklist_page,
            &tr("feature-dialog-dialog_checklist_title"),
        );
        let checklist_items = gtk::Box::new(gtk::Orientation::Vertical, 4);
        checklist_page.append(&checklist_items);
        form_entry(
            &checklist_items,
            &tr("feature-dialog-dialog_checklist_add_hint"),
        );
        let checklist_append = check_row(
            &checklist_page,
            &tr("feature-dialog-dialog_checklist_allow_append"),
            false,
        );
        let checklist_complete = check_row(
            &checklist_page,
            &tr("feature-dialog-dialog_checklist_allow_complete"),
            false,
        );
        let checklist_create =
            gtk::Button::with_label(&tr("feature-dialog-dialog_checklist_create"));
        checklist_page.append(&checklist_create);
        let checklist_current = gtk::Box::new(gtk::Orientation::Vertical, 5);
        checklist_page.append(&checklist_current);
        notebook_page_scroller(&notebook, checklist_page);

        let inline_page = notebook_page(&notebook, &tr("feature-dialog-dialog_bot"));
        let inline_entry = form_entry(&inline_page, "@bot query");
        let inline_input = sender.input_sender().clone();
        inline_entry.connect_changed(move |entry| {
            inline_input.emit(InteractionInput::SetInlineDraft(entry.text().to_string()));
        });
        let inline_results = gtk::Box::new(gtk::Orientation::Vertical, 5);
        inline_page.append(&inline_results);
        let keyboard = gtk::Box::new(gtk::Orientation::Vertical, 4);
        inline_page.append(&keyboard);
        notebook_page_scroller(&notebook, inline_page);

        let composer_page = notebook_page(&notebook, &tr("feature-dialog-dialog_attach"));
        let contact_phone = form_entry(&composer_page, &tr("feature-auth-auth_phone_label"));
        let contact_first = form_entry(&composer_page, &tr("desktop-contact-first-name"));
        let contact_last = form_entry(&composer_page, &tr("desktop-contact-last-name"));
        let contact_vcard = form_entry(&composer_page, &tr("desktop-contact-vcard"));
        let contact_send = gtk::Button::with_label(&tr("feature-dialog-dialog_media_contact"));
        composer_page.append(&contact_send);
        composer_page.append(&gtk::Label::new(Some(&tr(
            "feature-dialog-dialog_media_dice",
        ))));
        let dice = gtk::DropDown::from_strings(&DICE_EMOTICONS);
        dice.set_selected(0);
        composer_page.append(&dice);
        let dice_send = gtk::Button::with_label(&tr("feature-dialog-dialog_send"));
        composer_page.append(&dice_send);
        composer_page.append(&gtk::Label::new(Some(&tr("desktop-coordinates"))));
        let latitude = form_entry(&composer_page, "");
        let longitude = form_entry(&composer_page, "");
        let live_period = form_entry(&composer_page, &tr("feature-dialog-dialog_location_live"));
        live_period.set_text("0");
        let heading = form_entry(&composer_page, "");
        let location_send =
            gtk::Button::with_label(&tr("feature-dialog-dialog_location_send_this"));
        composer_page.append(&location_send);
        notebook_page_scroller(&notebook, composer_page);

        let pack_page = notebook_page(&notebook, &tr("feature-dialog-dialog_emoji_open"));
        let pack_results = gtk::Box::new(gtk::Orientation::Vertical, 5);
        pack_page.append(&pack_results);
        notebook_page_scroller(&notebook, pack_page);

        let topics_page = notebook_page(&notebook, &tr("desktop-topics"));
        let topic_results = gtk::Box::new(gtk::Orientation::Vertical, 4);
        topics_page.append(&topic_results);
        notebook_page_scroller(&notebook, topics_page);

        let people_page = notebook_page(&notebook, &tr("feature-dialog-dialog_seen_row"));
        let receipt_message = numeric_entry(&people_page, &tr("desktop-message"));
        let people_results = gtk::Box::new(gtk::Orientation::Vertical, 5);
        people_page.append(&people_results);
        notebook_page_scroller(&notebook, people_page);

        let mut command_buttons = vec![poll_create.clone(), checklist_create.clone()];
        let identity_buttons = vec![
            contact_send.clone(),
            dice_send.clone(),
            location_send.clone(),
        ];
        let model = InteractionModel::default();
        let poll_add = gtk::Button::with_label(&tr("feature-dialog-dialog_poll_add_option"));
        poll_options.append(&poll_add);
        command_buttons.push(poll_add.clone());
        let options = poll_options.clone();
        let add_button = poll_add.clone();
        let option_i18n = i18n.clone();
        poll_add.connect_clicked(move |_| {
            let count = entry_children(&options).len();
            if count < 10 {
                form_entry(
                    &options,
                    &i18n_arg(
                        &option_i18n,
                        "feature-dialog-dialog_poll_option",
                        &(count + 1).to_string(),
                    ),
                );
                add_button.set_sensitive(count + 1 < 10);
            }
        });
        let checklist_add = gtk::Button::with_label(&tr("feature-dialog-dialog_checklist_add"));
        checklist_items.append(&checklist_add);
        command_buttons.push(checklist_add.clone());
        let items = checklist_items.clone();
        let add_button = checklist_add.clone();
        let item_i18n = i18n.clone();
        checklist_add.connect_clicked(move |_| {
            let count = entry_children(&items).len();
            if count < 30 {
                form_entry(
                    &items,
                    &i18n_arg(
                        &item_i18n,
                        "feature-dialog-dialog_checklist_item",
                        &(count + 1).to_string(),
                    ),
                );
                add_button.set_sensitive(count + 1 < 30);
            }
        });
        let mut panel = Self {
            i18n: i18n.clone(),
            model,
            widgets: InteractionWidgets {
                status,
                spinner,
                poll_question: poll_question.clone(),
                poll_options,
                poll_anonymous: poll_anonymous.clone(),
                poll_multiple: poll_multiple.clone(),
                poll_quiz: poll_quiz.clone(),
                poll_correct: poll_correct.clone(),
                poll_solution: poll_solution.clone(),
                checklist_title: checklist_title.clone(),
                checklist_items,
                checklist_append: checklist_append.clone(),
                checklist_complete: checklist_complete.clone(),
                contact_phone: contact_phone.clone(),
                contact_first: contact_first.clone(),
                contact_last: contact_last.clone(),
                contact_vcard: contact_vcard.clone(),
                latitude: latitude.clone(),
                longitude: longitude.clone(),
                live_period: live_period.clone(),
                heading: heading.clone(),
                poll_current,
                checklist_current,
                voters,
                inline_results,
                pack_results,
                topic_results,
                people_results,
                receipt_message,
                keyboard,
                command_buttons,
                identity_buttons,
            },
            poll_selection: Default::default(),
            notice: None,
        };
        for button in panel
            .widgets
            .command_buttons
            .iter()
            .chain(&panel.widgets.identity_buttons)
        {
            button.set_sensitive(false);
        }
        let input = sender.input_sender().clone();
        let question = panel.widgets.poll_question.clone();
        let options = panel.widgets.poll_options.clone();
        let anonymous = panel.widgets.poll_anonymous.clone();
        let multiple = panel.widgets.poll_multiple.clone();
        let quiz = panel.widgets.poll_quiz.clone();
        let correct = panel.widgets.poll_correct.clone();
        let solution = panel.widgets.poll_solution.clone();
        poll_create.connect_clicked(move |_| {
            let answers = entry_texts(&options);
            let spec = poll_spec(
                &question.text(),
                &answers,
                anonymous.is_active(),
                multiple.is_active(),
                quiz.is_active(),
                quiz.is_active()
                    .then(|| correct.value_as_int().max(1) as u32 - 1),
                Some(solution.text().to_string()),
            );
            input.emit(spec.map_or_else(
                || InteractionInput::SetError(Some(ClientError::InvalidRequest)),
                InteractionInput::SendPoll,
            ));
        });
        let input = sender.input_sender().clone();
        let title = panel.widgets.checklist_title.clone();
        let items = panel.widgets.checklist_items.clone();
        let append = panel.widgets.checklist_append.clone();
        let complete = panel.widgets.checklist_complete.clone();
        checklist_create.connect_clicked(move |_| {
            let spec = checklist_spec(
                &title.text(),
                &entry_texts(&items),
                append.is_active(),
                complete.is_active(),
            );
            input.emit(spec.map_or_else(
                || InteractionInput::SetError(Some(ClientError::InvalidRequest)),
                InteractionInput::SendChecklist,
            ));
        });
        let input = sender.input_sender().clone();
        let phone = panel.widgets.contact_phone.clone();
        let first = panel.widgets.contact_first.clone();
        let last = panel.widgets.contact_last.clone();
        let vcard = panel.widgets.contact_vcard.clone();
        contact_send.connect_clicked(move |_| {
            let spec = contact_spec(&phone.text(), &first.text(), &last.text(), &vcard.text());
            input.emit(spec.map_or_else(
                || InteractionInput::SetError(Some(ClientError::InvalidRequest)),
                InteractionInput::SendContact,
            ));
        });
        let input = sender.input_sender().clone();
        dice_send.connect_clicked(move |_| {
            let emoticon = DICE_EMOTICONS[dice.selected() as usize].to_owned();
            input.emit(InteractionInput::SendDice(emoticon));
        });
        let input = sender.input_sender().clone();
        let lat = panel.widgets.latitude.clone();
        let long = panel.widgets.longitude.clone();
        let period = panel.widgets.live_period.clone();
        let heading = panel.widgets.heading.clone();
        location_send.connect_clicked(move |_| {
            let period_value = period.text().trim().parse::<i32>();
            let heading_value = if heading.text().trim().is_empty() {
                Ok(None)
            } else {
                heading.text().trim().parse::<i32>().map(Some)
            };
            let spec = match (period_value, heading_value) {
                (Ok(period), Ok(heading)) => {
                    location_spec(&lat.text(), &long.text(), period, heading)
                }
                _ => None,
            };
            input.emit(spec.map_or_else(
                || InteractionInput::SetError(Some(ClientError::InvalidRequest)),
                InteractionInput::SendLocation,
            ));
        });
        panel.sync(&sender);
        panel.refresh();
        ComponentParts {
            model: panel,
            widgets: (),
        }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        if matches!(input, InteractionInput::SetMessage(_)) {
            self.poll_selection.clear();
        }
        if matches!(input, InteractionInput::SetContext { .. }) {
            self.notice = None;
        }
        let commands = self.model.handle(input);
        for command in commands {
            if sender.output(InteractionOutput(command)).is_err() {
                self.model.error = Some(ClientError::Stopped);
                self.model.busy = false;
            }
        }
        self.side_effects(&sender);
        self.sync(&sender);
        self.refresh();
    }
}

impl InteractionsPanel {
    fn side_effects(&mut self, sender: &ComponentSender<Self>) {
        if let Some(notice) = self.model.bot_notice.take() {
            self.notice = Some(self.i18n.text(&notice, &[]).unwrap_or(notice));
        }
        if let Some(text) = self.model.copy_text.take() {
            let provider = gtk::gdk::ContentProvider::for_value(&text.to_value());
            let copied = gtk::gdk::Display::default()
                .is_some_and(|display| display.clipboard().set_content(Some(&provider)).is_ok());
            if !copied {
                self.model.error = Some(ClientError::Protocol);
            }
        }
        if let Some(url) = self.model.bot_url.take() {
            if !crate::platform::external_action_allowed(&url) {
                self.model.error = Some(ClientError::InvalidRequest);
                return;
            }
            let launcher = gtk::UriLauncher::new(&url);
            let window = self
                .widgets
                .status
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            let input = sender.input_sender().clone();
            launcher.launch(
                window.as_ref(),
                gtk::gio::Cancellable::NONE,
                move |result| {
                    if result.is_err() {
                        input.emit(InteractionInput::SetError(Some(ClientError::Protocol)));
                    }
                },
            );
        }
    }

    fn refresh(&self) {
        self.widgets.spinner.set_visible(self.model.busy);
        self.widgets.spinner.set_spinning(self.model.busy);
        let error = self.model.error.as_ref().map(|error| {
            let key = match error {
                ClientError::Network => "core-ui-status_network",
                ClientError::Unauthorized => "core-ui-status_session",
                ClientError::Forbidden => "core-ui-status_privacy",
                ClientError::FloodWait { seconds } => {
                    return self
                        .i18n
                        .text(
                            "core-ui-status_flood_wait",
                            &[("arg1", &seconds.to_string())],
                        )
                        .unwrap_or_default();
                }
                _ => "desktop-error-generic",
            };
            self.text(key)
        });
        if let Some(error) = error {
            self.widgets.status.set_text(&error);
            self.widgets.status.set_visible(true);
        } else if let Some(notice) = &self.notice {
            self.widgets.status.set_text(notice);
            self.widgets.status.set_visible(true);
        } else {
            self.widgets.status.set_text("");
            self.widgets.status.set_visible(false);
        }
        for button in &self.widgets.command_buttons {
            button.set_sensitive(!self.model.busy && self.model.chat_id != 0);
        }
        for button in &self.widgets.identity_buttons {
            button.set_sensitive(!self.model.busy && self.model.chat_id != 0);
        }
    }

    fn sync(&mut self, sender: &ComponentSender<Self>) {
        self.sync_poll(sender);
        self.sync_checklist(sender);
        self.sync_inline(sender);
        self.sync_packs(sender);
        self.sync_topics(sender);
        self.sync_people(sender);
        self.sync_keyboard(sender);
    }

    fn sync_poll(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.poll_current);
        clear_box(&self.widgets.voters);
        let Some(poll) = self
            .model
            .current
            .as_ref()
            .and_then(|current| current.poll.as_ref())
            .cloned()
        else {
            return;
        };
        let message_id = self
            .model
            .current
            .as_ref()
            .map_or(0, |current| current.message.id);
        append_label(&self.widgets.poll_current, &poll.q).add_css_class("heading");
        if poll.closed() {
            append_label(
                &self.widgets.poll_current,
                &self.text("feature-dialog-dialog_poll_closed"),
            );
        }
        let selection = std::rc::Rc::new(std::cell::RefCell::new(
            poll.a
                .iter()
                .filter(|answer| answer.chosen())
                .filter_map(PollAnswer::option)
                .collect::<std::collections::BTreeSet<_>>(),
        ));
        let mut first_single = None;
        let base = if poll.o != 0 || poll.closed() || poll.quiz() {
            poll.n
        } else {
            poll.a.iter().map(|answer| answer.v).max().unwrap_or(0)
        };
        for answer in &poll.a {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let check = gtk::CheckButton::new();
            if poll.m == 0 {
                if let Some(first) = &first_single {
                    check.set_group(Some(first));
                } else {
                    first_single = Some(check.clone());
                }
            }
            check.set_sensitive(!self.model.busy && !poll.closed());
            check.set_active(
                answer
                    .option()
                    .is_some_and(|option| selection.borrow().contains(&option)),
            );
            if let Some(option) = answer.option() {
                let selected = selection.clone();
                check.connect_toggled(move |check| {
                    if check.is_active() {
                        selected.borrow_mut().insert(option.clone());
                    } else {
                        selected.borrow_mut().remove(&option);
                    }
                });
            } else {
                check.set_sensitive(false);
            }
            row.append(&check);
            let mut text = answer.t.clone();
            if answer.v > 0 || base > 0 {
                let percent = if base > 0 {
                    answer.v.saturating_mul(100) / base
                } else {
                    0
                };
                text.push_str(&format!(" — {} · {percent}%", answer.v));
            }
            if answer.k != 0 {
                text.push_str(&format!(
                    " · {}",
                    self.text("feature-dialog-dialog_poll_solution")
                ));
            }
            append_label(&row, &text);
            self.widgets.poll_current.append(&row);
        }
        let selected = selection.clone();
        let vote = dynamic_button(
            &self.widgets.poll_current,
            &self.text("feature-dialog-dialog_poll_submit"),
            sender,
            move || InteractionInput::Vote {
                message_id,
                options: selected.borrow().iter().cloned().collect(),
            },
            !self.model.busy && !poll.closed(),
        );
        vote.set_sensitive(
            vote.is_sensitive()
                && poll.vote_allowed(&selection.borrow().iter().cloned().collect::<Vec<_>>()),
        );
        dynamic_button(
            &self.widgets.poll_current,
            &self.text("feature-dialog-dialog_poll_retract"),
            sender,
            move || InteractionInput::RetractVote { message_id },
            !self.model.busy && poll.retract_allowed(),
        );
        dynamic_button(
            &self.widgets.poll_current,
            &self.text("feature-dialog-dialog_poll_close"),
            sender,
            move || InteractionInput::ClosePoll { message_id },
            !self.model.busy && poll.creator != 0 && !poll.closed(),
        );
        if !poll.s.is_empty() {
            append_label(
                &self.widgets.poll_current,
                &self
                    .i18n
                    .text("feature-dialog-dialog_poll_solution", &[("arg1", &poll.s)])
                    .unwrap_or_else(|_| poll.s.clone()),
            );
        }
        if poll.o != 0 {
            dynamic_button(
                &self.widgets.voters,
                &self.text("feature-dialog-dialog_poll_voters_title"),
                sender,
                move || InteractionInput::LoadVoters {
                    message_id,
                    offset: None,
                },
                !self.model.busy,
            );
            if let Some(page) = &self.model.voters {
                for voter in &page.voters {
                    let options = voter
                        .options
                        .iter()
                        .filter_map(|option| {
                            poll.a
                                .iter()
                                .find(|answer| answer.option().as_ref() == Some(option))
                        })
                        .map(|answer| answer.t.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    append_label(
                        &self.widgets.voters,
                        &format!("{} · {options}", voter.title),
                    );
                }
                if let Some(offset) = page.next_offset.clone() {
                    dynamic_button(
                        &self.widgets.voters,
                        &self.text("feature-dialog-dialog_peer_list_load_more"),
                        sender,
                        move || InteractionInput::LoadVoters {
                            message_id,
                            offset: Some(offset.clone()),
                        },
                        !self.model.busy,
                    );
                }
            }
        }
    }

    fn sync_checklist(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.checklist_current);
        let Some(list) = self
            .model
            .current
            .as_ref()
            .and_then(|current| current.checklist.as_ref())
            .cloned()
        else {
            return;
        };
        let message_id = self
            .model
            .current
            .as_ref()
            .map_or(0, |current| current.message.id);
        let outgoing = self
            .model
            .current
            .as_ref()
            .is_some_and(|current| current.message.outgoing);
        let title = gtk::Entry::new();
        title.set_text(&list.t);
        title.set_hexpand(true);
        title.set_editable(outgoing && !self.model.busy);
        self.widgets.checklist_current.append(&title);
        let title_input = title.clone();
        dynamic_button(
            &self.widgets.checklist_current,
            &self.text("feature-dialog-dialog_edit"),
            sender,
            move || InteractionInput::EditChecklistTitle {
                message_id,
                text: title_input.text().to_string(),
            },
            outgoing && !self.model.busy,
        );
        let done = list.i.iter().filter(|item| item.d != 0).count();
        append_label(
            &self.widgets.checklist_current,
            &self
                .i18n
                .text(
                    "feature-dialog-dialog_checklist_progress",
                    &[
                        ("arg1", &done.to_string()),
                        ("arg2", &list.i.len().to_string()),
                    ],
                )
                .unwrap_or_default(),
        );
        for item in &list.i {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let check = gtk::CheckButton::new();
            check.set_active(item.d != 0);
            check.set_sensitive(!self.model.busy && (outgoing || list.c != 0));
            let item_id = item.id;
            let input = sender.input_sender().clone();
            check.connect_toggled(move |_| {
                input.emit(InteractionInput::ToggleChecklistItem {
                    message_id,
                    item_id,
                });
            });
            row.append(&check);
            let text = gtk::Entry::new();
            text.set_text(&item.t);
            text.set_hexpand(true);
            text.set_editable(outgoing && !self.model.busy);
            row.append(&text);
            let edit_input = text.clone();
            let save = dynamic_button(
                &row,
                &self.text("feature-dialog-dialog_edit"),
                sender,
                move || InteractionInput::EditChecklistItem {
                    message_id,
                    item_id,
                    text: edit_input.text().to_string(),
                },
                outgoing && !self.model.busy,
            );
            save.set_sensitive(save.is_sensitive() && item.id > 0);
            dynamic_button(
                &row,
                &self.text("feature-dialog-dialog_delete"),
                sender,
                move || InteractionInput::DeleteChecklistItem {
                    message_id,
                    item_id,
                },
                outgoing && !self.model.busy && list.i.len() > 1,
            );
            self.widgets.checklist_current.append(&row);
        }
        if outgoing || list.a != 0 {
            let append = form_entry(
                &self.widgets.checklist_current,
                &self.text("feature-dialog-dialog_checklist_add_hint"),
            );
            let append_input = append.clone();
            dynamic_button(
                &self.widgets.checklist_current,
                &self.text("feature-dialog-dialog_checklist_add_action"),
                sender,
                move || InteractionInput::AppendChecklist {
                    message_id,
                    titles: vec![append_input.text().to_string()],
                },
                !self.model.busy,
            );
        }
    }
}

impl InteractionsPanel {
    fn sync_inline(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.inline_results);
        if let Some(inline) = &self.model.inline {
            append_label(
                &self.widgets.inline_results,
                &format!("@{}", inline.username),
            );
        }
        if self.model.busy && self.model.inline_results.is_none() {
            append_label(
                &self.widgets.inline_results,
                &self.text("feature-dialog-dialog_inline_loading"),
            );
        }
        let Some(page) = self.model.inline_results.clone() else {
            return;
        };
        if page.results.is_empty() {
            append_label(
                &self.widgets.inline_results,
                &self.text("feature-dialog-dialog_inline_empty"),
            );
        }
        for result in page.results {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            if let Some(document_id) = result.document_id {
                if let Some(lease) = self.model.previews.get(&document_id) {
                    let picture = gtk::Picture::for_filename(lease.path());
                    picture.set_size_request(88, 88);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    row.append(&picture);
                } else {
                    sender
                        .input_sender()
                        .emit(InteractionInput::DownloadPreview { document_id });
                }
            }
            let title = result
                .title
                .filter(|title| !title.is_empty())
                .unwrap_or(result.kind);
            let description = result.description.filter(|text| !text.is_empty());
            let label = description.map_or(title.clone(), |description| {
                format!("{title}\n{description}")
            });
            let button = gtk::Button::with_label(&label);
            button.set_sensitive(!self.model.busy);
            let result_id = result.id;
            let input = sender.input_sender().clone();
            button.connect_clicked(move |_| {
                input.emit(InteractionInput::SendInlineResult(result_id.clone()))
            });
            row.append(&button);
            self.widgets.inline_results.append(&row);
        }
        if let Some(offset) = page.next_offset.filter(|offset| !offset.is_empty()) {
            dynamic_button(
                &self.widgets.inline_results,
                &self.text("feature-dialog-dialog_peer_list_load_more"),
                sender,
                move || InteractionInput::LoadInlineMore,
                !self.model.busy,
            );
            let _ = offset;
        }
    }

    fn sync_packs(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.pack_results);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        dynamic_button(
            &actions,
            &self.text("feature-dialog-dialog_panel_stickers"),
            sender,
            || InteractionInput::LoadStickerCatalog {
                emoji: false,
                hash: 0,
            },
            !self.model.busy,
        );
        dynamic_button(
            &actions,
            &self.text("feature-dialog-dialog_panel_emoji"),
            sender,
            || InteractionInput::LoadStickerCatalog {
                emoji: true,
                hash: 0,
            },
            !self.model.busy,
        );
        dynamic_button(
            &actions,
            &self.text("feature-dialog-dialog_saved_gifs"),
            sender,
            || InteractionInput::LoadSavedGifs,
            !self.model.busy,
        );
        self.widgets.pack_results.append(&actions);
        let search = gtk::Entry::new();
        search.set_placeholder_text(Some("😀"));
        search.set_max_width_chars(8);
        self.widgets.pack_results.append(&search);
        let query = search.clone();
        dynamic_button(
            &self.widgets.pack_results,
            &self.text("feature-dialog-dialog_search"),
            sender,
            move || InteractionInput::SearchStickers {
                emoticon: query.text().to_string(),
                hash: 0,
            },
            !self.model.busy,
        );
        if let Some(catalog) = &self.model.catalog {
            if catalog.sets.is_empty() {
                append_label(
                    &self.widgets.pack_results,
                    &self.text("feature-dialog-dialog_sticker_sets_empty"),
                );
            }
            for pack in &catalog.sets {
                let set_id = pack.id;
                let access_hash = pack.access_hash;
                dynamic_button(
                    &self.widgets.pack_results,
                    &pack.title,
                    sender,
                    move || InteractionInput::LoadStickerPack {
                        set_id,
                        access_hash,
                    },
                    !self.model.busy,
                );
            }
        }
        if let Some(pack) = self.model.pack.clone() {
            append_label(&self.widgets.pack_results, &pack.title).add_css_class("heading");
            let kind = if pack.is_emoji {
                DocumentKind::Emoji
            } else {
                DocumentKind::Sticker
            };
            if pack.preview_document_ids.is_empty() {
                append_label(
                    &self.widgets.pack_results,
                    &self.text("feature-dialog-dialog_pack_empty"),
                );
            }
            for document_id in pack.preview_document_ids {
                append_document(
                    &self.widgets.pack_results,
                    document_id,
                    kind,
                    &self.model,
                    sender,
                    &self.i18n,
                );
            }
        }
        if let Some(stickers) = self.model.stickers.clone() {
            let kind = self
                .model
                .pack
                .as_ref()
                .map_or(DocumentKind::Sticker, |pack| {
                    if pack.is_emoji {
                        DocumentKind::Emoji
                    } else {
                        DocumentKind::Sticker
                    }
                });
            for document_id in stickers.document_ids {
                append_document(
                    &self.widgets.pack_results,
                    document_id,
                    kind,
                    &self.model,
                    sender,
                    &self.i18n,
                );
            }
        }
        if self.model.saved_gifs.is_empty() {
            append_label(
                &self.widgets.pack_results,
                &self.text("feature-dialog-dialog_saved_gifs_empty"),
            );
        }
        for gif in self.model.saved_gifs.clone() {
            append_document(
                &self.widgets.pack_results,
                gif.document_id,
                DocumentKind::Gif,
                &self.model,
                sender,
                &self.i18n,
            );
        }
    }

    fn sync_topics(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.topic_results);
        dynamic_button(
            &self.widgets.topic_results,
            &self.text("feature-dialog-dialog_refresh"),
            sender,
            || InteractionInput::LoadTopics,
            !self.model.busy,
        );
        if self.model.topics.is_empty() && !self.model.busy {
            append_label(
                &self.widgets.topic_results,
                &self.text("feature-dialog-dialog_topics_empty"),
            );
        }
        if self.model.topics.iter().any(|topic| topic.hidden) {
            dynamic_button(
                &self.widgets.topic_results,
                &self.text(if self.model.show_hidden_general {
                    "feature-dialog-dialog_topic_collapse_archived"
                } else {
                    "feature-dialog-dialog_topic_archived_general"
                }),
                sender,
                || InteractionInput::ToggleHiddenGeneral,
                true,
            );
        }
        for topic in self.model.topics.clone() {
            if topic.hidden && !self.model.show_hidden_general {
                continue;
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let mut title = topic.title.clone();
            for (enabled, key) in [
                (topic.pinned, "feature-dialog-dialog_topic_pinned"),
                (topic.closed, "feature-dialog-dialog_topic_closed"),
                (topic.hidden, "feature-dialog-dialog_topic_hidden"),
            ] {
                if enabled {
                    title.push_str(&format!(" · {}", self.text(key)));
                }
            }
            append_label(&row, &title);
            if let Some(preview) = topic.last_message_preview.filter(|text| !text.is_empty()) {
                append_label(&row, &preview);
            }
            if topic.unread_count > 0 {
                append_label(
                    &row,
                    &self
                        .i18n
                        .text(
                            "feature-dialog-dialog_topic_unread",
                            &[("arg1", &topic.unread_count.to_string())],
                        )
                        .unwrap_or_default(),
                );
            }
            if topic.id == 1 && self.model.can_manage_topics {
                let hidden = !topic.hidden;
                dynamic_button(
                    &row,
                    &self.text(if hidden {
                        "feature-dialog-dialog_topic_hide_general"
                    } else {
                        "feature-dialog-dialog_topic_show_general"
                    }),
                    sender,
                    move || InteractionInput::SetGeneralHidden(hidden),
                    !self.model.busy,
                );
            }
            self.widgets.topic_results.append(&row);
        }
        if self.model.topics_count > self.model.topics.len() as i32 {
            dynamic_button(
                &self.widgets.topic_results,
                &self.text("feature-dialog-dialog_peer_list_load_more"),
                sender,
                || InteractionInput::LoadMoreTopics,
                !self.model.busy,
            );
        }
    }

    fn sync_people(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.people_results);
        let message = self.widgets.receipt_message.clone();
        let reaction_message = message.clone();
        dynamic_button(
            &self.widgets.people_results,
            &self.text("feature-dialog-dialog_reaction_users_title"),
            sender,
            move || InteractionInput::LoadReactionPeers {
                message_id: reaction_message.text().trim().parse().unwrap_or(0),
                offset: None,
            },
            !self.model.busy,
        );
        let read_message = message.clone();
        dynamic_button(
            &self.widgets.people_results,
            &self.text("feature-dialog-dialog_seen_row"),
            sender,
            move || InteractionInput::LoadReadReceipts {
                message_id: read_message.text().trim().parse().unwrap_or(0),
            },
            !self.model.busy,
        );
        let outbox_message = message.clone();
        dynamic_button(
            &self.widgets.people_results,
            &self.text("feature-dialog-dialog_read_show_when"),
            sender,
            move || InteractionInput::LoadOutboxReadDate {
                message_id: outbox_message.text().trim().parse().unwrap_or(0),
            },
            !self.model.busy,
        );
        dynamic_button(
            &self.widgets.people_results,
            &self.text("feature-dialog-dialog_refresh"),
            sender,
            || InteractionInput::LoadReadReceiptConfig,
            !self.model.busy,
        );
        if let Some(page) = &self.model.reaction_peers {
            for peer in &page.peers {
                let reaction = if peer.emoticon.is_empty() {
                    peer.document_id.to_string()
                } else {
                    peer.emoticon.clone()
                };
                append_label(
                    &self.widgets.people_results,
                    &format!("{} · {reaction}", peer.title),
                );
            }
            if let Some(offset) = page.next_offset.clone() {
                let message_id = message.text().trim().parse().unwrap_or(0);
                dynamic_button(
                    &self.widgets.people_results,
                    &self.text("feature-dialog-dialog_peer_list_load_more"),
                    sender,
                    move || InteractionInput::LoadReactionPeers {
                        message_id,
                        offset: Some(offset.clone()),
                    },
                    !self.model.busy,
                );
            }
        }
        if let Some(participants) = &self.model.read_participants {
            if let Some(issue) = participants.issue {
                append_label(
                    &self.widgets.people_results,
                    &self.text(self.receipt_issue(issue)),
                );
            } else if participants.participants.is_empty() {
                append_label(
                    &self.widgets.people_results,
                    &self.text("feature-dialog-dialog_nobody_viewed"),
                );
            }
            for participant in &participants.participants {
                append_label(
                    &self.widgets.people_results,
                    &self
                        .i18n
                        .text(
                            "feature-dialog-dialog_read_at",
                            &[("arg1", &participant.date.to_string())],
                        )
                        .unwrap_or_default(),
                );
            }
        }
        if let Some(read) = &self.model.outbox_read {
            if let Some(issue) = read.issue {
                append_label(
                    &self.widgets.people_results,
                    &self.text(self.receipt_issue(issue)),
                );
            } else if let Some(date) = read.date {
                append_label(
                    &self.widgets.people_results,
                    &self
                        .i18n
                        .text(
                            "feature-dialog-dialog_read_at",
                            &[("arg1", &date.to_string())],
                        )
                        .unwrap_or_default(),
                );
            } else {
                append_label(
                    &self.widgets.people_results,
                    &self.text("feature-dialog-dialog_read_unread"),
                );
            }
        }
        if let Some(config) = &self.model.receipt_config {
            for value in [
                config.chat_read_mark_size_threshold,
                config.chat_read_mark_expire_period,
                config.pm_read_date_expire_period,
            ] {
                append_label(&self.widgets.people_results, &value.to_string());
            }
        }
    }

    fn receipt_issue(&self, issue: ReadReceiptIssue) -> &'static str {
        match issue {
            ReadReceiptIssue::MessageTooOld => "feature-dialog-dialog_read_receipts_expired",
            ReadReceiptIssue::MessageNotRead => "feature-dialog-dialog_read_unread",
            _ => "feature-dialog-dialog_read_unknown",
        }
    }

    fn sync_keyboard(&mut self, sender: &ComponentSender<Self>) {
        clear_box(&self.widgets.keyboard);
        let Some(current) = self.model.current.as_ref() else {
            return;
        };
        let Some(markup) = current.markup.clone() else {
            return;
        };
        let message_id = current.message.id;
        let via_bot = current.message.via_bot.clone();
        let mut unsupported = false;
        for (row_index, buttons) in markup.rows.into_iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (column, button) in buttons.into_iter().enumerate() {
                let supported = match button.t.as_str() {
                    "text" | "copy" | "user" => true,
                    "url" => button.u.as_deref().is_some_and(|url| {
                        url.starts_with("https://")
                            || url.starts_with("http://")
                            || url.starts_with("tg://")
                    }),
                    "cb" => !button.pw && button.d.as_deref().is_some_and(valid_hex),
                    "sw" => button.same && via_bot.is_some(),
                    _ => false,
                };
                unsupported |= !supported;
                let widget = gtk::Button::with_label(&button.x);
                widget.set_sensitive(supported && !self.model.busy);
                if !supported {
                    widget.set_tooltip_text(Some(&self.text(if button.pw {
                        "feature-dialog-dialog_bot_callback_password"
                    } else {
                        "feature-dialog-dialog_unsupported_message"
                    })));
                }
                let input = sender.input_sender().clone();
                widget.connect_clicked(move |_| {
                    input.emit(InteractionInput::PressBotButton {
                        message_id,
                        row: row_index,
                        column,
                    });
                });
                row.append(&widget);
            }
            self.widgets.keyboard.append(&row);
        }
        if unsupported {
            append_label(
                &self.widgets.keyboard,
                &self.text("feature-dialog-dialog_unsupported_message"),
            );
        }
    }
}

fn notebook_page(notebook: &gtk::Notebook, title: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 6);
    page.set_margin_start(10);
    page.set_margin_end(10);
    page.set_margin_top(8);
    page.set_margin_bottom(8);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&page)
        .build();
    notebook.append_page(&scroll, Some(&gtk::Label::new(Some(title))));
    page
}

fn notebook_page_scroller(_notebook: &gtk::Notebook, page: gtk::Box) {
    page.set_vexpand(true);
}

fn form_entry(parent: &gtk::Box, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_hexpand(true);
    if !placeholder.is_empty() {
        entry.update_property(&[gtk::accessible::Property::Label(placeholder)]);
    }
    parent.append(&entry);
    entry
}

fn numeric_entry(parent: &gtk::Box, placeholder: &str) -> gtk::Entry {
    let entry = form_entry(parent, placeholder);
    entry.set_input_purpose(gtk::InputPurpose::Digits);
    entry
}

fn check_row(parent: &gtk::Box, label: &str, active: bool) -> gtk::CheckButton {
    let check = gtk::CheckButton::with_label(label);
    check.set_active(active);
    parent.append(&check);
    check
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn append_label(parent: &gtk::Box, text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    parent.append(&label);
    label
}

fn dynamic_button(
    parent: &gtk::Box,
    label: &str,
    sender: &ComponentSender<InteractionsPanel>,
    input: impl Fn() -> InteractionInput + Clone + 'static,
    sensitive: bool,
) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.set_sensitive(sensitive);
    let input_sender = sender.input_sender().clone();
    button.connect_clicked(move |_| input_sender.emit(input.clone()()));
    parent.append(&button);
    button
}

fn entry_children(parent: &gtk::Box) -> Vec<gtk::Entry> {
    let mut entries = Vec::new();
    let mut child = parent.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(entry) = widget.downcast::<gtk::Entry>() {
            entries.push(entry);
        }
    }
    entries
}

fn entry_texts(parent: &gtk::Box) -> Vec<String> {
    entry_children(parent)
        .into_iter()
        .map(|entry| entry.text().to_string())
        .collect()
}

fn i18n_arg(i18n: &I18n, key: &str, value: &str) -> String {
    i18n.text(key, &[("arg1", value)])
        .unwrap_or_else(|_| key.into())
}

fn append_document(
    parent: &gtk::Box,
    document_id: i64,
    kind: DocumentKind,
    model: &InteractionModel,
    sender: &ComponentSender<InteractionsPanel>,
    i18n: &I18n,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    if let Some(lease) = model.previews.get(&document_id) {
        let picture = gtk::Picture::for_filename(lease.path());
        picture.set_size_request(96, 96);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        row.append(&picture);
    } else {
        let key = match kind {
            DocumentKind::Sticker => "feature-dialog-dialog_media_sticker",
            DocumentKind::Emoji => "feature-dialog-dialog_panel_emoji",
            DocumentKind::Gif => "feature-dialog-dialog_media_gif",
        };
        let placeholder = i18n.text(key, &[]).unwrap_or_default();
        dynamic_button(
            &row,
            &placeholder,
            sender,
            move || InteractionInput::DownloadPreview { document_id },
            true,
        );
    }
    let key = match kind {
        DocumentKind::Sticker => "feature-dialog-dialog_reply_sticker",
        DocumentKind::Emoji => "feature-dialog-dialog_panel_emoji",
        DocumentKind::Gif => "feature-dialog-dialog_reply_gif",
    };
    let label = i18n.text(key, &[]).unwrap_or_default();
    dynamic_button(
        &row,
        &label,
        sender,
        move || InteractionInput::SendDocument { document_id, kind },
        !model.busy && model.chat_id != 0,
    );
    parent.append(&row);
}

#[cfg(test)]
#[path = "../../tests/unit/interactions_ui.rs"]
pub(crate) mod tests;
