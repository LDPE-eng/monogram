use std::ops::Range;

use super::*;

pub const INSTANT_VIEW_MEDIA_MESSAGE_ID: i32 =
    proto::adapter::messages::instant_view::INSTANT_VIEW_MEDIA_MSG;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebCommand {
    Preview { message: String, fix: bool },
    Page { url: String, hash: i32 },
}

#[derive(Clone, Debug)]
pub enum WebResponse {
    Preview(Option<Box<WebPreview>>),
    Page(Box<proto::InstantViewDto>),
}

#[derive(Clone, Debug)]
pub struct WebPreview {
    pub requested_url: String,
    pub url: String,
    pub fixed: bool,
    pub page: proto::InstantViewDto,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebLink {
    Anchor(String),
    Page { url: String, anchor: Option<String> },
    External(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebRelatedArticle {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub published_date: Option<i32>,
    pub photo_cache_key: Option<String>,
}

impl Runtime {
    pub fn execute_web(&self, command: WebCommand) -> Result<WebResponse, ClientError> {
        match command {
            WebCommand::Preview { message, fix } => self.web_page_preview(message, fix),
            WebCommand::Page { url, hash } => self.web_page(url, hash),
        }
    }

    fn web_page_preview(&self, message: String, fix: bool) -> Result<WebResponse, ClientError> {
        let message = message.trim();
        if message.is_empty() {
            return Err(ClientError::InvalidRequest);
        }
        let requested_url = preview_urls(message)
            .into_iter()
            .next()
            .unwrap_or_else(|| message.to_owned());
        let fixed = fix.then(|| fixed_preview_candidates(&requested_url));
        let candidates = match fixed.as_deref() {
            Some([]) => std::slice::from_ref(&requested_url),
            Some(urls) => urls,
            None => std::slice::from_ref(&requested_url),
        };
        let handle = self.authorized()?;
        let mut last_error = None;
        let mut received_page = false;
        for url in candidates {
            match proto::extras::get_web_page_preview(handle, url.clone()) {
                Ok(page) if preview_has_content(&page) => {
                    return Ok(WebResponse::Preview(Some(Box::new(WebPreview {
                        requested_url: requested_url.clone(),
                        url: url.clone(),
                        fixed: fixed.as_ref().is_some_and(|urls| !urls.is_empty())
                            && url != &requested_url,
                        page,
                    }))));
                }
                Ok(_) => received_page = true,
                Err(error) => last_error = Some(error),
            }
        }
        if !received_page && let Some(error) = last_error {
            Err(ClientError::from(error))
        } else {
            Ok(WebResponse::Preview(None))
        }
    }

    fn web_page(&self, url: String, hash: i32) -> Result<WebResponse, ClientError> {
        let url = url.trim();
        if parse_http_url(url).is_none() {
            return Err(ClientError::InvalidRequest);
        }
        let handle = self.authorized()?;
        let page =
            proto::extras::get_web_page(handle, url.to_owned(), hash).map_err(ClientError::from)?;
        let page = if let Some(full_url) = partial_page_url(&page) {
            proto::extras::get_web_page(handle, full_url, 0).map_err(ClientError::from)?
        } else {
            page
        };
        Ok(WebResponse::Page(Box::new(page)))
    }
}

pub fn preview_urls(text: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut search_from = 0;
    while search_from < text.len() {
        let Some(start) = text[search_from..].char_indices().find_map(|(index, _)| {
            let index = search_from + index;
            let rest = &text[index..];
            (rest.starts_with("http://")
                || rest.starts_with("https://")
                || rest
                    .get(..7)
                    .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
                || rest
                    .get(..8)
                    .is_some_and(|s| s.eq_ignore_ascii_case("https://")))
            .then_some(index)
        }) else {
            break;
        };
        let end = text[start..]
            .char_indices()
            .find_map(|(index, ch)| {
                (ch.is_whitespace() || matches!(ch, '<' | '>' | '(' | ')')).then_some(start + index)
            })
            .unwrap_or(text.len());
        let url = text[start..end].trim_end_matches(['.', ',', ';', ')', ']']);
        if url.len() >= 8 && !urls.iter().any(|existing| existing == url) {
            urls.push(url.to_owned());
        }
        search_from = end.max(start + 1);
    }
    urls
}

pub fn fixed_preview_candidates(url: &str) -> Vec<String> {
    let Some(parsed) = parse_http_url(url) else {
        return Vec::new();
    };
    let host = parsed.host().to_ascii_lowercase();
    let candidates: &[&str] = match host.as_str() {
        "twitter.com" | "x.com" | "mobile.twitter.com" | "mobile.x.com" => &["fxtwitter.com"],
        "bsky.app" => &["fxbsky.app"],
        "tiktok.com" | "www.tiktok.com" | "m.tiktok.com" => {
            &["tnktok.com", "tfxktok.com", "tiktxk.com", "tiktokez.com"]
        }
        "reddit.com" | "www.reddit.com" | "old.reddit.com" | "m.reddit.com" => {
            &["rxddit.com", "redditez.com"]
        }
        "pixiv.net" | "www.pixiv.net" => &["phixiv.net"],
        _ => return Vec::new(),
    };
    candidates
        .iter()
        .map(|candidate| parsed.rewrite_host(candidate))
        .collect()
}

pub fn rewrite_preview_url(text: &str, preferred_fixed_url: Option<&str>) -> String {
    let Some(current) = preview_urls(text).into_iter().next() else {
        return text.to_owned();
    };
    let candidates = fixed_preview_candidates(&current);
    let replacement = preferred_fixed_url
        .filter(|preferred| candidates.iter().any(|url| url == preferred))
        .map(str::to_owned)
        .or_else(|| candidates.into_iter().next());
    match replacement {
        Some(replacement) => text.replacen(&current, &replacement, 1),
        None => text.to_owned(),
    }
}

pub fn resolve_web_link(current_url: &str, page_url: Option<&str>, href: &str) -> WebLink {
    let trimmed = href.trim();
    if trimmed.is_empty() {
        return WebLink::External(href.to_owned());
    }
    if let Some(fragment) = trimmed.strip_prefix('#') {
        return WebLink::Anchor(decode_fragment(fragment));
    }
    let hash_at = trimmed.rfind('#');
    let base = hash_at.map_or(trimmed, |index| &trimmed[..index]);
    let fragment = hash_at.map(|index| decode_fragment(&trimmed[index + 1..]));
    let current = page_url
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(current_url)
        .to_lowercase();
    let base_lower = base.to_lowercase();
    if let Some(anchor) = fragment.as_ref()
        && (base.is_empty()
            || current.contains(&base_lower)
            || base_lower.contains(&current)
            || current.split('#').next() == Some(base_lower.as_str()))
    {
        return WebLink::Anchor(anchor.clone());
    }
    if ["mailto:", "tel:", "tg:", "geo:"]
        .iter()
        .any(|scheme| starts_with_ignore_ascii_case(trimmed, scheme))
    {
        return WebLink::External(trimmed.to_owned());
    }
    WebLink::Page {
        url: if base.is_empty() { trimmed } else { base }.to_owned(),
        anchor: fragment,
    }
}

pub fn find_page_anchor(blocks_json: &str, name: &str) -> Option<usize> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let blocks = serde_json::from_str::<serde_json::Value>(blocks_json).ok()?;
    blocks
        .as_array()?
        .iter()
        .position(|block| block_has_anchor(block, name))
}

pub fn related_articles(blocks_json: &str) -> Vec<WebRelatedArticle> {
    let Ok(blocks) = serde_json::from_str::<serde_json::Value>(blocks_json) else {
        return Vec::new();
    };
    blocks
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| json_string(block, "k") == Some("related"))
        .filter_map(|block| block.get("articles")?.as_array())
        .flatten()
        .filter_map(|article| {
            Some(WebRelatedArticle {
                url: json_string(article, "url")?.to_owned(),
                title: json_owned(article, "title"),
                description: json_owned(article, "description"),
                author: json_owned(article, "author"),
                published_date: article
                    .get("date")
                    .and_then(serde_json::Value::as_i64)
                    .map(|date| date as i32),
                photo_cache_key: json_owned(article, "photo"),
            })
        })
        .collect()
}

fn preview_has_content(page: &proto::InstantViewDto) -> bool {
    !page.url.trim().is_empty()
        || page.title.as_deref().is_some_and(|s| !s.trim().is_empty())
        || page
            .site_name
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        || page
            .description
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        || page.has_instant_view
}

fn partial_page_url(page: &proto::InstantViewDto) -> Option<String> {
    (page.has_instant_view && page.part && !page.not_modified && !page.url.trim().is_empty())
        .then(|| page.url.clone())
}

fn block_has_anchor(block: &serde_json::Value, name: &str) -> bool {
    let Some(kind) = json_string(block, "k") else {
        return false;
    };
    match kind {
        "anchor" => json_string(block, "n").is_some_and(|anchor| anchor.eq_ignore_ascii_case(name)),
        "quote" => {
            entities_have_anchor(block.get("e"), name)
                || nested_blocks_have_anchor(block.get("blocks"), name)
        }
        "list" => block
            .get("items")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| {
                entities_have_anchor(item.get("e"), name)
                    || nested_blocks_have_anchor(item.get("blocks"), name)
            }),
        "table" => block
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_array)
            .flatten()
            .any(|cell| entities_have_anchor(cell.get("e"), name)),
        "cover" => block
            .get("block")
            .is_some_and(|nested| block_has_anchor(nested, name)),
        "details" => {
            block
                .get("title")
                .is_some_and(|title| entities_have_anchor(title.get("e"), name))
                || nested_blocks_have_anchor(block.get("blocks"), name)
        }
        "embedPost" => nested_blocks_have_anchor(block.get("blocks"), name),
        "buttons" => block
            .get("items")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .any(|item| entities_have_anchor(item.get("e"), name)),
        _ => entities_have_anchor(block.get("e"), name),
    }
}

