use super::*;
use crate::domain::{DraftRevision, StoredFolder, StoredProfile};
use mono_proto_client::{CloudDraft, FolderDto, ProfileDto, ScheduledMessageDto};

impl Storage {
    pub fn cache_profile(&self, profile: &ProfileDto) -> Result<()> {
        self.connection.execute(
            "INSERT INTO profiles(peer_id,payload) VALUES(?1,?2) ON CONFLICT(peer_id) DO UPDATE SET payload=excluded.payload",
            params![profile.id, encode(&StoredProfile(profile.clone()))?],
        )?;
        Ok(())
    }

    pub fn profile(&self, peer_id: i64) -> Result<Option<ProfileDto>> {
        self.connection
            .query_row(
                "SELECT payload FROM profiles WHERE peer_id=?1",
                [peer_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| decode::<StoredProfile>(&payload).map(|value| value.0))
            .transpose()
    }

    pub fn cache_folders(&mut self, folders: &[FolderDto]) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM folders", [])?;
        for (position, folder) in folders.iter().enumerate() {
            tx.execute(
                "INSERT INTO folders(folder_id,position,payload) VALUES(?1,?2,?3)",
                params![
                    folder.id,
                    position as i64,
                    encode(&StoredFolder(folder.clone()))?
                ],
            )?;
        }
        tx.commit()
    }

