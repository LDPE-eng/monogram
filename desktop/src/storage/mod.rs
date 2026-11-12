use std::{path::Path, time::Duration};

use mono_proto_client::{
    ChatDto, MessageDto, OutgoingMessageResult, UpdateBatchDto, UpdateEventDto,
};
use rusqlite::{Connection, Error, OptionalExtension, Result, Transaction, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::domain::{PendingSend, TextSend};

mod media;
mod preferences;
mod projections;
mod special;
pub(crate) use media::MediaRecord;

pub struct Storage {
    connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialsMetadata {
    pub api_id: i32,
    pub user_id: Option<i64>,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let transaction = connection.transaction()?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !(0..=7).contains(&version) {
            return Err(Error::InvalidQuery);
        }
        if version == 0 {
            transaction.execute_batch(
                "CREATE TABLE app_settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 PRAGMA user_version = 1;",
            )?;
        }
        if version < 2 {
            transaction.execute_batch(
                "CREATE TABLE accounts (
                    id INTEGER PRIMARY KEY CHECK(id = 1), api_id INTEGER NOT NULL,
                    user_id INTEGER
                 );
                 CREATE TABLE chats (chat_id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
                 CREATE TABLE messages (
                    chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL, payload TEXT,
                    version_date INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY(chat_id, message_id)
                 );
                 CREATE TABLE global_deletions (message_id INTEGER PRIMARY KEY);
                 CREATE TABLE message_reactions (
                    chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL, payload TEXT NOT NULL,
                    PRIMARY KEY(chat_id, message_id)
                 );
                 CREATE TABLE chat_reads (
                    chat_id INTEGER PRIMARY KEY, inbox INTEGER NOT NULL DEFAULT 0,
                    outbox INTEGER NOT NULL DEFAULT 0, unread INTEGER
                 );
                 CREATE TABLE outgoing (
                    random_id INTEGER PRIMARY KEY CHECK(random_id != 0), payload TEXT NOT NULL,
                    uncertain INTEGER NOT NULL DEFAULT 0, acknowledged INTEGER NOT NULL DEFAULT 0,
                    result TEXT
                 );
                 CREATE TABLE update_receipts (
                    generation INTEGER NOT NULL, batch_id INTEGER NOT NULL,
                    PRIMARY KEY(generation, batch_id)
                 );
                 CREATE TABLE update_events (
                    generation INTEGER NOT NULL, batch_id INTEGER NOT NULL,
                    event_index INTEGER NOT NULL, payload TEXT NOT NULL,
                    PRIMARY KEY(generation, batch_id, event_index),
                    FOREIGN KEY(generation, batch_id) REFERENCES update_receipts(generation, batch_id)
                 );
                 CREATE TABLE cursors (
                    generation INTEGER NOT NULL, scope INTEGER NOT NULL,
                    pts INTEGER NOT NULL, qts INTEGER NOT NULL, date INTEGER NOT NULL, seq INTEGER NOT NULL,
                    PRIMARY KEY(generation, scope)
                 );
                 CREATE TABLE scheduled (
                    chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL, payload TEXT NOT NULL,
                    PRIMARY KEY(chat_id, message_id)
                 );
                 CREATE TABLE drafts (
                    chat_id INTEGER NOT NULL, thread_id INTEGER NOT NULL, payload TEXT NOT NULL,
                    PRIMARY KEY(chat_id, thread_id)
                 );
                 PRAGMA user_version = 2;",
            )?;
        }
        if version < 3 {
            transaction.execute_batch(
                "CREATE TABLE profiles(peer_id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
                 CREATE TABLE folders(folder_id INTEGER PRIMARY KEY, position INTEGER NOT NULL, payload TEXT NOT NULL);
                 CREATE TABLE dialog_marks(chat_id INTEGER PRIMARY KEY, unread INTEGER NOT NULL);
                 CREATE TABLE scheduled_deletions(chat_id INTEGER NOT NULL, message_id INTEGER NOT NULL, PRIMARY KEY(chat_id,message_id));
                 ALTER TABLE drafts ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
                 ALTER TABLE drafts ADD COLUMN pending INTEGER NOT NULL DEFAULT 0;
                 PRAGMA user_version=3;",
            )?;
        }
        if version < 4 {
            transaction.execute_batch(
                "CREATE TABLE media_records (
                    remote_key TEXT PRIMARY KEY, category TEXT NOT NULL, chat_id INTEGER,
                    bytes INTEGER NOT NULL DEFAULT 0, complete INTEGER NOT NULL DEFAULT 0,
                    last_used INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX media_records_scope ON media_records(category,chat_id,last_used);
                 CREATE TABLE media_outbox (
                    album_id INTEGER PRIMARY KEY CHECK(album_id != 0), payload TEXT NOT NULL,
                    uncertain INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE media_outbox_items (
                    album_id INTEGER NOT NULL, position INTEGER NOT NULL,
                    random_id INTEGER NOT NULL UNIQUE CHECK(random_id != 0),
                    acknowledged INTEGER NOT NULL DEFAULT 0, result TEXT,
                    PRIMARY KEY(album_id,position),
                    FOREIGN KEY(album_id) REFERENCES media_outbox(album_id) ON DELETE CASCADE
                 );
                 PRAGMA user_version=4;",
            )?;
        }
        if version < 5 {
            transaction.execute_batch(
                "UPDATE app_settings SET value='limited:'||value
                 WHERE key='media.cache_limit' AND value NOT GLOB '*[^0-9]*';
                 PRAGMA user_version=5;",
            )?;
        }
        if version < 6 {
            transaction.execute_batch(
                "CREATE TABLE special_outbox (
                    random_id INTEGER PRIMARY KEY CHECK(random_id != 0), payload TEXT NOT NULL,
                    uncertain INTEGER NOT NULL DEFAULT 0,
                    acknowledged INTEGER NOT NULL DEFAULT 0, result TEXT
                 );
                 PRAGMA user_version=6;",
            )?;
        }
        if version < 7 {
            transaction.execute_batch(
                "CREATE TABLE app_preferences (
                    namespace TEXT PRIMARY KEY NOT NULL, version INTEGER NOT NULL,
                    payload TEXT NOT NULL
                 );
                 PRAGMA user_version=7;",
            )?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }

    pub fn credentials_metadata(&self) -> Result<Option<CredentialsMetadata>> {
        self.connection
            .query_row(
                "SELECT api_id, user_id FROM accounts WHERE id=1",
                [],
                |row| {
                    Ok(CredentialsMetadata {
                        api_id: row.get(0)?,
                        user_id: row.get(1)?,
                    })
                },
            )
            .optional()
    }

    pub fn configure(&self, api_id: i32) -> Result<()> {
        if api_id <= 0
            || self
                .credentials_metadata()?
                .is_some_and(|meta| meta.api_id != api_id)
        {
            return Err(Error::InvalidQuery);
        }
        self.connection.execute(
            "INSERT INTO accounts(id, api_id) VALUES(1, ?1) ON CONFLICT(id) DO NOTHING",
            [api_id],
        )?;
        Ok(())
    }

    pub fn set_user(&mut self, user_id: i64) -> Result<()> {
        if self
            .credentials_metadata()?
            .and_then(|meta| meta.user_id)
            .is_some_and(|id| id != user_id)
        {
            self.clear_account()?;
        }
        self.connection
            .execute("UPDATE accounts SET user_id=?1 WHERE id=1", [user_id])?;
        Ok(())
    }

    pub fn clear_account(&mut self) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute_batch(
            "DELETE FROM update_events; DELETE FROM update_receipts; DELETE FROM cursors;
             DELETE FROM chats; DELETE FROM messages; DELETE FROM outgoing;
             DELETE FROM global_deletions; DELETE FROM message_reactions; DELETE FROM chat_reads;
             DELETE FROM scheduled; DELETE FROM drafts; DELETE FROM profiles; DELETE FROM folders;
             DELETE FROM dialog_marks; DELETE FROM scheduled_deletions;
             DELETE FROM media_outbox_items; DELETE FROM media_outbox; DELETE FROM media_records;
             DELETE FROM special_outbox; UPDATE accounts SET user_id=NULL;",
        )?;
        tx.commit()
    }

    pub fn cache_chats(&mut self, chats: &[ChatDto]) -> Result<()> {
        let tx = self.connection.transaction()?;
        for chat in chats {
            save_chat(&tx, chat)?;
        }
        tx.commit()
    }

    pub fn chats(&self) -> Result<Vec<ChatDto>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload FROM chats ORDER BY chat_id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut chats = Vec::new();
        for row in rows {
            let mut chat: ChatDto = decode(&row?)?;
            if let Some((inbox, outbox, unread)) = self
                .connection
                .query_row(
                    "SELECT inbox, outbox, unread FROM chat_reads WHERE chat_id=?1",
                    [chat.id],
                    |row| {
                        Ok((
                            row.get::<_, i32>(0)?,
                            row.get::<_, i32>(1)?,
                            row.get::<_, Option<i32>>(2)?,
                        ))
                    },
                )
                .optional()?
            {
                chat.read_inbox_max_id = chat.read_inbox_max_id.max(inbox);
                chat.read_outbox_max_id = chat.read_outbox_max_id.max(outbox);
                if let Some(unread) = unread {
                    chat.unread_count = unread;
                }
            }
            if let Some(unread) = self
                .connection
                .query_row(
                    "SELECT unread FROM dialog_marks WHERE chat_id=?1",
                    [chat.id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()?
            {
                chat.unread_mark = unread;
            }
            chats.push(chat);
        }
        Ok(chats)
    }

    pub fn cache_messages(&mut self, messages: &[MessageDto]) -> Result<()> {
        let tx = self.connection.transaction()?;
        for message in messages {
            save_message(&tx, message)?;
        }
        tx.commit()
    }

    pub fn messages(&self, chat_id: i64, limit: i32) -> Result<Vec<MessageDto>> {
        let mut statement = self.connection.prepare(
            "SELECT m.payload, r.payload FROM messages m
             LEFT JOIN message_reactions r ON r.chat_id=m.chat_id AND r.message_id=m.message_id
             WHERE m.chat_id=?1 AND m.deleted=0 ORDER BY m.message_id DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![chat_id, limit.clamp(1, 100)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (payload, reactions) = row?;
            let mut message: MessageDto = decode(&payload)?;
            if let Some(reactions) = reactions {
                message.reactions_json = Some(reactions);
            }
            messages.push(message);
        }
        Ok(messages)
    }

    pub fn project_messages(&self, snapshot: &[MessageDto]) -> Result<Vec<MessageDto>> {
        let mut messages = Vec::new();
        for requested in snapshot {
            let row = self
                .connection
                .query_row(
                    "SELECT m.payload, r.payload FROM messages m
                 LEFT JOIN message_reactions r ON r.chat_id=m.chat_id AND r.message_id=m.message_id
                 WHERE m.chat_id=?1 AND m.message_id=?2 AND m.deleted=0",
                    params![requested.chat_id, requested.id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            if let Some((payload, reactions)) = row {
                let mut message: MessageDto = decode(&payload)?;
                if let Some(reactions) = reactions {
                    message.reactions_json = Some(reactions);
                }
                messages.push(message);
            }
        }
        Ok(messages)
    }

    pub fn enqueue(&self, message: &TextSend) -> Result<i64> {
        let payload = encode(message)?;
        loop {
            let random_id = {
                use rand::{TryRng, rngs::SysRng};
                SysRng.try_next_u64().map_err(|_| Error::InvalidQuery)? as i64
            };
            if random_id == 0 {
                continue;
            }
            let inserted = self.connection.execute(
                "INSERT OR IGNORE INTO outgoing(random_id,payload) SELECT ?1,?2
                 WHERE NOT EXISTS(SELECT 1 FROM media_outbox_items WHERE random_id=?1)
                 AND NOT EXISTS(SELECT 1 FROM special_outbox WHERE random_id=?1)",
                params![random_id, payload],
            )?;
            if inserted == 1 {
                return Ok(random_id);
            }
        }
    }

    pub fn pending(&self) -> Result<Vec<PendingSend>> {
        let mut statement = self.connection.prepare(
            "SELECT random_id,payload,uncertain FROM outgoing WHERE acknowledged=0 ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (random_id, payload, uncertain) = row?;
            Ok(PendingSend {
                random_id,
                message: decode(&payload)?,
                uncertain,
            })
        })
        .collect()
    }

    pub fn pending_send(&self, random_id: i64) -> Result<Option<PendingSend>> {
        let value = self
            .connection
            .query_row(
                "SELECT payload,uncertain FROM outgoing WHERE random_id=?1 AND acknowledged=0",
                [random_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        value
            .map(|(payload, uncertain)| {
                Ok(PendingSend {
                    random_id,
                    message: decode(&payload)?,
                    uncertain,
                })
            })
            .transpose()
    }

    pub fn mark_attempted(&self, random_id: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE outgoing SET uncertain=1 WHERE random_id=?1",
            [random_id],
        )?;
        Ok(())
    }

    pub fn delete_message(&mut self, chat_id: i64, message_id: i32) -> Result<()> {
        let tx = self.connection.transaction()?;
        tombstone(&tx, chat_id, message_id)?;
        tx.commit()
    }

    pub fn apply_batch(&mut self, batch: &UpdateBatchDto) -> Result<bool> {
        if batch.account_generation <= 0 || batch.batch_id <= 0 {
            return Err(Error::InvalidQuery);
        }
        let tx = self.connection.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO update_receipts(generation,batch_id) VALUES(?1,?2)",
            params![batch.account_generation, batch.batch_id],
        )?;
        if inserted == 0 {
            return Ok(false);
        }
        for (index, event) in batch.events.iter().enumerate() {
            tx.execute(
                "INSERT INTO update_events(generation,batch_id,event_index,payload) VALUES(?1,?2,?3,?4)",
                params![batch.account_generation, batch.batch_id, index as i64, encode(event)?],
            )?;
            apply_event(&tx, event)?;
        }
        if let Some(cursor) = &batch.cursor_after {
            tx.execute(
                "INSERT INTO cursors(generation,scope,pts,qts,date,seq) VALUES(?1,0,?2,?3,?4,?5)
                 ON CONFLICT(generation,scope) DO UPDATE SET
                 pts=MAX(pts,excluded.pts), qts=MAX(qts,excluded.qts),
                 date=MAX(date,excluded.date), seq=MAX(seq,excluded.seq)",
                params![
                    batch.account_generation,
                    cursor.pts,
                    cursor.qts,
                    cursor.date,
                    cursor.seq
                ],
            )?;
        }
        for channel in &batch.channel_pts_after {
            tx.execute(
                "INSERT INTO cursors(generation,scope,pts,qts,date,seq) VALUES(?1,?2,?3,0,0,0)
                 ON CONFLICT(generation,scope) DO UPDATE SET pts=MAX(pts,excluded.pts)",
                params![batch.account_generation, channel.chat_id, channel.pts],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }
}

fn encode(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::InvalidQuery)
}

fn decode<T: DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|_| Error::InvalidQuery)
}

fn save_chat(tx: &Transaction<'_>, chat: &ChatDto) -> Result<()> {
    tx.execute(
        "INSERT INTO chats(chat_id,payload) VALUES(?1,?2)
         ON CONFLICT(chat_id) DO UPDATE SET payload=excluded.payload",
        params![chat.id, encode(chat)?],
    )?;
    Ok(())
}

fn save_message(tx: &Transaction<'_>, message: &MessageDto) -> Result<()> {
    let deleted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM global_deletions WHERE message_id=?1) AND ?2 > -1000000000000",
        params![message.id, message.chat_id],
        |row| row.get(0),
    )?;
    if deleted {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO messages(chat_id,message_id,payload,version_date) VALUES(?1,?2,?3,?4)
         ON CONFLICT(chat_id,message_id) DO UPDATE SET payload=excluded.payload,
         version_date=excluded.version_date WHERE messages.deleted=0
         AND excluded.version_date >= messages.version_date",
        params![
            message.chat_id,
            message.id,
            encode(message)?,
            message.edit_date.unwrap_or(message.date)
        ],
    )?;
    Ok(())
}

