use super::*;

impl Runtime {
    fn refresh_folders(&mut self) -> Result<WorkerResponse, ClientError> {
        let folders =
            proto::dispatch::get_folders(self.authorized()?).map_err(ClientError::from)?;
        self.storage
            .cache_folders(&folders)
            .map_err(|_| ClientError::Storage)?;
        Ok(WorkerResponse::Folders(folders))
    }

    fn refresh_scheduled(&mut self, chat_id: i64) -> Result<WorkerResponse, ClientError> {
        valid_chat(chat_id)?;
        self.drain()?;
        let messages = proto::scheduled::get_scheduled_history(self.authorized()?, chat_id)
            .map_err(ClientError::from)?;
        self.storage
            .replace_scheduled(chat_id, &messages)
            .map_err(|_| ClientError::Storage)?;
        self.drain()?;
        Ok(WorkerResponse::Scheduled {
            chat_id,
            messages: self
                .storage
                .scheduled(chat_id)
                .map_err(|_| ClientError::Storage)?,
        })
    }

    fn save_draft(
        &mut self,
        mut draft: proto::CloudDraft,
        expected_revision: i64,
    ) -> Result<WorkerResponse, ClientError> {
        let handle = self.authorized()?;
        valid_chat(draft.chat_id)?;
        if draft.thread_id < 0 || draft.reply_to_message_id < 0 || expected_revision < 0 {
            return Err(ClientError::InvalidRequest);
        }
        valid_entities(draft.entities_json.as_deref())?;
        let remote = proto::drafts::get_cloud_drafts(handle).map_err(ClientError::from)?;
        if remote.iter().any(|value| {
            value.chat_id == draft.chat_id
                && value.thread_id == draft.thread_id
                && value.has_unsupported_content
        }) {
            return Err(ClientError::UnsupportedContent);
        }
        draft.date = unix_date()?;
        self.storage
            .reconcile_drafts(&remote, draft.date)
            .map_err(|_| ClientError::Storage)?;
        let current = self
            .storage
            .draft_revision(draft.chat_id, draft.thread_id)
            .map_err(|_| ClientError::Storage)?;
        if current.as_ref().map_or(0, |value| value.revision) != expected_revision {
            return Err(ClientError::RevisionConflict);
        }
        if draft.has_unsupported_content
            || current
                .as_ref()
                .is_some_and(|value| value.draft.has_unsupported_content)
        {
            return Err(ClientError::UnsupportedContent);
        }
        let staged = self
            .storage
            .stage_draft(&draft, expected_revision)
            .map_err(|_| ClientError::Storage)?
            .ok_or(ClientError::RevisionConflict)?;
        proto::drafts::save_cloud_draft(handle, draft.clone()).map_err(ClientError::from)?;
        self.storage
            .acknowledge_draft(draft.chat_id, draft.thread_id, staged.revision)
            .map_err(|_| ClientError::Storage)?;
        Ok(WorkerResponse::Draft(crate::domain::DraftRevision {
            pending: false,
            ..staged
        }))
    }

    pub(super) fn send_special(
        &mut self,
        request_id: u64,
        send: crate::domain::SpecialSend,
    ) -> Result<WorkerResponse, ClientError> {
        self.authorized()?;
        send.validate()?;
        let random_id = self
            .storage
            .enqueue_special(&send)
            .map_err(|_| ClientError::Storage)?;
        self.emit(WorkerEvent::SpecialQueued {
            request_id,
            random_id,
        });
        self.send_pending_special(random_id)
    }

