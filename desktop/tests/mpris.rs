mod media {
    #![allow(dead_code)]

    pub mod playback {
        use std::sync::mpsc::{self, Receiver, Sender};

        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub enum LoopMode {
            #[default]
            Off,
            Track,
            Playlist,
        }

        #[derive(Debug)]
        pub enum PlaybackCommand {
            Play,
            Pause,
            Stop,
            Seek { milliseconds: u64 },
            Speed(f64),
            Mute(bool),
            Loop(LoopMode),
            Next,
            Previous,
            QueryPosition,
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum PlaybackState {
            Stopped,
            Paused,
            Playing,
        }

        #[derive(Debug)]
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
            SeekFailed,
        }

        pub struct PlaybackOwner {
            commands: Sender<PlaybackCommand>,
        }

        impl PlaybackOwner {
            pub fn channel() -> (Self, Receiver<PlaybackCommand>) {
                let (commands, receiver) = mpsc::channel();
                (Self { commands }, receiver)
            }

            pub fn send(
                &self,
                command: PlaybackCommand,
            ) -> Result<(), mpsc::SendError<PlaybackCommand>> {
                self.commands.send(command)
            }
        }
    }
}

#[path = "../src/platform/mpris.rs"]
mod mpris;

use media::playback::{LoopMode, PlaybackCommand, PlaybackEvent, PlaybackOwner, PlaybackState};
use mpris::{MprisAdapter, MprisMetadata};
use mpris_server::{
    Time,
    zbus::{
        self, Connection, MatchRule, Message, MessageStream, OwnedObjectPath, OwnedVariant, Proxy,
        export::futures_core::Stream, fdo, message::Type as MessageType,
    },
};
use relm4::gtk::glib;
use std::{
    collections::HashMap,
    future::poll_fn,
    pin::Pin,
    process::Command,
    sync::{Arc, mpsc::TryRecvError},
    task::Poll,
    time::{Duration, Instant},
};

const CHILD: &str = "MONOGRAM_MPRIS_ISOLATED_CHILD";
const PARENT_ADDRESS: &str = "MONOGRAM_MPRIS_PARENT_ADDRESS";

