use super::*;
use relm4::gtk::gio::prelude::InputStreamExt;
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

const SAMPLE: &[u8] = include_bytes!(
    "../../../../../vendor/tlottie/examples/android/app/src/main/assets/sample.json"
);
const ALPHA_WEBM_BASE64: &str = "GkXfo59ChoEBQveBAULygQRC84EIQoKEd2VibUKHgQJChYECGFOAZwEAAAAAAAL0EU2bdLpNu4tTq4QVSalmU6yBoU27i1OrhBZUrmtTrIHYTbuMU6uEElTDZ1OsggEpTbuMU6uEHFO7a1OsggLe7AEAAAAAAABZAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAVSalmsirXsYMPQkBNgI1MYXZmNjIuMTIuMTAyV0GNTGF2ZjYyLjEyLjEwMkSJiEB5AAAAAAAAFlSua8yuAQAAAAAAAEPXgQFzxYj5lBmJPiz4qZyBACK1nIN1bmSIgQCGhVZfVlA5g4EBI+ODhAX14QDglLCBQLqBQJqBAlPAgQFVsIRVuYEBElTDZ0CAc3OgY8CAZ8iaRaOHRU5DT0RFUkSHjUxhdmY2Mi4xMi4xMDJzc9pjwItjxYj5lBmJPiz4qWfIpUWjh0VOQ09ERVJEh5hMYXZjNjIuMjguMTAyIGxpYnZweC12cDlnyKFFo4hEVVJBVElPTkSHkzAwOjAwOjAwLjQwMDAwMDAwMAAfQ7Z1QSnngQCgQIqh5oEAAACCSYNCAAPwA/YGOCQcGEoAACBAAGtD//+njQAbd7H6hFVixmzJ/roJl4u6EX1slqAufcP/+8ZhOxwcuUE3weoGzJ/roJl4u6EX1slqAufbd/9K6////+Yav/////Udg56pAHWhn6ad7oEBpZiCSYNCAAPwA/YGOCQcGEoAACBAAHKO0UCgsaGTgQBkAIYAQJKcSFAAAANwAABUcHWhlqaU7oEBpY+GAECSnEhQAAADcAAAVHD7gZygsaGTgQDIAIYAQJKcQE7gAANwAABUcHWhlqaU7oEBpY+GAECSnEBO4AADcAAAVHD7gZygsaGTgQEsAIYAQJKcSFAAAANwAABUcHWhlqaU7oEBpY+GAECSnEhQAAADcAAAVHD7gZwcU7trkbuPs4EAt4r3gQHxggGv8IED";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!(
                "sticker-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let input = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes.to_vec()));
    let compressor = gio::ZlibCompressor::new(gio::ZlibCompressorFormat::Gzip, -1);
    let stream = gio::ConverterInputStream::new(&input, &compressor);
    let mut output = Vec::new();
    loop {
        let chunk = stream.read_bytes(8192, None::<&gio::Cancellable>).unwrap();
        if chunk.is_empty() {
            break;
        }
        output.extend_from_slice(&chunk);
    }
    output
}

#[test]
fn local_tlottie_fixture_renders_real_distinct_premultiplied_frames() {
    let mut animation = TgsAnimation::from_json(SAMPLE, 64).unwrap();
    assert_eq!(animation.frame_count, 180);
    assert!(animation.frame_interval >= MIN_FRAME_INTERVAL);
    let first = animation.render(0).unwrap();
    let middle = animation.render(45).unwrap();
    assert_eq!((first.width(), first.height()), (64, 64));
    assert_eq!(first.stride(), 64 * 4);
    assert_eq!(first.bytes().len(), 64 * 64 * 4);
    assert_eq!(first.alpha(), AlphaMode::Premultiplied);
    assert!(
        first
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0)
    );
    assert_ne!(first.bytes(), middle.bytes());
}

#[test]
fn gzip_tgs_is_decoded_and_invalid_inputs_are_rejected() {
    let compressed = gzip(SAMPLE);
    assert!(compressed.starts_with(&[0x1f, 0x8b]));
    let mut animation = TgsAnimation::from_tgs(&compressed, 64).unwrap();
    assert_eq!(animation.render(0).unwrap().width(), 64);
    assert_eq!(decompress_tgs(&[]), Err(StickerError::EmptyInput));
    assert_eq!(decompress_tgs(b"not gzip"), Err(StickerError::InvalidTgs));
    assert!(matches!(
        TgsAnimation::from_json(SAMPLE, 0),
        Err(StickerError::InvalidSize)
    ));
    assert!(matches!(
        TgsAnimation::from_json(SAMPLE, MAX_RENDER_DIMENSION + 1),
        Err(StickerError::InvalidSize)
    ));
}

