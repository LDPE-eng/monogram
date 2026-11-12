use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    rc::Rc,
    sync::{Arc, mpsc::TryRecvError},
    time::Duration,
};

use relm4::{
    ComponentParts, ComponentSender, SimpleComponent, adw, adw::prelude::*, gtk, gtk::glib,
};

use crate::{
    i18n::I18n,
    media::{
        playback::{
            LoopMode, PlaybackCommand, PlaybackError, PlaybackEvent, PlaybackOwner, PlaybackState,
            Track,
        },
        video::GtkVideoSink,
    },
    mpris::{MprisAdapter, MprisMetadata},
};

enum MprisUpdate {
    Event(PlaybackEvent),
    Metadata(MprisMetadata),
    Queue {
        track_index: Option<usize>,
        queue_len: usize,
    },
    Clear,
}

struct MprisRuntimeInner {
    adapter: RefCell<Option<Rc<MprisAdapter>>>,
    queue: RefCell<VecDeque<MprisUpdate>>,
    scheduled: Cell<bool>,
    active: Cell<bool>,
}

struct MprisRuntime {
    inner: Rc<MprisRuntimeInner>,
}

impl MprisRuntime {
    fn new(owner: Arc<PlaybackOwner>) -> Option<Self> {
        let adapter = MprisAdapter::new(owner).ok()?;
        Some(Self {
            inner: Rc::new(MprisRuntimeInner {
                adapter: RefCell::new(Some(Rc::new(adapter))),
                queue: RefCell::new(VecDeque::new()),
                scheduled: Cell::new(false),
                active: Cell::new(true),
            }),
        })
    }

    fn dispatch(&self, update: MprisUpdate) {
        self.inner.queue.borrow_mut().push_back(update);
        if self.inner.scheduled.replace(true) {
            return;
        }
        let weak = Rc::downgrade(&self.inner);
        glib::MainContext::ref_thread_default().spawn_local(async move {
            loop {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if !inner.active.get() {
                    return;
                }
                let update = inner.queue.borrow_mut().pop_front();
                let Some(update) = update else {
                    inner.scheduled.set(false);
                    return;
                };
                let result = {
                    let adapter = inner.adapter.borrow().as_ref().cloned();
                    let Some(adapter) = adapter else {
                        return;
                    };
                    match &update {
                        MprisUpdate::Event(event) => adapter.handle_event(event).await,
                        MprisUpdate::Metadata(metadata) => {
                            adapter.set_current_metadata(metadata.clone()).await
                        }
                        MprisUpdate::Queue {
                            track_index,
                            queue_len,
                        } => adapter.update_queue(*track_index, *queue_len).await,
                        MprisUpdate::Clear => adapter.clear().await,
                    }
                };
                let _ = result;
            }
        });
    }

    fn clear(&self) {
        self.dispatch(MprisUpdate::Clear);
    }
}

impl Drop for MprisRuntime {
    fn drop(&mut self) {
        self.inner.active.set(false);
        self.inner.queue.borrow_mut().clear();
        self.inner.adapter.borrow_mut().take();
    }
}

pub enum MediaInput {
    Play {
        tracks: Vec<Track>,
        start: usize,
        autoplay: bool,
    },
    OpenViewer(gtk::Window),
    CloseViewer,
    TogglePlayback,
    Seek(u64),
    SeekRelative(i64),
    Speed(f64),
    ToggleMute,
    CycleLoop,
    Next,
    Previous,
    Close,
    Event(PlaybackEvent),
    ReceiverClosed,
    Tick,
}

impl fmt::Debug for MediaInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MediaInput(..)")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSnapshot {
    pub available: bool,
    pub track_id: Option<String>,
    pub track_index: Option<usize>,
    pub queue_len: usize,
    pub state: PlaybackState,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub speed: f64,
    pub muted: bool,
    pub loop_mode: LoopMode,
    pub buffering: Option<u8>,
    pub error: Option<PlaybackError>,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            available: true,
            track_id: None,
            track_index: None,
            queue_len: 0,
            state: PlaybackState::Stopped,
            position_ms: 0,
            duration_ms: None,
            speed: 1.0,
            muted: false,
            loop_mode: LoopMode::Off,
            buffering: None,
            error: None,
        }
    }
}

