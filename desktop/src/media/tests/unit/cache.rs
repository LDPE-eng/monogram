use super::*;
use std::{
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scratch")
            .join(format!(
                "cache-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        Self(root)
    }
    fn cache(&self, total: u64) -> MediaCache {
        MediaCache::open(
            &self.0,
            CacheLimits {
                total_bytes: total,
                file_bytes: total,
                partial_max_age: Duration::from_secs(60),
            },
        )
        .unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn key(name: &str) -> CacheKey {
    CacheKey::new(1, name).unwrap()
}
fn writer(cache: &MediaCache, name: &str, size: Option<u64>) -> DownloadLease {
    match cache.acquire(key(name), size).unwrap() {
        Acquire::Download(writer) => writer,
        _ => panic!("expected download lease"),
    }
}
fn put(cache: &MediaCache, name: &str, bytes: &[u8]) -> CacheLease {
    let mut writer = writer(cache, name, Some(bytes.len() as u64));
    writer.append(0, bytes).unwrap();
    writer.finish().unwrap()
}

#[test]
fn keys_are_account_scoped_and_cannot_escape_root() {
    assert_ne!(
        CacheKey::new(1, "../x").unwrap(),
        CacheKey::new(2, "../x").unwrap()
    );
    let key = CacheKey::new(-1, "../../☃").unwrap();
    assert!(!key.0.contains('/'));
    assert_eq!(CacheKey::from_filename(&key.0), Some(key));
    assert!(CacheKey::new(0, "").is_err());
    assert!(CacheKey::new(0, &"x".repeat(101)).is_err());
    assert!(CacheKey::from_filename("../../x").is_none());
}

#[test]
fn one_writer_and_multiple_pinned_readers() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(20);
    let mut download = writer(&cache, "file", Some(4));
    assert!(matches!(
        cache.acquire(key("file"), Some(4)).unwrap(),
        Acquire::Busy
    ));
    download.append(0, b"data").unwrap();
    let mut first = download.finish().unwrap();
    let second = match cache.acquire(key("file"), None).unwrap() {
        Acquire::Cached(lease) => lease,
        _ => panic!("expected cached data"),
    };
    cache.clear().unwrap();
    assert_eq!(cache.usage().unwrap().complete_bytes, 4);
    let mut bytes = Vec::new();
    first.file().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"data");
    drop(first);
    assert_eq!(cache.usage().unwrap().pinned_entries, 1);
    drop(second);
    cache.clear().unwrap();
    assert_eq!(cache.usage().unwrap(), CacheUsage::default());
}

#[test]
fn partial_resumes_after_owner_and_cache_restart() {
    let sandbox = Sandbox::new();
    {
        let cache = sandbox.cache(20);
        let mut download = writer(&cache, "file", Some(6));
        download.append(0, b"abc").unwrap();
        assert_eq!(cache.usage().unwrap().partial_bytes, 3);
        download.cancel(true).unwrap();
    }
    let cache = sandbox.cache(20);
    let mut download = writer(&cache, "file", Some(6));
    assert_eq!(download.offset().unwrap(), 3);
    assert!(download.append(0, b"bad").is_err());
    download.append(3, b"def").unwrap();
    let mut lease = download.finish().unwrap();
    let mut bytes = Vec::new();
    lease.file().read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"abcdef");
    assert_eq!(cache.usage().unwrap().partial_bytes, 0);
    assert_eq!(cache.usage().unwrap().complete_bytes, 6);
}

#[test]
fn incomplete_finish_keeps_resumable_partial() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(10);
    let mut download = writer(&cache, "file", Some(4));
    download.append(0, b"ab").unwrap();
    assert!(download.finish().is_err());
    let mut download = writer(&cache, "file", Some(4));
    assert_eq!(download.offset().unwrap(), 2);
    assert!(download.append(2, b"too big").is_err());
    download.append(2, b"cd").unwrap();
    assert!(download.finish().is_ok());
}

