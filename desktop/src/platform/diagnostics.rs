use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_LOG_ENTRIES: usize = 4096;
pub const MAX_LOG_BYTES: usize = 512 * 1024;
pub const MAX_ENTRY_CHARS: usize = 4096;
const ENTRY_OVERHEAD: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp_seconds: u64,
    pub level: LogLevel,
    pub message: String,
}

impl LogEntry {
    fn rendered_len(&self) -> usize {
        ENTRY_OVERHEAD + self.message.len()
    }

    pub fn render(&self) -> String {
        format!(
            "{} {} {}",
            self.timestamp_seconds,
            self.level.as_str(),
            self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogStats {
    pub entries: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugStats {
    pub app_version: String,
    pub build_profile: &'static str,
    pub target_arch: &'static str,
    pub target_os: &'static str,
    pub log: LogStats,
}

impl Default for DebugStats {
    fn default() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            target_arch: std::env::consts::ARCH,
            target_os: std::env::consts::OS,
            log: LogStats::default(),
        }
    }
}

/// Sanitizes a log message before it enters the ring buffer. The patterns below
/// cover the credential shapes the client can produce; they are not a substitute
/// for callers not logging secrets in the first place.
pub fn sanitize(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut output = String::with_capacity(message.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'+' {
            let mut end = index + 1;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if (8..=15).contains(&(end - index - 1)) {
                output.push_str("<phone>");
                index = end;
                continue;
            }
        }
        if bytes[index].is_ascii_hexdigit() {
            let mut end = index;
            while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
                end += 1;
            }
            if end - index >= 32 {
                output.push_str("<hex>");
                index = end;
                continue;
            }
        }
        let character = message[index..]
            .chars()
            .next()
            .expect("index is on a UTF-8 boundary");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

#[derive(Debug, Default)]
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    bytes: usize,
}

impl LogBuffer {
    pub fn record(&mut self, level: LogLevel, message: &str) {
        let message = sanitize(message);
        let message: String = message.chars().take(MAX_ENTRY_CHARS).collect();
        let timestamp_seconds = unix_seconds();
        let entry = LogEntry {
            timestamp_seconds,
            level,
            message,
        };
        self.bytes = self.bytes.saturating_add(entry.rendered_len());
        self.entries.push_back(entry);
        self.evict();
    }

    pub fn search(&self, query: &str, level: Option<LogLevel>) -> Vec<LogEntry> {
        let query = query.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| level.is_none_or(|selected| entry.level == selected))
            .filter(|entry| query.is_empty() || entry.message.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn export(&self) -> String {
        let mut output = String::new();
        for entry in &self.entries {
            output.push_str(&entry.render());
            output.push('\n');
        }
        output
    }

    pub fn stats(&self) -> LogStats {
        LogStats {
            entries: self.entries.len(),
            bytes: self.bytes,
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > MAX_LOG_ENTRIES
            || (self.bytes > MAX_LOG_BYTES && self.entries.len() > 1)
        {
            if let Some(entry) = self.entries.pop_front() {
                self.bytes = self.bytes.saturating_sub(entry.rendered_len());
            }
        }
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub struct LogStore {
    inner: Mutex<LogBuffer>,
}

static LOG_STORE: OnceLock<LogStore> = OnceLock::new();

impl LogStore {
    pub fn global() -> &'static Self {
        LOG_STORE.get_or_init(|| Self {
            inner: Mutex::new(LogBuffer::default()),
        })
    }

    pub fn record(&self, level: LogLevel, message: &str) {
        if let Ok(mut buffer) = self.inner.lock() {
            buffer.record(level, message);
        }
    }

    pub fn search(&self, query: &str, level: Option<LogLevel>) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|buffer| buffer.search(query, level))
            .unwrap_or_default()
    }

    pub fn clear(&self) {
        if let Ok(mut buffer) = self.inner.lock() {
            buffer.clear();
        }
    }

    pub fn export(&self) -> String {
        self.inner
            .lock()
            .map(|buffer| buffer.export())
            .unwrap_or_default()
    }

    pub fn stats(&self) -> LogStats {
        self.inner
            .lock()
            .map(|buffer| buffer.stats())
            .unwrap_or_default()
    }
}

pub fn collect_debug_stats() -> DebugStats {
    DebugStats {
        log: LogStore::global().stats(),
        ..DebugStats::default()
    }
}

pub fn export_logs(directory: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("monogram-debug-{}.log", unix_seconds()));
    fs::write(&path, LogStore::global().export())?;
    Ok(path)
}

#[cfg(test)]
#[path = "../../tests/unit/diagnostics.rs"]
mod tests;