#[derive(Debug)]
pub enum MediaOutput {
    Snapshot(PlaybackSnapshot),
    ViewerClosed,
}

struct PlaybackControls {
    root: gtk::Box,
    previous: gtk::Button,
    play: gtk::Button,
    next: gtk::Button,
    backward: gtk::Button,
    forward: gtk::Button,
    mute: gtk::Button,
    repeat: gtk::Button,
    speed: gtk::SpinButton,
    seek: gtk::Scale,
    position: gtk::Label,
    buffering: gtk::Spinner,
}

impl PlaybackControls {
    fn new(i18n: &I18n, sender: &ComponentSender<MediaPlayer>, suppress: &Rc<Cell<bool>>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_margin_top(6);
        root.set_margin_bottom(8);
        let previous = icon_button(
            "media-skip-backward-symbolic",
            &localized(i18n, "desktop-back"),
        );
        connect(&previous, sender, || MediaInput::Previous);
        root.append(&previous);
        let play = icon_button(
            "media-playback-start-symbolic",
            &localized(i18n, "core-ui-media_mini_player_play"),
        );
        connect(&play, sender, || MediaInput::TogglePlayback);
        root.append(&play);
        let next = icon_button(
            "media-skip-forward-symbolic",
            &localized(i18n, "core-ui-media_mini_player_next"),
        );
        connect(&next, sender, || MediaInput::Next);
        root.append(&next);
        let backward = icon_button(
            "media-seek-backward-symbolic",
            &localized(i18n, "core-ui-media_video_back"),
        );
        connect(&backward, sender, || MediaInput::SeekRelative(-10));
        root.append(&backward);
        let forward = icon_button(
            "media-seek-forward-symbolic",
            &localized(i18n, "core-ui-media_video_forward"),
        );
        connect(&forward, sender, || MediaInput::SeekRelative(10));
        root.append(&forward);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_hexpand(true);
        seek.set_draw_value(false);
        seek.update_property(&[gtk::accessible::Property::Label(&localized(
            i18n,
            "core-ui-media_video_seek",
        ))]);
        let input = sender.input_sender().clone();
        let seek_suppress = suppress.clone();
        seek.connect_change_value(move |_, _, value| {
            if !seek_suppress.get() && value.is_finite() && value >= 0.0 && value <= u64::MAX as f64
            {
                input.emit(MediaInput::Seek(value as u64));
            }
            glib::Propagation::Proceed
        });
        root.append(&seek);
        let position = gtk::Label::new(Some("0:00 / --:--"));
        root.append(&position);
        let buffering = gtk::Spinner::new();
        buffering.update_property(&[gtk::accessible::Property::Label(&localized(
            i18n,
            "feature-dialog-dialog_media_loading",
        ))]);
        root.append(&buffering);
        let speed = gtk::SpinButton::with_range(0.25, 4.0, 0.25);
        speed.set_digits(2);
        speed.set_value(1.0);
        speed.update_property(&[gtk::accessible::Property::Label(&text(
            i18n,
            "core-ui-media_menu_speed",
            &[("arg1", "")],
        ))]);
        let input = sender.input_sender().clone();
        let speed_suppress = suppress.clone();
        speed.connect_value_changed(move |button| {
            if !speed_suppress.get() {
                input.emit(MediaInput::Speed(button.value()));
            }
        });
        root.append(&speed);
        let mute = icon_button(
            "audio-volume-high-symbolic",
            &localized(i18n, "core-ui-media_video_mute"),
        );
        connect(&mute, sender, || MediaInput::ToggleMute);
        root.append(&mute);
        let repeat = icon_button(
            "media-playlist-consecutive-symbolic",
            &localized(i18n, "core-ui-media_menu_loop"),
        );
        connect(&repeat, sender, || MediaInput::CycleLoop);
        root.append(&repeat);
        Self {
            root,
            previous,
            play,
            next,
            backward,
            forward,
            mute,
            repeat,
            speed,
            seek,
            position,
            buffering,
        }
    }