    pub fn folders(&self) -> Result<Vec<FolderDto>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM folders ORDER BY position")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| decode::<StoredFolder>(&row?).map(|value| value.0))
            .collect()
    }

    pub fn drafts(&self) -> Result<Vec<DraftRevision>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload,revision,pending FROM drafts ORDER BY chat_id,thread_id")?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?
            .map(|row| {
                let (payload, revision, pending) = row?;
                Ok(DraftRevision {
                    draft: decode(&payload)?,
                    revision,
                    pending,
                })
            })
            .collect()
    }

    pub fn draft_revision(&self, chat_id: i64, thread_id: i32) -> Result<Option<DraftRevision>> {
        let row = self
            .connection
            .query_row(
                "SELECT payload,revision,pending FROM drafts WHERE chat_id=?1 AND thread_id=?2",
                params![chat_id, thread_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(payload, revision, pending)| {
            Ok(DraftRevision {
                draft: decode(&payload)?,
                revision,
                pending,
            })
        })
        .transpose()
    }

    pub fn stage_draft(
        &mut self,
        draft: &CloudDraft,
        expected_revision: i64,
    ) -> Result<Option<DraftRevision>> {
        let tx = self.connection.transaction()?;
        let current = tx
            .query_row(
                "SELECT revision,payload FROM drafts WHERE chat_id=?1 AND thread_id=?2",
                params![draft.chat_id, draft.thread_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let revision = current.as_ref().map_or(0, |row| row.0);
        if revision != expected_revision {
            return Ok(None);
        }
        if current
            .as_ref()
            .map(|row| decode::<CloudDraft>(&row.1))
            .transpose()?
            .is_some_and(|value| value.has_unsupported_content)
            || draft.has_unsupported_content
        {
            return Err(Error::InvalidQuery);
        }
        let revision = revision
            .checked_add(1)
            .filter(|value| *value > 0)
            .ok_or(Error::InvalidQuery)?;
        tx.execute("INSERT INTO drafts(chat_id,thread_id,payload,revision,pending) VALUES(?1,?2,?3,?4,1) ON CONFLICT(chat_id,thread_id) DO UPDATE SET payload=excluded.payload,revision=excluded.revision,pending=1", params![draft.chat_id,draft.thread_id,encode(draft)?,revision])?;
        tx.commit()?;
        Ok(Some(DraftRevision {
            draft: draft.clone(),
            revision,
            pending: true,
        }))
    }

    pub fn acknowledge_draft(&self, chat_id: i64, thread_id: i32, revision: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE drafts SET pending=0 WHERE chat_id=?1 AND thread_id=?2 AND revision=?3",
            params![chat_id, thread_id, revision],
        )?;
        Ok(())
    }

    pub fn cache_drafts(&mut self, drafts: &[CloudDraft]) -> Result<()> {
        let tx = self.connection.transaction()?;
        for draft in drafts {
            save_draft(&tx, draft)?;
        }
        tx.commit()
    }

    pub fn reconcile_drafts(&mut self, drafts: &[CloudDraft], date: i32) -> Result<()> {
        let previous = self.drafts()?;
        let tx = self.connection.transaction()?;
        for draft in drafts {
            save_draft(&tx, draft)?;
        }
        for previous in previous {
            if !drafts.iter().any(|draft| {
                draft.chat_id == previous.draft.chat_id
                    && draft.thread_id == previous.draft.thread_id
            }) {
                let mut empty = previous.draft;
                if empty.text.is_empty()
                    && empty.entities_json.is_none()
                    && empty.reply_to_message_id == 0
                    && !empty.no_webpage
                    && !empty.has_unsupported_content
                {
                    continue;
                }
                empty.text.clear();
                empty.entities_json = None;
                empty.reply_to_message_id = 0;
                empty.no_webpage = false;
                empty.has_unsupported_content = false;
                empty.date = date.max(empty.date);
                save_draft(&tx, &empty)?;
            }
        }
        tx.commit()
    }

    pub fn replace_scheduled(
        &mut self,
        chat_id: i64,
        messages: &[ScheduledMessageDto],
    ) -> Result<()> {
        if messages
            .iter()
            .any(|value| value.message.chat_id != chat_id)
        {
            return Err(Error::InvalidQuery);
        }
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM scheduled WHERE chat_id=?1", [chat_id])?;
        for message in messages {
            save_scheduled(&tx, message)?;
        }
        tx.commit()
    }

    pub fn scheduled(&self, chat_id: i64) -> Result<Vec<ScheduledMessageDto>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM scheduled WHERE chat_id=?1 ORDER BY message_id DESC")?;
        statement
            .query_map([chat_id], |row| row.get::<_, String>(0))?
            .map(|row| decode(&row?))
            .collect()
    }

    pub fn mark_read(&mut self, chat_id: i64, max_id: i32) -> Result<()> {
        let tx = self.connection.transaction()?;
        let last_id = tx
            .query_row(
                "SELECT json_extract(payload,'$.last_message_id') FROM chats WHERE chat_id=?1",
                [chat_id],
                |row| row.get::<_, i32>(0),
            )
            .optional()?;
        let inbox = if max_id == 0 {
            last_id.unwrap_or(0)
        } else {
            max_id
        };
        let unread = if max_id == 0 || last_id.is_some_and(|id| max_id >= id) {
            Some(0)
        } else {
            None::<i32>
        };
        tx.execute("INSERT INTO chat_reads(chat_id,inbox,unread) VALUES(?1,?2,?3) ON CONFLICT(chat_id) DO UPDATE SET inbox=MAX(inbox,excluded.inbox),unread=COALESCE(excluded.unread,unread)", params![chat_id,inbox,unread])?;
        tx.execute("INSERT INTO dialog_marks(chat_id,unread) VALUES(?1,0) ON CONFLICT(chat_id) DO UPDATE SET unread=0", [chat_id])?;
        tx.commit()
    }

    pub fn mark_unread(&self, chat_id: i64, unread: bool) -> Result<()> {
        self.connection.execute("INSERT INTO dialog_marks(chat_id,unread) VALUES(?1,?2) ON CONFLICT(chat_id) DO UPDATE SET unread=excluded.unread", params![chat_id,unread])?;
        Ok(())
    }
}

pub(super) fn save_draft(tx: &Transaction<'_>, draft: &CloudDraft) -> Result<()> {
    tx.execute("INSERT INTO drafts(chat_id,thread_id,payload,revision,pending) VALUES(?1,?2,?3,1,0) ON CONFLICT(chat_id,thread_id) DO UPDATE SET payload=excluded.payload,revision=drafts.revision+1 WHERE drafts.pending=0 AND json_extract(drafts.payload,'$.date') <= ?4 AND drafts.payload != excluded.payload", params![draft.chat_id,draft.thread_id,encode(draft)?,draft.date])?;
    Ok(())
}

pub(super) fn save_scheduled(tx: &Transaction<'_>, message: &ScheduledMessageDto) -> Result<()> {
    tx.execute("INSERT INTO scheduled(chat_id,message_id,payload) SELECT ?1,?2,?3 WHERE NOT EXISTS(SELECT 1 FROM scheduled_deletions WHERE chat_id=?1 AND message_id=?2) ON CONFLICT(chat_id,message_id) DO UPDATE SET payload=excluded.payload", params![message.message.chat_id,message.message.id,encode(message)?])?;
    Ok(())
}
