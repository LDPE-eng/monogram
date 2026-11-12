use super::*;

fn username(name: &str, message_id: Option<i64>, start: Option<&str>) -> DeepLink {
    DeepLink::Username {
        username: name.into(),
        message_id,
        start: start.map(str::to_owned),
    }
}

#[test]
fn username_links_resolve_with_and_without_scheme() {
    for link in [
        "https://t.me/monogram",
        "http://t.me/monogram",
        "t.me/monogram",
        "@monogram",
    ] {
        assert_eq!(
            parse(link),
            Some(username("monogram", None, None)),
            "{link}"
        );
    }
}

#[test]
fn message_links_map_public_channel_posts_to_peer_message_ids() {
    let link = parse("https://t.me/monogram/123").unwrap();
    assert_eq!(
        link,
        DeepLink::Username {
            username: "monogram".into(),
            message_id: Some(123),
            start: None,
        }
    );
    assert_eq!(link.chat_id(), None);
}

#[test]
fn private_channel_links_use_the_channel_peer_offset() {
    let link = parse("https://t.me/c/123456789/456").unwrap();
    assert_eq!(
        link,
        DeepLink::PrivateChannel {
            channel_id: 123456789,
            message_id: 456,
        }
    );
    assert_eq!(link.chat_id(), Some(-(1_000_000_000_000 + 123456789)));
}

#[test]
fn invite_links_carry_the_hash() {
    assert_eq!(
        parse("https://t.me/+AbCdEf123"),
        Some(DeepLink::Invite {
            hash: "AbCdEf123".into()
        })
    );
    assert_eq!(
        parse("https://t.me/joinchat/AbCdEf123"),
        Some(DeepLink::Invite {
            hash: "AbCdEf123".into()
        })
    );
    assert_eq!(
        parse("tg:join?invite=AbCdEf123"),
        Some(DeepLink::Invite {
            hash: "AbCdEf123".into()
        })
    );
}

#[test]
fn share_links_carry_the_shared_url_or_text() {
    assert_eq!(
        parse("https://t.me/share/url?url=https%3A%2F%2Fexample.com"),
        Some(DeepLink::Share {
            text: "https://example.com".into()
        })
    );
    assert_eq!(
        parse("https://t.me/share?text=hello"),
        Some(DeepLink::Share {
            text: "hello".into()
        })
    );
    assert_eq!(
        parse("tg:msg?url=https%3A%2F%2Fexample.com"),
        Some(DeepLink::Share {
            text: "https://example.com".into()
        })
    );
}

#[test]
fn tg_resolve_supports_domain_post_and_start() {
    assert_eq!(
        parse("tg:resolve?domain=monogram"),
        Some(username("monogram", None, None))
    );
    assert_eq!(
        parse("tg://resolve?domain=monogram&post=7&start=hello"),
        Some(username("monogram", Some(7), Some("hello")))
    );
}

#[test]
fn tg_host_form_is_a_username() {
    assert_eq!(
        parse("tg://monogram"),
        Some(username("monogram", None, None))
    );
}

#[test]
fn extra_telegram_hosts_are_accepted() {
    for link in [
        "https://www.t.me/monogram",
        "https://telegram.me/monogram",
        "https://www.telegram.me/monogram",
        "https://telegram.dog/monogram",
    ] {
        assert_eq!(
            parse(link),
            Some(username("monogram", None, None)),
            "{link}"
        );
    }
}

#[test]
fn reserved_paths_and_unknown_hosts_are_dropped() {
    for link in [
        "https://t.me/addstickers/whatever",
        "https://t.me/setlanguage/en",
        "https://t.me/proxy?server=x",
        "https://example.com/monogram",
        "https://t.me/",
        "",
    ] {
        assert_eq!(parse(link), None, "{link}");
    }
}

#[test]
fn malformed_targets_are_dropped() {
    for link in [
        "https://t.me/c/abc/456",
        "https://t.me/c/123456789/not-a-number",
        "https://t.me/joinchat",
        "https://t.me/+",
        "https://t.me/abc",
        "https://t.me/1starts-with-digit",
        "tg:resolve?post=5",
        "tg:join",
    ] {
        assert_eq!(parse(link), None, "{link}");
    }
}

#[test]
fn non_numeric_message_segment_falls_back_to_the_chat() {
    assert_eq!(
        parse("https://t.me/monogram/not-a-number"),
        Some(username("monogram", None, None))
    );
}

#[test]
fn start_parameter_is_carried_for_https_links() {
    assert_eq!(
        parse("https://t.me/monogram?start=payload"),
        Some(username("monogram", None, Some("payload")))
    );
}

#[test]
fn percent_encoded_segments_are_decoded() {
    assert_eq!(
        parse("https://t.me/%6Donogram"),
        Some(username("monogram", None, None))
    );
}

#[test]
fn launch_allowlist_checks_scheme_and_host_only() {
    let allowlist = LaunchAllowlist::new()
        .allow_scheme("https")
        .allow_scheme("mailto")
        .allow_host("example.com");
    assert!(allowlist.can_launch("https://example.com/path?q=1"));
    assert!(allowlist.can_launch("https://EXAMPLE.com/"));
    assert!(allowlist.can_launch("mailto:user@example.com"));
    assert!(!allowlist.can_launch("https://other.com/"));
    assert!(!allowlist.can_launch("http://example.com/"));
    assert!(!allowlist.can_launch("file:///etc/passwd"));
    assert!(!allowlist.can_launch("not a uri"));
}

#[test]
fn external_actions_reject_session_and_proxy_changes() {
    for link in [
        "tg://login?token=secret",
        "tg://proxy?server=example.com&port=443&secret=abc",
        "tg://socks?server=example.com&port=1080",
        "https://t.me/addstickers/example",
        "https://example.com/monogram",
        "tg://monogram/\u{0}",
    ] {
        assert!(!external_action_allowed(link), "{link}");
    }
    assert!(external_action_allowed("tg://monogram"));
    assert!(external_action_allowed("tg:resolve?domain=monogram&post=7"));
    assert!(external_action_allowed("https://t.me/c/123456789/456"));
}
