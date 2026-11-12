use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey(String);

impl CacheKey {
    pub fn new(account: i64, remote_key: &str) -> io::Result<Self> {
        if remote_key.is_empty() || remote_key.len() > 100 {
            return Err(invalid("media cache key must contain 1..=100 bytes"));
        }
        let mut name = format!("{:016x}-", account as u64);
        for byte in remote_key.bytes() {
            use std::fmt::Write;
            write!(name, "{byte:02x}").expect("formatting a string");
        }
        Ok(Self(name))
    }

    pub fn for_message(
        account: i64,
        message: &mono_proto_client::MessageDto,
    ) -> io::Result<Option<Self>> {
        message
            .media_cache_key
            .as_deref()
            .map(|key| Self::new(account, key))
            .transpose()
    }

    fn from_filename(name: &str) -> Option<Self> {
        let (account, key) = name.split_once('-')?;
        (account.len() == 16
            && !key.is_empty()
            && key.len() <= 200
            && key.len().is_multiple_of(2)
            && account
                .bytes()
                .chain(key.bytes())
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
        .then(|| Self(name.to_owned()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CacheLimits {
    pub total_bytes: u64,
    pub file_bytes: u64,
    pub partial_max_age: Duration,
}

impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            total_bytes: 2 * 1024 * 1024 * 1024,
            file_bytes: 512 * 1024 * 1024,
            partial_max_age: Duration::from_secs(7 * 24 * 60 * 60),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub complete_bytes: u64,
    pub partial_bytes: u64,
    pub pinned_entries: usize,
}

struct Entry {
    bytes: u64,
    complete: bool,
    last_used: SystemTime,
    readers: usize,
    writing: bool,
}

struct Inner {
    root: PathBuf,
    limits: CacheLimits,
    entries: HashMap<CacheKey, Entry>,
    _lock: File,
}

#[derive(Clone)]
pub struct MediaCache(Arc<Mutex<Inner>>);

pub enum Acquire {
    Cached(CacheLease),
    Download(DownloadLease),
    Busy,
}

pub struct CacheLease {
    cache: MediaCache,
    key: CacheKey,
    path: PathBuf,
    file: File,
}

pub struct DownloadLease {
    cache: MediaCache,
    key: CacheKey,
    expected_bytes: Option<u64>,
    active: bool,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl MediaCache {
    pub fn discover(limits: CacheLimits) -> io::Result<Self> {
        let dirs = xdg::BaseDirectories::with_prefix("monogram");
        Self::open(dirs.create_cache_directory("media")?, limits)
    }

    pub fn open(root: impl AsRef<Path>, limits: CacheLimits) -> io::Result<Self> {
        if limits.total_bytes == 0
            || limits.file_bytes == 0
            || limits.file_bytes > limits.total_bytes
        {
            return Err(invalid("invalid media cache limits"));
        }
        let root = root.as_ref().to_path_buf();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)?;
        let root = fs::canonicalize(root)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(root.join("owner.lock"))?;
        lock.try_lock().map_err(io::Error::other)?;
        let mut inner = Inner {
            root,
            limits,
            entries: HashMap::new(),
            _lock: lock,
        };
        for child in fs::read_dir(&inner.root)? {
            let child = child?;
            if !child.file_type()?.is_file() {
                continue;
            }
            let path = child.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(key) = CacheKey::from_filename(stem) else {
                continue;
            };
            let complete = match path.extension().and_then(|s| s.to_str()) {
                Some("data") => true,
                Some("part") => false,
                _ => continue,
            };
            let metadata = child.metadata()?;
            if inner.entries.get(&key).is_some_and(|entry| entry.complete) && !complete {
                fs::remove_file(path)?;
                continue;
            }
            if complete && inner.entries.contains_key(&key) {
                fs::remove_file(inner.path(&key, false))?;
            }
            inner.entries.insert(
                key,
                Entry {
                    bytes: metadata.len(),
                    complete,
                    last_used: metadata.modified()?,
                    readers: 0,
                    writing: false,
                },
            );
        }
        inner.trim(SystemTime::now(), 0, None)?;
        Ok(Self(Arc::new(Mutex::new(inner))))
    }

    fn lock(&self) -> io::Result<MutexGuard<'_, Inner>> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("media cache owner poisoned"))
    }

    pub fn usage(&self) -> io::Result<CacheUsage> {
        let inner = self.lock()?;
        let mut usage = CacheUsage::default();
        for entry in inner.entries.values() {
            if entry.complete {
                usage.complete_bytes += entry.bytes;
            } else {
                usage.partial_bytes += entry.bytes;
            }
            usage.pinned_entries += usize::from(entry.readers > 0 || entry.writing);
        }
        Ok(usage)
    }

    pub fn trim(&self, now: SystemTime) -> io::Result<()> {
        self.lock()?.trim(now, 0, None)
    }

    pub fn clear(&self) -> io::Result<()> {
        let mut inner = self.lock()?;
        let keys: Vec<_> = inner
            .entries
            .iter()
            .filter(|(_, entry)| entry.readers == 0 && !entry.writing)
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            inner.remove(&key)?;
        }
        Ok(())
    }

    pub fn acquire(&self, key: CacheKey, expected_bytes: Option<u64>) -> io::Result<Acquire> {
        let mut inner = self.lock()?;
        if expected_bytes.is_some_and(|size| size > inner.limits.file_bytes) {
            return Err(invalid("media exceeds per-file cache limit"));
        }
        if let Some(entry) = inner.entries.get(&key) {
            if entry.writing {
                return Ok(Acquire::Busy);
            }
            if entry.complete {
                if expected_bytes.is_some_and(|size| size != entry.bytes) {
                    if entry.readers > 0 {
                        return Ok(Acquire::Busy);
                    }
                    inner.remove(&key)?;
                } else {
                    return inner.lease(self.clone(), key).map(Acquire::Cached);
                }
            }
        }
        if inner
            .entries
            .get(&key)
            .is_some_and(|entry| expected_bytes.is_some_and(|size| entry.bytes > size))
        {
            inner.remove(&key)?;
        }
        inner.trim(SystemTime::now(), 0, Some(&key))?;
        let path = inner.path(&key, false);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let bytes = file.metadata()?.len();
        inner.entries.insert(
            key.clone(),
            Entry {
                bytes,
                complete: false,
                last_used: SystemTime::now(),
                readers: 0,
                writing: true,
            },
        );
        Ok(Acquire::Download(DownloadLease {
            cache: self.clone(),
            key,
            expected_bytes,
            active: true,
        }))
    }
}

impl Inner {
    fn path(&self, key: &CacheKey, complete: bool) -> PathBuf {
        self.root.join(format!(
            "{}.{}",
            key.0,
            if complete { "data" } else { "part" }
        ))
    }

    fn remove(&mut self, key: &CacheKey) -> io::Result<()> {
        if let Some(entry) = self.entries.get(key) {
            match fs::remove_file(self.path(key, entry.complete)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.entries.remove(key);
        }
        Ok(())
    }

    fn trim(
        &mut self,
        now: SystemTime,
        incoming: u64,
        protected: Option<&CacheKey>,
    ) -> io::Result<()> {
        let expired: Vec<_> = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                Some(*key) != protected
                    && entry.readers == 0
                    && !entry.writing
                    && (entry.bytes > self.limits.file_bytes
                        || (!entry.complete
                            && now.duration_since(entry.last_used).unwrap_or_default()
                                > self.limits.partial_max_age))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.remove(&key)?;
        }
        loop {
            let total: u64 = self.entries.values().map(|entry| entry.bytes).sum();
            if total.saturating_add(incoming) <= self.limits.total_bytes {
                return Ok(());
            }
            let victim = self
                .entries
                .iter()
                .filter(|(key, entry)| {
                    Some(*key) != protected && entry.readers == 0 && !entry.writing
                })
                .min_by_key(|(key, entry)| (entry.last_used, key.0.clone()))
                .map(|(key, _)| key.clone());
            match victim {
                Some(key) => self.remove(&key)?,
                None => return Err(io::Error::other("cache capacity is pinned by active media")),
            }
        }
    }

    fn lease(&mut self, cache: MediaCache, key: CacheKey) -> io::Result<CacheLease> {
        let path = self.path(&key, true);
        let file = File::open(&path)?;
        let now = SystemTime::now();
        file.set_modified(now)?;
        let entry = self.entries.get_mut(&key).expect("existing complete entry");
        entry.last_used = now;
        entry.readers += 1;
        Ok(CacheLease {
            cache,
            key,
            path,
            file,
        })
    }
}

impl CacheLease {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn file(&mut self) -> &mut File {
        &mut self.file
    }
}

impl Drop for CacheLease {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.cache.lock()
            && let Some(entry) = inner.entries.get_mut(&self.key)
        {
            entry.readers = entry.readers.saturating_sub(1);
        }
    }
}

impl DownloadLease {
    pub fn offset(&self) -> io::Result<u64> {
        Ok(self.cache.lock()?.entries[&self.key].bytes)
    }