#[test]
fn extreme_frame_rates_do_not_overflow_the_frame_clock() {
    let json = String::from_utf8(SAMPLE.to_vec())
        .unwrap()
        .replacen("\"fr\":60", "\"fr\":1e-10", 1);
    let animation = TgsAnimation::from_json(json.as_bytes(), 64).unwrap();
    assert_eq!(animation.frame_interval, Duration::from_secs(1));
    let json = json.replacen("\"fr\":1e-10", "\"fr\":1000", 1);
    let animation = TgsAnimation::from_json(json.as_bytes(), 64).unwrap();
    assert_eq!(animation.frame_interval, MIN_FRAME_INTERVAL);
}

#[test]
fn dimensions_fit_without_exceeding_the_frame_budget() {
    assert_eq!(fit_dimensions(512, 512, 128), Ok((128, 128)));
    assert_eq!(fit_dimensions(512, 256, 128), Ok((128, 64)));
    assert_eq!(fit_dimensions(256, 512, 128), Ok((64, 128)));
    assert_eq!(fit_dimensions(64, 32, 128), Ok((64, 32)));
    assert_eq!(fit_dimensions(512, 1, 128), Ok((128, 1)));
}

#[test]
fn owner_stops_even_when_the_bounded_frame_queue_is_not_drained() {
    let (owner, events) = StickerOwner::spawn(StickerSource::Tgs(gzip(SAMPLE)), 64).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    drop(owner);
    let mut disconnected = false;
    for _ in 0..=EVENT_CAPACITY {
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                disconnected = true;
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("sticker worker did not stop"),
        }
    }
    assert!(disconnected);
}

#[test]
fn real_local_vp9_webm_reaches_bounded_rgba_frames() {
    gst::init().unwrap();
    let sandbox = Sandbox::new("webm");
    let path = sandbox.0.join("fixture.webm");
    encode_vp9_fixture(&path);
    let (owner, events) = StickerOwner::spawn(StickerSource::Webm(path), 64).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut first = None;
    for _ in 0..8 {
        let frame = loop {
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(StickerEvent::Frame(frame)) => break frame,
                Ok(StickerEvent::Failed(error)) => panic!("local VP9 failed: {error:?}"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("VP9 worker stopped early"),
            }
            assert!(Instant::now() < deadline, "no local VP9 frame");
        };
        first.get_or_insert(frame);
    }
    let frame = first.unwrap();
    assert_eq!((frame.width(), frame.height()), (64, 64));
    assert_eq!(frame.stride(), 64 * 4);
    assert_eq!(frame.bytes().len(), 64 * 64 * 4);
    assert_eq!(frame.alpha(), AlphaMode::Straight);
    drop(owner);
}

#[test]
fn vp9_codec_alpha_produces_one_thumbnail_and_stops() {
    use base64::Engine as _;

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(ALPHA_WEBM_BASE64)
        .unwrap();
    let sandbox = Sandbox::new("webm-alpha");
    let path = sandbox.0.join("fixture.webm");
    fs::write(&path, bytes).unwrap();
    let (owner, events) = StickerOwner::spawn(StickerSource::Webm(path), 64).unwrap();
    let frame = match events.recv_timeout(Duration::from_secs(5)) {
        Ok(StickerEvent::Frame(frame)) => frame,
        Ok(StickerEvent::Failed(error)) => panic!("alpha VP9 has no thumbnail: {error:?}"),
        Err(error) => panic!("alpha VP9 did not finish: {error}"),
    };
    assert_eq!((frame.width(), frame.height()), (64, 64));
    assert_eq!(frame.bytes().len(), 64 * 64 * 4);
    match events.recv_timeout(Duration::from_secs(5)) {
        Ok(StickerEvent::Failed(StickerError::UnsupportedVp9)) => {}
        Ok(StickerEvent::Failed(error)) => panic!("unexpected alpha VP9 error: {error:?}"),
        Ok(StickerEvent::Frame(_)) => panic!("alpha VP9 continued after its thumbnail"),
        Err(error) => panic!("alpha VP9 did not stop: {error}"),
    }
    drop(owner);
}

fn encode_vp9_fixture(path: &Path) {
    let description = format!(
        "videotestsrc num-buffers=4 pattern=ball ! video/x-raw,width=64,height=64,framerate=10/1 ! vp9enc deadline=1 cpu-used=8 ! matroskamux ! filesink location={}",
        path.display()
    );
    let pipeline = gst::parse::launch(&description)
        .unwrap()
        .downcast::<gst::Pipeline>()
        .unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let bus = pipeline.bus().unwrap();
    let message = bus
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(10),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("VP9 encoder did not finish");
    assert!(matches!(message.view(), gst::MessageView::Eos(_)));
    pipeline.set_state(gst::State::Null).unwrap();
}
