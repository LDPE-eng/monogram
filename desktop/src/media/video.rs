use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSink;
use gstreamer_video::{VideoFrameExt, VideoFrameRef, VideoInfo};
use relm4::{gtk, gtk::glib};

struct OwnedFrame {
    width: i32,
    height: i32,
    stride: usize,
    bytes: Vec<u8>,
}

pub struct GtkVideoSink {
    element: gst::Element,
    picture: gtk::Picture,
    frames: Arc<Mutex<Option<OwnedFrame>>>,
    drain_source: Option<glib::SourceId>,
}

impl GtkVideoSink {
    pub fn new() -> Result<Self, String> {
        gst::init().map_err(|_| "GStreamer initialization failed".to_owned())?;
        let frames = Arc::new(Mutex::new(None));
        let sample_frames = frames.clone();
        let preroll_frames = frames.clone();
        let callbacks = gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| store_sample(sink, &sample_frames, false))
            .new_preroll(move |sink| store_sample(sink, &preroll_frames, true))
            .build();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build();
        let sink = AppSink::builder()
            .caps(&caps)
            .callbacks(callbacks)
            .max_buffers(2)
            .drop(true)
            .sync(true)
            .build();
        let element = sink.upcast::<gst::Element>();
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let weak_picture = picture.downgrade();
        let drain_frames = frames.clone();
        let drain_source = glib::timeout_add_local(Duration::from_millis(33), move || {
            let Some(picture) = weak_picture.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Ok(mut slot) = drain_frames.lock() else {
                return glib::ControlFlow::Break;
            };
            if let Some(frame) = slot.take() {
                let bytes = glib::Bytes::from_owned(frame.bytes);
                let texture = gtk::gdk::MemoryTexture::new(
                    frame.width,
                    frame.height,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    frame.stride,
                );
                picture.set_paintable(Some(&texture));
            }
            glib::ControlFlow::Continue
        });
        Ok(Self {
            element,
            picture,
            frames,
            drain_source: Some(drain_source),
        })
    }

    pub fn element(&self) -> gst::Element {
        self.element.clone()
    }

    pub fn widget(&self) -> &gtk::Picture {
        &self.picture
    }

    pub fn clear(&self) {
        if let Ok(mut slot) = self.frames.lock() {
            slot.take();
        }
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
    }
}

impl Drop for GtkVideoSink {
    fn drop(&mut self) {
        if let Some(source) = self.drain_source.take() {
            source.remove();
        }
        self.clear();
    }
}

fn store_sample(
    sink: &AppSink,
    frames: &Arc<Mutex<Option<OwnedFrame>>>,
    preroll: bool,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let sample = if preroll {
        sink.pull_preroll()
    } else {
        sink.pull_sample()
    }
    .map_err(|_| gst::FlowError::Eos)?;
    let mut slot = frames.lock().map_err(|_| gst::FlowError::Error)?;
    *slot = Some(copy_sample(&sample)?);
    Ok(gst::FlowSuccess::Ok)
}

fn copy_sample(sample: &gst::Sample) -> Result<OwnedFrame, gst::FlowError> {
    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
    let info = VideoInfo::from_caps(caps).map_err(|_| gst::FlowError::Error)?;
    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
    let frame = VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|_| gst::FlowError::Error)?;
    let width = i32::try_from(frame.width()).map_err(|_| gst::FlowError::Error)?;
    let height = i32::try_from(frame.height()).map_err(|_| gst::FlowError::Error)?;
    let stride = usize::try_from(frame.plane_stride()[0]).map_err(|_| gst::FlowError::Error)?;
    let row_bytes = frame.width() as usize * 4;
    if width <= 0 || height <= 0 || stride < row_bytes {
        return Err(gst::FlowError::Error);
    }
    let bytes = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;
    let required = stride
        .checked_mul(frame.height() as usize)
        .ok_or(gst::FlowError::Error)?;
    if bytes.len() < required {
        return Err(gst::FlowError::Error);
    }
    Ok(OwnedFrame {
        width,
        height,
        stride,
        bytes: bytes[..required].to_vec(),
    })
}

#[cfg(test)]
#[path = "tests/unit/video.rs"]
mod tests;
