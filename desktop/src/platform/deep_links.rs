use std::collections::BTreeSet;

const DEFAULT_HOSTS: [&str; 5] = [
    "t.me",
    "www.t.me",
    "telegram.me",
    "www.telegram.me",
    "telegram.dog",
];

const RESERVED_PATHS: [&str; 22] = [
    "addemoji",
    "addlist",
    "addstickers",
    "addstyle",
    "addtheme",
    "auction",
    "auth",
    "boost",
    "bg",
    "contact",
    "giftcode",
    "invoice",
    "login",
    "proxy",
    "setlanguage",
    "share",
    "socks",
    "iv",
    "s",
    "nft",
    "slug",
    "addlist",
];

const CHANNEL_PEER_OFFSET: i64 = 1_000_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLink {
    Username {
        username: String,
        message_id: Option<i64>,
        start: Option<String>,
    },
    PrivateChannel {
        channel_id: i64,
        message_id: i64,
    },
    Invite {
        hash: String,
    },
    Share {
        text: String,
    },
}

impl DeepLink {
    /// Telegram peer id for the chat this link points at, mirroring the
    /// Android `channelPeerId` mapping for private channels.
    pub fn chat_id(&self) -> Option<i64> {
        match self {
            Self::PrivateChannel { channel_id, .. } => Some(-(CHANNEL_PEER_OFFSET + channel_id)),
            Self::Username { .. } | Self::Invite { .. } | Self::Share { .. } => None,
        }
    }
}

/// Parses a `t.me` / `tg:` deep link. Returns `None` for URIs the Android
/// client drops (unknown schemes, unknown hosts, reserved paths, or malformed
/// targets) so callers can fall back to the launch allowlist.
pub fn parse(input: &str) -> Option<DeepLink> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(username) = trimmed.strip_prefix('@')
        && !username.contains('/')
        && is_username(username)
    {
        return Some(DeepLink::Username {
            username: username.to_owned(),
            message_id: None,
            start: None,
        });
    }
    let url = normalize(trimmed)?;
    if url.scheme == "tg" {
        return parse_tg(&url);
    }
    if DEFAULT_HOSTS.contains(&url.host.as_str()) {
        return parse_https(&url);
    }
    None
}

pub fn external_action_allowed(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    let action = if trimmed
        .get(..3)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("tg:"))
    {
        let rest = &trimmed[3..];
        rest.strip_prefix("//")
            .unwrap_or(rest)
            .split(['/', '?'])
            .next()
            .unwrap_or_default()
    } else if let Some((_, path)) = trimmed.split_once("://") {
        path.split('/').nth(1).unwrap_or_default()
    } else {
        return false;
    }
    .to_ascii_lowercase();
    if matches!(
        action.as_str(),
        "addemoji"
            | "addlist"
            | "addstickers"
            | "addstyle"
            | "addtheme"
            | "auction"
            | "auth"
            | "boost"
            | "bg"
            | "contact"
            | "giftcode"
            | "invoice"
            | "login"
            | "proxy"
            | "setlanguage"
            | "socks"
            | "iv"
            | "s"
            | "nft"
            | "slug"
            | "user"
            | "unknown"
    ) {
        return false;
    }
    parse(trimmed).is_some()
}

#[derive(Debug)]
struct ParsedUrl {
    scheme: String,
    host: String,
    path: Vec<String>,
    query: Vec<(String, String)>,
}

fn normalize(raw: &str) -> Option<ParsedUrl> {
    let value = raw.trim();
    if let Some(rest) = value.strip_prefix("tg:") {
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        let (host_and_path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let host = host_and_path
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let path = host_and_path
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or_default()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(percent_decode)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        return Some(ParsedUrl {
            scheme: "tg".into(),
            host,
            path,
            query: query_map(query),
        });
    }
    if !value.contains("://") {
        if !(value.starts_with("t.me/")
            || value.starts_with("telegram.me/")
            || value.starts_with("telegram.dog/"))
        {
            return None;
        }
        return normalize_https(value);
    }
    let scheme = value.split_once("://")?.0.to_ascii_lowercase();
    let without_scheme = value.split_once("://")?.1;
    normalize_https_parts(scheme, without_scheme)
}

fn normalize_https(value: &str) -> Option<ParsedUrl> {
    let (scheme, without_scheme) = value.split_once("://").unwrap_or(("https", value));
    normalize_https_parts(scheme.to_ascii_lowercase(), without_scheme)
}

fn normalize_https_parts(scheme: String, without_scheme: &str) -> Option<ParsedUrl> {
    let host_part = without_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let after_host = without_scheme
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    let (path, query) = after_host.split_once('?').unwrap_or((after_host, ""));
    let path = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(percent_decode)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(ParsedUrl {
        scheme,
        host: host_part,
        path,
        query: query_map(query),
    })
}

fn query_map(raw: &str) -> Vec<(String, String)> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split('&')
        .filter_map(|part| {
            let key = part.split_once('=').map(|(key, _)| key).unwrap_or(part);
            if key.is_empty() {
                return None;
            }
            let value = part
                .split_once('=')
                .map(|(_, value)| value)
                .unwrap_or_default();
            let key = percent_decode(key).ok()?;
            let value = percent_decode(value).unwrap_or_default();
            Some((key, value))
        })
        .collect()
}

