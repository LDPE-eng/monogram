use super::*;
use crate::media::playback::{MediaSource, PlaybackCommand, PlaybackEvent, PlaybackOwner, Track};
use relm4::gtk::gdk::prelude::TextureExt;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn yuv4mpeg() -> Vec<u8> {
    let mut bytes = b"YUV4MPEG2 C420jpeg W16 H16 Ip F1:1 A1:1\n".to_vec();
    for (y, u, v) in [(81, 90, 240), (145, 54, 34)] {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&[y; 256]);
        bytes.extend_from_slice(&[u; 64]);
        bytes.extend_from_slice(&[v; 64]);
    }
    bytes
}

#[test]
fn local_video_file_reaches_gtk_paintable_and_drops_cleanly() {
    relm4::adw::init().expect("GTK test display");
    gst::init().unwrap();
    let sandbox = Sandbox(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!(
                "video-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
    );
    fs::create_dir_all(&sandbox.0).unwrap();
    let path = sandbox.0.join("frame.y4m");
    fs::write(&path, yuv4mpeg()).unwrap();
    let surface = GtkVideoSink::new().unwrap();
    let audio = gst::ElementFactory::make("fakesink")
        .property("sync", true)
        .build()
        .unwrap();
    let (owner, events) = PlaybackOwner::spawn(Some(surface.element()), Some(audio)).unwrap();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![Track {
                id: "local-video".into(),
                source: MediaSource::Uri(
                    gst::glib::filename_to_uri(&path, None).unwrap().to_string(),
                ),
            }],
            start: 0,
            autoplay: true,
        })
        .unwrap();
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while surface.widget().paintable().is_none() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        if let Ok(PlaybackEvent::Error(error)) = events.try_recv() {
            panic!("local video playback failed: {error:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let texture = surface
        .widget()
        .paintable()
        .expect("decoded local video frame")
        .downcast::<gtk::gdk::MemoryTexture>()
        .expect("memory texture paintable");
    assert_eq!(texture.width(), 16);
    assert_eq!(texture.height(), 16);
    drop(owner);
    drop(surface);
    while context.pending() {
        context.iteration(false);
    }
    crate::ui::media::tests::mini_player_and_viewer_control_real_local_playback();
    crate::ui::media::tests::sticker_lifecycle();
    crate::ui::interactions::tests::component_builds_real_gtk_panels_and_updates_without_network();
    crate::ui::web::tests::instant_view_scenario();
    crate::ui::appearance::tests::wallpaper_scenario();
    crate::ui::dialog::tests::message_body_labels_scenario();
    crate::ui::dialog::tests::wallpaper_dialog_scenario();
}