fn tombstone(tx: &Transaction<'_>, chat_id: i64, message_id: i32) -> Result<()> {
    tx.execute(
        "INSERT INTO messages(chat_id,message_id,deleted) VALUES(?1,?2,1)
         ON CONFLICT(chat_id,message_id) DO UPDATE SET deleted=1,payload=NULL",
        params![chat_id, message_id],
    )?;
    Ok(())
}

fn save_outgoing_result(
    tx: &Transaction<'_>,
    random_id: i64,
    result: &OutgoingMessageResult,
) -> Result<()> {
    match result {
        OutgoingMessageResult::Sent { message } => save_message(tx, message)?,
        OutgoingMessageResult::Scheduled { message } => projections::save_scheduled(tx, message)?,
    }
    let payload = encode(result)?;
    tx.execute(
        "UPDATE outgoing SET acknowledged=1,result=?2 WHERE random_id=?1",
        params![random_id, payload],
    )?;
    tx.execute(
        "UPDATE media_outbox_items SET acknowledged=1,result=?2 WHERE random_id=?1",
        params![random_id, payload],
    )?;
    tx.execute(
        "UPDATE special_outbox SET acknowledged=1,result=?2 WHERE random_id=?1",
        params![random_id, payload],
    )?;
    Ok(())
}

