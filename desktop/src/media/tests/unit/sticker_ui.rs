use super::*;
use crate::media::sticker::StickerSource;
use relm4::gtk::gio::prelude::InputStreamExt;
use std::{fs, path::PathBuf, time::Instant};

const SAMPLE: &[u8] = include_bytes!(
    "../../../../../vendor/tlottie/examples/android/app/src/main/assets/sample.json"
);
const ALPHA_WEBM_BASE64: &str = "GkXfo59ChoEBQveBAULygQRC84EIQoKEd2VibUKHgQJChYECGFOAZwEAAAAAAAL0EU2bdLpNu4tTq4QVSalmU6yBoU27i1OrhBZUrmtTrIHYTbuMU6uEElTDZ1OsggEpTbuMU6uEHFO7a1OsggLe7AEAAAAAAABZAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAVSalmsirXsYMPQkBNgI1MYXZmNjIuMTIuMTAyV0GNTGF2ZjYyLjEyLjEwMkSJiEB5AAAAAAAAFlSua8yuAQAAAAAAAEPXgQFzxYj5lBmJPiz4qZyBACK1nIN1bmSIgQCGhVZfVlA5g4EBI+ODhAX14QDglLCBQLqBQJqBAlPAgQFVsIRVuYEBElTDZ0CAc3OgY8CAZ8iaRaOHRU5DT0RFUkSHjUxhdmY2Mi4xMi4xMDJzc9pjwItjxYj5lBmJPiz4qWfIpUWjh0VOQ09ERVJEh5hMYXZjNjIuMjguMTAyIGxpYnZweC12cDlnyKFFo4hEVVJBVElPTkSHkzAwOjAwOjAwLjQwMDAwMDAwMAAfQ7Z1QSnngQCgQIqh5oEAAACCSYNCAAPwA/YGOCQcGEoAACBAAGtD//+njQAbd7H6hFVixmzJ/roJl4u6EX1slqAufcP/+8ZhOxwcuUE3weoGzJ/roJl4u6EX1slqAufbd/9K6////+Yav/////Udg56pAHWhn6ad7oEBpZiCSYNCAAPwA/YGOCQcGEoAACBAAHKO0UCgsaGTgQBkAIYAQJKcSFAAAANwAABUcHWhlqaU7oEBpY+GAECSnEhQAAADcAAAVHD7gZygsaGTgQDIAIYAQJKcQE7gAANwAABUcHWhlqaU7oEBpY+GAECSnEBO4AADcAAAVHD7gZygsaGTgQEsAIYAQJKcSFAAAANwAABUcHWhlqaU7oEBpY+GAECSnEhQAAADcAAAVHD7gZwcU7trkbuPs4EAt4r3gQHxggGv8IED";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!("sticker-ui-alpha-{}", std::process::id()));
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
    let input = gtk::gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes.to_vec()));
    let compressor = gtk::gio::ZlibCompressor::new(gtk::gio::ZlibCompressorFormat::Gzip, -1);
    let stream = gtk::gio::ConverterInputStream::new(&input, &compressor);
    let mut output = Vec::new();
    loop {
        let chunk = stream
            .read_bytes(8192, None::<&gtk::gio::Cancellable>)
            .unwrap();
        if chunk.is_empty() {
            break;
        }
        output.extend_from_slice(&chunk);
    }
    output
}

fn fallback_texture() -> gtk::gdk::Texture {
    let bytes = glib::Bytes::from_static(&[255, 0, 0, 255]);
    gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4).upcast()
}

fn pump_until(condition: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(condition(), "GTK condition was not reached before timeout");
}

pub(crate) fn sticker_lifecycle_runs_on_one_gtk_thread() {
    let i18n = I18n::new("en").unwrap();
    let view = StickerView::with_max_dimension(&i18n, 64).unwrap();
    view.set_active(false);
    view.load(StickerSource::Tgs(gzip(SAMPLE)), Some(fallback_texture()));
    assert!(!view.is_playing());
    assert!(view.state.borrow().tick.is_none());
    assert!(view.widget().paintable().is_some());
    view.clear();

    view.set_active(true);
    view.load(StickerSource::Tgs(gzip(SAMPLE)), None);
    pump_until(|| view.widget().paintable().is_some());
    let texture = view
        .widget()
        .paintable()
        .expect("animated sticker texture")
        .downcast::<gtk::gdk::MemoryTexture>()
        .unwrap();
    assert_eq!((texture.width(), texture.height()), (64, 64));
    assert!(view.is_playing());
    let fallback = fallback_texture();
    let expected = fallback.clone().upcast::<gtk::gdk::Paintable>();
    view.set_fallback(Some(fallback));
    view.detach();
    assert!(!view.is_playing());
    assert!(view.state.borrow().tick.is_none());
    assert_eq!(view.widget().paintable().unwrap(), expected);
    view.clear();
    assert!(view.widget().paintable().is_none());
    assert!(view.state.borrow().last_frame.is_none());
    drop(view);

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(ALPHA_WEBM_BASE64)
        .unwrap();
    let sandbox = Sandbox::new();
    let path = sandbox.0.join("fixture.webm");
    fs::write(&path, bytes).unwrap();
    let alpha_view = StickerView::with_max_dimension(&i18n, 64).unwrap();
    alpha_view.load(StickerSource::Webm(path), None);
    pump_until(|| !alpha_view.is_playing() && alpha_view.widget().paintable().is_some());
    assert!(alpha_view.state.borrow().tick.is_none());
    assert!(alpha_view.state.borrow().last_frame.is_some());
    let texture = alpha_view
        .widget()
        .paintable()
        .expect("VP9 static thumbnail")
        .downcast::<gtk::gdk::MemoryTexture>()
        .unwrap();
    assert_eq!((texture.width(), texture.height()), (64, 64));
    alpha_view.clear();
    assert!(alpha_view.widget().paintable().is_none());
}
