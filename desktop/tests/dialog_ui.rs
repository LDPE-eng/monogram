#![allow(dead_code, unused_imports)]

#[path = "../src/client/mod.rs"]
mod client;
#[path = "../src/ui/dialog.rs"]
pub mod dialog;
#[path = "../src/domain/mod.rs"]
mod domain;
#[path = "../src/i18n.rs"]
mod i18n;
#[path = "../src/ui/interactions.rs"]
pub mod interactions;
#[path = "../src/ui/markup.rs"]
pub mod markup;
#[path = "../src/media/mod.rs"]
pub mod media_backend;
#[path = "../src/media/sticker.rs"]
pub mod media_sticker;
#[path = "../src/ui/appearance.rs"]
pub mod ui_appearance;
mod media {
    pub use crate::media_backend::{cache, playback, video};
    pub use crate::media_sticker as sticker;
}
#[path = "../src/ui/media.rs"]
pub mod ui_media;
#[path = "../src/ui/sticker.rs"]
pub mod ui_sticker;
#[path = "../src/ui/web.rs"]
pub mod ui_web;
mod ui {
    pub use crate::dialog;
    pub use crate::interactions;
    pub use crate::markup;
    pub use crate::ui_appearance as appearance;
    pub use crate::ui_media as media;
    pub use crate::ui_sticker as sticker;
    pub use crate::ui_web as web;
}
#[path = "../src/platform/mpris.rs"]
pub mod mpris;
#[path = "../src/platform/network.rs"]
pub mod network;
#[path = "../src/platform/mod.rs"]
pub mod platform_base;
mod platform {
    pub use crate::platform_base::*;
}
#[path = "../src/storage/mod.rs"]
mod storage;
