use super::super::cache::{Acquire, CacheKey, CacheLimits, MediaCache};
use super::*;
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

fn wav() -> Vec<u8> {
    let samples = 8_000_u32 * 3;
    let size = samples * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&8_000_u32.to_le_bytes());
    bytes.extend_from_slice(&16_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.resize(44 + size as usize, 0);
    bytes
}

fn track(name: &str, cache: &MediaCache) -> Track {
    let bytes = wav();
    let mut download = match cache
        .acquire(CacheKey::new(1, name).unwrap(), Some(bytes.len() as u64))
        .unwrap()
    {
        Acquire::Download(download) => download,
        _ => panic!(),
    };
    download.append(0, &bytes).unwrap();
    Track {
        id: name.to_owned(),
        source: MediaSource::Cached(download.finish().unwrap()),
    }
}

fn fixture() -> (Sandbox, MediaCache, PlaybackOwner, Receiver<PlaybackEvent>) {
    gst::init().unwrap();
    let sandbox = Sandbox(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!(
                "playback-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
    );
    let cache = MediaCache::open(&sandbox.0, CacheLimits::default()).unwrap();
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", true)
        .build()
        .unwrap();
    let (owner, events) = PlaybackOwner::spawn(None, Some(sink)).unwrap();
    (sandbox, cache, owner, events)
}

fn wait(
    events: &Receiver<PlaybackEvent>,
    predicate: impl Fn(&PlaybackEvent) -> bool,
) -> PlaybackEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let event = events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("timed out waiting for GStreamer event");
        assert!(
            !matches!(event, PlaybackEvent::Error(_)),
            "unexpected playback error: {event:?}"
        );
        if predicate(&event) {
            return event;
        }
    }
}

#[test]
fn local_wav_play_pause_seek_speed_mute_and_stop() {
    let (_sandbox, cache, owner, events) = fixture();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner.send(PlaybackCommand::Speed(1.5)).unwrap();
    wait(
        &events,
        |event| matches!(event, PlaybackEvent::Speed(rate) if *rate == 1.5),
    );
    owner
        .send(PlaybackCommand::Seek { milliseconds: 500 })
        .unwrap();
    owner.send(PlaybackCommand::Mute(true)).unwrap();
    wait(&events, |event| matches!(event, PlaybackEvent::Muted(true)));
    owner.send(PlaybackCommand::Play).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Playing))
    });
    owner.send(PlaybackCommand::Pause).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner.send(PlaybackCommand::QueryPosition).unwrap();
    let event = wait(&events, |event| {
        matches!(event, PlaybackEvent::Position { .. })
    });
    if let PlaybackEvent::Position {
        milliseconds,
        duration_ms,
    } = event
    {
        assert!(
            (450..1500).contains(&milliseconds),
            "position: {milliseconds}"
        );
        assert_eq!(duration_ms, Some(3000));
    }
    owner.send(PlaybackCommand::Stop).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Stopped))
    });
}

#[test]
fn playlist_next_previous_loop_and_release_cache_pins() {
    let (_sandbox, cache, owner, events) = fixture();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache), track("two", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 0, .. })
    });
    owner.send(PlaybackCommand::Next).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 1, .. })
    });
    owner.send(PlaybackCommand::Previous).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 0, .. })
    });
    owner
        .send(PlaybackCommand::Loop(LoopMode::Playlist))
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::Loop(LoopMode::Playlist))
    });
    owner.send(PlaybackCommand::Previous).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 0, .. })
    });
    owner.send(PlaybackCommand::Next).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 1, .. })
    });
    owner.send(PlaybackCommand::Next).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 0, .. })
    });
    assert_eq!(cache.usage().unwrap().pinned_entries, 2);
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Stopped))
    });
    drop(owner);
    assert_eq!(cache.usage().unwrap().pinned_entries, 0);
}

#[test]
fn eos_advances_and_reports_end_of_queue() {
    let (_sandbox, cache, owner, events) = fixture();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner
        .send(PlaybackCommand::Seek { milliseconds: 2950 })
        .unwrap();
    owner.send(PlaybackCommand::Play).unwrap();
    wait(&events, |event| matches!(event, PlaybackEvent::EndOfQueue));
}

#[test]
fn track_loop_restarts_on_real_eos() {
    let (_sandbox, cache, owner, events) = fixture();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner.send(PlaybackCommand::Loop(LoopMode::Track)).unwrap();
    owner
        .send(PlaybackCommand::Seek { milliseconds: 2950 })
        .unwrap();
    owner.send(PlaybackCommand::Play).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Playing))
    });
    std::thread::sleep(Duration::from_millis(250));
    owner.send(PlaybackCommand::QueryPosition).unwrap();
    let event = wait(
        &events,
        |event| matches!(event, PlaybackEvent::Position { milliseconds, .. } if *milliseconds < 1000),
    );
    assert!(matches!(event, PlaybackEvent::Position { .. }));
}

#[test]
fn eos_advances_to_the_next_track() {
    let (_sandbox, cache, owner, events) = fixture();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache), track("two", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner
        .send(PlaybackCommand::Seek { milliseconds: 2950 })
        .unwrap();
    owner.send(PlaybackCommand::Play).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::TrackChanged { index: 1, .. })
    });
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Playing))
    });
}

#[test]
fn speed_is_applied_after_preroll_and_replay() {
    let (_sandbox, cache, owner, events) = fixture();
    owner.send(PlaybackCommand::Speed(4.0)).unwrap();
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![track("one", &cache)],
            start: 0,
            autoplay: false,
        })
        .unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Paused))
    });
    owner.send(PlaybackCommand::Next).unwrap();
    wait(&events, |event| matches!(event, PlaybackEvent::EndOfQueue));
    owner.send(PlaybackCommand::Play).unwrap();
    wait(&events, |event| {
        matches!(event, PlaybackEvent::State(PlaybackState::Playing))
    });
    std::thread::sleep(Duration::from_millis(150));
    owner.send(PlaybackCommand::QueryPosition).unwrap();
    let event = wait(&events, |event| {
        matches!(event, PlaybackEvent::Position { .. })
    });
    if let PlaybackEvent::Position { milliseconds, .. } = event {
        assert!(
            milliseconds >= 350,
            "4x playback was not retained: {milliseconds}"
        );
    }
}

#[test]
fn invalid_speed_and_queue_are_typed_errors() {
    let (_sandbox, _cache, owner, events) = fixture();
    owner.send(PlaybackCommand::Speed(f64::NAN)).unwrap();
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        PlaybackEvent::Error(PlaybackError::InvalidSpeed)
    ));
    owner
        .send(PlaybackCommand::SetQueue {
            tracks: vec![Track {
                id: "bad".into(),
                source: MediaSource::Uri("https://example.invalid/not-fetched".into()),
            }],
            start: 1,
            autoplay: false,
        })
        .unwrap();
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        PlaybackEvent::Error(PlaybackError::InvalidQueue)
    ));
}