    pub fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let mut inner = self.cache.lock()?;
        let current = inner.entries[&self.key].bytes;
        if offset != current {
            return Err(invalid("non-contiguous media chunk"));
        }
        let next = current
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| invalid("media size overflow"))?;
        if next > inner.limits.file_bytes || self.expected_bytes.is_some_and(|size| next > size) {
            return Err(invalid("media chunk exceeds expected size or cache limit"));
        }
        inner.trim(SystemTime::now(), bytes.len() as u64, Some(&self.key))?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(inner.path(&self.key, false))?;
        let result = file.write_all(bytes);
        let entry = inner
            .entries
            .get_mut(&self.key)
            .expect("owned partial entry");
        entry.bytes = file.metadata()?.len();
        entry.last_used = SystemTime::now();
        result
    }

    pub fn finish(mut self) -> io::Result<CacheLease> {
        let mut inner = self.cache.lock()?;
        if self
            .expected_bytes
            .is_some_and(|size| size != inner.entries[&self.key].bytes)
        {
            return Err(invalid("incomplete media download"));
        }
        let part = inner.path(&self.key, false);
        File::open(&part)?.sync_all()?;
        fs::rename(part, inner.path(&self.key, true))?;
        let entry = inner
            .entries
            .get_mut(&self.key)
            .expect("owned partial entry");
        entry.complete = true;
        entry.writing = false;
        self.active = false;
        inner.lease(self.cache.clone(), self.key.clone())
    }

    pub fn cancel(mut self, keep_partial: bool) -> io::Result<()> {
        let mut inner = self.cache.lock()?;
        if keep_partial {
            inner
                .entries
                .get_mut(&self.key)
                .expect("owned partial entry")
                .writing = false;
        } else {
            inner.remove(&self.key)?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for DownloadLease {
    fn drop(&mut self) {
        if self.active
            && let Ok(mut inner) = self.cache.lock()
            && let Some(entry) = inner.entries.get_mut(&self.key)
        {
            entry.writing = false;
        }
    }
}

#[cfg(test)]
#[path = "tests/unit/cache.rs"]
mod tests;
