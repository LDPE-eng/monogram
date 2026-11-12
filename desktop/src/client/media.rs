use super::*;
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
};

use crate::{
    domain::{
        AutoDownloadRule, CacheLimit, ClearMediaScope, MediaCategory, MediaCategoryUsage,
        MediaChatUsage, MediaDecision, MediaDownloadKind, MediaDownloadMode, MediaSend,
        MediaUpload, MediaUsage, PendingMediaSend,
    },
    media::cache::{Acquire, CacheKey, CacheLease, DownloadLease},
    storage::MediaRecord,
};

pub(super) struct DownloadRequest {
    pub request_id: u64,
    pub handle: u64,
    pub chat_id: i64,
    pub message_id: i32,
    pub kind: MediaDownloadKind,
    pub mode: MediaDownloadMode,
    pub category: MediaCategory,
    pub expected_bytes: Option<u64>,
    pub remote_key: String,
}

impl Runtime {
    pub(super) fn install_media_progress(&self, handle: u64) -> Result<(), ClientError> {
        let client = proto::registry::get_client(handle).map_err(ClientError::from)?;
        let events = self.events.clone();
        let stopping = self.stopping.clone();
        client
            .policy
            .own_progress(Some(Arc::new(move |path, downloaded, total| {
                let Some(transfer_id) = transfer_id_from_path(path) else {
                    return;
                };
                if events
                    .send(WorkerEvent::MediaProgress {
                        transfer_id,
                        downloaded: downloaded.max(0) as u64,
                        total: (total > 0).then_some(total as u64),
                    })
                    .is_err()
                {
                    stopping.store(true, Ordering::Release);
                }
            })));
        Ok(())
    }