    fn send_pending_special(&mut self, random_id: i64) -> Result<WorkerResponse, ClientError> {
        let handle = self.authorized()?;
        self.replay()?;
        let pending = self
            .storage
            .pending_special_send(random_id)
            .map_err(|_| ClientError::Storage)?
            .ok_or(ClientError::InvalidRequest)?;
        pending.send.validate()?;
        self.storage
            .mark_special_attempted(random_id)
            .map_err(|_| ClientError::Storage)?;
        let send = pending.send;
        let options = SendOptions {
            silent: send.silent,
            no_webpage: false,
            schedule_date: send.schedule_date,
        };
        let result = match send.content {
            crate::domain::SpecialContent::Poll(spec) => proto::dispatch::send_poll_with_options(
                handle,
                send.chat_id,
                spec,
                send.reply_to_msg_id,
                send.top_msg_id,
                random_id,
                options,
            )
            .map_err(ClientError::from)?,
            crate::domain::SpecialContent::Checklist(spec) => {
                proto::dispatch::send_checklist_with_options(
                    handle,
                    send.chat_id,
                    spec,
                    send.reply_to_msg_id,
                    send.top_msg_id,
                    random_id,
                    options,
                )
                .map_err(ClientError::from)?
            }
            crate::domain::SpecialContent::Contact(spec) => {
                proto::dispatch::send_contact_with_options(
                    handle,
                    send.chat_id,
                    spec,
                    send.reply_to_msg_id,
                    send.top_msg_id,
                    random_id,
                    options,
                )
                .map_err(ClientError::from)?
            }
            crate::domain::SpecialContent::Dice { emoticon } => {
                proto::dispatch::send_dice_with_options(
                    handle,
                    send.chat_id,
                    emoticon,
                    send.reply_to_msg_id,
                    send.top_msg_id,
                    random_id,
                    options,
                )
                .map_err(ClientError::from)?
            }
            crate::domain::SpecialContent::Location(spec) => {
                proto::dispatch::send_location_with_options(
                    handle,
                    send.chat_id,
                    spec,
                    send.reply_to_msg_id,
                    send.top_msg_id,
                    random_id,
                    options,
                )
                .map_err(ClientError::from)?
            }
            crate::domain::SpecialContent::InlineBotResult {
                query_id,
                result_id,
            } => super::interactions::send_inline_bot_result(
                handle,
                super::interactions::InlineBotResultSend {
                    chat_id: send.chat_id,
                    query_id,
                    result_id,
                    reply_to_msg_id: send.reply_to_msg_id,
                    top_msg_id: send.top_msg_id,
                    random_id,
                    options,
                },
            )?,
        };
        self.replay()?;
        Ok(WorkerResponse::SpecialOutgoing {
            random_id,
            result: Box::new(result),
        })
    }

    fn edit_checklist(
        &mut self,
        handle: u64,
        chat_id: i64,
        message_id: i32,
        edit: crate::domain::ChecklistTextEdit,
        item: bool,
    ) -> Result<WorkerResponse, ClientError> {
        valid_message(chat_id, message_id)?;
        edit.validate(false)?;
        if item != edit.item_id.is_some() {
            return Err(ClientError::InvalidRequest);
        }
        proto::checklists::edit_checklist_text(handle, chat_id, message_id, checklist_edit(edit))
            .map_err(ClientError::from)?;
        self.drain()?;
        Ok(WorkerResponse::Done)
    }

