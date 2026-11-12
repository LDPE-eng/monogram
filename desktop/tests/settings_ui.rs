#![allow(dead_code, unused_imports)]

#[path = "../src/ui/appearance.rs"]
pub(crate) mod appearance;
#[path = "../src/media/cache.rs"]
pub mod cache_impl;
#[path = "../src/client/mod.rs"]
mod client;
#[path = "../src/ui/connectivity.rs"]
pub(crate) mod connectivity;
#[path = "../src/domain/mod.rs"]
mod domain;
#[path = "../src/i18n.rs"]
mod i18n;
#[path = "../src/ui/maintenance.rs"]
mod maintenance;
mod media {
    pub use super::cache_impl as cache;
}
#[path = "../src/platform/mod.rs"]
mod platform_impl;
mod platform {
    pub use super::platform_impl::*;
}
#[path = "../src/ui/settings.rs"]
mod settings;
#[path = "../src/storage/mod.rs"]
mod storage;
