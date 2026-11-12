use crate::media::playback::{
    LoopMode, PlaybackCommand, PlaybackEvent, PlaybackOwner, PlaybackState,
};
use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};
use relm4::gtk::glib;
use std::{cell::RefCell, rc::Rc, sync::Arc};

const MIN_RATE: f64 = 0.25;
const MAX_RATE: f64 = 4.0;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MprisMetadata {
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum MprisError {
    #[error("MPRIS session bus registration failed")]
    RegistrationFailed,
    #[error("MPRIS property or signal delivery failed")]
    SignalFailed,
}

#[derive(Debug)]
struct CurrentTrack {
    id: TrackId,
    index: usize,
    metadata: MprisMetadata,
}

#[derive(Debug)]
struct State {
    current: Option<CurrentTrack>,
    next_id: u64,
    queue_len: usize,
    loop_mode: LoopMode,
    position: Time,
    pending_seek: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current: None,
            next_id: 0,
            queue_len: 0,
            loop_mode: LoopMode::Off,
            position: Time::ZERO,
            pending_seek: false,
        }
    }
}

impl State {
    fn metadata(&self) -> Metadata {
        let Some(current) = &self.current else {
            return Metadata::new();
        };
        let mut metadata = Metadata::new();
        metadata.set_trackid(Some(current.id.clone()));
        metadata.set_title(current.metadata.title.clone());
        metadata.set_artist(Some(current.metadata.artists.clone()));
        metadata.set_album(current.metadata.album.clone());
        metadata.set_length(current.metadata.duration_ms.map(time_from_millis));
        metadata
    }

    fn replace_track(&mut self, index: usize) {
        self.next_id = self.next_id.wrapping_add(1);
        self.queue_len = self.queue_len.max(index.saturating_add(1));
        let path = format!("/io/github/monogram/desktop/track/{}", self.next_id);
        let id = TrackId::try_from(path).expect("generated MPRIS track id is a valid object path");
        self.current = Some(CurrentTrack {
            id,
            index,
            metadata: MprisMetadata::default(),
        });
        self.position = Time::ZERO;
        self.pending_seek = false;
    }

    fn clear_current(&mut self) {
        self.current = None;
        self.position = Time::ZERO;
        self.pending_seek = false;
    }

    fn clear(&mut self) {
        self.clear_current();
        self.queue_len = 0;
    }

    fn update_queue(&mut self, track_index: Option<usize>, queue_len: usize) {
        self.queue_len = queue_len;
        match track_index {
            None => self.clear_current(),
            Some(index) if index < queue_len => {
                if let Some(current) = &mut self.current {
                    current.index = index;
                }
            }
            Some(_) => self.clear_current(),
        }
    }

    fn navigation(&self) -> (bool, bool) {
        let Some(current) = &self.current else {
            return (false, false);
        };
        let next = current.index.saturating_add(1) < self.queue_len
            || (self.queue_len > 0 && self.loop_mode == LoopMode::Playlist);
        (next, current.index > 0)
    }

    fn seek_offset(&mut self, offset: Time) -> Option<SeekAction> {
        self.current.as_ref()?;
        let target = (i128::from(self.position.as_micros()) + i128::from(offset.as_micros()))
            .clamp(0, i128::from(i64::MAX)) as i64;
        let target = Time::from_micros(target);
        if self
            .current
            .as_ref()
            .and_then(|track| track.metadata.duration_ms)
            .is_some_and(|duration| target > time_from_millis(duration))
        {
            return Some(SeekAction::Next);
        }
        self.pending_seek = true;
        Some(SeekAction::Seek(time_to_millis(target)))
    }

    fn set_position(&mut self, track_id: &TrackId, position: Time) -> Option<u64> {
        let current = self.current.as_ref()?;
        if current.id != *track_id || position.is_negative() {
            return None;
        }
        if current
            .metadata
            .duration_ms
            .is_some_and(|duration| position > time_from_millis(duration))
        {
            return None;
        }
        self.pending_seek = true;
        Some(time_to_millis(position))
    }