    fn refresh(&self, i18n: &I18n, snapshot: &PlaybackSnapshot) {
        let loaded = snapshot.queue_len > 0;
        let playing = snapshot.state == PlaybackState::Playing;
        let play_key = if playing {
            "core-ui-media_mini_player_pause"
        } else {
            "core-ui-media_mini_player_play"
        };
        set_icon(
            &self.play,
            if playing {
                "media-playback-pause-symbolic"
            } else {
                "media-playback-start-symbolic"
            },
            &localized(i18n, play_key),
        );
        self.play.set_sensitive(loaded && snapshot.available);
        self.previous.set_sensitive(
            loaded && snapshot.available && snapshot.track_index.is_some_and(|index| index > 0),
        );
        self.next.set_sensitive(
            loaded
                && snapshot.available
                && (snapshot
                    .track_index
                    .is_some_and(|index| index + 1 < snapshot.queue_len)
                    || snapshot.loop_mode == LoopMode::Playlist),
        );
        self.backward.set_sensitive(loaded && snapshot.available);
        self.forward.set_sensitive(loaded && snapshot.available);
        self.seek.set_sensitive(
            loaded
                && snapshot.available
                && snapshot.duration_ms.is_some_and(|duration| duration > 0),
        );
        self.seek
            .adjustment()
            .set_upper(snapshot.duration_ms.map_or(0.0, |duration| duration as f64));
        self.seek.set_value(
            snapshot
                .position_ms
                .min(snapshot.duration_ms.unwrap_or(u64::MAX)) as f64,
        );
        self.position.set_text(&format!(
            "{} / {}",
            timestamp(snapshot.position_ms),
            snapshot
                .duration_ms
                .map_or_else(|| "--:--".into(), timestamp)
        ));
        self.speed.set_value(snapshot.speed);
        self.speed.set_sensitive(snapshot.available);
        let mute_key = if snapshot.muted {
            "core-ui-media_video_unmute"
        } else {
            "core-ui-media_video_mute"
        };
        set_icon(
            &self.mute,
            if snapshot.muted {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            },
            &localized(i18n, mute_key),
        );
        self.mute.set_sensitive(snapshot.available);
        set_icon(
            &self.repeat,
            match snapshot.loop_mode {
                LoopMode::Off => "media-playlist-consecutive-symbolic",
                LoopMode::Track => "media-playlist-repeat-one-symbolic",
                LoopMode::Playlist => "media-playlist-repeat-symbolic",
            },
            &localized(i18n, "core-ui-media_menu_loop"),
        );
        self.repeat.set_sensitive(snapshot.available);
        let buffering = snapshot.buffering.is_some_and(|percent| percent < 100);
        self.buffering.set_visible(buffering);
        self.buffering.set_spinning(buffering);
    }
}

struct Viewer {
    window: adw::Window,
    title: gtk::Label,
    status: gtk::Label,
    controls: PlaybackControls,
}

impl Viewer {
    fn close(&self) {
        self.window.set_visible(false);
        self.window.set_content(None::<&gtk::Widget>);
    }
}

pub struct MediaPlayer {
    i18n: Rc<I18n>,
    root: gtk::Box,
    title: gtk::Label,
    status: gtk::Label,
    controls: PlaybackControls,
    viewer: Option<Viewer>,
    video: Option<GtkVideoSink>,
    owner: Option<Arc<PlaybackOwner>>,
    mpris: Option<MprisRuntime>,
    event_source: Option<glib::SourceId>,
    snapshot: PlaybackSnapshot,
    suppress: Rc<Cell<bool>>,
}

