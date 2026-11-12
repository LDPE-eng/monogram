use std::cmp::Ordering;

use super::*;

#[test]
fn rpm_version_comparison_covers_standard_orderings() {
    let cases: &[(&str, &str, Ordering)] = &[
        ("1.0", "1.0", Ordering::Equal),
        ("1.0", "1.1", Ordering::Less),
        ("1.10", "1.9", Ordering::Greater),
        ("1.0", "1.0.1", Ordering::Less),
        ("1.0", "2.0", Ordering::Less),
        ("1.0~rc1", "1.0", Ordering::Less),
        ("1.0", "1.0~rc1", Ordering::Greater),
        ("01", "1", Ordering::Equal),
        ("1.0.0", "1.0", Ordering::Greater),
        ("a", "1", Ordering::Less),
    ];
    for &(left, right, expected) in cases {
        assert_eq!(rpmvercmp(left, right), expected, "{left} vs {right}");
    }
}

#[test]
fn release_comparison_respects_epoch_and_release() {
    assert_eq!(compare_releases("1:1.0-1", "1:1.0-2"), Ordering::Less);
    assert_eq!(compare_releases("2:0.1-1", "1:9.9-1"), Ordering::Greater);
    assert_eq!(compare_releases("0.4.3", "0.4.3-1"), Ordering::Less);
    assert_eq!(compare_releases("0.4.3", "0.4.3"), Ordering::Equal);
}

#[test]
fn package_ids_parse_and_missing_required_fields_are_rejected() {
    let parsed = parse_package_id("monogram-desktop;0.5.0;x86_64;updates").unwrap();
    assert_eq!(parsed.name, "monogram-desktop");
    assert_eq!(parsed.version, "0.5.0");
    assert_eq!(parsed.arch, "x86_64");
    assert_eq!(parsed.data, "updates");
    assert_eq!(parse_package_id("monogram-desktop;"), None);
    assert_eq!(parse_package_id("monogram-desktop;0.5.0"), None);
}

#[test]
fn update_info_reports_newer_self_and_security_flag() {
    let ids = vec![
        "monogram-desktop;0.5.0;x86_64;updates".to_owned(),
        "kernel;6.10.0;x86_64;security".to_owned(),
    ];
    let info = update_info_from_package_ids("0.4.3", &ids);
    assert_eq!(info.available_version.as_deref(), Some("0.5.0"));
    assert!(info.security);
    assert_eq!(info.package_count, 2);
}

#[test]
fn update_info_ignores_older_or_equal_self_and_absent_security() {
    let ids = vec!["monogram-desktop;0.4.3;x86_64;updates".to_owned()];
    let info = update_info_from_package_ids("0.4.3", &ids);
    assert_eq!(info.available_version, None);
    assert!(!info.security);
    assert_eq!(info.package_count, 1);
}
