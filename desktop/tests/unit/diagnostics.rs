use std::fs;

use super::*;

#[test]
fn sanitize_redacts_long_hex_and_phone_numbers_without_breaking_utf8() {
    let input = "hash=0123456789abcdef0123456789abcdef call +79991234567 and ünïcode ✓";
    let output = sanitize(input);
    assert!(output.contains("<hex>"));
    assert!(output.contains("<phone>"));
    assert!(output.contains("ünïcode ✓"));
    assert!(!output.contains("0123456789abcdef"));
    assert!(!output.contains("79991234567"));
}

#[test]
fn sanitize_keeps_short_hex_and_short_numbers() {
    let input = "id 42, small 0x1a2b, seven 1234567, twelve 123456789012";
    let output = sanitize(input);
    assert_eq!(output, input);
}

#[test]
fn buffer_evicts_oldest_entry_once_byte_budget_is_exceeded() {
    let mut buffer = LogBuffer::default();
    for index in 0..160 {
        buffer.record(
            LogLevel::Info,
            &format!("entry {index} {}", "x".repeat(4096)),
        );
    }
    assert!(buffer.stats().bytes <= MAX_LOG_BYTES);
    assert!(buffer.stats().entries < 160);
    let first = buffer
        .search("", None)
        .first()
        .expect("one entry")
        .message
        .clone();
    assert!(
        first.starts_with("entry "),
        "oldest entries are dropped first"
    );
}

#[test]
fn entries_are_truncated_to_a_single_entry_budget() {
    let mut buffer = LogBuffer::default();
    buffer.record(LogLevel::Info, &"y".repeat(MAX_ENTRY_CHARS * 2));
    let entry = buffer.search("", None).pop().expect("one entry");
    assert_eq!(entry.message.chars().count(), MAX_ENTRY_CHARS);
}

#[test]
fn search_is_case_insensitive_and_filter_restricts_level() {
    let mut buffer = LogBuffer::default();
    buffer.record(LogLevel::Info, "Media cache resized");
    buffer.record(LogLevel::Error, "Media download failed");
    assert_eq!(buffer.search("MEDIA", None).len(), 2);
    assert_eq!(buffer.search("media", Some(LogLevel::Error)).len(), 1);
    assert_eq!(buffer.search("missing", None).len(), 0);
}

#[test]
fn clear_and_export_round_trip() {
    let mut buffer = LogBuffer::default();
    buffer.record(LogLevel::Warn, "before clear");
    buffer.clear();
    assert!(buffer.search("", None).is_empty());
    assert_eq!(buffer.export(), "");
    buffer.record(LogLevel::Info, "after clear");
    assert!(buffer.export().contains("after clear"));
}

#[test]
fn export_logs_writes_bounded_text_into_private_directory() {
    let directory = std::env::temp_dir().join(format!(
        "monogram-diagnostics-{}-{}",
        std::process::id(),
        unix_seconds()
    ));
    LogStore::global().record(LogLevel::Info, "export probe");
    let path = export_logs(&directory).expect("export succeeds");
    let contents = fs::read_to_string(&path).expect("readable export");
    assert!(contents.contains("export probe"));
    fs::remove_dir_all(&directory).ok();
}

#[test]
fn debug_stats_reports_version_profile_and_log_counts() {
    let stats = collect_debug_stats();
    assert_eq!(stats.app_version, env!("CARGO_PKG_VERSION"));
    assert!(matches!(stats.build_profile, "debug" | "release"));
    assert_eq!(stats.log.entries, LogStore::global().stats().entries);
    assert_eq!(stats.log.bytes, LogStore::global().stats().bytes);
}