impl SimpleComponent for MediaPlayer {
    type Init = Rc<I18n>;
    type Input = MediaInput;
    type Output = MediaOutput;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 4)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.set_visible(false);
        root.add_css_class("card");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        top.set_margin_start(8);
        top.set_margin_end(8);
        top.set_margin_top(6);
        let title = gtk::Label::new(None);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&title);
        let expand = icon_button(
            "view-fullscreen-symbolic",
            &localized(&i18n, "core-ui-media_mini_player_expand"),
        );
        top.append(&expand);
        let close = icon_button(
            "window-close-symbolic",
            &localized(&i18n, "core-ui-media_mini_player_collapse"),
        );
        connect(&close, &sender, || MediaInput::Close);
        top.append(&close);
        root.append(&top);
        let status = gtk::Label::new(Some(&localized(&i18n, "core-ui-media_video_error")));
        status.set_visible(false);
        status.add_css_class("error");
        root.append(&status);
        let suppress = Rc::new(Cell::new(false));
        let controls = PlaybackControls::new(&i18n, &sender, &suppress);
        root.append(&controls.root);
        let video = GtkVideoSink::new().ok();
        let playback = PlaybackOwner::spawn(video.as_ref().map(GtkVideoSink::element), None);
        let (owner, mpris, event_source, available) = match playback {
            Ok((owner, events)) => {
                let owner = Arc::new(owner);
                let mpris = MprisRuntime::new(Arc::clone(&owner));
                let input = sender.input_sender().clone();
                let mut disconnected = false;
                let source = glib::timeout_add_local(Duration::from_millis(50), move || {
                    for _ in 0..256 {
                        match events.try_recv() {
                            Ok(event) => input.emit(MediaInput::Event(event)),
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => {
                                if !disconnected {
                                    input.emit(MediaInput::ReceiverClosed);
                                    disconnected = true;
                                }
                                break;
                            }
                        }
                    }
                    input.emit(MediaInput::Tick);
                    glib::ControlFlow::Continue
                });
                (Some(owner), mpris, Some(source), true)
            }
            Err(_) => (None, None, None, false),
        };
        let parent = sender.input_sender().clone();
        expand.clone().connect_clicked(move |_| {
            if let Some(window) = expand
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok())
            {
                parent.emit(MediaInput::OpenViewer(window));
            }
        });
        let snapshot = PlaybackSnapshot {
            available,
            ..PlaybackSnapshot::default()
        };
        let model = Self {
            i18n,
            root,
            title,
            status,
            controls,
            viewer: None,
            video,
            owner,
            mpris,
            event_source,
            snapshot,
            suppress,
        };
        model.refresh();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        match input {
            MediaInput::Play {
                tracks,
                start,
                autoplay,
            } => {
                self.close_viewer();
                let queue_len = tracks.len();
                if let Some(mpris) = &self.mpris {
                    if queue_len == 0 {
                        mpris.clear();
                    } else {
                        mpris.dispatch(MprisUpdate::Queue {
                            track_index: None,
                            queue_len,
                        });
                    }
                }
                self.snapshot.track_id = None;
                self.snapshot.track_index = None;
                self.snapshot.queue_len = queue_len;
                self.snapshot.state = PlaybackState::Stopped;
                self.snapshot.position_ms = 0;
                self.snapshot.duration_ms = None;
                self.snapshot.buffering = None;
                self.snapshot.error = None;
                if let Some(video) = &self.video {
                    video.clear();
                }
                self.command(PlaybackCommand::SetQueue {
                    tracks,
                    start,
                    autoplay,
                });
            }
            MediaInput::OpenViewer(parent) => self.open_viewer(parent, &sender),
            MediaInput::CloseViewer => {
                if self.viewer.is_some() {
                    self.close_viewer();
                    let _ = sender.output(MediaOutput::ViewerClosed);
                }
            }
            MediaInput::TogglePlayback => {
                self.command(if self.snapshot.state == PlaybackState::Playing {
                    PlaybackCommand::Pause
                } else {
                    PlaybackCommand::Play
                });
            }
            MediaInput::Seek(milliseconds) => {
                self.command(PlaybackCommand::Seek { milliseconds });
            }
            MediaInput::SeekRelative(seconds) => {
                let position = self.snapshot.position_ms as i128 + seconds as i128 * 1000;
                let maximum = self.snapshot.duration_ms.unwrap_or(u64::MAX) as i128;
                self.command(PlaybackCommand::Seek {
                    milliseconds: position.clamp(0, maximum) as u64,
                });
            }
            MediaInput::Speed(rate) => self.command(PlaybackCommand::Speed(rate)),
            MediaInput::ToggleMute => {
                self.command(PlaybackCommand::Mute(!self.snapshot.muted));
            }
            MediaInput::CycleLoop => {
                self.command(PlaybackCommand::Loop(match self.snapshot.loop_mode {
                    LoopMode::Off => LoopMode::Track,
                    LoopMode::Track => LoopMode::Playlist,
                    LoopMode::Playlist => LoopMode::Off,
                }));
            }
            MediaInput::Next => self.command(PlaybackCommand::Next),
            MediaInput::Previous => self.command(PlaybackCommand::Previous),
            MediaInput::Close => {
                self.close_viewer();
                if let Some(mpris) = &self.mpris {
                    mpris.clear();
                }
                self.snapshot.queue_len = 0;
                self.snapshot.track_id = None;
                self.snapshot.track_index = None;
                self.snapshot.state = PlaybackState::Stopped;
                self.snapshot.position_ms = 0;
                self.snapshot.duration_ms = None;
                self.snapshot.buffering = None;
                if let Some(video) = &self.video {
                    video.clear();
                }
                self.command(PlaybackCommand::SetQueue {
                    tracks: Vec::new(),
                    start: 0,
                    autoplay: false,
                });
            }
            MediaInput::Event(event) => self.playback_event(event),
            MediaInput::ReceiverClosed => self.snapshot.available = false,
            MediaInput::Tick => {
                if self.snapshot.queue_len > 0 && self.snapshot.available {
                    self.command(PlaybackCommand::QueryPosition);
                }
            }
        }
        self.refresh();
        let _ = sender.output(MediaOutput::Snapshot(self.snapshot.clone()));
    }

    fn shutdown(&mut self, _: &mut Self::Widgets, _: relm4::Sender<Self::Output>) {
        self.stop();
    }
}

