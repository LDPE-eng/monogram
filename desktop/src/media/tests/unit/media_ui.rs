use super::*;
use crate::media::playback::MediaSource;
use gstreamer::{self as gst};
use relm4::{Component, ComponentController};
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

fn write_video(path: &std::path::Path) {
    let mut bytes = b"YUV4MPEG2 C420jpeg W16 H16 Ip F1:1 A1:1\n".to_vec();
    for _ in 0..2 {
        bytes.extend_from_slice(b"FRAME\n");
        bytes.extend_from_slice(&[81; 256]);
        bytes.extend_from_slice(&[90; 64]);
        bytes.extend_from_slice(&[240; 64]);
    }
    fs::write(path, bytes).unwrap();
}

fn wait(
    context: &glib::MainContext,
    player: &impl ComponentController<MediaPlayer>,
    predicate: impl Fn(&MediaPlayer) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        if predicate(&player.model()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for media UI state");
}

pub(crate) fn mini_player_and_viewer_control_real_local_playback() {
    let sandbox = Sandbox(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!(
                "media-ui-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
    );
    fs::create_dir_all(&sandbox.0).unwrap();
    let path = sandbox.0.join("media.y4m");
    write_video(&path);
    let uri = gst::glib::filename_to_uri(&path, None).unwrap().to_string();
    let player = MediaPlayer::builder().launch(Rc::new(I18n::new("en").unwrap()));
    let context = glib::MainContext::default();
    assert!(player.model().mpris.is_some(), "MPRIS registration failed");
    assert!(!player.widget().is_visible());
    player.emit(MediaInput::Play {
        tracks: vec![
            Track {
                id: "one".into(),
                source: MediaSource::Uri(uri.clone()),
            },
            Track {
                id: "two".into(),
                source: MediaSource::Uri(uri),
            },
        ],
        start: 0,
        autoplay: false,
    });
    wait(&context, &player, |model| {
        model.snapshot.track_id.as_deref() == Some("one")
            && model.snapshot.state == PlaybackState::Paused
    });
    assert!(player.widget().is_visible());
    let parent = adw::Window::new();
    player.emit(MediaInput::OpenViewer(parent.upcast::<gtk::Window>()));
    wait(&context, &player, |model| {
        model
            .viewer
            .as_ref()
            .is_some_and(|viewer| viewer.window.is_visible())
    });
    let viewer_window = player.model().viewer.as_ref().unwrap().window.clone();
    wait(&context, &player, |model| {
        model
            .video
            .as_ref()
            .is_some_and(|video| video.widget().paintable().is_some())
    });
    player.model().controls.speed.set_value(2.0);
    wait(&context, &player, |model| model.snapshot.speed == 2.0);
    player.model().controls.mute.emit_clicked();
    wait(&context, &player, |model| model.snapshot.muted);
    player.model().controls.repeat.emit_clicked();
    wait(&context, &player, |model| {
        model.snapshot.loop_mode == LoopMode::Track
    });
    player.model().controls.next.emit_clicked();
    wait(&context, &player, |model| {
        model.snapshot.track_id.as_deref() == Some("two")
    });
    player.model().controls.previous.emit_clicked();
    wait(&context, &player, |model| {
        model.snapshot.track_id.as_deref() == Some("one")
    });
    player.model().controls.play.emit_clicked();
    wait(&context, &player, |model| {
        model.snapshot.state == PlaybackState::Playing
    });
    player.model().controls.play.emit_clicked();
    wait(&context, &player, |model| {
        model.snapshot.state == PlaybackState::Paused
    });
    player.emit(MediaInput::Seek(500));
    wait(&context, &player, |model| model.snapshot.position_ms >= 450);
    player.emit(MediaInput::CloseViewer);
    wait(&context, &player, |model| model.viewer.is_none());
    assert!(!viewer_window.is_visible());
    player.emit(MediaInput::Close);
    wait(&context, &player, |model| model.snapshot.queue_len == 0);
    assert!(!player.widget().is_visible());
    wait(&context, &player, |model| {
        model.mpris.as_ref().is_none_or(|mpris| {
            mpris.inner.queue.borrow().is_empty() && !mpris.inner.scheduled.get()
        })
    });
    let mpris_weak = player
        .model()
        .mpris
        .as_ref()
        .map(|mpris| Rc::downgrade(&mpris.inner));
    let adapter_weak = player
        .model()
        .mpris
        .as_ref()
        .and_then(|mpris| mpris.inner.adapter.borrow().as_ref().map(Rc::downgrade));
    drop(player);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (mpris_weak
        .as_ref()
        .is_some_and(|weak| weak.upgrade().is_some())
        || adapter_weak
            .as_ref()
            .is_some_and(|weak| weak.upgrade().is_some()))
        && Instant::now() < deadline
    {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(mpris_weak.is_none_or(|weak| weak.upgrade().is_none()));
    assert!(adapter_weak.is_none_or(|weak| weak.upgrade().is_none()));
}

pub(crate) fn sticker_lifecycle() {
    crate::ui::sticker::tests::sticker_lifecycle_runs_on_one_gtk_thread();
}