fn parse_https(url: &ParsedUrl) -> Option<DeepLink> {
    let first = url.path.first()?;
    match first.as_str() {
        "c" if url.path.len() >= 3 => {
            let channel_id = url.path[1].parse::<i64>().ok()?;
            let message_id = url.path[2].parse::<i64>().ok()?;
            if channel_id <= 0 {
                return None;
            }
            Some(DeepLink::PrivateChannel {
                channel_id,
                message_id,
            })
        }
        "joinchat" if url.path.len() >= 2 => {
            let hash = &url.path[1];
            if hash.is_empty() {
                return None;
            }
            Some(DeepLink::Invite { hash: hash.clone() })
        }
        "joinchat" => None,
        _ if first.starts_with('+') => {
            let hash = first.trim_start_matches('+');
            if hash.is_empty() {
                return None;
            }
            Some(DeepLink::Invite {
                hash: hash.to_owned(),
            })
        }
        "share" => {
            let text =
                query_value(&url.query, "url").or_else(|| query_value(&url.query, "text"))?;
            Some(DeepLink::Share { text })
        }
        _ if RESERVED_PATHS.contains(&first.to_ascii_lowercase().as_str()) => None,
        _ if is_username(first) => {
            let message_id = url.path.get(1).and_then(|value| value.parse::<i64>().ok());
            Some(DeepLink::Username {
                username: first.clone(),
                message_id,
                start: query_value(&url.query, "start"),
            })
        }
        _ => None,
    }
}

fn parse_tg(url: &ParsedUrl) -> Option<DeepLink> {
    match url.host.as_str() {
        "resolve" => {
            let domain = query_value(&url.query, "domain")?;
            if !is_username(&domain) {
                return None;
            }
            let post = query_value(&url.query, "post").and_then(|value| value.parse::<i64>().ok());
            Some(DeepLink::Username {
                username: domain,
                message_id: post,
                start: query_value(&url.query, "start"),
            })
        }
        "join" => query_value(&url.query, "invite").map(|hash| DeepLink::Invite { hash }),
        "msg" | "share" => query_value(&url.query, "url")
            .or_else(|| query_value(&url.query, "text"))
            .map(|text| DeepLink::Share { text }),
        _ if is_username(&url.host) => Some(DeepLink::Username {
            username: url.host.clone(),
            message_id: None,
            start: None,
        }),
        _ => None,
    }
}

fn query_value(query: &[(String, String)], key: &str) -> Option<String> {
    query
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
}

fn is_username(value: &str) -> bool {
    if value.len() < 4 || value.len() > 32 {
        return false;
    }
    if !value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn percent_decode(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(());
                }
                let high = hex(bytes[index + 1]).ok_or(())?;
                let low = hex(bytes[index + 2]).ok_or(())?;
                output.push(high << 4 | low);
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| ())
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// URIs the desktop client is allowed to hand to an external launcher. Hosts
/// are matched case-insensitively; path and query are passed through untouched.
#[derive(Debug, Clone, Default)]
pub struct LaunchAllowlist {
    schemes: BTreeSet<&'static str>,
    hosts: BTreeSet<&'static str>,
}

impl LaunchAllowlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow_scheme(mut self, scheme: &'static str) -> Self {
        self.schemes.insert(scheme);
        self
    }

    pub fn allow_host(mut self, host: &'static str) -> Self {
        self.hosts.insert(host);
        self
    }

    /// Validates a launch URI without executing it. Scheme and host are
    /// checked against the allowlist; the value is never fetched or launched.
    pub fn can_launch(&self, uri: &str) -> bool {
        let Some((scheme, rest)) = split_uri(uri) else {
            return false;
        };
        if !self.schemes.iter().any(|allowed| *allowed == scheme) {
            return false;
        }
        let Some(authority) = rest.strip_prefix("//").map(|rest| {
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            &rest[..end]
        }) else {
            return true;
        };
        let host = authority
            .split('@')
            .next_back()
            .unwrap_or_default()
            .split(':')
            .next()
            .unwrap_or_default();
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        self.hosts.iter().any(|allowed| *allowed == host.as_str())
    }
}

fn split_uri(uri: &str) -> Option<(String, &str)> {
    let colon = uri.find(':')?;
    if colon == 0 {
        return None;
    }
    let scheme = &uri[..colon];
    if !scheme
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'-' || byte == b'.')
    {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), &uri[colon + 1..]))
}

#[cfg(test)]
#[path = "../../tests/unit/deep_links.rs"]
mod tests;
