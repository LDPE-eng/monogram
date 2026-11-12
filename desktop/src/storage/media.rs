use super::*;
use crate::domain::{
    AutoDownloadSettings, CacheLimit, ClearMediaScope, MediaCategory, MediaSend, NetworkClass,
    NetworkPolicy, PendingMediaSend,
};

pub(crate) struct MediaRecord {
    pub remote_key: String,
    pub category: MediaCategory,
    pub chat_id: Option<i64>,
    pub bytes: u64,
    pub complete: bool,
}

impl Storage {
    pub fn message(&self, chat_id: i64, message_id: i32) -> Result<Option<MessageDto>> {
        self.connection
            .query_row(
                "SELECT payload FROM messages WHERE chat_id=?1 AND message_id=?2 AND deleted=0",
                params![chat_id, message_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|payload| decode(&payload))
            .transpose()
    }

    pub fn cache_limit(&self) -> Result<CacheLimit> {
        let Some(value) = self.setting("media.cache_limit")? else {
            return Ok(CacheLimit::default());
        };
        let limit = if value == "unlimited" {
            CacheLimit::Unlimited
        } else {
            let bytes = value
                .strip_prefix("limited:")
                .unwrap_or(&value)
                .parse::<u64>()
                .map_err(|_| Error::InvalidQuery)?;
            CacheLimit::Limited(bytes)
        };
        limit.validate().map_err(|_| Error::InvalidQuery)?;
        Ok(limit)
    }

    pub fn set_cache_limit(&self, limit: CacheLimit) -> Result<()> {
        limit.validate().map_err(|_| Error::InvalidQuery)?;
        let value = match limit {
            CacheLimit::Limited(bytes) => format!("limited:{bytes}"),
            CacheLimit::Unlimited => "unlimited".into(),
        };
        self.set_setting("media.cache_limit", &value)
    }

    pub fn network_class(&self) -> Result<NetworkClass> {
        match self.setting("media.network")?.as_deref() {
            None | Some("wifi") => Ok(NetworkClass::Wifi),
            Some("mobile") => Ok(NetworkClass::Mobile),
            Some("roaming") => Ok(NetworkClass::Roaming),
            _ => Err(Error::InvalidQuery),
        }
    }

    pub fn set_network_class(&self, network: NetworkClass) -> Result<()> {
        let value = match network {
            NetworkClass::Wifi => "wifi",
            NetworkClass::Mobile => "mobile",
            NetworkClass::Roaming => "roaming",
        };
        self.set_setting("media.network", value)
    }

    pub fn auto_download(&self) -> Result<AutoDownloadSettings> {
        self.setting("media.auto_download")?
            .map_or(Ok(AutoDownloadSettings::default()), |value| decode(&value))
    }

    pub fn set_auto_download(&self, settings: AutoDownloadSettings) -> Result<()> {
        settings.validate().map_err(|_| Error::InvalidQuery)?;
        self.set_setting("media.auto_download", &encode(&settings)?)
    }

    pub fn autoplay(&self) -> Result<NetworkPolicy> {
        self.network_policy("media.autoplay")
    }

    pub fn set_autoplay(&self, policy: NetworkPolicy) -> Result<()> {
        self.set_setting("media.autoplay", &encode(&policy)?)
    }

    pub fn preload(&self) -> Result<NetworkPolicy> {
        self.network_policy("media.preload")
    }

    pub fn set_preload(&self, policy: NetworkPolicy) -> Result<()> {
        self.set_setting("media.preload", &encode(&policy)?)
    }

    pub fn media_settings(&self) -> Result<crate::domain::MediaSettings> {
        Ok(crate::domain::MediaSettings {
            cache_limit: self.cache_limit()?,
            network: self.network_class()?,
            auto_download: self.auto_download()?,
            autoplay: self.autoplay()?,
            preload: self.preload()?,
        })
    }

    fn network_policy(&self, key: &str) -> Result<NetworkPolicy> {
        self.setting(key)?
            .map_or(Ok(NetworkPolicy::default()), |value| decode(&value))
    }

    pub fn record_media(
        &self,
        remote_key: &str,
        category: MediaCategory,
        chat_id: Option<i64>,
        bytes: u64,
        complete: bool,
    ) -> Result<()> {
        if remote_key.is_empty() || remote_key.len() > 100 || bytes > i64::MAX as u64 {
            return Err(Error::InvalidQuery);
        }
        self.connection.execute(
            "INSERT INTO media_records(remote_key,category,chat_id,bytes,complete,last_used)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(remote_key) DO UPDATE SET category=excluded.category,
             chat_id=excluded.chat_id,bytes=excluded.bytes,complete=excluded.complete,
             last_used=excluded.last_used",
            params![
                remote_key,
                category_name(category),
                chat_id,
                bytes as i64,
                complete,
                unix_time()?
            ],
        )?;
        Ok(())
    }

    pub(crate) fn media_records(&self, scope: ClearMediaScope) -> Result<Vec<MediaRecord>> {
        let columns = "remote_key,category,chat_id,bytes,complete";
        let mut statement = match scope {
            ClearMediaScope::All => self.connection.prepare(&format!(
                "SELECT {columns} FROM media_records ORDER BY last_used,remote_key"
            ))?,
            ClearMediaScope::Category(_) => self.connection.prepare(&format!(
                "SELECT {columns} FROM media_records WHERE category=?1 ORDER BY last_used,remote_key"
            ))?,
            ClearMediaScope::Chat(_) => self.connection.prepare(&format!(
                "SELECT {columns} FROM media_records WHERE chat_id=?1 ORDER BY last_used,remote_key"
            ))?,
            ClearMediaScope::ChatCategory { .. } => self.connection.prepare(&format!(
                "SELECT {columns} FROM media_records WHERE chat_id=?1 AND category=?2
                 ORDER BY last_used,remote_key"
            ))?,
        };
        let records = match scope {
            ClearMediaScope::All => statement.query_map([], read_media_record)?,
            ClearMediaScope::Category(category) => {
                statement.query_map([category_name(category)], read_media_record)?
            }
            ClearMediaScope::Chat(chat_id) => statement.query_map([chat_id], read_media_record)?,
            ClearMediaScope::ChatCategory { chat_id, category } => {
                statement.query_map(params![chat_id, category_name(category)], read_media_record)?
            }
        };
        records.collect()
    }

    pub fn media_record_keys(&self, scope: ClearMediaScope) -> Result<Vec<String>> {
        Ok(self
            .media_records(scope)?
            .into_iter()
            .map(|record| record.remote_key)
            .collect())
    }

    pub fn remove_media_record(&self, remote_key: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM media_records WHERE remote_key=?1",
            [remote_key],
        )?;
        Ok(())
    }

