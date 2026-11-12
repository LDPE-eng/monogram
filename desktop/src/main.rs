#![forbid(unsafe_code)]

pub mod client;
pub mod domain;
pub mod i18n;
pub mod media {
    pub mod cache;
    pub mod playback;
    pub mod sticker;
    pub mod video;
}
#[path = "platform/mpris.rs"]
pub mod mpris;
#[path = "platform/network.rs"]
pub mod network;
pub mod platform {
    include!("platform/mod.rs");
}
pub mod storage;
pub mod ui;
pub use ui::{appearance, connectivity};

fn main() {
    let app = relm4::RelmApp::new("io.github.monogram.desktop");
    app.run::<ui::App>(());
}