#[test]
fn isolated_mpris_registration_and_signals() {
    if std::env::var_os(CHILD).is_some() {
        verify_isolated_mpris();
        return;
    }
    let output = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .arg("isolated_mpris_registration_and_signals")
        .arg("--exact")
        .arg("--nocapture")
        .env(CHILD, "1")
        .env(
            PARENT_ADDRESS,
            std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default(),
        )
        .output()
        .expect("dbus-run-session is required for the isolated MPRIS integration test");
    assert!(
        output.status.success(),
        "isolated MPRIS child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_isolated_mpris() {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
    let parent = std::env::var(PARENT_ADDRESS).expect("missing isolated-bus guard");
    assert!(!address.is_empty());
    assert_ne!(address, parent, "test must not use the user's session bus");

    let context = glib::MainContext::default();
    let (owner, commands) = PlaybackOwner::channel();
    let owner = Arc::new(owner);
    let adapter = MprisAdapter::new(Arc::clone(&owner)).unwrap();
    let bus_name = format!(
        "org.mpris.MediaPlayer2.monogram.instance{}",
        std::process::id()
    );

    context.block_on(async {
        let connection = Connection::session().await.unwrap();
        let bus = fdo::DBusProxy::new(&connection).await.unwrap();
        assert!(
            bus.name_has_owner(zbus::names::BusName::try_from(bus_name.as_str()).unwrap())
                .await
                .unwrap()
        );
        let player = zbus::proxy::Builder::<Proxy<'_>>::new(&connection)
            .destination(bus_name.as_str())
            .path("/org/mpris/MediaPlayer2")
            .interface("org.mpris.MediaPlayer2.Player")
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        assert!(!player.get_property::<bool>("CanGoNext").await.unwrap());
        assert!(!player.get_property::<bool>("CanGoPrevious").await.unwrap());

        let mut properties = signal_stream(
            &connection,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .await;
        let mut seeked =
            signal_stream(&connection, "org.mpris.MediaPlayer2.Player", "Seeked").await;

        adapter.update_queue(None, 3).await.unwrap();
        assert!(!player.get_property::<bool>("CanPlay").await.unwrap());
        adapter
            .handle_event(&PlaybackEvent::TrackChanged {
                id: "/home/test/private/source.mp3".into(),
                index: 0,
            })
            .await
            .unwrap();
        adapter
            .set_current_metadata(MprisMetadata {
                title: Some("Title".into()),
                artists: vec!["Artist".into()],
                album: Some("Album".into()),
                duration_ms: Some(3000),
            })
            .await
            .unwrap();
        adapter
            .handle_event(&PlaybackEvent::State(PlaybackState::Paused))
            .await
            .unwrap();

        let metadata = player
            .get_property::<HashMap<String, OwnedVariant>>("Metadata")
            .await
            .unwrap();
        assert!(!metadata.contains_key("xesam:url"));
        assert!(!metadata.contains_key("mpris:artUrl"));
        assert_eq!(
            String::try_from(metadata["xesam:title"].try_clone().unwrap()).unwrap(),
            "Title"
        );
        assert_eq!(
            Vec::<String>::try_from(metadata["xesam:artist"].try_clone().unwrap()).unwrap(),
            vec!["Artist"]
        );
        assert_eq!(
            String::try_from(metadata["xesam:album"].try_clone().unwrap()).unwrap(),
            "Album"
        );
        assert_eq!(
            i64::try_from(metadata["mpris:length"].try_clone().unwrap()).unwrap(),
            3_000_000
        );
        let track_id =
            OwnedObjectPath::try_from(metadata["mpris:trackid"].try_clone().unwrap()).unwrap();
        assert!(
            track_id
                .as_str()
                .starts_with("/io/github/monogram/desktop/track/")
        );
        assert!(!track_id.as_str().contains("private"));

        assert!(player.get_property::<bool>("CanGoNext").await.unwrap());
        assert!(!player.get_property::<bool>("CanGoPrevious").await.unwrap());
        adapter.update_queue(Some(1), 3).await.unwrap();
        assert!(player.get_property::<bool>("CanGoNext").await.unwrap());
        assert!(player.get_property::<bool>("CanGoPrevious").await.unwrap());
        adapter.update_queue(Some(2), 3).await.unwrap();
        assert!(!player.get_property::<bool>("CanGoNext").await.unwrap());
        assert!(player.get_property::<bool>("CanGoPrevious").await.unwrap());
        adapter
            .handle_event(&PlaybackEvent::Loop(LoopMode::Playlist))
            .await
            .unwrap();
        assert!(player.get_property::<bool>("CanGoNext").await.unwrap());
        adapter
            .handle_event(&PlaybackEvent::Loop(LoopMode::Off))
            .await
            .unwrap();
        assert!(!player.get_property::<bool>("CanGoNext").await.unwrap());

        wait_properties(
            &mut properties,
            &["Metadata", "CanGoNext", "CanGoPrevious", "PlaybackStatus"],
        )
        .await;

        adapter.update_queue(Some(0), 1).await.unwrap();
        let _: () = player
            .call("Seek", &(Time::from_millis(1000),))
            .await
            .unwrap();
        assert!(matches!(
            commands.try_recv().unwrap(),
            PlaybackCommand::Seek { milliseconds: 1000 }
        ));
        assert!(matches!(
            commands.try_recv().unwrap(),
            PlaybackCommand::QueryPosition
        ));
        adapter
            .handle_event(&PlaybackEvent::Position {
                milliseconds: 1000,
                duration_ms: Some(3000),
            })
            .await
            .unwrap();
        let message = next_message(&mut seeked).await;
        let (position,): (i64,) = message.body().deserialize().unwrap();
        assert_eq!(position, 1_000_000);

        adapter.clear().await.unwrap();
        assert!(!player.get_property::<bool>("CanGoNext").await.unwrap());
        assert!(!player.get_property::<bool>("CanGoPrevious").await.unwrap());
        assert!(!player.get_property::<bool>("CanPlay").await.unwrap());
        let metadata = player
            .get_property::<HashMap<String, OwnedVariant>>("Metadata")
            .await
            .unwrap();
        assert!(metadata.is_empty());

        while commands.try_recv().is_ok() {}
        adapter.shutdown();
        assert_eq!(Arc::strong_count(&owner), 1);
        drop(owner);
        assert!(matches!(
            commands.try_recv(),
            Err(TryRecvError::Disconnected)
        ));
        wait_name_released(&bus, &bus_name).await;
    });
}

async fn signal_stream(connection: &Connection, interface: &str, member: &str) -> MessageStream {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(interface)
        .member(member)
        .path("/org/mpris/MediaPlayer2")
        .build()
        .unwrap();
    MessageStream::for_match_rule(rule, connection, None)
        .await
        .unwrap()
}

async fn next_message(stream: &mut MessageStream) -> Message {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let poll = poll_fn(|cx| {
            let result = Pin::new(&mut *stream).poll_next(cx);
            Poll::Ready(result)
        })
        .await;
        if let Poll::Ready(Some(message)) = poll {
            return message.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for D-Bus signal"
        );
        glib::timeout_future(Duration::from_millis(10)).await;
    }
}

async fn wait_properties(stream: &mut MessageStream, properties: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut remaining = properties.to_vec();
    while !remaining.is_empty() {
        assert!(
            Instant::now() < deadline,
            "missing property signals: {remaining:?}"
        );
        let message = next_message(stream).await;
        let (_, changed, _): (String, HashMap<String, OwnedVariant>, Vec<String>) =
            message.body().deserialize().unwrap();
        remaining.retain(|property| !changed.contains_key(*property));
    }
}

async fn wait_name_released(bus: &fdo::DBusProxy<'_>, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while bus
        .name_has_owner(zbus::names::BusName::try_from(name).unwrap())
        .await
        .unwrap()
    {
        assert!(Instant::now() < deadline, "MPRIS name survived shutdown");
        glib::timeout_future(Duration::from_millis(10)).await;
    }
}