    pub(super) fn execute_features(
        &mut self,
        command: WorkerCommand,
    ) -> Result<WorkerResponse, ClientError> {
        let handle = self.authorized()?;
        match command {
            WorkerCommand::Interaction(command) => self
                .execute_interaction(command)
                .map(WorkerResponse::Interaction),
            WorkerCommand::Web(command) => self.execute_web(command).map(WorkerResponse::Web),
            WorkerCommand::PendingSpecial => {
                self.replay()?;
                Ok(WorkerResponse::PendingSpecial(
                    self.storage
                        .pending_special()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::RetrySpecial { random_id } => self.send_pending_special(random_id),
            WorkerCommand::VotePoll {
                chat_id,
                message_id,
                options,
            } => {
                valid_message(chat_id, message_id)?;
                crate::domain::validate_poll_vote(&options)?;
                proto::extras::send_poll_vote(handle, chat_id, message_id, options)
                    .map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::RetractPollVote {
                chat_id,
                message_id,
            } => {
                valid_message(chat_id, message_id)?;
                proto::extras::send_poll_vote(handle, chat_id, message_id, Vec::new())
                    .map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::ClosePoll {
                chat_id,
                message_id,
            } => {
                valid_message(chat_id, message_id)?;
                proto::polls::close_poll(handle, chat_id, message_id).map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::PollVoters {
                chat_id,
                message_id,
                offset,
            } => {
                valid_message(chat_id, message_id)?;
                let page = proto::extras::get_poll_votes_page(handle, chat_id, message_id, offset)
                    .map_err(ClientError::from)?;
                Ok(WorkerResponse::PollVoters {
                    chat_id,
                    message_id,
                    page,
                })
            }
            WorkerCommand::AppendChecklist {
                chat_id,
                message_id,
                first_id,
                titles,
            } => {
                valid_message(chat_id, message_id)?;
                crate::domain::validate_append_checklist(first_id, &titles)?;
                proto::extras::append_todo_items(handle, chat_id, message_id, first_id, titles)
                    .map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::ToggleChecklistCompleted {
                chat_id,
                message_id,
                completed,
                incompleted,
            } => {
                valid_message(chat_id, message_id)?;
                crate::domain::validate_checklist_completion(&completed, &incompleted)?;
                proto::extras::toggle_todo_completed(
                    handle,
                    chat_id,
                    message_id,
                    completed,
                    incompleted,
                )
                .map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::EditChecklistTitle {
                chat_id,
                message_id,
                edit,
            } => self.edit_checklist(handle, chat_id, message_id, edit, false),
            WorkerCommand::EditChecklistItem {
                chat_id,
                message_id,
                edit,
            } => self.edit_checklist(handle, chat_id, message_id, edit, true),
            WorkerCommand::DeleteChecklistItem {
                chat_id,
                message_id,
                edit,
            } => {
                valid_message(chat_id, message_id)?;
                edit.validate(true)?;
                proto::checklists::remove_checklist_item(
                    handle,
                    chat_id,
                    message_id,
                    checklist_edit(edit),
                )
                .map_err(ClientError::from)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::ContactsSearch { query, limit } => Ok(WorkerResponse::ContactsSearch(
                proto::dispatch::contacts_search(handle, query, page_limit(limit)?)
                    .map_err(ClientError::from)?,
            )),
            WorkerCommand::SearchGlobal {
                query,
                offset_rate,
                offset_peer_id,
                offset_id,
                limit,
                folder_id,
            } => Ok(WorkerResponse::GlobalSearch(
                proto::dispatch::search_global(
                    handle,
                    query,
                    offset_rate,
                    offset_peer_id,
                    offset_id,
                    page_limit(limit)?,
                    folder_id,
                )
                .map_err(ClientError::from)?,
            )),
            WorkerCommand::SearchMessagesPage {
                chat_id,
                query,
                filter,
                offset_id,
                add_offset,
                limit,
            } => {
                valid_chat(chat_id)?;
                valid_filter(&filter)?;
                let page = proto::dispatch::search_messages_page(
                    handle,
                    chat_id,
                    query,
                    filter,
                    offset_id,
                    add_offset,
                    page_limit(limit)?,
                )
                .map_err(ClientError::from)?;
                Ok(WorkerResponse::MessagesPage { chat_id, page })
            }
            WorkerCommand::GetProfile { peer_id } => {
                let profile =
                    proto::dispatch::get_profile(handle, peer_id).map_err(ClientError::from)?;
                self.storage
                    .cache_profile(&profile)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::Profile(Some(Box::new(profile))))
            }
            WorkerCommand::GetCachedProfile { peer_id } => {
                let peer_id = if peer_id == 0 {
                    match self.state.auth {
                        AuthState::Authorized { user_id } => user_id,
                        _ => return Err(ClientError::Unauthorized),
                    }
                } else {
                    peer_id
                };
                Ok(WorkerResponse::Profile(
                    self.storage
                        .profile(peer_id)
                        .map_err(|_| ClientError::Storage)?
                        .map(Box::new),
                ))
            }
            WorkerCommand::GetMembers {
                chat_id,
                filter,
                query,
                offset,
            } => {
                valid_chat(chat_id)?;
                if offset < 0 {
                    return Err(ClientError::InvalidRequest);
                }
                if !query.is_empty()
                    || !matches!(filter.as_str(), "" | "recent" | "admins" | "bots")
                    || (chat_id > -1_000_000_000_000
                        && matches!(filter.as_str(), "admins" | "bots"))
                {
                    return Err(ClientError::UnsupportedContent);
                }
                let json =
                    proto::dispatch::get_participants(handle, chat_id, filter, query, offset, 50)
                        .map_err(ClientError::from)?;
                Ok(WorkerResponse::Members { chat_id, json })
            }
            WorkerCommand::GetCommonChats {
                user_id,
                max_id,
                limit,
            } => {
                valid_chat(user_id)?;
                Ok(WorkerResponse::CommonChats {
                    user_id,
                    json: proto::dispatch::get_common_chats(
                        handle,
                        user_id,
                        max_id,
                        page_limit(limit)?,
                    )
                    .map_err(ClientError::from)?,
                })
            }
            WorkerCommand::GetFolders => self.refresh_folders(),
            WorkerCommand::GetCachedFolders => Ok(WorkerResponse::Folders(
                self.storage.folders().map_err(|_| ClientError::Storage)?,
            )),
            WorkerCommand::SaveFolder(folder) => {
                if folder.id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                proto::dispatch::update_folder(handle, folder).map_err(ClientError::from)?;
                self.refresh_folders()
            }
            WorkerCommand::DeleteFolder { id } => {
                if id <= 0 {
                    return Err(ClientError::InvalidRequest);
                }
                proto::dispatch::delete_folder(handle, id).map_err(ClientError::from)?;
                self.refresh_folders()
            }
            WorkerCommand::ReorderFolders { order } => {
                if order.iter().any(|id| *id < 0)
                    || order.iter().collect::<std::collections::HashSet<_>>().len() != order.len()
                {
                    return Err(ClientError::InvalidRequest);
                }
                proto::dispatch::update_folder_order(handle, order).map_err(ClientError::from)?;
                self.refresh_folders()
            }
            WorkerCommand::GetReplies {
                chat_id,
                message_id,
                limit,
                offset_id,
                add_offset,
            } => {
                valid_message(chat_id, message_id)?;
                let messages = proto::dispatch::get_replies(
                    handle,
                    chat_id,
                    message_id,
                    page_limit(limit)?,
                    offset_id,
                    add_offset,
                )
                .map_err(ClientError::from)?;
                self.history_projection(chat_id, messages)
            }
            WorkerCommand::GetPinnedMessages { chat_id, limit } => {
                valid_chat(chat_id)?;
                let messages =
                    proto::dispatch::get_pinned_messages(handle, chat_id, page_limit(limit)?)
                        .map_err(ClientError::from)?;
                self.history_projection(chat_id, messages)
            }
            WorkerCommand::ReadHistory { chat_id, max_id } => {
                valid_chat(chat_id)?;
                if max_id < 0 {
                    return Err(ClientError::InvalidRequest);
                }
                proto::dispatch::read_history(handle, chat_id, max_id)
                    .map_err(ClientError::from)?;
                self.storage
                    .mark_read(chat_id, max_id)
                    .map_err(|_| ClientError::Storage)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::MarkDialogUnread { chat_id, unread } => {
                valid_chat(chat_id)?;
                proto::dispatch::mark_dialog_unread(handle, chat_id, unread)
                    .map_err(ClientError::from)?;
                self.storage
                    .mark_unread(chat_id, unread)
                    .map_err(|_| ClientError::Storage)?;
                self.drain()?;
                Ok(WorkerResponse::Done)
            }
            WorkerCommand::GetCloudDrafts => {
                self.drain()?;
                let drafts = proto::drafts::get_cloud_drafts(handle).map_err(ClientError::from)?;
                self.storage
                    .reconcile_drafts(&drafts, unix_date()?)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::Drafts(
                    self.storage.drafts().map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::GetCachedCloudDrafts => Ok(WorkerResponse::Drafts(
                self.storage.drafts().map_err(|_| ClientError::Storage)?,
            )),
            WorkerCommand::SaveCloudDraft {
                draft,
                expected_revision,
            } => self.save_draft(draft, expected_revision),
            WorkerCommand::DiscardCloudDraft {
                chat_id,
                thread_id,
                expected_revision,
            } => {
                let draft = proto::CloudDraft {
                    chat_id,
                    thread_id,
                    text: String::new(),
                    entities_json: None,
                    reply_to_message_id: 0,
                    date: 0,
                    no_webpage: false,
                    has_unsupported_content: false,
                };
                self.save_draft(draft, expected_revision)
            }
            WorkerCommand::GetScheduledHistory { chat_id } => self.refresh_scheduled(chat_id),
            WorkerCommand::GetCachedScheduledHistory { chat_id } => {
                valid_chat(chat_id)?;
                Ok(WorkerResponse::Scheduled {
                    chat_id,
                    messages: self
                        .storage
                        .scheduled(chat_id)
                        .map_err(|_| ClientError::Storage)?,
                })
            }
            WorkerCommand::SendScheduledNow {
                chat_id,
                message_ids,
            } => {
                valid_selection(chat_id, &message_ids)?;
                proto::scheduled::send_scheduled_messages(handle, chat_id, message_ids)
                    .map_err(ClientError::from)?;
                self.drain()?;
                self.refresh_scheduled(chat_id)
            }
            WorkerCommand::DeleteScheduled {
                chat_id,
                message_ids,
            } => {
                valid_selection(chat_id, &message_ids)?;
                proto::scheduled::delete_scheduled_messages(handle, chat_id, message_ids)
                    .map_err(ClientError::from)?;
                self.drain()?;
                self.refresh_scheduled(chat_id)
            }
            WorkerCommand::Reschedule {
                chat_id,
                message_id,
                schedule_date,
            } => {
                valid_message(chat_id, message_id)?;
                proto::scheduled::edit_scheduled_message(
                    handle,
                    chat_id,
                    message_id,
                    schedule_date,
                    None,
                    None,
                    false,
                )
                .map_err(ClientError::from)?;
                self.drain()?;
                self.refresh_scheduled(chat_id)
            }
            WorkerCommand::EditScheduled {
                chat_id,
                message_id,
                schedule_date,
                text,
                entities_json,
                no_webpage,
            } => {
                valid_message(chat_id, message_id)?;
                valid_entities(entities_json.as_deref())?;
                proto::scheduled::edit_scheduled_message(
                    handle,
                    chat_id,
                    message_id,
                    schedule_date,
                    Some(text),
                    entities_json,
                    no_webpage,
                )
                .map_err(ClientError::from)?;
                self.drain()?;
                self.refresh_scheduled(chat_id)
            }
            _ => Err(ClientError::InvalidRequest),
        }
    }

    fn history_projection(
        &mut self,
        chat_id: i64,
        messages: Vec<MessageDto>,
    ) -> Result<WorkerResponse, ClientError> {
        self.storage
            .cache_messages(&messages)
            .map_err(|_| ClientError::Storage)?;
        Ok(WorkerResponse::History {
            chat_id,
            messages: self
                .storage
                .project_messages(&messages)
                .map_err(|_| ClientError::Storage)?,
        })
    }
}

fn checklist_edit(
    edit: crate::domain::ChecklistTextEdit,
) -> proto::checklists::ChecklistTextEditDto {
    proto::checklists::ChecklistTextEditDto {
        item_id: edit.item_id,
        text: edit.text,
        entities_json: edit.entities_json,
        expected_text: edit.expected_text,
        expected_entities_json: edit.expected_entities_json,
    }
}

fn unix_date() -> Result<i32, ClientError> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ClientError::InvalidRequest)?
        .as_secs();
    i32::try_from(seconds).map_err(|_| ClientError::InvalidRequest)
}

pub(super) fn valid_chat(chat_id: i64) -> Result<(), ClientError> {
    if chat_id == 0 {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}

pub(super) fn valid_selection(chat_id: i64, ids: &[i32]) -> Result<(), ClientError> {
    valid_chat(chat_id)?;
    if ids.is_empty()
        || ids.len() > 100
        || ids.iter().any(|id| *id <= 0)
        || ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len()
    {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}

pub(super) fn valid_filter(filter: &str) -> Result<(), ClientError> {
    if matches!(
        filter,
        "" | "photo_video"
            | "document"
            | "url"
            | "gif"
            | "voice"
            | "music"
            | "photos"
            | "videos"
            | "chat_photos"
            | "pinned"
    ) {
        Ok(())
    } else {
        Err(ClientError::UnsupportedContent)
    }
}

pub(super) fn valid_entities(value: Option<&str>) -> Result<(), ClientError> {
    if let Some(value) = value
        && !serde_json::from_str::<serde_json::Value>(value)
            .map_err(|_| ClientError::InvalidRequest)?
            .is_array()
    {
        return Err(ClientError::InvalidRequest);
    }
    Ok(())
}
