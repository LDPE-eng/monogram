use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSink;
use gstreamer_video::{VideoFrameExt, VideoFrameRef, VideoInfo};
use relm4::gtk::{gio, glib};
use tlottie::{CPURenderer, Composition, Limits, RenderOptions};

pub const MAX_RENDER_DIMENSION: u32 = 512;
const MAX_TGS_INPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TGS_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_WEBM_BYTES: u64 = 32 * 1024 * 1024;
const EVENT_CAPACITY: usize = 2;
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StickerSource {
    Tgs(Vec<u8>),
    Webm(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Straight,
    Premultiplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StickerError {
    EmptyInput,
    InputTooLarge,
    InvalidSize,
    InvalidTgs,
    InvalidWebm,
    UnsupportedVp9,
    RenderFailed,
    BackendUnavailable,
}

#[derive(Debug)]
pub struct RgbaFrame {
    width: i32,
    height: i32,
    stride: usize,
    bytes: Vec<u8>,
    alpha: AlphaMode,
}

impl RgbaFrame {
    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn stride(&self) -> usize {
        self.stride
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn alpha(&self) -> AlphaMode {
        self.alpha
    }

    fn new(
        width: i32,
        height: i32,
        bytes: Vec<u8>,
        alpha: AlphaMode,
    ) -> Result<Self, StickerError> {
        if width <= 0
            || height <= 0
            || width > MAX_RENDER_DIMENSION as i32
            || height > MAX_RENDER_DIMENSION as i32
        {
            return Err(StickerError::InvalidSize);
        }
        let stride = (width as usize)
            .checked_mul(4)
            .ok_or(StickerError::InvalidSize)?;
        let required = stride
            .checked_mul(height as usize)
            .ok_or(StickerError::InvalidSize)?;
        if bytes.len() != required {
            return Err(StickerError::RenderFailed);
        }
        Ok(Self {
            width,
            height,
            stride,
            bytes,
            alpha,
        })
    }

    pub fn into_bytes(self) -> (i32, i32, usize, Vec<u8>, AlphaMode) {
        (self.width, self.height, self.stride, self.bytes, self.alpha)
    }
}

#[derive(Debug)]
pub enum StickerEvent {
    Frame(RgbaFrame),
    Failed(StickerError),
}

enum StickerCommand {
    Shutdown,
}

pub struct StickerOwner {
    commands: mpsc::Sender<StickerCommand>,
    worker: Option<JoinHandle<()>>,
}

impl StickerOwner {
    pub fn spawn(
        source: StickerSource,
        max_dimension: u32,
    ) -> Result<(Self, Receiver<StickerEvent>), StickerError> {
        if max_dimension == 0 || max_dimension > MAX_RENDER_DIMENSION {
            return Err(StickerError::InvalidSize);
        }
        let (commands, command_receiver) = mpsc::channel();
        let (events, event_receiver) = mpsc::sync_channel(EVENT_CAPACITY);
        let worker = thread::Builder::new()
            .name("monogram-sticker".into())
            .spawn(move || match source {
                StickerSource::Tgs(bytes) => {
                    run_tgs(&bytes, max_dimension, &command_receiver, &events)
                }
                StickerSource::Webm(path) => {
                    run_webm(&path, max_dimension, &command_receiver, &events)
                }
            })
            .map_err(|_| StickerError::BackendUnavailable)?;
        Ok((
            Self {
                commands,
                worker: Some(worker),
            },
            event_receiver,
        ))
    }
}

impl Drop for StickerOwner {
    fn drop(&mut self) {
        let _ = self.commands.send(StickerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_tgs(
    bytes: &[u8],
    max_dimension: u32,
    commands: &Receiver<StickerCommand>,
    events: &SyncSender<StickerEvent>,
) {
    let mut animation = match TgsAnimation::from_tgs(bytes, max_dimension) {
        Ok(animation) => animation,
        Err(error) => {
            send_terminal(events, error);
            return;
        }
    };
    let mut index = 0;
    let mut next_frame = Instant::now();
    loop {
        let frame = match animation.render(index) {
            Ok(frame) => frame,
            Err(error) => {
                send_terminal(events, error);
                return;
            }
        };
        match events.try_send(StickerEvent::Frame(frame)) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => return,
        }
        if animation.frame_count == 1 {
            let _ = commands.recv();
            return;
        }
        index = (index + 1) % animation.frame_count;
        next_frame += animation.frame_interval;
        let now = Instant::now();
        if next_frame <= now {
            next_frame = now + animation.frame_interval;
        }
        match commands.recv_timeout(next_frame.saturating_duration_since(now)) {
            Ok(StickerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

struct TgsAnimation {
    renderer: CPURenderer,
    in_point: f32,
    frame_count: u32,
    frame_interval: Duration,
    width: i32,
    height: i32,
}

impl TgsAnimation {
    fn from_tgs(bytes: &[u8], max_dimension: u32) -> Result<Self, StickerError> {
        let json = decompress_tgs(bytes)?;
        Self::from_json(&json, max_dimension)
    }

    fn from_json(json: &[u8], max_dimension: u32) -> Result<Self, StickerError> {
        if max_dimension == 0 || max_dimension > MAX_RENDER_DIMENSION {
            return Err(StickerError::InvalidSize);
        }
        let limits = Limits {
            max_input_bytes: MAX_TGS_JSON_BYTES,
            max_dimension: MAX_RENDER_DIMENSION,
            ..Limits::default()
        };
        let composition =
            Composition::parse(json, &limits).map_err(|_| StickerError::InvalidTgs)?;
        if !composition.frame_rate.is_finite()
            || composition.frame_rate <= 0.0
            || !composition.in_point.is_finite()
            || composition.frame_count() == 0
        {
            return Err(StickerError::InvalidTgs);
        }
        let (width, height) = fit_dimensions(composition.width, composition.height, max_dimension)?;
        let frame_seconds = (1.0 / f64::from(composition.frame_rate)).clamp(0.0, 1.0);
        let frame_interval = Duration::from_secs_f64(frame_seconds).max(MIN_FRAME_INTERVAL);
        Ok(Self {
            in_point: composition.in_point,
            frame_count: composition.frame_count(),
            frame_interval,
            width: width as i32,
            height: height as i32,
            renderer: CPURenderer::new(composition),
        })
    }

    fn render(&mut self, index: u32) -> Result<RgbaFrame, StickerError> {
        if index >= self.frame_count {
            return Err(StickerError::RenderFailed);
        }
        let pixel_count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or(StickerError::InvalidSize)?;
        let mut pixels = vec![0u32; pixel_count];
        self.renderer
            .render(
                self.in_point + index as f32,
                &mut pixels,
                self.width as u32,
                self.height as u32,
                RenderOptions::default(),
            )
            .map_err(|_| StickerError::RenderFailed)?;
        let mut bytes = Vec::with_capacity(pixel_count * 4);
        for pixel in pixels {
            bytes.extend_from_slice(&pixel.to_le_bytes());
        }
        RgbaFrame::new(self.width, self.height, bytes, AlphaMode::Premultiplied)
    }
}

fn fit_dimensions(width: u32, height: u32, maximum: u32) -> Result<(u32, u32), StickerError> {
    if width == 0
        || height == 0
        || maximum == 0
        || width > MAX_RENDER_DIMENSION
        || height > MAX_RENDER_DIMENSION
        || maximum > MAX_RENDER_DIMENSION
    {
        return Err(StickerError::InvalidSize);
    }
    if width <= maximum && height <= maximum {
        return Ok((width, height));
    }
    if width >= height {
        Ok((
            maximum,
            ((u64::from(height) * u64::from(maximum) / u64::from(width)) as u32).max(1),
        ))
    } else {
        Ok((
            ((u64::from(width) * u64::from(maximum) / u64::from(height)) as u32).max(1),
            maximum,
        ))
    }
}

fn decompress_tgs(bytes: &[u8]) -> Result<Vec<u8>, StickerError> {
    if bytes.is_empty() {
        return Err(StickerError::EmptyInput);
    }
    if bytes.len() > MAX_TGS_INPUT_BYTES {
        return Err(StickerError::InputTooLarge);
    }
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Err(StickerError::InvalidTgs);
    }
    let input = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes.to_vec()));
    let decompressor = gio::ZlibDecompressor::new(gio::ZlibCompressorFormat::Gzip);
    let stream = gio::ConverterInputStream::new(&input, &decompressor);
    let mut output = Vec::new();
    loop {
        let chunk =
            gio::prelude::InputStreamExt::read_bytes(&stream, 8192, None::<&gio::Cancellable>)
                .map_err(|_| StickerError::InvalidTgs)?;
        if chunk.is_empty() {
            break;
        }
        if output
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > MAX_TGS_JSON_BYTES)
        {
            return Err(StickerError::InputTooLarge);
        }
        output.extend_from_slice(&chunk);
    }
    if output.is_empty() {
        return Err(StickerError::EmptyInput);
    }
    Ok(output)
}

fn run_webm(
    path: &Path,
    max_dimension: u32,
    commands: &Receiver<StickerCommand>,
    events: &SyncSender<StickerEvent>,
) {
    let decoder = match WebmDecoder::new(path, max_dimension, events) {
        Ok(decoder) => decoder,
        Err(error) => {
            send_terminal(events, error);
            return;
        }
    };
    loop {
        match commands.recv_timeout(Duration::from_millis(10)) {
            Ok(StickerCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if decoder.terminal.load(Ordering::Acquire) {
            break;
        }
        for _ in 0..32 {
            let Some(message) = decoder.pipeline.bus().and_then(|bus| bus.pop()) else {
                break;
            };
            match message.view() {
                gst::MessageView::Error(_) => {
                    send_terminal(events, StickerError::InvalidWebm);
                    decoder.terminal.store(true, Ordering::Release);
                    break;
                }
                gst::MessageView::Eos(_) => {
                    if !decoder.saw_vp9.load(Ordering::Acquire)
                        || decoder.thumbnail_only.load(Ordering::Acquire)
                    {
                        send_terminal(events, StickerError::UnsupportedVp9);
                        decoder.terminal.store(true, Ordering::Release);
                        break;
                    }
                    if decoder
                        .pipeline
                        .seek_simple(
                            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                            gst::ClockTime::ZERO,
                        )
                        .is_err()
                    {
                        send_terminal(events, StickerError::InvalidWebm);
                        decoder.terminal.store(true, Ordering::Release);
                        break;
                    }
                }
                _ => {}
            }
        }
        let frame = decoder.frames.lock().ok().and_then(|mut slot| slot.take());
        if let Some(frame) = frame {
            match events.try_send(StickerEvent::Frame(frame)) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => break,
            }
            if decoder.thumbnail_only.load(Ordering::Acquire) {
                send_terminal(events, StickerError::UnsupportedVp9);
                decoder.terminal.store(true, Ordering::Release);
                break;
            }
        }
    }
    let _ = decoder.pipeline.set_state(gst::State::Null);
}

struct WebmDecoder {
    pipeline: gst::Pipeline,
    frames: Arc<Mutex<Option<RgbaFrame>>>,
    saw_vp9: Arc<AtomicBool>,
    thumbnail_only: Arc<AtomicBool>,
    terminal: Arc<AtomicBool>,
}

impl WebmDecoder {
    fn new(
        path: &Path,
        max_dimension: u32,
        events: &SyncSender<StickerEvent>,
    ) -> Result<Self, StickerError> {
        let metadata = fs::metadata(path).map_err(|_| StickerError::InvalidWebm)?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(StickerError::InvalidWebm);
        }
        if metadata.len() > MAX_WEBM_BYTES {
            return Err(StickerError::InputTooLarge);
        }
        let location = path.to_str().ok_or(StickerError::InvalidWebm)?;
        gst::init().map_err(|_| StickerError::BackendUnavailable)?;
        let pipeline = gst::Pipeline::new();
        let source = element("filesrc")?;
        source.set_property("location", location);
        let demux = element("matroskademux")?;
        let queue = element("queue")?;
        let decoder = gst::ElementFactory::make("avdec_vp9")
            .build()
            .or_else(|_| element("vp9dec"))?;
        let scale = element("videoscale")?;
        let convert = element("videoconvert")?;
        let frames = Arc::new(Mutex::new(None));
        let sample_frames = frames.clone();
        let preroll_frames = frames.clone();
        let callbacks = gstreamer_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| store_sample(sink, &sample_frames, max_dimension, false))
            .new_preroll(move |sink| store_sample(sink, &preroll_frames, max_dimension, true))
            .build();
        let range = gst::IntRange::new(1, max_dimension as i32);
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", range)
            .field("height", range)
            .build();
        let sink = AppSink::builder()
            .caps(&caps)
            .callbacks(callbacks)
            .max_buffers(2)
            .drop(true)
            .sync(true)
            .build();
        let sink_element = sink.upcast::<gst::Element>();
        pipeline
            .add_many([
                &source,
                &demux,
                &queue,
                &decoder,
                &scale,
                &convert,
                &sink_element,
            ])
            .map_err(|_| StickerError::BackendUnavailable)?;
        gst::Element::link_many([&source, &demux]).map_err(|_| StickerError::BackendUnavailable)?;
        gst::Element::link_many([&queue, &decoder, &scale, &convert, &sink_element])
            .map_err(|_| StickerError::BackendUnavailable)?;
        let saw_vp9 = Arc::new(AtomicBool::new(false));
        let thumbnail_only = Arc::new(AtomicBool::new(false));
        let terminal = Arc::new(AtomicBool::new(false));
        let pad_saw_vp9 = saw_vp9.clone();
        let pad_thumbnail_only = thumbnail_only.clone();
        let pad_terminal = terminal.clone();
        let pad_events = events.clone();
        demux.connect_pad_added(move |_, pad| {
            let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
            let Some(structure) = caps.structure(0) else {
                return;
            };
            if structure.name() != "video/x-vp9" {
                return;
            }
            if structure.get::<bool>("codec-alpha").unwrap_or(false) {
                pad_thumbnail_only.store(true, Ordering::Release);
            }
            if pad_saw_vp9.swap(true, Ordering::AcqRel) {
                return;
            }
            let Some(sink_pad) = queue.static_pad("sink") else {
                send_terminal(&pad_events, StickerError::BackendUnavailable);
                pad_terminal.store(true, Ordering::Release);
                return;
            };
            if sink_pad.is_linked() || pad.link(&sink_pad).is_err() {
                send_terminal(&pad_events, StickerError::UnsupportedVp9);
                pad_terminal.store(true, Ordering::Release);
            }
        });
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| StickerError::BackendUnavailable)?;
        Ok(Self {
            pipeline,
            frames,
            saw_vp9,
            thumbnail_only,
            terminal,
        })
    }
}

impl Drop for WebmDecoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn element(name: &str) -> Result<gst::Element, StickerError> {
    gst::ElementFactory::make(name)
        .build()
        .map_err(|_| StickerError::BackendUnavailable)
}

fn store_sample(
    sink: &AppSink,
    frames: &Arc<Mutex<Option<RgbaFrame>>>,
    max_dimension: u32,
    preroll: bool,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let sample = if preroll {
        sink.pull_preroll()
    } else {
        sink.pull_sample()
    }
    .map_err(|_| gst::FlowError::Eos)?;
    let frame = copy_sample(&sample, max_dimension)?;
    let mut slot = frames.lock().map_err(|_| gst::FlowError::Error)?;
    *slot = Some(frame);
    Ok(gst::FlowSuccess::Ok)
}

fn copy_sample(sample: &gst::Sample, max_dimension: u32) -> Result<RgbaFrame, gst::FlowError> {
    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
    let info = VideoInfo::from_caps(caps).map_err(|_| gst::FlowError::Error)?;
    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
    let frame = VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|_| gst::FlowError::Error)?;
    let width = i32::try_from(frame.width()).map_err(|_| gst::FlowError::Error)?;
    let height = i32::try_from(frame.height()).map_err(|_| gst::FlowError::Error)?;
    if width <= 0 || height <= 0 || width > max_dimension as i32 || height > max_dimension as i32 {
        return Err(gst::FlowError::Error);
    }
    let input_stride =
        usize::try_from(frame.plane_stride()[0]).map_err(|_| gst::FlowError::Error)?;
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or(gst::FlowError::Error)?;
    if input_stride < row_bytes {
        return Err(gst::FlowError::Error);
    }
    let input = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;
    let mut bytes = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row.checked_mul(input_stride).ok_or(gst::FlowError::Error)?;
        let end = start.checked_add(row_bytes).ok_or(gst::FlowError::Error)?;
        let source = input.get(start..end).ok_or(gst::FlowError::Error)?;
        bytes.extend_from_slice(source);
    }
    RgbaFrame::new(width, height, bytes, AlphaMode::Straight).map_err(|_| gst::FlowError::Error)
}

fn send_terminal(events: &SyncSender<StickerEvent>, error: StickerError) {
    let _ = events.try_send(StickerEvent::Failed(error));
}

#[cfg(test)]
#[path = "tests/unit/sticker.rs"]
mod tests;