    fn update_position(&mut self, milliseconds: u64, duration_ms: Option<u64>) -> bool {
        self.position = time_from_millis(milliseconds);
        let Some(current) = &mut self.current else {
            self.pending_seek = false;
            return false;
        };
        let duration_changed = current.metadata.duration_ms != duration_ms;
        current.metadata.duration_ms = duration_ms;
        duration_changed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SeekAction {
    Seek(u64),
    Next,
}

pub struct MprisAdapter {
    player: Rc<Player>,
    state: Rc<RefCell<State>>,
    runner: glib::JoinHandle<()>,
}

impl MprisAdapter {
    /// Must be created on the GTK main thread. Returns an error unless the
    /// session bus accepted the real MPRIS name registration.
    pub fn new(owner: Arc<PlaybackOwner>) -> Result<Self, MprisError> {
        let context = glib::MainContext::ref_thread_default();
        let bus_name = format!("monogram.instance{}", std::process::id());
        let player = Rc::new(
            context
                .block_on(
                    Player::builder(&bus_name)
                        .identity("Monogram")
                        .desktop_entry("io.github.monogram.desktop")
                        .minimum_rate(MIN_RATE)
                        .maximum_rate(MAX_RATE)
                        .build(),
                )
                .map_err(|_| MprisError::RegistrationFailed)?,
        );
        let state = Rc::new(RefCell::new(State::default()));

        let target = Arc::clone(&owner);
        player.connect_next(move |player| {
            if player.can_go_next() {
                let _ = target.send(PlaybackCommand::Next);
            }
        });
        let target = Arc::clone(&owner);
        player.connect_previous(move |player| {
            if player.can_go_previous() {
                let _ = target.send(PlaybackCommand::Previous);
            }
        });
        let target = Arc::clone(&owner);
        player.connect_play(move |player| {
            if player.can_play() {
                let _ = target.send(PlaybackCommand::Play);
            }
        });
        let target = Arc::clone(&owner);
        player.connect_pause(move |player| {
            if player.can_pause() {
                let _ = target.send(PlaybackCommand::Pause);
            }
        });
        let target = Arc::clone(&owner);
        player.connect_play_pause(move |player| match player.playback_status() {
            PlaybackStatus::Playing if player.can_pause() => {
                let _ = target.send(PlaybackCommand::Pause);
            }
            PlaybackStatus::Playing => {}
            _ if player.can_play() => {
                let _ = target.send(PlaybackCommand::Play);
            }
            _ => {}
        });
        let target = Arc::clone(&owner);
        player.connect_stop(move |_| {
            let _ = target.send(PlaybackCommand::Stop);
        });

        let target = Arc::clone(&owner);
        let seek_state = Rc::clone(&state);
        player.connect_seek(move |player, offset| {
            if !player.can_seek() {
                return;
            }
            match seek_state.borrow_mut().seek_offset(offset) {
                Some(SeekAction::Seek(milliseconds)) => {
                    let _ = target.send(PlaybackCommand::Seek { milliseconds });
                    let _ = target.send(PlaybackCommand::QueryPosition);
                }
                Some(SeekAction::Next) => {
                    let _ = target.send(PlaybackCommand::Next);
                }
                None => {}
            }
        });
        let target = Arc::clone(&owner);
        let position_state = Rc::clone(&state);
        player.connect_set_position(move |player, track_id, position| {
            if !player.can_seek() {
                return;
            }
            if let Some(milliseconds) = position_state.borrow_mut().set_position(track_id, position)
            {
                let _ = target.send(PlaybackCommand::Seek { milliseconds });
                let _ = target.send(PlaybackCommand::QueryPosition);
            }
        });
        let target = Arc::clone(&owner);
        player.connect_set_rate(move |_, rate| {
            if rate == 0.0 {
                let _ = target.send(PlaybackCommand::Pause);
            } else if rate.is_finite() && (MIN_RATE..=MAX_RATE).contains(&rate) {
                let _ = target.send(PlaybackCommand::Speed(rate));
            }
        });
        let target = Arc::clone(&owner);
        player.connect_set_loop_status(move |_, status| {
            let _ = target.send(PlaybackCommand::Loop(status.into()));
        });
        let target = Arc::clone(&owner);
        player.connect_set_volume(move |_, volume| {
            if volume.is_finite() {
                let _ = target.send(PlaybackCommand::Mute(volume <= 0.0));
            }
        });

        let runner = context.spawn_local(player.run());
        Ok(Self {
            player,
            state,
            runner,
        })
    }

    /// Applies an event already received by the UI's single playback receiver.
    pub async fn handle_event(&self, event: &PlaybackEvent) -> Result<(), MprisError> {
        match event {
            PlaybackEvent::TrackChanged { index, .. } => {
                self.state.borrow_mut().replace_track(*index);
                self.player.set_position(Time::ZERO);
                self.set_capabilities().await?;
                let metadata = self.state.borrow().metadata();
                signal(self.player.set_metadata(metadata).await)
            }
            PlaybackEvent::State(state) => {
                signal(self.player.set_playback_status((*state).into()).await)
            }
            PlaybackEvent::Position {
                milliseconds,
                duration_ms,
            } => {
                let (duration_changed, seeked, position) = {
                    let mut state = self.state.borrow_mut();
                    let duration_changed = state.update_position(*milliseconds, *duration_ms);
                    let seeked = std::mem::take(&mut state.pending_seek);
                    (duration_changed, seeked, state.position)
                };
                self.player.set_position(position);
                if duration_changed {
                    let metadata = self.state.borrow().metadata();
                    signal(self.player.set_metadata(metadata).await)?;
                }
                if seeked {
                    signal(self.player.seeked(position).await)?;
                }
                Ok(())
            }
            PlaybackEvent::Speed(rate) => signal(self.player.set_rate(*rate).await),
            PlaybackEvent::Muted(muted) => {
                signal(self.player.set_volume(if *muted { 0.0 } else { 1.0 }).await)
            }
            PlaybackEvent::Loop(mode) => {
                self.state.borrow_mut().loop_mode = *mode;
                signal(self.player.set_loop_status((*mode).into()).await)?;
                self.set_capabilities().await
            }
            PlaybackEvent::Error(_) => {
                self.state.borrow_mut().pending_seek = false;
                Ok(())
            }
            PlaybackEvent::Buffering(_) | PlaybackEvent::EndOfQueue => Ok(()),
        }
    }

    /// Publishes only descriptive fields; media source and local artwork paths
    /// are deliberately not part of this API.
    pub async fn set_current_metadata(&self, metadata: MprisMetadata) -> Result<(), MprisError> {
        let metadata = {
            let mut state = self.state.borrow_mut();
            let Some(current) = &mut state.current else {
                return Ok(());
            };
            current.metadata = metadata;
            state.metadata()
        };
        signal(self.player.set_metadata(metadata).await)
    }

    /// Synchronizes PlaybackSnapshot's track_index and queue_len. A missing
    /// index clears stale metadata while the next TrackChanged is pending.
    pub async fn update_queue(
        &self,
        track_index: Option<usize>,
        queue_len: usize,
    ) -> Result<(), MprisError> {
        let (reset, metadata) = {
            let mut state = self.state.borrow_mut();
            let had_current = state.current.is_some();
            state.update_queue(track_index, queue_len);
            (had_current && state.current.is_none(), state.metadata())
        };
        if reset {
            self.player.set_position(Time::ZERO);
            signal(self.player.set_metadata(metadata).await)?;
            signal(
                self.player
                    .set_playback_status(PlaybackStatus::Stopped)
                    .await,
            )?;
        }
        self.set_capabilities().await
    }

    /// Must be called when a new queue is empty: playback emits no TrackChanged
    /// event in that case.
    pub async fn clear(&self) -> Result<(), MprisError> {
        self.state.borrow_mut().clear();
        self.player.set_position(Time::ZERO);
        signal(self.player.set_metadata(Metadata::new()).await)?;
        signal(
            self.player
                .set_playback_status(PlaybackStatus::Stopped)
                .await,
        )?;
        self.set_capabilities().await
    }

    async fn set_capabilities(&self) -> Result<(), MprisError> {
        let (next, previous, present) = {
            let state = self.state.borrow();
            let (next, previous) = state.navigation();
            (next, previous, state.current.is_some())
        };
        signal(self.player.set_can_go_next(next).await)?;
        signal(self.player.set_can_go_previous(previous).await)?;
        signal(self.player.set_can_play(present).await)?;
        signal(self.player.set_can_pause(present).await)?;
        signal(self.player.set_can_seek(present).await)
    }

    /// Releases the MPRIS task and owner callbacks. PlaybackOwner shuts down
    /// its worker when the caller drops the last remaining Arc.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for MprisAdapter {
    fn drop(&mut self) {
        self.runner.abort();
        self.state.borrow_mut().clear();
    }
}

impl From<PlaybackState> for PlaybackStatus {
    fn from(state: PlaybackState) -> Self {
        match state {
            PlaybackState::Stopped => Self::Stopped,
            PlaybackState::Paused => Self::Paused,
            PlaybackState::Playing => Self::Playing,
        }
    }
}

impl From<LoopMode> for LoopStatus {
    fn from(mode: LoopMode) -> Self {
        match mode {
            LoopMode::Off => Self::None,
            LoopMode::Track => Self::Track,
            LoopMode::Playlist => Self::Playlist,
        }
    }
}

impl From<LoopStatus> for LoopMode {
    fn from(status: LoopStatus) -> Self {
        match status {
            LoopStatus::None => Self::Off,
            LoopStatus::Track => Self::Track,
            LoopStatus::Playlist => Self::Playlist,
        }
    }
}

fn time_from_millis(milliseconds: u64) -> Time {
    let milliseconds = milliseconds.min((i64::MAX / 1000) as u64) as i64;
    Time::from_millis(milliseconds)
}

fn time_to_millis(time: Time) -> u64 {
    (time.as_micros().max(0) as u64) / 1000
}

fn signal(result: Result<(), mpris_server::zbus::Error>) -> Result<(), MprisError> {
    result.map_err(|_| MprisError::SignalFailed)
}