    pub(super) fn execute_media(
        &mut self,
        request_id: u64,
        command: WorkerCommand,
    ) -> Result<WorkerResponse, ClientError> {
        match command {
            WorkerCommand::DownloadMessageMedia {
                chat_id,
                message_id,
                kind,
                mode,
            } => self.download_message(request_id, chat_id, message_id, kind, mode),
            WorkerCommand::DownloadPeerAvatar {
                peer_id,
                kind,
                mode,
            } => self.download_avatar(request_id, peer_id, kind, mode),
            WorkerCommand::DownloadCustomEmoji { document_id, mode } => {
                self.download_custom_emoji(request_id, document_id, mode)
            }
            WorkerCommand::DownloadMediaRange {
                chat_id,
                message_id,
                offset,
                mode,
            } => self.download_range(request_id, chat_id, message_id, offset, mode),
            WorkerCommand::CancelMedia { .. } => Ok(WorkerResponse::Done),
            WorkerCommand::MediaUsage => Ok(WorkerResponse::MediaUsage(self.media_usage()?)),
            WorkerCommand::SetCacheLimit { limit } => {
                limit.validate()?;
                self.trim_media(limit)?;
                self.storage
                    .set_cache_limit(limit)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaUsage(self.media_usage()?))
            }
            WorkerCommand::ClearMedia(scope) => {
                if matches!(
                    scope,
                    ClearMediaScope::Chat(0) | ClearMediaScope::ChatCategory { chat_id: 0, .. }
                ) {
                    return Err(ClientError::InvalidRequest);
                }
                for remote_key in self
                    .storage
                    .media_record_keys(scope)
                    .map_err(|_| ClientError::Storage)?
                {
                    let _ = self.evict_media(&remote_key)?;
                }
                Ok(WorkerResponse::MediaUsage(self.media_usage()?))
            }
            WorkerCommand::GetMediaSettings => Ok(WorkerResponse::MediaSettings(
                self.storage
                    .media_settings()
                    .map_err(|_| ClientError::Storage)?,
            )),
            WorkerCommand::SetNetworkClass(network) => {
                self.storage
                    .set_network_class(network)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaSettings(
                    self.storage
                        .media_settings()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::SetAutoDownload(settings) => {
                settings.validate()?;
                self.storage
                    .set_auto_download(settings)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaSettings(
                    self.storage
                        .media_settings()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::SetAutoplay(policy) => {
                self.storage
                    .set_autoplay(policy)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaSettings(
                    self.storage
                        .media_settings()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::SetPreload(policy) => {
                self.storage
                    .set_preload(policy)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaSettings(
                    self.storage
                        .media_settings()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::MediaDecision { category } => Ok(WorkerResponse::MediaDecision(
                self.media_decision(category)?,
            )),
            WorkerCommand::PendingMedia => {
                self.authorized()?;
                self.replay()?;
                Ok(WorkerResponse::PendingMedia(
                    self.storage
                        .pending_media()
                        .map_err(|_| ClientError::Storage)?,
                ))
            }
            WorkerCommand::SendMedia(send) => {
                self.authorized()?;
                validate_sources(&send)?;
                let (album_id, random_ids) = self
                    .storage
                    .enqueue_media(&send)
                    .map_err(|_| ClientError::Storage)?;
                self.emit(WorkerEvent::MediaQueued {
                    request_id,
                    album_id,
                    random_ids,
                });
                self.send_pending_media(album_id)
            }
            WorkerCommand::RetryMedia { album_id } => self.send_pending_media(album_id),
            WorkerCommand::ReplaceMedia {
                chat_id,
                message_id,
                expected_key,
                item,
            } => {
                valid_message(chat_id, message_id)?;
                item.replacement_validate()?;
                validate_source(&item)?;
                if expected_key.is_empty() {
                    return Err(ClientError::InvalidRequest);
                }
                let message = proto::dispatch::replace_uploaded_media(
                    self.authorized()?,
                    chat_id,
                    message_id,
                    expected_key,
                    upload_item(&item, 0),
                )
                .map_err(ClientError::from)?;
                self.replay()?;
                self.storage
                    .cache_messages(std::slice::from_ref(&message))
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::Message(Box::new(message)))
            }
            WorkerCommand::SendSavedGif {
                chat_id,
                document_id,
                reply_to_msg_id,
                top_msg_id,
            }
            | WorkerCommand::SendSticker {
                chat_id,
                document_id,
                reply_to_msg_id,
                top_msg_id,
            }
            | WorkerCommand::SendCustomEmoji {
                chat_id,
                document_id,
                reply_to_msg_id,
                top_msg_id,
            } => self.send_indexed_document(chat_id, document_id, reply_to_msg_id, top_msg_id),
            command => self.execute_features(command),
        }
    }

    pub(super) fn media_usage(&self) -> Result<MediaUsage, ClientError> {
        let records = self
            .storage
            .media_records(ClearMediaScope::All)
            .map_err(|_| ClientError::Storage)?;
        let mut categories = BTreeMap::<MediaCategory, (u64, u64)>::new();
        let mut chats = BTreeMap::<i64, (u64, u64)>::new();
        for record in records {
            let Some(record) = self.reconcile_media_record(record)? else {
                continue;
            };
            let category = categories.entry(record.category).or_default();
            let target = if record.complete {
                &mut category.0
            } else {
                &mut category.1
            };
            *target = target.saturating_add(record.bytes);
            if let Some(chat_id) = record.chat_id {
                let chat = chats.entry(chat_id).or_default();
                let target = if record.complete {
                    &mut chat.0
                } else {
                    &mut chat.1
                };
                *target = target.saturating_add(record.bytes);
            }
        }
        let usage = self.media_cache.usage().map_err(|_| ClientError::Storage)?;
        Ok(MediaUsage {
            complete_bytes: usage.complete_bytes,
            partial_bytes: usage.partial_bytes,
            pinned_entries: usage.pinned_entries,
            cache_limit: self
                .storage
                .cache_limit()
                .map_err(|_| ClientError::Storage)?,
            by_category: categories
                .into_iter()
                .map(
                    |(category, (complete_bytes, partial_bytes))| MediaCategoryUsage {
                        category,
                        complete_bytes,
                        partial_bytes,
                    },
                )
                .collect(),
            by_chat: chats
                .into_iter()
                .map(
                    |(chat_id, (complete_bytes, partial_bytes))| MediaChatUsage {
                        chat_id,
                        complete_bytes,
                        partial_bytes,
                    },
                )
                .collect(),
        })
    }

    fn reconcile_media_record(
        &self,
        record: MediaRecord,
    ) -> Result<Option<MediaRecord>, ClientError> {
        let key = CacheKey::new(self.account_id()?, &record.remote_key)
            .map_err(|_| ClientError::InvalidRequest)?;
        match self.media_cache.acquire(key, None) {
            Ok(Acquire::Cached(lease)) => {
                let bytes = lease
                    .path()
                    .metadata()
                    .map_err(|_| ClientError::Storage)?
                    .len();
                drop(lease);
                self.storage
                    .record_media(
                        &record.remote_key,
                        record.category,
                        record.chat_id,
                        bytes,
                        true,
                    )
                    .map_err(|_| ClientError::Storage)?;
                Ok(Some(MediaRecord {
                    bytes,
                    complete: true,
                    ..record
                }))
            }
            Ok(Acquire::Download(lease)) => {
                let bytes = lease.offset().map_err(|_| ClientError::Storage)?;
                lease.cancel(bytes > 0).map_err(|_| ClientError::Storage)?;
                if bytes == 0 {
                    self.storage
                        .remove_media_record(&record.remote_key)
                        .map_err(|_| ClientError::Storage)?;
                    return Ok(None);
                }
                self.storage
                    .record_media(
                        &record.remote_key,
                        record.category,
                        record.chat_id,
                        bytes,
                        false,
                    )
                    .map_err(|_| ClientError::Storage)?;
                Ok(Some(MediaRecord {
                    bytes,
                    complete: false,
                    ..record
                }))
            }
            Ok(Acquire::Busy) => Ok(Some(record)),
            Err(_) => Err(ClientError::Storage),
        }
    }

    pub(super) fn media_decision(
        &self,
        category: MediaCategory,
    ) -> Result<MediaDecision, ClientError> {
        let settings = self
            .storage
            .media_settings()
            .map_err(|_| ClientError::Storage)?;
        let auto_download = settings
            .auto_download
            .policy(settings.network)
            .rule(category)
            .is_some_and(|rule| rule.enabled);
        Ok(MediaDecision {
            autoplay: category == MediaCategory::Video
                && settings.autoplay.allows(settings.network),
            preload: auto_download && settings.preload.allows(settings.network),
            auto_download,
        })
    }

    fn transfer_rule(
        &self,
        mode: MediaDownloadMode,
        category: MediaCategory,
    ) -> Result<Option<AutoDownloadRule>, ClientError> {
        if mode == MediaDownloadMode::Manual {
            return Ok(None);
        }
        let decision = self.media_decision(category)?;
        let allowed = match mode {
            MediaDownloadMode::Manual => true,
            MediaDownloadMode::Automatic => decision.auto_download,
            MediaDownloadMode::Autoplay => decision.autoplay,
            MediaDownloadMode::Preload => decision.preload,
        };
        if !allowed {
            return Err(ClientError::Forbidden);
        }
        if matches!(
            mode,
            MediaDownloadMode::Automatic | MediaDownloadMode::Preload
        ) {
            let settings = self
                .storage
                .media_settings()
                .map_err(|_| ClientError::Storage)?;
            return Ok(settings
                .auto_download
                .policy(settings.network)
                .rule(category));
        }
        Ok(None)
    }

    fn trim_media(&self, limit: CacheLimit) -> Result<(), ClientError> {
        let Some(limit) = limit.bytes() else {
            return Ok(());
        };
        loop {
            let usage = self.media_cache.usage().map_err(|_| ClientError::Storage)?;
            if usage.complete_bytes.saturating_add(usage.partial_bytes) <= limit {
                return Ok(());
            }
            let keys = self
                .storage
                .media_record_keys(ClearMediaScope::All)
                .map_err(|_| ClientError::Storage)?;
            let mut evicted = false;
            for key in keys {
                if self.evict_media(&key)? {
                    evicted = true;
                    break;
                }
            }
            if !evicted {
                return Err(ClientError::InvalidRequest);
            }
        }
    }

    fn evict_media(&self, remote_key: &str) -> Result<bool, ClientError> {
        let account = self.account_id()?;
        let key = CacheKey::new(account, remote_key).map_err(|_| ClientError::InvalidRequest)?;
        match self.media_cache.acquire(key, Some(0)) {
            Ok(Acquire::Download(lease)) => {
                lease.cancel(false).map_err(|_| ClientError::Storage)?;
                self.storage
                    .remove_media_record(remote_key)
                    .map_err(|_| ClientError::Storage)?;
                Ok(true)
            }
            Ok(Acquire::Busy) => Ok(false),
            Ok(Acquire::Cached(_)) => Ok(false),
            Err(_) => Err(ClientError::Storage),
        }
    }

    fn account_id(&self) -> Result<i64, ClientError> {
        match self.state.auth {
            AuthState::Authorized { user_id } => Ok(user_id),
            _ => Err(ClientError::Unauthorized),
        }
    }

    fn message_for_download(
        &mut self,
        handle: u64,
        chat_id: i64,
        message_id: i32,
    ) -> Result<MessageDto, ClientError> {
        if let Some(message) = self
            .storage
            .message(chat_id, message_id)
            .map_err(|_| ClientError::Storage)?
        {
            return Ok(message);
        }
        let messages = proto::dispatch::get_history(handle, chat_id, 1, message_id, 0, -1)
            .map_err(ClientError::from)?;
        self.storage
            .cache_messages(&messages)
            .map_err(|_| ClientError::Storage)?;
        self.storage
            .message(chat_id, message_id)
            .map_err(|_| ClientError::Storage)?
            .ok_or(ClientError::InvalidRequest)
    }

    fn download_message(
        &mut self,
        request_id: u64,
        chat_id: i64,
        message_id: i32,
        kind: MediaDownloadKind,
        mode: MediaDownloadMode,
    ) -> Result<WorkerResponse, ClientError> {
        valid_message(chat_id, message_id)?;
        if request_id == 0 {
            return Err(ClientError::InvalidRequest);
        }
        let _active = ActiveMedia::enter(&self.active_media, request_id);
        let handle = self.authorized()?;
        let message = self.message_for_download(handle, chat_id, message_id)?;
        let base_key = message
            .media_cache_key
            .as_deref()
            .ok_or(ClientError::UnsupportedContent)?;
        let remote_key = match kind {
            MediaDownloadKind::Full => base_key.to_owned(),
            MediaDownloadKind::Thumb => message
                .thumb_cache_key
                .as_deref()
                .unwrap_or(base_key)
                .to_owned(),
            MediaDownloadKind::Display => derived_cache_key("display", base_key),
        };
        let expected_bytes = (kind == MediaDownloadKind::Full)
            .then_some(message.file_size)
            .flatten()
            .and_then(|size| u64::try_from(size).ok())
            .filter(|size| *size > 0);
        let category = MediaCategory::from_media_kind(message.media_kind.as_deref());
        self.download(DownloadRequest {
            request_id,
            handle,
            chat_id,
            message_id,
            kind,
            mode,
            category,
            expected_bytes,
            remote_key,
        })
    }

    fn download_avatar(
        &mut self,
        request_id: u64,
        peer_id: i64,
        kind: MediaDownloadKind,
        mode: MediaDownloadMode,
    ) -> Result<WorkerResponse, ClientError> {
        features::valid_chat(peer_id)?;
        if request_id == 0 {
            return Err(ClientError::InvalidRequest);
        }
        let _active = ActiveMedia::enter(&self.active_media, request_id);
        let handle = self.authorized()?;
        let profile = match self.storage.profile(peer_id) {
            Ok(Some(profile)) if profile.avatar_cache_key.is_some() => profile,
            _ => {
                let profile =
                    proto::dispatch::get_profile(handle, peer_id).map_err(ClientError::from)?;
                self.storage
                    .cache_profile(&profile)
                    .map_err(|_| ClientError::Storage)?;
                profile
            }
        };
        let base_key = profile
            .avatar_cache_key
            .as_deref()
            .ok_or(ClientError::UnsupportedContent)?;
        let remote_key = match kind {
            MediaDownloadKind::Full => base_key.to_owned(),
            MediaDownloadKind::Thumb => derived_cache_key("thumb", base_key),
            MediaDownloadKind::Display => derived_cache_key("display", base_key),
        };
        self.download(DownloadRequest {
            request_id,
            handle,
            chat_id: peer_id,
            message_id: 0,
            kind,
            mode,
            category: MediaCategory::Avatar,
            expected_bytes: None,
            remote_key,
        })
    }

    fn download_custom_emoji(
        &mut self,
        request_id: u64,
        document_id: i64,
        mode: MediaDownloadMode,
    ) -> Result<WorkerResponse, ClientError> {
        if request_id == 0 || document_id == 0 {
            return Err(ClientError::InvalidRequest);
        }
        let _active = ActiveMedia::enter(&self.active_media, request_id);
        let handle = self.authorized()?;
        let automatic_rule = self.transfer_rule(mode, MediaCategory::Document)?;
        self.trim_media(
            self.storage
                .cache_limit()
                .map_err(|_| ClientError::Storage)?,
        )?;
        let remote_key = format!("custom-emoji:{document_id}");
        let key = CacheKey::new(self.account_id()?, &remote_key)
            .map_err(|_| ClientError::InvalidRequest)?;
        let mut writer = match self
            .media_cache
            .acquire(key.clone(), None)
            .map_err(|_| ClientError::Storage)?
        {
            Acquire::Cached(lease) => {
                let bytes = lease
                    .path()
                    .metadata()
                    .map_err(|_| ClientError::Storage)?
                    .len();
                self.storage
                    .record_media(&remote_key, MediaCategory::Document, None, bytes, true)
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::MediaReady {
                    transfer_id: request_id,
                    cache_key: remote_key,
                    lease: Box::new(lease),
                });
            }
            Acquire::Busy => return Err(ClientError::InvalidRequest),
            Acquire::Download(writer) => writer,
        };
        if writer.offset().map_err(|_| ClientError::Storage)? > 0 {
            writer.cancel(false).map_err(|_| ClientError::Storage)?;
            writer = match self
                .media_cache
                .acquire(key, None)
                .map_err(|_| ClientError::Storage)?
            {
                Acquire::Download(writer) => writer,
                Acquire::Busy => return Err(ClientError::InvalidRequest),
                Acquire::Cached(lease) => {
                    let bytes = lease
                        .path()
                        .metadata()
                        .map_err(|_| ClientError::Storage)?
                        .len();
                    self.storage
                        .record_media(&remote_key, MediaCategory::Document, None, bytes, true)
                        .map_err(|_| ClientError::Storage)?;
                    return Ok(WorkerResponse::MediaReady {
                        transfer_id: request_id,
                        cache_key: remote_key,
                        lease: Box::new(lease),
                    });
                }
            };
        }
        if automatic_rule.is_some_and(|rule| !rule.enabled) {
            writer.cancel(true).map_err(|_| ClientError::Storage)?;
            return Err(ClientError::Forbidden);
        }
        let staging = self.media_staging.join(request_id.to_string());
        let _ = std::fs::remove_file(&staging);
        let result = proto::media_download::download_custom_emoji(
            handle,
            document_id,
            staging.to_string_lossy().into_owned(),
        )
        .map_err(ClientError::from)
        .and_then(|_| std::fs::read(&staging).map_err(|_| ClientError::Storage));
        let _ = std::fs::remove_file(&staging);
        match result {
            Ok(bytes) => {
                if bytes.is_empty()
                    || automatic_rule.is_some_and(|rule| bytes.len() as u64 > rule.max_bytes)
                {
                    writer.cancel(false).map_err(|_| ClientError::Storage)?;
                    return Err(ClientError::InvalidRequest);
                }
                if writer.append(0, &bytes).is_err() {
                    writer.cancel(true).map_err(|_| ClientError::Storage)?;
                    return Err(ClientError::Storage);
                }
                self.emit(WorkerEvent::MediaProgress {
                    transfer_id: request_id,
                    downloaded: bytes.len() as u64,
                    total: Some(bytes.len() as u64),
                });
                let lease = writer.finish().map_err(|_| ClientError::Storage)?;
                self.storage
                    .record_media(
                        &remote_key,
                        MediaCategory::Document,
                        None,
                        bytes.len() as u64,
                        true,
                    )
                    .map_err(|_| ClientError::Storage)?;
                self.trim_media(
                    self.storage
                        .cache_limit()
                        .map_err(|_| ClientError::Storage)?,
                )?;
                Ok(WorkerResponse::MediaReady {
                    transfer_id: request_id,
                    cache_key: remote_key,
                    lease: Box::new(lease),
                })
            }
            Err(error) => {
                writer.cancel(true).map_err(|_| ClientError::Storage)?;
                Err(error)
            }
        }
    }

    fn download_range(
        &mut self,
        request_id: u64,
        chat_id: i64,
        message_id: i32,
        offset: u64,
        mode: MediaDownloadMode,
    ) -> Result<WorkerResponse, ClientError> {
        valid_message(chat_id, message_id)?;
        if request_id == 0 {
            return Err(ClientError::InvalidRequest);
        }
        let _active = ActiveMedia::enter(&self.active_media, request_id);
        let handle = self.authorized()?;
        let message = self.message_for_download(handle, chat_id, message_id)?;
        let category = MediaCategory::from_media_kind(message.media_kind.as_deref());
        let automatic_rule = self.transfer_rule(mode, category)?;
        let expected_bytes = message
            .file_size
            .and_then(|size| u64::try_from(size).ok())
            .filter(|size| *size > 0);
        if automatic_rule.is_some_and(|rule| offset > rule.max_bytes)
            || expected_bytes.is_some_and(|total| offset > total)
        {
            return Err(ClientError::InvalidRequest);
        }
        let chunk_size = proto::registry::get_client(handle)
            .map_err(ClientError::from)?
            .policy
            .chunk_size()
            .max(1) as usize;
        if !offset.is_multiple_of(chunk_size as u64) {
            return Err(ClientError::InvalidRequest);
        }
        let remote_key = message
            .media_cache_key
            .ok_or(ClientError::UnsupportedContent)?;
        let key = CacheKey::new(self.account_id()?, &remote_key)
            .map_err(|_| ClientError::InvalidRequest)?;
        match self
            .media_cache
            .acquire(key, expected_bytes)
            .map_err(|_| ClientError::Storage)?
        {
            Acquire::Cached(mut lease) => {
                let (bytes, done) =
                    read_cached_range(&mut lease, offset, chunk_size, expected_bytes)?;
                self.storage
                    .record_media(
                        &remote_key,
                        category,
                        Some(chat_id),
                        lease
                            .path()
                            .metadata()
                            .map_err(|_| ClientError::Storage)?
                            .len(),
                        true,
                    )
                    .map_err(|_| ClientError::Storage)?;
                return Ok(WorkerResponse::MediaChunk {
                    transfer_id: request_id,
                    cache_key: remote_key,
                    offset,
                    bytes,
                    done,
                    total: expected_bytes,
                });
            }
            Acquire::Download(writer) => {
                writer.cancel(true).map_err(|_| ClientError::Storage)?;
            }
            Acquire::Busy => return Err(ClientError::InvalidRequest),
        }
        let staging = self.media_staging.join(request_id.to_string());
        let _ = std::fs::remove_file(&staging);
        let result = proto::media_download::download_message_media_range(
            handle,
            chat_id,
            message_id,
            staging.to_string_lossy().into_owned(),
            proto::adapter::media::location::MediaDownloadKind::Full,
            Some(i64::try_from(offset).map_err(|_| ClientError::InvalidRequest)?),
        )
        .map_err(ClientError::from)
        .and_then(|_| std::fs::read(&staging).map_err(|_| ClientError::Storage));
        let _ = std::fs::remove_file(&staging);
        let bytes = result?;
        if bytes.len() > chunk_size
            || (bytes.is_empty() && expected_bytes != Some(offset))
            || automatic_rule
                .is_some_and(|rule| offset.saturating_add(bytes.len() as u64) > rule.max_bytes)
        {
            return Err(ClientError::InvalidRequest);
        }
        let next = offset.saturating_add(bytes.len() as u64);
        let done = bytes.len() < chunk_size || expected_bytes == Some(next);
        self.emit(WorkerEvent::MediaProgress {
            transfer_id: request_id,
            downloaded: next,
            total: expected_bytes,
        });
        Ok(WorkerResponse::MediaChunk {
            transfer_id: request_id,
            cache_key: remote_key,
            offset,
            bytes,
            done,
            total: expected_bytes,
        })
    }

    pub(super) fn download(
        &mut self,
        request: DownloadRequest,
    ) -> Result<WorkerResponse, ClientError> {
        let account = self.account_id()?;
        let chat = Some(request.chat_id);
        let automatic_rule = self.transfer_rule(request.mode, request.category)?;
        if automatic_rule.is_some_and(|rule| {
            request
                .expected_bytes
                .is_some_and(|size| size > rule.max_bytes)
        }) {
            return Err(ClientError::InvalidRequest);
        }
        self.trim_media(
            self.storage
                .cache_limit()
                .map_err(|_| ClientError::Storage)?,
        )?;
        let key =
            CacheKey::new(account, &request.remote_key).map_err(|_| ClientError::InvalidRequest)?;
        let acquired = self
            .media_cache
            .acquire(key, request.expected_bytes)
            .map_err(|_| ClientError::Storage)?;
        match acquired {
            Acquire::Cached(lease) => {
                let bytes = lease
                    .path()
                    .metadata()
                    .map_err(|_| ClientError::Storage)?
                    .len();
                self.storage
                    .record_media(&request.remote_key, request.category, chat, bytes, true)
                    .map_err(|_| ClientError::Storage)?;
                Ok(WorkerResponse::MediaReady {
                    transfer_id: request.request_id,
                    cache_key: request.remote_key,
                    lease: Box::new(lease),
                })
            }
            Acquire::Busy => Err(ClientError::InvalidRequest),
            Acquire::Download(mut writer) => {
                if let Some(rule) = automatic_rule
                    && !rule.enabled
                {
                    writer.cancel(true).map_err(|_| ClientError::Storage)?;
                    return Err(ClientError::Forbidden);
                }
                let result = self.import_download(&request, automatic_rule, chat, &mut writer);
                match result {
                    Ok(()) => {
                        let lease = writer.finish().map_err(|_| ClientError::Storage)?;
                        let bytes = lease
                            .path()
                            .metadata()
                            .map_err(|_| ClientError::Storage)?
                            .len();
                        self.storage
                            .record_media(&request.remote_key, request.category, chat, bytes, true)
                            .map_err(|_| ClientError::Storage)?;
                        self.trim_media(
                            self.storage
                                .cache_limit()
                                .map_err(|_| ClientError::Storage)?,
                        )?;
                        Ok(WorkerResponse::MediaReady {
                            transfer_id: request.request_id,
                            cache_key: request.remote_key,
                            lease: Box::new(lease),
                        })
                    }
                    Err(error) => {
                        writer
                            .cancel(error != ClientError::InvalidRequest)
                            .map_err(|_| ClientError::Storage)?;
                        Err(error)
                    }
                }
            }
        }
    }

    fn import_download(
        &mut self,
        request: &DownloadRequest,
        automatic_rule: Option<AutoDownloadRule>,
        chat: Option<i64>,
        writer: &mut DownloadLease,
    ) -> Result<(), ClientError> {
        let staging = self.media_staging.join(request.request_id.to_string());
        let cancel_marker = staging.with_extension("cancel");
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&cancel_marker);
        let download_kind = match request.kind {
            MediaDownloadKind::Full => proto::adapter::media::location::MediaDownloadKind::Full,
            MediaDownloadKind::Thumb => proto::adapter::media::location::MediaDownloadKind::Thumb,
            MediaDownloadKind::Display => {
                proto::adapter::media::location::MediaDownloadKind::Display
            }
        };
        let chunk_size = proto::registry::get_client(request.handle)
            .map_err(ClientError::from)?
            .policy
            .chunk_size()
            .max(1) as usize;
        let result = (|| {
            loop {
                let offset = writer.offset().map_err(|_| ClientError::Storage)?;
                if request.expected_bytes == Some(offset) {
                    return Ok(());
                }
                if !offset.is_multiple_of(chunk_size as u64)
                    || automatic_rule.is_some_and(|rule| offset > rule.max_bytes)
                {
                    return Err(ClientError::InvalidRequest);
                }
                let offset_i64 = i64::try_from(offset).map_err(|_| ClientError::InvalidRequest)?;
                proto::media_download::download_message_media_range(
                    request.handle,
                    request.chat_id,
                    request.message_id,
                    staging.to_string_lossy().into_owned(),
                    download_kind,
                    Some(offset_i64),
                )
                .map_err(ClientError::from)?;
                let bytes = std::fs::read(&staging).map_err(|_| ClientError::Storage)?;
                if bytes.len() > chunk_size || (bytes.is_empty() && offset == 0) {
                    return Err(ClientError::Protocol);
                }
                let next = offset
                    .checked_add(bytes.len() as u64)
                    .ok_or(ClientError::InvalidRequest)?;
                if automatic_rule.is_some_and(|rule| next > rule.max_bytes) {
                    return Err(ClientError::InvalidRequest);
                }
                writer
                    .append(offset, &bytes)
                    .map_err(|_| ClientError::Storage)?;
                self.storage
                    .record_media(&request.remote_key, request.category, chat, next, false)
                    .map_err(|_| ClientError::Storage)?;
                self.emit(WorkerEvent::MediaProgress {
                    transfer_id: request.request_id,
                    downloaded: next,
                    total: request.expected_bytes,
                });
                if bytes.len() < chunk_size {
                    return Ok(());
                }
                request_control::check().map_err(|_| ClientError::Cancelled)?;
            }
        })();
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&cancel_marker);
        result
    }

    fn send_pending_media(&mut self, album_id: i64) -> Result<WorkerResponse, ClientError> {
        let handle = self.authorized()?;
        self.replay()?;
        let PendingMediaSend {
            album_id,
            random_ids,
            send,
            ..
        } = self
            .storage
            .pending_media_send(album_id)
            .map_err(|_| ClientError::Storage)?
            .ok_or(ClientError::InvalidRequest)?;
        send.validate()?;
        if random_ids.len() != send.items.len() {
            return Err(ClientError::InvalidRequest);
        }
        validate_sources(&send)?;
        self.storage
            .mark_media_attempted(album_id)
            .map_err(|_| ClientError::Storage)?;
        let items = send
            .items
            .iter()
            .zip(&random_ids)
            .map(|(item, random_id)| upload_item(item, *random_id))
            .collect::<Vec<_>>();
        let options = SendOptions {
            silent: send.silent,
            no_webpage: false,
            schedule_date: send.schedule_date,
        };
        let results = if items.len() == 1 {
            vec![
                proto::dispatch::send_uploaded_media_with_options(
                    handle,
                    send.chat_id,
                    items[0].clone(),
                    send.reply_to_msg_id,
                    send.top_msg_id,
                    send.items[0].entities_json.clone(),
                    options,
                )
                .map_err(ClientError::from)?,
            ]
        } else {
            proto::dispatch::send_uploaded_album_with_options(
                handle,
                send.chat_id,
                items,
                send.reply_to_msg_id,
                send.top_msg_id,
                options,
            )
            .map_err(ClientError::from)?
        };
        self.replay()?;
        Ok(WorkerResponse::MediaOutgoing {
            album_id,
            random_ids,
            results,
        })
    }

    fn send_indexed_document(
        &mut self,
        chat_id: i64,
        document_id: i64,
        reply_to_msg_id: i32,
        top_msg_id: i32,
    ) -> Result<WorkerResponse, ClientError> {
        features::valid_chat(chat_id)?;
        if document_id == 0 || reply_to_msg_id < 0 || top_msg_id < 0 {
            return Err(ClientError::InvalidRequest);
        }
        let message = proto::extras::send_saved_gif(
            self.authorized()?,
            chat_id,
            document_id,
            reply_to_msg_id,
            top_msg_id,
        )
        .map_err(ClientError::from)?;
        self.storage
            .cache_messages(std::slice::from_ref(&message))
            .map_err(|_| ClientError::Storage)?;
        self.drain()?;
        Ok(WorkerResponse::Message(Box::new(message)))
    }
}

pub(super) struct ActiveMedia {
    active: Arc<AtomicU64>,
}

impl ActiveMedia {
    pub(super) fn enter(active: &Arc<AtomicU64>, transfer_id: u64) -> Self {
        active.store(transfer_id, Ordering::Release);
        Self {
            active: active.clone(),
        }
    }
}

impl Drop for ActiveMedia {
    fn drop(&mut self) {
        self.active.store(0, Ordering::Release);
    }
}

pub(super) fn read_cached_range(
    lease: &mut CacheLease,
    offset: u64,
    chunk_size: usize,
    total: Option<u64>,
) -> Result<(Vec<u8>, bool), ClientError> {
    let file = lease.file();
    let length = file.metadata().map_err(|_| ClientError::Storage)?.len();
    if offset > length || total.is_some_and(|total| total != length) {
        return Err(ClientError::InvalidRequest);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| ClientError::Storage)?;
    let mut bytes = vec![0; chunk_size.min((length - offset) as usize)];
    file.read_exact(&mut bytes)
        .map_err(|_| ClientError::Storage)?;
    let next = offset.saturating_add(bytes.len() as u64);
    Ok((bytes, next == length || total == Some(next)))
}

pub(super) fn transfer_id_from_path(path: &str) -> Option<u64> {
    std::path::Path::new(path)
        .file_stem()?
        .to_str()?
        .parse()
        .ok()
}

pub(super) fn derived_cache_key(prefix: &str, key: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}:{hash:016x}")
}

fn upload_item(item: &MediaUpload, random_id: i64) -> proto::UploadItemDto {
    proto::UploadItemDto {
        path: item.path.clone(),
        kind: item.kind.clone(),
        mime_type: item.mime_type.clone(),
        file_name: item.file_name.clone(),
        caption: item.caption.clone(),
        entities_json: item.entities_json.clone(),
        duration: item.duration,
        width: item.width,
        height: item.height,
        random_id,
    }
}

fn validate_sources(send: &MediaSend) -> Result<(), ClientError> {
    send.validate()?;
    for item in &send.items {
        validate_source(item)?;
    }
    Ok(())
}

fn validate_source(item: &MediaUpload) -> Result<(), ClientError> {
    let metadata = std::fs::metadata(&item.path).map_err(|_| ClientError::InvalidRequest)?;
    if !metadata.is_file() || metadata.len() == 0 {
        Err(ClientError::InvalidRequest)
    } else {
        Ok(())
    }
}