fn apply_event(tx: &Transaction<'_>, event: &UpdateEventDto) -> Result<()> {
    match event {
        UpdateEventDto::NewMessage { message } | UpdateEventDto::MessageEdited { message } => {
            save_message(tx, message)?
        }
        UpdateEventDto::OutgoingMessageAcknowledged { random_id, result } => {
            save_outgoing_result(tx, *random_id, result)?
        }
        UpdateEventDto::MessagesDeleted {
            chat_id,
            message_ids,
        } => {
            for id in message_ids {
                if let Some(chat_id) = chat_id {
                    tombstone(tx, *chat_id, *id)?;
                } else {
                    tx.execute(
                        "INSERT OR IGNORE INTO global_deletions(message_id) VALUES(?1)",
                        [id],
                    )?;
                    tx.execute("UPDATE messages SET deleted=1,payload=NULL WHERE message_id=?1 AND chat_id > -1000000000000", [id])?;
                }
            }
        }
        UpdateEventDto::MessageReactions {
            chat_id,
            message_id,
            reactions_json,
        } => {
            tx.execute(
                "INSERT INTO message_reactions(chat_id,message_id,payload) VALUES(?1,?2,?3)
                 ON CONFLICT(chat_id,message_id) DO UPDATE SET payload=excluded.payload",
                params![chat_id, message_id, reactions_json],
            )?;
        }
        UpdateEventDto::ReadInbox {
            chat_id,
            max_id,
            still_unread,
        } => {
            tx.execute(
                "INSERT INTO chat_reads(chat_id,inbox,unread) VALUES(?1,?2,?3)
                 ON CONFLICT(chat_id) DO UPDATE SET inbox=MAX(inbox,excluded.inbox),unread=excluded.unread",
                params![chat_id, max_id, still_unread],
            )?;
        }
        UpdateEventDto::ReadOutbox { chat_id, max_id } => {
            tx.execute(
                "INSERT INTO chat_reads(chat_id,outbox) VALUES(?1,?2)
                 ON CONFLICT(chat_id) DO UPDATE SET outbox=MAX(outbox,excluded.outbox)",
                params![chat_id, max_id],
            )?;
        }
        UpdateEventDto::DialogsPatched { chats } => {
            for chat in chats {
                save_chat(tx, chat)?;
            }
        }
        UpdateEventDto::DraftChanged { draft } => projections::save_draft(tx, draft)?,
        UpdateEventDto::ScheduledMessageChanged { message } => {
            projections::save_scheduled(tx, message)?
        }
        UpdateEventDto::Ignored { kind } if kind.starts_with("dialog_unread_mark:") => {
            if let Some((chat_id, unread)) = kind
                .strip_prefix("dialog_unread_mark:")
                .and_then(|value| value.split_once(':'))
                && let (Ok(chat_id), Ok(unread)) = (chat_id.parse::<i64>(), unread.parse::<u8>())
                && chat_id != 0
                && unread <= 1
            {
                tx.execute("INSERT INTO dialog_marks(chat_id,unread) VALUES(?1,?2) ON CONFLICT(chat_id) DO UPDATE SET unread=excluded.unread", params![chat_id,unread])?;
            }
        }
        UpdateEventDto::FoldersChanged => {
            tx.execute("DELETE FROM folders", [])?;
        }
        UpdateEventDto::ScheduledMessagesDeleted {
            chat_id,
            message_ids,
            ..
        } => {
            for id in message_ids {
                tx.execute(
                    "INSERT OR IGNORE INTO scheduled_deletions(chat_id,message_id) VALUES(?1,?2)",
                    params![chat_id, id],
                )?;
                tx.execute(
                    "DELETE FROM scheduled WHERE chat_id=?1 AND message_id=?2",
                    params![chat_id, id],
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/unit/storage.rs"]
mod tests;
