use super::*;
use serde::{Deserialize, Serialize};

pub type ContactImport = proto::adapter::messages::contacts::ContactImportSpec;
pub type ContactImportResult = proto::adapter::messages::contacts::ContactImportResult;

pub(super) struct InlineBotResultSend {
    pub(super) chat_id: i64,
    pub(super) query_id: i64,
    pub(super) result_id: String,
    pub(super) reply_to_msg_id: i32,
    pub(super) top_msg_id: i32,
    pub(super) random_id: i64,
    pub(super) options: SendOptions,
}

#[derive(Debug, Clone)]
pub enum InteractionCommand {
    SendContact {
        chat_id: i64,
        contact: proto::ContactSendSpec,
        reply_to_msg_id: i32,
        top_msg_id: i32,
        persisted_random_id: i64,
        options: SendOptions,
    },
    ImportContacts {
        contacts: Vec<ContactImport>,
    },
    SendDice {
        chat_id: i64,
        emoticon: String,
        reply_to_msg_id: i32,
        top_msg_id: i32,
        persisted_random_id: i64,
        options: SendOptions,
    },
    SendLocation {
        chat_id: i64,
        location: proto::LocationSendSpec,
        reply_to_msg_id: i32,
        top_msg_id: i32,
        persisted_random_id: i64,
        options: SendOptions,
    },
    UpdateLiveLocation {
        chat_id: i64,
        message_id: i32,
        point: GeoPoint,
    },
    StopLiveLocation {
        chat_id: i64,
        message_id: i32,
    },
    ResumeLiveLocation {
        chat_id: i64,
        message_id: i32,
        point: GeoPoint,
    },
    ResolveInlineBot {
        username: String,
    },
    InlineBotResults {
        chat_id: i64,
        bot_id: i64,
        query: String,
        offset: String,
    },
    SendInlineBotResult {
        chat_id: i64,
        query_id: i64,
        result_id: String,
        reply_to_msg_id: i32,
        top_msg_id: i32,
        persisted_random_id: i64,
        options: SendOptions,
    },
    BotCallback {
        chat_id: i64,
        message_id: i32,
        data_hex: String,
    },
    GetMessage {
        chat_id: i64,
        message_id: i32,
    },
    SearchMentions {
        chat_id: i64,
        query: String,
        offset: i32,
        limit: i32,
    },
    SearchMentionContacts {
        query: String,
        limit: i32,
    },
    StickerCatalog {
        emoji: bool,
        hash: i64,
    },
    StickerPackByDocument {
        document_id: i64,
    },
    StickerPack {
        set_id: i64,
        access_hash: i64,
    },
    Stickers {
        emoticon: String,
        hash: i64,
    },
    SavedGifs,
    RecentReactions,
    AnimatedEmojiMax,
    CustomEmojiIsFree {
        document_id: i64,
    },
    ReactionPeers {
        chat_id: i64,
        message_id: i32,
        offset: Option<String>,
    },
    MessageReadParticipants {
        chat_id: i64,
        message_id: i32,
    },
    OutboxReadDate {
        chat_id: i64,
        message_id: i32,
    },
    ReadReceiptConfig,
    ForumTopics {
        chat_id: i64,
        offset_date: i32,
        offset_id: i32,
        offset_topic: i32,
        limit: i32,
    },
    ForumTopicsById {
        chat_id: i64,
        topic_ids: Vec<i32>,
    },
    SetGeneralTopicHidden {
        chat_id: i64,
        hidden: bool,
    },
}

