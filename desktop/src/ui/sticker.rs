use std::{cell::RefCell, rc::Rc, sync::mpsc::TryRecvError, time::Duration};

use relm4::{adw::prelude::*, gtk, gtk::glib};

use crate::{
    i18n::I18n,
    media::sticker::{
        AlphaMode, MAX_RENDER_DIMENSION, RgbaFrame, StickerError, StickerEvent, StickerOwner,
        StickerSource,
    },
};

struct State {
    source: Option<StickerSource>,
    fallback: Option<gtk::gdk::Texture>,
    last_frame: Option<gtk::gdk::MemoryTexture>,
    owner: Option<StickerOwner>,
    events: Option<std::sync::mpsc::Receiver<StickerEvent>>,
    tick: Option<glib::SourceId>,
    max_dimension: u32,
    active: bool,
    detached: bool,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
        self.events.take();
        self.owner.take();
    }
}

pub struct StickerView {
    picture: gtk::Picture,
    state: Rc<RefCell<State>>,
}

impl StickerView {
    pub fn new(i18n: &I18n) -> Self {
        Self::build(i18n, MAX_RENDER_DIMENSION)
    }

    pub fn with_max_dimension(i18n: &I18n, max_dimension: u32) -> Result<Self, StickerError> {
        if max_dimension == 0 || max_dimension > MAX_RENDER_DIMENSION {
            return Err(StickerError::InvalidSize);
        }
        Ok(Self::build(i18n, max_dimension))
    }

    fn build(i18n: &I18n, max_dimension: u32) -> Self {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        picture.update_property(&[gtk::accessible::Property::Label(
            &i18n
                .text("feature-dialog-dialog_media_sticker", &[])
                .unwrap_or_default(),
        )]);
        let state = Rc::new(RefCell::new(State {
            source: None,
            fallback: None,
            last_frame: None,
            owner: None,
            events: None,
            tick: None,
            max_dimension,
            active: true,
            detached: false,
        }));
        let callback_state = Rc::downgrade(&state);
        let callback_picture = picture.downgrade();
        picture.connect_unmap(move |_| {
            let (Some(state), Some(picture)) =
                (callback_state.upgrade(), callback_picture.upgrade())
            else {
                return;
            };
            state.borrow_mut().detached = true;
            stop_animation(&state, &picture);
        });
        let callback_state = Rc::downgrade(&state);
        let callback_picture = picture.downgrade();
        picture.connect_map(move |_| {
            let (Some(state), Some(picture)) =
                (callback_state.upgrade(), callback_picture.upgrade())
            else {
                return;
            };
            state.borrow_mut().detached = false;
            start_animation(&state, &picture);
        });
        Self { picture, state }
    }

    pub fn widget(&self) -> &gtk::Picture {
        &self.picture
    }

    pub fn load(&self, source: StickerSource, fallback: Option<gtk::gdk::Texture>) {
        stop_animation(&self.state, &self.picture);
        {
            let mut state = self.state.borrow_mut();
            state.source = Some(source);
            state.fallback = fallback;
            state.last_frame = None;
        }
        show_fallback(&self.state, &self.picture);
        start_animation(&self.state, &self.picture);
    }

    pub fn set_fallback(&self, fallback: Option<gtk::gdk::Texture>) {
        self.state.borrow_mut().fallback = fallback;
        if self.state.borrow().owner.is_none() {
            show_fallback(&self.state, &self.picture);
        }
    }

    pub fn set_active(&self, active: bool) {
        self.state.borrow_mut().active = active;
        if active {
            start_animation(&self.state, &self.picture);
        } else {
            stop_animation(&self.state, &self.picture);
        }
    }

    pub fn detach(&self) {
        self.state.borrow_mut().detached = true;
        stop_animation(&self.state, &self.picture);
    }

    pub fn clear(&self) {
        stop_animation(&self.state, &self.picture);
        let mut state = self.state.borrow_mut();
        state.source = None;
        state.fallback = None;
        state.last_frame = None;
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
    }

    pub fn is_playing(&self) -> bool {
        self.state.borrow().owner.is_some()
    }
}

fn start_animation(state: &Rc<RefCell<State>>, picture: &gtk::Picture) {
    {
        let state = state.borrow();
        if !state.active || state.detached || state.owner.is_some() {
            return;
        }
    }
    let (source, max_dimension) = {
        let state = state.borrow();
        let Some(source) = state.source.clone() else {
            return;
        };
        (source, state.max_dimension)
    };
    let Ok((owner, events)) = StickerOwner::spawn(source, max_dimension) else {
        show_fallback(state, picture);
        return;
    };
    {
        let mut state = state.borrow_mut();
        if !state.active || state.detached || state.owner.is_some() {
            drop(state);
            drop(owner);
            return;
        }
        state.owner = Some(owner);
        state.events = Some(events);
    }
    ensure_tick(state, picture);
}

fn stop_animation(state: &Rc<RefCell<State>>, picture: &gtk::Picture) {
    let (owner, tick) = {
        let mut state = state.borrow_mut();
        state.events.take();
        (state.owner.take(), state.tick.take())
    };
    if let Some(tick) = tick {
        tick.remove();
    }
    drop(owner);
    show_fallback(state, picture);
}

fn show_fallback(state: &Rc<RefCell<State>>, picture: &gtk::Picture) {
    let (fallback, last_frame) = {
        let state = state.borrow();
        (state.fallback.clone(), state.last_frame.clone())
    };
    if let Some(fallback) = fallback {
        picture.set_paintable(Some(&fallback));
    } else {
        picture.set_paintable(last_frame.as_ref());
    }
}

fn ensure_tick(state: &Rc<RefCell<State>>, picture: &gtk::Picture) {
    if state.borrow().tick.is_some() {
        return;
    }
    let callback_state = Rc::downgrade(state);
    let callback_picture = picture.downgrade();
    let tick = glib::timeout_add_local(Duration::from_millis(16), move || {
        let (Some(state), Some(picture)) = (callback_state.upgrade(), callback_picture.upgrade())
        else {
            return glib::ControlFlow::Break;
        };
        let mut terminal = false;
        for _ in 0..8 {
            let received = {
                let state = state.borrow();
                let Some(events) = &state.events else {
                    return glib::ControlFlow::Break;
                };
                events.try_recv()
            };
            match received {
                Ok(StickerEvent::Frame(frame)) => show_frame(&state, &picture, frame),
                Ok(StickerEvent::Failed(_)) | Err(TryRecvError::Disconnected) => {
                    terminal = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if terminal {
            stop_animation(&state, &picture);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    state.borrow_mut().tick = Some(tick);
}

fn show_frame(state: &Rc<RefCell<State>>, picture: &gtk::Picture, frame: RgbaFrame) {
    let (width, height, stride, bytes, alpha) = frame.into_bytes();
    let format = match alpha {
        AlphaMode::Straight => gtk::gdk::MemoryFormat::R8g8b8a8,
        AlphaMode::Premultiplied => gtk::gdk::MemoryFormat::R8g8b8a8Premultiplied,
    };
    let texture = gtk::gdk::MemoryTexture::new(
        width,
        height,
        format,
        &glib::Bytes::from_owned(bytes),
        stride,
    );
    state.borrow_mut().last_frame = Some(texture.clone());
    picture.set_paintable(Some(&texture));
}

#[cfg(test)]
#[path = "../../src/media/tests/unit/sticker_ui.rs"]
pub(crate) mod tests;