fn nested_blocks_have_anchor(blocks: Option<&serde_json::Value>, name: &str) -> bool {
    blocks
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|block| block_has_anchor(block, name))
}

fn entities_have_anchor(entities: Option<&serde_json::Value>, name: &str) -> bool {
    entities
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|entity| {
            json_string(entity, "k").is_some_and(|kind| kind.eq_ignore_ascii_case("anchor"))
                && json_string(entity, "u")
                    .and_then(|url| url.strip_prefix('#'))
                    .is_some_and(|anchor| anchor.eq_ignore_ascii_case(name))
        })
}

fn json_string<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

fn json_owned(value: &serde_json::Value, key: &str) -> Option<String> {
    json_string(value, key).map(str::to_owned)
}

fn decode_fragment(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let Some(high) = hex(bytes[index + 1]) else {
                    return raw.to_owned();
                };
                let Some(low) = hex(bytes[index + 2]) else {
                    return raw.to_owned();
                };
                decoded.push(high << 4 | low);
                index += 3;
            }
            b'%' => return raw.to_owned(),
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

struct HttpUrl<'a> {
    value: &'a str,
    host: Range<usize>,
}

impl<'a> HttpUrl<'a> {
    fn host(&self) -> &'a str {
        &self.value[self.host.clone()]
    }

    fn rewrite_host(&self, host: &str) -> String {
        let mut rewritten = String::with_capacity(self.value.len() + host.len());
        rewritten.push_str(&self.value[..self.host.start]);
        rewritten.push_str(host);
        rewritten.push_str(&self.value[self.host.end..]);
        rewritten
    }
}

fn parse_http_url(value: &str) -> Option<HttpUrl<'_>> {
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let scheme_end = value.find("://")?;
    let scheme = &value[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority_start = scheme_end + 3;
    let authority_end = value[authority_start..]
        .find(['/', '?', '#'])
        .map(|index| authority_start + index)
        .unwrap_or(value.len());
    let authority = &value[authority_start..authority_end];
    if authority.is_empty() {
        return None;
    }
    let host_start = authority
        .rfind('@')
        .map(|index| authority_start + index + 1)
        .unwrap_or(authority_start);
    let host_end = if value[host_start..authority_end].starts_with('[') {
        let close = value[host_start..authority_end].find(']')? + host_start;
        if value[close + 1..authority_end]
            .strip_prefix(':')
            .is_some_and(|port| port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        close + 1
    } else {
        let end = value[host_start..authority_end]
            .rfind(':')
            .map(|index| host_start + index)
            .unwrap_or(authority_end);
        if end < authority_end
            && value[end + 1..authority_end]
                .bytes()
                .any(|byte| !byte.is_ascii_digit())
        {
            return None;
        }
        end
    };
    if host_start == host_end {
        return None;
    }
    Some(HttpUrl {
        value,
        host: host_start..host_end,
    })
}