#[derive(Debug, Clone)]
pub enum InteractionResponse {
    Done,
    Outgoing {
        random_id: i64,
        result: Box<proto::OutgoingMessageResult>,
    },
    ContactsImported(ContactImportResult),
    Message(Option<Box<proto::MessageDto>>),
    ResolvedPeer(proto::ResolvedPeerDto),
    InlineBotResults(proto::InlineBotResultsDto),
    BotCallback(proto::BotCallbackAnswerDto),
    Mentions(MentionPage),
    MentionContacts(proto::ContactsSearchDto),
    StickerCatalog(proto::StickerCatalogDto),
    StickerPack(proto::StickerPackDto),
    Stickers(proto::StickerListDto),
    SavedGifs(Vec<proto::SavedGifDto>),
    RecentReactions(Vec<proto::ReactionChoiceDto>),
    AnimatedEmojiMax(i32),
    CustomEmojiIsFree(bool),
    ReactionPeers(ReactionPeersPage),
    MessageReadParticipants(ReadParticipants),
    OutboxReadDate(OutboxRead),
    ReadReceiptConfig(ReadReceiptConfig),
    ForumTopics(proto::ForumTopicsPageDto),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub heading: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MentionMember {
    #[serde(rename = "peer")]
    pub peer_id: i64,
    pub title: String,
    pub username: Option<String>,
    #[serde(rename = "avatar")]
    pub avatar_cache_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MentionPage {
    pub count: i32,
    pub members: Vec<MentionMember>,
    pub next_offset: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionPeer {
    pub peer_id: i64,
    pub title: String,
    pub date: i32,
    pub emoticon: String,
    pub document_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReactionPeersPage {
    pub next_offset: Option<String>,
    pub peers: Vec<ReactionPeer>,
    pub count: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadReceiptIssue {
    ChatTooBig,
    MessageTooOld,
    MessageNotRead,
    InvalidMessage,
    PrivacyRestricted,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadParticipant {
    pub peer_id: i64,
    pub date: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadParticipants {
    pub participants: Vec<ReadParticipant>,
    pub issue: Option<ReadReceiptIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboxRead {
    pub date: Option<i32>,
    pub issue: Option<ReadReceiptIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadReceiptConfig {
    pub chat_read_mark_size_threshold: i32,
    pub chat_read_mark_expire_period: i32,
    pub pm_read_date_expire_period: i32,
    pub from_server: bool,
}

impl Runtime {
    pub(super) fn execute_interaction(
        &mut self,
        command: InteractionCommand,
    ) -> Result<InteractionResponse, ClientError> {
        let handle = self.authorized()?;
        match command {
            InteractionCommand::SendContact {
                chat_id,
                contact,
                reply_to_msg_id,
                top_msg_id,
                persisted_random_id,
                options,
            } => {
                validate_send(
                    chat_id,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    &options,
                )?;
                let result = proto::dispatch::send_contact_with_options(
                    handle,
                    chat_id,
                    contact,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    options,
                )
                .map_err(ClientError::from)?;
                self.replay()?;
                Ok(outgoing(persisted_random_id, result))
            }
            InteractionCommand::ImportContacts { contacts } => {
                if contacts.is_empty() || contacts.len() > 100 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::ContactsImported(
                    proto::contacts::import_contacts(handle, contacts)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::SendDice {
                chat_id,
                emoticon,
                reply_to_msg_id,
                top_msg_id,
                persisted_random_id,
                options,
            } => {
                validate_send(
                    chat_id,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    &options,
                )?;
                let result = proto::dispatch::send_dice_with_options(
                    handle,
                    chat_id,
                    emoticon,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    options,
                )
                .map_err(ClientError::from)?;
                self.replay()?;
                Ok(outgoing(persisted_random_id, result))
            }
            InteractionCommand::SendLocation {
                chat_id,
                location,
                reply_to_msg_id,
                top_msg_id,
                persisted_random_id,
                options,
            } => {
                validate_send(
                    chat_id,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    &options,
                )?;
                let result = proto::dispatch::send_location_with_options(
                    handle,
                    chat_id,
                    location,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    options,
                )
                .map_err(ClientError::from)?;
                self.replay()?;
                Ok(outgoing(persisted_random_id, result))
            }
            InteractionCommand::UpdateLiveLocation {
                chat_id,
                message_id,
                point,
            }
            | InteractionCommand::ResumeLiveLocation {
                chat_id,
                message_id,
                point,
            } => {
                edit_live_location(handle, chat_id, message_id, Some(point), false)?;
                self.replay()?;
                Ok(InteractionResponse::Done)
            }
            InteractionCommand::StopLiveLocation {
                chat_id,
                message_id,
            } => {
                edit_live_location(handle, chat_id, message_id, None, true)?;
                self.replay()?;
                Ok(InteractionResponse::Done)
            }
            InteractionCommand::ResolveInlineBot { username } => {
                if username.trim().trim_start_matches('@').trim().is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::ResolvedPeer(
                    proto::extras::resolve_username(handle, username).map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::InlineBotResults {
                chat_id,
                bot_id,
                query,
                offset,
            } => {
                if bot_id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::InlineBotResults(
                    proto::extras::get_inline_bot_results(handle, chat_id, bot_id, query, offset)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::SendInlineBotResult {
                chat_id,
                query_id,
                result_id,
                reply_to_msg_id,
                top_msg_id,
                persisted_random_id,
                options,
            } => {
                validate_send(
                    chat_id,
                    reply_to_msg_id,
                    top_msg_id,
                    persisted_random_id,
                    &options,
                )?;
                if query_id == 0 || result_id.is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                let result = send_inline_bot_result(
                    handle,
                    InlineBotResultSend {
                        chat_id,
                        query_id,
                        result_id,
                        reply_to_msg_id,
                        top_msg_id,
                        random_id: persisted_random_id,
                        options,
                    },
                )?;
                self.replay()?;
                Ok(outgoing(persisted_random_id, result))
            }
            InteractionCommand::BotCallback {
                chat_id,
                message_id,
                data_hex,
            } => {
                valid_message(chat_id, message_id)?;
                Ok(InteractionResponse::BotCallback(
                    proto::extras::get_bot_callback_answer(handle, chat_id, message_id, data_hex)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::GetMessage {
                chat_id,
                message_id,
            } => {
                valid_message(chat_id, message_id)?;
                Ok(InteractionResponse::Message(
                    proto::extras::get_message(handle, chat_id, message_id)
                        .map_err(ClientError::from)?
                        .map(Box::new),
                ))
            }
            InteractionCommand::SearchMentions {
                chat_id,
                query,
                offset,
                limit,
            } => {
                if chat_id == 0 || offset < 0 {
                    return Err(ClientError::InvalidRequest);
                }
                let limit = page_limit(limit)?;
                let json = proto::dispatch::get_participants(
                    handle,
                    chat_id,
                    "recent".into(),
                    query,
                    offset,
                    limit,
                )
                .map_err(ClientError::from)?;
                let raw: RawMentionPage =
                    serde_json::from_str(&json).map_err(|_| ClientError::Protocol)?;
                if raw.count < 0 {
                    return Err(ClientError::Protocol);
                }
                let next = offset.saturating_add(
                    i32::try_from(raw.members.len()).map_err(|_| ClientError::Protocol)?,
                );
                Ok(InteractionResponse::Mentions(MentionPage {
                    count: raw.count,
                    next_offset: (next > offset && next < raw.count).then_some(next),
                    members: raw.members,
                }))
            }
            InteractionCommand::SearchMentionContacts { query, limit } => {
                Ok(InteractionResponse::MentionContacts(
                    proto::dispatch::contacts_search(handle, query, page_limit(limit)?)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::StickerCatalog { emoji, hash } => {
                let catalog = if emoji {
                    proto::extras::get_emoji_stickers(handle, hash)
                } else {
                    proto::extras::get_all_stickers(handle, hash)
                }
                .map_err(ClientError::from)?;
                Ok(InteractionResponse::StickerCatalog(catalog))
            }
            InteractionCommand::StickerPackByDocument { document_id } => {
                if document_id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::StickerPack(
                    proto::extras::get_sticker_pack(handle, document_id)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::StickerPack {
                set_id,
                access_hash,
            } => {
                if set_id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::StickerPack(
                    proto::extras::get_sticker_set(handle, set_id, access_hash)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::Stickers { emoticon, hash } => {
                if emoticon.is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::Stickers(
                    proto::extras::get_stickers(handle, emoticon, hash)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::SavedGifs => Ok(InteractionResponse::SavedGifs(
                proto::extras::get_saved_gifs(handle).map_err(ClientError::from)?,
            )),
            InteractionCommand::RecentReactions => Ok(InteractionResponse::RecentReactions(
                proto::extras::get_recent_reactions(handle).map_err(ClientError::from)?,
            )),
            InteractionCommand::AnimatedEmojiMax => Ok(InteractionResponse::AnimatedEmojiMax(
                proto::extras::animated_emoji_max(handle).map_err(ClientError::from)?,
            )),
            InteractionCommand::CustomEmojiIsFree { document_id } => {
                if document_id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::CustomEmojiIsFree(
                    proto::extras::custom_emoji_is_free(handle, document_id)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::ReactionPeers {
                chat_id,
                message_id,
                offset,
            } => {
                valid_message(chat_id, message_id)?;
                let page =
                    proto::extras::get_message_reactions_page(handle, chat_id, message_id, offset)
                        .map_err(ClientError::from)?;
                Ok(InteractionResponse::ReactionPeers(ReactionPeersPage {
                    next_offset: page.next_offset,
                    peers: page
                        .peers
                        .into_iter()
                        .map(|peer| ReactionPeer {
                            peer_id: peer.peer_id,
                            title: peer.title,
                            date: peer.date,
                            emoticon: peer.emoticon,
                            document_id: peer.document_id,
                        })
                        .collect(),
                    count: page.count,
                }))
            }
            InteractionCommand::MessageReadParticipants {
                chat_id,
                message_id,
            } => {
                valid_message(chat_id, message_id)?;
                let page =
                    proto::extras::get_message_read_participants(handle, chat_id, message_id)
                        .map_err(ClientError::from)?;
                Ok(InteractionResponse::MessageReadParticipants(
                    ReadParticipants {
                        participants: page
                            .participants
                            .into_iter()
                            .map(|participant| ReadParticipant {
                                peer_id: participant.peer_id,
                                date: participant.date,
                            })
                            .collect(),
                        issue: receipt_issue(page.error.as_deref()),
                    },
                ))
            }
            InteractionCommand::OutboxReadDate {
                chat_id,
                message_id,
            } => {
                valid_message(chat_id, message_id)?;
                let read = proto::extras::get_outbox_read_date(handle, chat_id, message_id)
                    .map_err(ClientError::from)?;
                let issue = receipt_issue(read.error.as_deref());
                Ok(InteractionResponse::OutboxReadDate(OutboxRead {
                    date: (issue.is_none() && read.date >= 0).then_some(read.date),
                    issue,
                }))
            }
            InteractionCommand::ReadReceiptConfig => {
                let config =
                    proto::extras::get_read_receipt_config(handle).map_err(ClientError::from)?;
                Ok(InteractionResponse::ReadReceiptConfig(ReadReceiptConfig {
                    chat_read_mark_size_threshold: config.chat_read_mark_size_threshold,
                    chat_read_mark_expire_period: config.chat_read_mark_expire_period,
                    pm_read_date_expire_period: config.pm_read_date_expire_period,
                    from_server: config.from_server,
                }))
            }
            InteractionCommand::ForumTopics {
                chat_id,
                offset_date,
                offset_id,
                offset_topic,
                limit,
            } => {
                if chat_id == 0 || offset_date < 0 || offset_id < 0 || offset_topic < 0 {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::ForumTopics(
                    proto::dispatch::get_forum_topics(
                        handle,
                        chat_id,
                        offset_date,
                        offset_id,
                        offset_topic,
                        page_limit(limit)?,
                    )
                    .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::ForumTopicsById { chat_id, topic_ids } => {
                if chat_id == 0
                    || topic_ids.is_empty()
                    || topic_ids.len() > 100
                    || topic_ids.iter().any(|id| *id <= 0)
                    || topic_ids
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != topic_ids.len()
                {
                    return Err(ClientError::InvalidRequest);
                }
                Ok(InteractionResponse::ForumTopics(
                    proto::dispatch::get_forum_topics_by_id(handle, chat_id, topic_ids)
                        .map_err(ClientError::from)?,
                ))
            }
            InteractionCommand::SetGeneralTopicHidden { chat_id, hidden } => {
                if chat_id == 0 {
                    return Err(ClientError::InvalidRequest);
                }
                proto::dispatch::edit_forum_topic_hidden(handle, chat_id, 1, hidden)
                    .map_err(ClientError::from)?;
                self.replay()?;
                Ok(InteractionResponse::Done)
            }
        }
    }
}

#[derive(Deserialize)]
struct RawMentionPage {
    count: i32,
    members: Vec<MentionMember>,
}

fn outgoing(random_id: i64, result: proto::OutgoingMessageResult) -> InteractionResponse {
    InteractionResponse::Outgoing {
        random_id,
        result: Box::new(result),
    }
}

fn validate_send(
    chat_id: i64,
    reply_to_msg_id: i32,
    top_msg_id: i32,
    persisted_random_id: i64,
    options: &SendOptions,
) -> Result<(), ClientError> {
    if chat_id == 0
        || reply_to_msg_id < 0
        || top_msg_id < 0
        || persisted_random_id == 0
        || options.no_webpage
    {
        return Err(ClientError::InvalidRequest);
    }
    options
        .validate(persisted_random_id)
        .map_err(ClientError::from)
}

fn validate_point(point: GeoPoint) -> Result<(), ClientError> {
    if !point.latitude.is_finite()
        || !point.longitude.is_finite()
        || !(-90.0..=90.0).contains(&point.latitude)
        || !(-180.0..=180.0).contains(&point.longitude)
        || point
            .heading
            .is_some_and(|heading| !(1..=360).contains(&heading))
    {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn edit_live_location(
    handle: u64,
    chat_id: i64,
    message_id: i32,
    point: Option<GeoPoint>,
    stopped: bool,
) -> Result<(), ClientError> {
    valid_message(chat_id, message_id)?;
    if stopped != point.is_none() {
        return Err(ClientError::InvalidRequest);
    }
    if let Some(point) = point {
        validate_point(point)?;
        proto::extras::edit_live_location(
            handle,
            chat_id,
            message_id,
            Some(point.latitude),
            Some(point.longitude),
            point.heading,
            stopped,
        )
        .map_err(ClientError::from)
    } else {
        proto::extras::edit_live_location(handle, chat_id, message_id, None, None, None, stopped)
            .map_err(ClientError::from)
    }
}

fn receipt_issue(error: Option<&str>) -> Option<ReadReceiptIssue> {
    error.map(|error| {
        if error.contains("CHAT_TOO_BIG") {
            ReadReceiptIssue::ChatTooBig
        } else if error.contains("MSG_TOO_OLD") || error.contains("MESSAGE_TOO_OLD") {
            ReadReceiptIssue::MessageTooOld
        } else if error.contains("MESSAGE_NOT_READ_YET") {
            ReadReceiptIssue::MessageNotRead
        } else if error.contains("MSG_ID_INVALID") || error.contains("MESSAGE_ID_INVALID") {
            ReadReceiptIssue::InvalidMessage
        } else if error.contains("PRIVACY_RESTRICTED") {
            ReadReceiptIssue::PrivacyRestricted
        } else {
            ReadReceiptIssue::Unavailable
        }
    })
}

pub(super) fn send_inline_bot_result(
    handle: u64,
    send: InlineBotResultSend,
) -> Result<proto::OutgoingMessageResult, ClientError> {
    use proto::adapter::protocol::latest::api::{
        MessagesSendInlineBotResultRequest, True, TrueConstructor, Updates,
    };

    send.options
        .validate(send.random_id)
        .map_err(ClientError::from)?;
    let client = proto::registry::get_client(handle).map_err(ClientError::from)?;
    let (result, session_id) = proto::access::with_interactive_client_mut(handle, |state| {
        let generation = state.journal.account_generation;
        if !state.journal.admission_open() {
            return Err(proto::MtprotoError::Message(
                "outgoing receipt journal requires acknowledgment".into(),
            ));
        }
        let result = proto::session::call_with_migrate(state, |state| {
            proto::media_download::with_peer_refresh(state, send.chat_id, |state| {
                let peer = input_peer(state, send.chat_id)?;
                let reply_to = inline_reply(send.reply_to_msg_id, send.top_msg_id);
                let mut flags = MessagesSendInlineBotResultRequest::CLEAR_DRAFT_FLAG;
                if reply_to.is_some() {
                    flags |= MessagesSendInlineBotResultRequest::REPLY_TO_FLAG;
                }
                if send.options.silent {
                    flags |= MessagesSendInlineBotResultRequest::SILENT_FLAG;
                }
                if send.options.schedule_date.is_some() {
                    flags |= MessagesSendInlineBotResultRequest::SCHEDULE_DATE_FLAG;
                }
                let updates: Updates = proto::adapter::runtime::api_invoke::invoke_api(
                    &mut state.snapshot,
                    state.api_id,
                    MessagesSendInlineBotResultRequest {
                        flags,
                        silent: send
                            .options
                            .silent
                            .then(|| Box::new(True::True(TrueConstructor {}))),
                        background: None,
                        clear_draft: Some(Box::new(True::True(TrueConstructor {}))),
                        hide_via: None,
                        peer: Box::new(peer),
                        reply_to,
                        random_id: send.random_id,
                        query_id: send.query_id,
                        id: send.result_id.clone(),
                        schedule_date: send.options.schedule_date,
                        send_as: None,
                        quick_reply_shortcut: None,
                        allow_paid_stars: None,
                    },
                )?;
                proto::management::cache_response_peers(state, &updates);
                proto::adapter::messages::messages_send_result::send_result(
                    updates,
                    send.chat_id,
                    "",
                    send.random_id,
                    &state.peers,
                    &mut state.media,
                )
            })
        })?;
        let session_id = proto::updates::stage_outgoing_result(
            &client,
            generation,
            send.random_id,
            result.clone(),
        )?;
        Ok((result, session_id))
    })
    .map_err(ClientError::from)?;
    proto::persist::persist_journal_sync(&client, session_id).map_err(ClientError::from)?;
    Ok(result)
}

fn inline_reply(
    reply_to_msg_id: i32,
    top_msg_id: i32,
) -> Option<Box<proto::adapter::protocol::latest::api::InputReplyTo>> {
    use proto::adapter::protocol::latest::api::{InputReplyTo, InputReplyToMessageConstructor};

    if reply_to_msg_id <= 0 && top_msg_id <= 0 {
        return None;
    }
    Some(Box::new(InputReplyTo::InputReplyToMessage(
        InputReplyToMessageConstructor {
            flags: if top_msg_id > 0 {
                InputReplyToMessageConstructor::TOP_MSG_ID_FLAG
            } else {
                0
            },
            reply_to_msg_id: if reply_to_msg_id > 0 {
                reply_to_msg_id
            } else {
                top_msg_id
            },
            top_msg_id: (top_msg_id > 0).then_some(top_msg_id),
            reply_to_peer_id: None,
            quote_text: None,
            quote_entities: None,
            quote_offset: None,
            monoforum_peer_id: None,
            todo_item_id: None,
            poll_option: None,
            quote_rich: None,
        },
    )))
}

fn input_peer(
    state: &proto::state::ClientState,
    chat_id: i64,
) -> Result<proto::adapter::protocol::latest::api::InputPeer, proto::MtprotoError> {
    use proto::adapter::protocol::latest::api::{
        InputPeer, InputPeerChannelConstructor, InputPeerChatConstructor, InputPeerUserConstructor,
    };

    let cached = state
        .peers
        .get(&chat_id)
        .ok_or_else(|| proto::MtprotoError::Message("unknown peer; refresh chats first".into()))?;
    if chat_id > 0 {
        if cached.min_hash || cached.access_hash == 0 {
            return Err(proto::MtprotoError::Message(
                "peer access hash unavailable".into(),
            ));
        }
        Ok(InputPeer::InputPeerUser(InputPeerUserConstructor {
            user_id: cached.id,
            access_hash: cached.access_hash,
        }))
    } else if chat_id <= -1_000_000_000_000 {
        if cached.min_hash || cached.access_hash == 0 {
            return Err(proto::MtprotoError::Message(
                "peer access hash unavailable".into(),
            ));
        }
        Ok(InputPeer::InputPeerChannel(InputPeerChannelConstructor {
            channel_id: cached.id,
            access_hash: cached.access_hash,
        }))
    } else {
        Ok(InputPeer::InputPeerChat(InputPeerChatConstructor {
            chat_id: cached.id,
        }))
    }
}