impl MediaPlayer {
    fn command(&mut self, command: PlaybackCommand) {
        let sent = self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.send(command).is_ok());
        if !sent {
            self.snapshot.available = false;
        }
    }

    fn playback_event(&mut self, event: PlaybackEvent) {
        if let Some(mpris) = &self.mpris {
            mpris.dispatch(MprisUpdate::Event(event.clone()));
            if let PlaybackEvent::TrackChanged { id, .. } = &event {
                mpris.dispatch(MprisUpdate::Metadata(MprisMetadata {
                    title: Some(id.clone()),
                    artists: Vec::new(),
                    album: None,
                    duration_ms: None,
                }));
            }
        }
        match event {
            PlaybackEvent::TrackChanged { id, index } => {
                self.snapshot.track_id = Some(id);
                self.snapshot.track_index = Some(index);
                self.snapshot.position_ms = 0;
                self.snapshot.duration_ms = None;
                self.snapshot.buffering = None;
                self.snapshot.error = None;
            }
            PlaybackEvent::State(state) => {
                self.snapshot.state = state;
                if state == PlaybackState::Playing {
                    self.snapshot.error = None;
                }
            }
            PlaybackEvent::Position {
                milliseconds,
                duration_ms,
            } => {
                self.snapshot.position_ms = milliseconds;
                self.snapshot.duration_ms = duration_ms;
            }
            PlaybackEvent::Buffering(percent) => self.snapshot.buffering = Some(percent),
            PlaybackEvent::Speed(rate) => self.snapshot.speed = rate,
            PlaybackEvent::Muted(muted) => self.snapshot.muted = muted,
            PlaybackEvent::Loop(mode) => self.snapshot.loop_mode = mode,
            PlaybackEvent::EndOfQueue => self.snapshot.buffering = None,
            PlaybackEvent::Error(error) => {
                self.snapshot.error = Some(error);
                self.snapshot.buffering = None;
            }
        }
    }

    fn open_viewer(&mut self, parent: gtk::Window, sender: &ComponentSender<Self>) {
        if self.snapshot.queue_len == 0 {
            return;
        }
        if let Some(viewer) = &self.viewer {
            viewer.window.present();
            return;
        }
        let window = adw::Window::builder()
            .default_width(960)
            .default_height(640)
            .title(localized(&self.i18n, "core-ui-media_viewer_label"))
            .build();
        window.set_transient_for(Some(&parent));
        window.set_destroy_with_parent(true);
        if let Some(application) = parent.application() {
            window.set_application(Some(&application));
        }
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = gtk::Label::new(None);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title));
        let fullscreen = icon_button(
            "view-fullscreen-symbolic",
            &localized(&self.i18n, "feature-dialog-dialog_media_fullscreen"),
        );
        let fullscreen_window = window.clone();
        let fullscreen_button = fullscreen.clone();
        let enter_label = localized(&self.i18n, "feature-dialog-dialog_media_fullscreen");
        let exit_label = localized(&self.i18n, "feature-dialog-dialog_media_exit_fullscreen");
        fullscreen.connect_clicked(move |_| {
            if fullscreen_window.is_fullscreen() {
                fullscreen_window.unfullscreen();
            } else {
                fullscreen_window.fullscreen();
            }
            let label = if fullscreen_window.is_fullscreen() {
                &exit_label
            } else {
                &enter_label
            };
            fullscreen_button.set_tooltip_text(Some(label));
        });
        header.pack_end(&fullscreen);
        toolbar.add_top_bar(&header);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        if let Some(video) = &self.video {
            body.append(video.widget());
        }
        let status = gtk::Label::new(Some(&localized(&self.i18n, "core-ui-media_video_error")));
        status.add_css_class("error");
        status.set_visible(false);
        body.append(&status);
        let controls = PlaybackControls::new(&self.i18n, sender, &self.suppress);
        body.append(&controls.root);
        toolbar.set_content(Some(&body));
        window.set_content(Some(&toolbar));
        let input = sender.input_sender().clone();
        window.connect_close_request(move |_| {
            input.emit(MediaInput::CloseViewer);
            glib::Propagation::Stop
        });
        let escape = gtk::EventControllerKey::new();
        let input = sender.input_sender().clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                input.emit(MediaInput::CloseViewer);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(escape);
        self.viewer = Some(Viewer {
            window: window.clone(),
            title,
            status,
            controls,
        });
        self.refresh();
        window.present();
    }

    fn close_viewer(&mut self) {
        if let Some(viewer) = self.viewer.take() {
            viewer.close();
        }
    }

    fn refresh(&self) {
        self.suppress.set(true);
        self.root.set_visible(self.snapshot.queue_len > 0);
        self.title
            .set_text(self.snapshot.track_id.as_deref().unwrap_or_default());
        let accessible = text(
            &self.i18n,
            "core-ui-media_mini_player_now_playing",
            &[
                (
                    "arg1",
                    self.snapshot.track_id.as_deref().unwrap_or_default(),
                ),
                (
                    "arg2",
                    &(self.snapshot.track_index.unwrap_or(0) + 1).to_string(),
                ),
                ("arg3", &self.snapshot.queue_len.to_string()),
                (
                    "arg4",
                    &(self.snapshot.track_index.unwrap_or(0) + 1).to_string(),
                ),
                ("arg5", &self.snapshot.queue_len.to_string()),
            ],
        );
        self.root
            .update_property(&[gtk::accessible::Property::Label(&accessible)]);
        self.status
            .set_visible(!self.snapshot.available || self.snapshot.error.is_some());
        self.controls.refresh(&self.i18n, &self.snapshot);
        if let Some(viewer) = &self.viewer {
            viewer
                .title
                .set_text(self.snapshot.track_id.as_deref().unwrap_or_default());
            viewer
                .status
                .set_visible(!self.snapshot.available || self.snapshot.error.is_some());
            viewer.controls.refresh(&self.i18n, &self.snapshot);
        }
        self.suppress.set(false);
    }

    fn stop(&mut self) {
        if let Some(source) = self.event_source.take() {
            source.remove();
        }
        self.close_viewer();
        self.mpris.take();
        self.owner.take();
    }
}

impl Drop for MediaPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn icon_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

fn set_icon(button: &gtk::Button, icon: &str, label: &str) {
    button.set_icon_name(icon);
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
}

fn connect(
    button: &gtk::Button,
    sender: &ComponentSender<MediaPlayer>,
    input: impl Fn() -> MediaInput + 'static,
) {
    let sender = sender.input_sender().clone();
    button.connect_clicked(move |_| sender.emit(input()));
}

fn localized(i18n: &I18n, key: &str) -> String {
    text(i18n, key, &[])
}

fn text(i18n: &I18n, key: &str, args: &[(&str, &str)]) -> String {
    i18n.text(key, args)
        .or_else(|_| i18n.text("desktop-error-generic", &[]))
        .unwrap_or_default()
}

fn timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let hours = seconds / 3600;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
#[path = "../../src/media/tests/unit/media_ui.rs"]
pub(crate) mod tests;
