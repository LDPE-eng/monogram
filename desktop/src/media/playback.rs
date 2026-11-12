use super::cache::CacheLease;
use gstreamer::{self as gst, prelude::*};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

pub enum MediaSource {
    Cached(CacheLease),
    Uri(String),
}

pub struct Track {
    pub id: String,
    pub source: MediaSource,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LoopMode {
    #[default]
    Off,
    Track,
    Playlist,
}

pub enum PlaybackCommand {
    SetQueue {
        tracks: Vec<Track>,
        start: usize,
        autoplay: bool,
    },
    Play,
    Pause,
    Stop,
    Seek {
        milliseconds: u64,
    },
    Speed(f64),
    Mute(bool),
    Loop(LoopMode),
    Next,
    Previous,
    QueryPosition,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Paused,
    Playing,
}

#[derive(Clone, Debug)]
pub enum PlaybackEvent {
    TrackChanged {
        id: String,
        index: usize,
    },
    State(PlaybackState),
    Position {
        milliseconds: u64,
        duration_ms: Option<u64>,
    },
    Buffering(u8),
    Speed(f64),
    Muted(bool),
    Loop(LoopMode),
    EndOfQueue,
    Error(PlaybackError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackError {
    InvalidQueue,
    InvalidUri,
    InvalidSpeed,
    SeekFailed,
    StateFailed,
    StreamFailed,
}

pub struct PlaybackOwner {
    commands: Sender<PlaybackCommand>,
    worker: Option<JoinHandle<()>>,
}

impl PlaybackOwner {
    // Consume events on GTK's main context; neither the worker nor events touch GTK widgets.
    // Without an injected video sink playback is audio-only, never an unmanaged video window.
    pub fn spawn(
        video_sink: Option<gst::Element>,
        audio_sink: Option<gst::Element>,
    ) -> Result<(Self, Receiver<PlaybackEvent>), String> {
        gst::init().map_err(|_| "GStreamer initialization failed".to_owned())?;
        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .map_err(|_| "GStreamer playbin is unavailable".to_owned())?;
        let video_sink = match video_sink {
            Some(sink) => sink,
            None => gst::ElementFactory::make("fakesink")
                .build()
                .map_err(|_| "GStreamer fakesink is unavailable".to_owned())?,
        };
        playbin.set_property("video-sink", video_sink);
        if let Some(sink) = audio_sink {
            playbin.set_property("audio-sink", sink);
        }
        let bus = playbin
            .bus()
            .ok_or_else(|| "GStreamer bus is unavailable".to_owned())?;
        let (commands, receiver) = mpsc::channel();
        let (events, event_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("monogram-playback".into())
            .spawn(move || {
                let mut player = Player {
                    playbin,
                    bus,
                    events,
                    queue: Vec::new(),
                    index: None,
                    desired_playing: false,
                    buffering: false,
                    rate: 1.0,
                    pending_rate: false,
                    loop_mode: LoopMode::Off,
                };
                loop {
                    match receiver.recv_timeout(Duration::from_millis(20)) {
                        Ok(PlaybackCommand::Shutdown)
                        | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(command) => {
                            if let Err(error) = player.command(command) {
                                player.emit(PlaybackEvent::Error(error));
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    // Bound each bus drain so continuously producing sources cannot starve commands.
                    for _ in 0..64 {
                        let Some(message) = player.bus.pop() else {
                            break;
                        };
                        player.message(&message);
                    }
                }
                let _ = player.playbin.set_state(gst::State::Null);
            })
            .map_err(|_| "Could not start playback worker".to_owned())?;
        Ok((
            Self {
                commands,
                worker: Some(worker),
            },
            event_receiver,
        ))
    }

    pub fn send(&self, command: PlaybackCommand) -> Result<(), mpsc::SendError<PlaybackCommand>> {
        self.commands.send(command)
    }
}

impl Drop for PlaybackOwner {
    fn drop(&mut self) {
        let _ = self.commands.send(PlaybackCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Player {
    playbin: gst::Element,
    bus: gst::Bus,
    events: Sender<PlaybackEvent>,
    queue: Vec<Track>,
    index: Option<usize>,
    desired_playing: bool,
    buffering: bool,
    rate: f64,
    pending_rate: bool,
    loop_mode: LoopMode,
}

impl Player {
    fn emit(&self, event: PlaybackEvent) {
        let _ = self.events.send(event);
    }

    fn state(&self, state: gst::State) -> Result<(), PlaybackError> {
        self.playbin
            .set_state(state)
            .map(|_| ())
            .map_err(|_| PlaybackError::StateFailed)
    }

    fn command(&mut self, command: PlaybackCommand) -> Result<(), PlaybackError> {
        match command {
            PlaybackCommand::SetQueue {
                tracks,
                start,
                autoplay,
            } => {
                if !tracks.is_empty() && start >= tracks.len() {
                    return Err(PlaybackError::InvalidQueue);
                }
                self.state(gst::State::Null)?;
                self.bus.set_flushing(true);
                self.bus.set_flushing(false);
                self.queue = tracks;
                self.index = None;
                self.desired_playing = autoplay;
                if self.queue.is_empty() {
                    self.emit(PlaybackEvent::State(PlaybackState::Stopped));
                } else {
                    self.load(start)?;
                }
            }
            PlaybackCommand::Play => {
                self.desired_playing = true;
                if self.index.is_some() && !self.buffering {
                    self.state(gst::State::Playing)?;
                }
            }
            PlaybackCommand::Pause => {
                self.desired_playing = false;
                if self.index.is_some() {
                    self.state(gst::State::Paused)?;
                }
            }
            PlaybackCommand::Stop => {
                self.desired_playing = false;
                self.state(gst::State::Null)?;
                self.pending_rate = self.rate != 1.0;
                self.emit(PlaybackEvent::State(PlaybackState::Stopped));
            }
            PlaybackCommand::Seek { milliseconds } => {
                self.seek(milliseconds, self.rate)?;
            }
            PlaybackCommand::Speed(rate) => {
                if !rate.is_finite() || !(0.25..=4.0).contains(&rate) {
                    return Err(PlaybackError::InvalidSpeed);
                }
                if self.index.is_some() && self.playbin.current_state() >= gst::State::Paused {
                    let position = self
                        .playbin
                        .query_position::<gst::ClockTime>()
                        .map_or(0, |time| time.mseconds());
                    self.seek(position, rate)?;
                    self.pending_rate = false;
                } else {
                    self.pending_rate = true;
                }
                self.rate = rate;
                self.emit(PlaybackEvent::Speed(rate));
            }
            PlaybackCommand::Mute(muted) => {
                self.playbin.set_property("mute", muted);
                self.emit(PlaybackEvent::Muted(muted));
            }
            PlaybackCommand::Loop(mode) => {
                self.loop_mode = mode;
                self.emit(PlaybackEvent::Loop(mode));
            }
            PlaybackCommand::Next => self.advance(false)?,
            PlaybackCommand::Previous => {
                if let Some(index) = self.index {
                    self.load(index.saturating_sub(1))?;
                }
            }
            PlaybackCommand::QueryPosition => {
                if let Some(position) = self.playbin.query_position::<gst::ClockTime>() {
                    self.emit(PlaybackEvent::Position {
                        milliseconds: position.mseconds(),
                        duration_ms: self
                            .playbin
                            .query_duration::<gst::ClockTime>()
                            .map(|time| time.mseconds()),
                    });
                }
            }
            PlaybackCommand::Shutdown => {}
        }
        Ok(())
    }

    fn load(&mut self, index: usize) -> Result<(), PlaybackError> {
        let track = self.queue.get(index).ok_or(PlaybackError::InvalidQueue)?;
        let uri = match &track.source {
            MediaSource::Cached(lease) => gst::glib::filename_to_uri(lease.path(), None)
                .map_err(|_| PlaybackError::InvalidUri)?
                .to_string(),
            MediaSource::Uri(uri)
                if uri.starts_with("file://")
                    || uri.starts_with("https://")
                    || uri.starts_with("http://") =>
            {
                uri.clone()
            }
            MediaSource::Uri(_) => return Err(PlaybackError::InvalidUri),
        };
        self.state(gst::State::Null)?;
        self.bus.set_flushing(true);
        self.bus.set_flushing(false);
        self.playbin.set_property("uri", uri);
        self.index = Some(index);
        self.buffering = false;
        self.pending_rate = self.rate != 1.0;
        self.emit(PlaybackEvent::TrackChanged {
            id: self.queue[index].id.clone(),
            index,
        });
        self.state(if self.desired_playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        })
    }

    fn seek(&self, milliseconds: u64, rate: f64) -> Result<(), PlaybackError> {
        if self.index.is_none() {
            return Err(PlaybackError::SeekFailed);
        }
        let nanos = milliseconds
            .checked_mul(1_000_000)
            .ok_or(PlaybackError::SeekFailed)?;
        if nanos == u64::MAX {
            return Err(PlaybackError::SeekFailed);
        }
        let position = gst::ClockTime::from_nseconds(nanos);
        self.playbin
            .seek(
                rate,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                position,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .map_err(|_| PlaybackError::SeekFailed)
    }

    fn advance(&mut self, eos: bool) -> Result<(), PlaybackError> {
        let Some(index) = self.index else {
            return Ok(());
        };
        if eos && self.loop_mode == LoopMode::Track {
            self.seek(0, self.rate)?;
            return self.state(if self.desired_playing {
                gst::State::Playing
            } else {
                gst::State::Paused
            });
        }
        if index + 1 < self.queue.len() {
            return self.load(index + 1);
        }
        if self.loop_mode == LoopMode::Playlist {
            return self.load(0);
        }
        self.desired_playing = false;
        self.state(gst::State::Null)?;
        self.pending_rate = self.rate != 1.0;
        self.emit(PlaybackEvent::State(PlaybackState::Stopped));
        self.emit(PlaybackEvent::EndOfQueue);
        Ok(())
    }

    fn message(&mut self, message: &gst::MessageRef) {
        match message.view() {
            gst::MessageView::Error(_) => {
                self.desired_playing = false;
                let _ = self.state(gst::State::Null);
                // GStreamer debug strings can include credential-bearing URLs and local paths.
                self.emit(PlaybackEvent::Error(PlaybackError::StreamFailed));
                self.emit(PlaybackEvent::State(PlaybackState::Stopped));
            }
            gst::MessageView::Eos(_) => {
                if let Err(error) = self.advance(true) {
                    self.emit(PlaybackEvent::Error(error));
                }
            }
            gst::MessageView::Buffering(buffering) => {
                let percent = buffering.percent().clamp(0, 100) as u8;
                self.buffering = percent < 100;
                self.emit(PlaybackEvent::Buffering(percent));
                if self.desired_playing {
                    let target = if self.buffering {
                        gst::State::Paused
                    } else {
                        gst::State::Playing
                    };
                    if let Err(error) = self.state(target) {
                        self.emit(PlaybackEvent::Error(error));
                    }
                }
            }
            gst::MessageView::AsyncDone(_) if self.pending_rate => {
                self.pending_rate = false;
                if let Err(error) = self.seek(0, self.rate) {
                    self.emit(PlaybackEvent::Error(error));
                }
            }
            gst::MessageView::StateChanged(changed)
                if message.src() == Some(self.playbin.upcast_ref()) =>
            {
                let state = match changed.current() {
                    gst::State::Playing => Some(PlaybackState::Playing),
                    gst::State::Paused => Some(PlaybackState::Paused),
                    gst::State::Null => Some(PlaybackState::Stopped),
                    _ => None,
                };
                if let Some(state) = state {
                    self.emit(PlaybackEvent::State(state));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "tests/unit/playback.rs"]
mod tests;