#[test]
fn cancellation_can_delete_partial() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(10);
    let mut download = writer(&cache, "file", None);
    download.append(0, b"ab").unwrap();
    download.cancel(false).unwrap();
    assert_eq!(cache.usage().unwrap().partial_bytes, 0);
    assert_eq!(writer(&cache, "file", None).offset().unwrap(), 0);
}

#[test]
fn byte_budget_evicts_lru_but_never_active_media() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(8);
    let pinned = put(&cache, "pinned", b"1234");
    drop(put(&cache, "old", b"abcd"));
    let mut download = writer(&cache, "new", Some(4));
    download.append(0, b"new!").unwrap();
    let newest = download.finish().unwrap();
    assert_eq!(cache.usage().unwrap().complete_bytes, 8);
    assert!(!cache.lock().unwrap().entries.contains_key(&key("old")));
    let mut blocked = writer(&cache, "blocked", Some(1));
    assert!(blocked.append(0, b"x").is_err());
    assert_eq!(blocked.offset().unwrap(), 0);
    drop(pinned);
    blocked.append(0, b"x").unwrap();
    assert_eq!(cache.usage().unwrap().complete_bytes, 4);
    drop(newest);
}

#[test]
fn access_refreshes_lru_order() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(6);
    drop(put(&cache, "a", b"aaa"));
    drop(put(&cache, "b", b"bbb"));
    drop(match cache.acquire(key("a"), None).unwrap() {
        Acquire::Cached(lease) => lease,
        _ => panic!(),
    });
    drop(put(&cache, "c", b"ccc"));
    let inner = cache.lock().unwrap();
    assert!(inner.entries.contains_key(&key("a")));
    assert!(!inner.entries.contains_key(&key("b")));
}

#[test]
fn partial_expiry_and_oversized_files_are_reconciled_on_open() {
    let sandbox = Sandbox::new();
    {
        let cache = sandbox.cache(20);
        let mut download = writer(&cache, "partial", None);
        download.append(0, b"abc").unwrap();
        drop(download);
        cache
            .trim(SystemTime::now() + Duration::from_secs(61))
            .unwrap();
        assert_eq!(cache.usage().unwrap().partial_bytes, 0);
        drop(put(&cache, "large", b"123456"));
    }
    let cache = sandbox.cache(4);
    assert_eq!(cache.usage().unwrap(), CacheUsage::default());
}

#[test]
fn cache_directory_has_one_process_owner() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(20);
    assert!(MediaCache::open(&sandbox.0, CacheLimits::default()).is_err());
    drop(cache);
    assert!(MediaCache::open(&sandbox.0, CacheLimits::default()).is_ok());
}

#[test]
fn pinned_size_mismatch_is_busy_then_replaced() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(20);
    let lease = put(&cache, "file", b"old");
    assert!(matches!(
        cache.acquire(key("file"), Some(4)).unwrap(),
        Acquire::Busy
    ));
    drop(lease);
    let download = writer(&cache, "file", Some(4));
    assert_eq!(download.offset().unwrap(), 0);
}

#[test]
fn recovery_prefers_complete_file_over_leftover_partial() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(20);
    let lease = put(&cache, "file", b"complete");
    let complete = lease.path().to_path_buf();
    drop(lease);
    fs::write(complete.with_extension("part"), b"stale").unwrap();
    drop(cache);
    let cache = sandbox.cache(20);
    assert_eq!(cache.usage().unwrap().complete_bytes, 8);
    assert_eq!(cache.usage().unwrap().partial_bytes, 0);
    assert!(!complete.with_extension("part").exists());
}

#[test]
fn active_partial_cannot_expire_or_be_cleared() {
    let sandbox = Sandbox::new();
    let cache = sandbox.cache(20);
    let mut download = writer(&cache, "file", Some(4));
    download.append(0, b"ab").unwrap();
    cache
        .trim(SystemTime::now() + Duration::from_secs(100))
        .unwrap();
    cache.clear().unwrap();
    assert_eq!(cache.usage().unwrap().partial_bytes, 2);
    drop(download);
    cache
        .trim(SystemTime::now() + Duration::from_secs(100))
        .unwrap();
    assert_eq!(cache.usage().unwrap().partial_bytes, 0);
}
