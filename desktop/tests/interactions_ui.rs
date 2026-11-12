#![allow(dead_code, unused_imports)]

#[path = "../src/media/cache.rs"]
pub mod cache_impl;
#[path = "../src/client/mod.rs"]
mod client;
#[path = "../src/domain/mod.rs"]
mod domain;
#[path = "../src/i18n.rs"]
mod i18n;
mod media {
    pub use super::cache_impl as cache;
}
#[path = "../src/ui/interactions.rs"]
mod interactions;
#[path = "../src/platform/mod.rs"]
mod platform_impl;
mod platform {
    pub use super::platform_impl::*;
}
#[path = "../src/storage/mod.rs"]
mod storage;