    pub fn clear_media_records(&self) -> Result<()> {
        self.connection.execute("DELETE FROM media_records", [])?;
        Ok(())
    }

    pub fn enqueue_media(&mut self, send: &MediaSend) -> Result<(i64, Vec<i64>)> {
        send.validate().map_err(|_| Error::InvalidQuery)?;
        let payload = encode(send)?;
        let tx = self.connection.transaction()?;
        let album_id = fresh_album_id(&tx)?;
        tx.execute(
            "INSERT INTO media_outbox(album_id,payload) VALUES(?1,?2)",
            params![album_id, payload],
        )?;
        let mut random_ids = Vec::with_capacity(send.items.len());
        for (position, _) in send.items.iter().enumerate() {
            let random_id = fresh_random_id(&tx)?;
            tx.execute(
                "INSERT INTO media_outbox_items(album_id,position,random_id) VALUES(?1,?2,?3)",
                params![album_id, position as i64, random_id],
            )?;
            random_ids.push(random_id);
        }
        tx.commit()?;
        Ok((album_id, random_ids))
    }

    pub fn pending_media(&self) -> Result<Vec<PendingMediaSend>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT album_id FROM media_outbox_items WHERE acknowledged=0 ORDER BY album_id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|album_id| {
                self.pending_media_send(album_id)?
                    .ok_or(Error::InvalidQuery)
            })
            .collect()
    }

    pub fn pending_media_send(&self, album_id: i64) -> Result<Option<PendingMediaSend>> {
        let Some((payload, uncertain)) = self
            .connection
            .query_row(
                "SELECT payload,uncertain FROM media_outbox WHERE album_id=?1
                 AND EXISTS(SELECT 1 FROM media_outbox_items WHERE album_id=?1 AND acknowledged=0)",
                [album_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare(
            "SELECT random_id FROM media_outbox_items WHERE album_id=?1 ORDER BY position",
        )?;
        let random_ids = statement
            .query_map([album_id], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(PendingMediaSend {
            album_id,
            random_ids,
            send: decode(&payload)?,
            uncertain,
        }))
    }

    pub fn mark_media_attempted(&self, album_id: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE media_outbox SET uncertain=1 WHERE album_id=?1",
            [album_id],
        )?;
        Ok(())
    }

    fn setting(&self, key: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

fn read_media_record(row: &rusqlite::Row<'_>) -> Result<MediaRecord> {
    let category = match row.get::<_, String>(1)?.as_str() {
        "photo" => MediaCategory::Photo,
        "video" => MediaCategory::Video,
        "document" => MediaCategory::Document,
        "avatar" => MediaCategory::Avatar,
        "other" => MediaCategory::Other,
        _ => return Err(Error::InvalidQuery),
    };
    let bytes = row.get::<_, i64>(3)?;
    Ok(MediaRecord {
        remote_key: row.get(0)?,
        category,
        chat_id: row.get(2)?,
        bytes: u64::try_from(bytes).map_err(|_| Error::InvalidQuery)?,
        complete: row.get(4)?,
    })
}

fn category_name(category: MediaCategory) -> &'static str {
    match category {
        MediaCategory::Photo => "photo",
        MediaCategory::Video => "video",
        MediaCategory::Document => "document",
        MediaCategory::Avatar => "avatar",
        MediaCategory::Other => "other",
    }
}

fn unix_time() -> Result<i64> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::InvalidQuery)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| Error::InvalidQuery)
}

fn random_id() -> Result<i64> {
    use rand::{TryRng, rngs::SysRng};
    loop {
        let id = SysRng.try_next_u64().map_err(|_| Error::InvalidQuery)? as i64;
        if id != 0 {
            return Ok(id);
        }
    }
}

fn fresh_album_id(tx: &Transaction<'_>) -> Result<i64> {
    loop {
        let id = random_id()?;
        let exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM media_outbox WHERE album_id=?1)",
            [id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Ok(id);
        }
    }
}

pub(super) fn fresh_random_id(tx: &Transaction<'_>) -> Result<i64> {
    loop {
        let id = random_id()?;
        let exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM outgoing WHERE random_id=?1)
             OR EXISTS(SELECT 1 FROM media_outbox_items WHERE random_id=?1)
             OR EXISTS(SELECT 1 FROM special_outbox WHERE random_id=?1)",
            [id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Ok(id);
        }
    }
}
