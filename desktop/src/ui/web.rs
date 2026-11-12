use std::{cell::RefCell, collections::BTreeMap, fmt, rc::Rc};

use mono_proto_client::InstantViewDto;
use monogram_markup::MarkupEntityDto;
use relm4::{ComponentParts, ComponentSender, SimpleComponent, adw, adw::prelude::*, gtk};
use serde::Deserialize;

use crate::{
    client::web::{WebCommand, WebResponse},
    i18n::I18n,
    ui::markup::{Document, EntityKind, LinkTarget, ParseMode, RichText},
};

const MAX_NESTING: usize = 16;
const MAX_TABLE_ROWS: usize = 128;
const MAX_TABLE_COLUMNS: usize = 64;

pub enum WebInput {
    Loading,
    Page(InstantViewDto),
    Response(WebResponse),
    Error(String),
    Back,
    Retry,
    OpenOriginal,
    OpenLink(String),
    OpenMedia(String),
}

pub enum WebOutput {
    Back,
    Command(WebCommand),
    OpenExternal(String),
    OpenMedia(String),
}

impl fmt::Debug for WebInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Loading => "Loading",
            Self::Page(_) => "Page(..)",
            Self::Response(_) => "Response(..)",
            Self::Error(_) => "Error(..)",
            Self::Back => "Back",
            Self::Retry => "Retry",
            Self::OpenOriginal => "OpenOriginal",
            Self::OpenLink(_) => "OpenLink(..)",
            Self::OpenMedia(_) => "OpenMedia(..)",
        })
    }
}

impl fmt::Debug for WebOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Back => "Back",
            Self::Command(_) => "Command(..)",
            Self::OpenExternal(_) => "OpenExternal(..)",
            Self::OpenMedia(_) => "OpenMedia(..)",
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct EntityDto {
    #[serde(rename = "k")]
    kind: String,
    #[serde(rename = "o")]
    offset: i32,
    #[serde(rename = "l")]
    length: i32,
    #[serde(rename = "u")]
    url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct RichTextDto {
    #[serde(rename = "t")]
    text: String,
    #[serde(rename = "e")]
    entities: Vec<EntityDto>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct CaptionDto {
    #[serde(flatten)]
    text: RichTextDto,
    credit: Option<RichTextDto>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ListItemDto {
    #[serde(flatten)]
    text: RichTextDto,
    blocks: Vec<BlockDto>,
    #[serde(rename = "num")]
    number: Option<String>,
    checkbox: bool,
    checked: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct TableCellDto {
    #[serde(flatten)]
    text: RichTextDto,
    #[serde(rename = "h")]
    header: bool,
    #[serde(rename = "cs")]
    colspan: i32,
    #[serde(rename = "rs")]
    rowspan: i32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct RelatedArticleDto {
    url: String,
    title: Option<String>,
    description: Option<String>,
    author: Option<String>,
    #[serde(rename = "date")]
    published_date: Option<i32>,
    photo: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct BlockDto {
    #[serde(rename = "k")]
    kind: String,
    #[serde(flatten)]
    text: RichTextDto,
    #[serde(rename = "l")]
    level: i32,
    #[serde(rename = "lang")]
    language: Option<String>,
    #[serde(rename = "d")]
    published_date: Option<i32>,
    pull: bool,
    caption: Option<CaptionDto>,
    ordered: bool,
    items: Vec<serde_json::Value>,
    rows: Vec<Vec<TableCellDto>>,
    title: Option<serde_json::Value>,
    bordered: bool,
    striped: bool,
    open: bool,
    blocks: Vec<BlockDto>,
    id: i64,
    cache: Option<String>,
    #[serde(rename = "w")]
    width: i32,
    #[serde(rename = "h")]
    height: i32,
    url: Option<String>,
    #[serde(rename = "name")]
    file_name: Option<String>,
    #[serde(rename = "mime")]
    mime_type: Option<String>,
    #[serde(rename = "size")]
    file_size: Option<i64>,
    #[serde(rename = "duration")]
    duration_seconds: Option<i32>,
    performer: Option<String>,
    autoplay: bool,
    #[serde(rename = "loop")]
    looped: bool,
    block: Option<Box<BlockDto>>,
    html: Option<String>,
    full: bool,
    scroll: bool,
    poster: Option<String>,
    author: Option<String>,
    date: Option<i32>,
    photo: Option<String>,
    articles: Vec<RelatedArticleDto>,
    lat: f64,
    lng: f64,
    zoom: i32,
    #[serde(rename = "src")]
    math_source: String,
    #[serde(rename = "n")]
    anchor_name: String,
    username: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PageError {
    InvalidBlocks,
}

struct InstantViewPage {
    url: String,
    display_url: String,
    title: Option<String>,
    site_name: Option<String>,
    description: Option<String>,
    hash: i32,
    has_instant_view: bool,
    part: bool,
    rtl: bool,
    not_modified: bool,
    blocks: Vec<BlockDto>,
}

impl InstantViewPage {
    fn from_dto(dto: InstantViewDto) -> Result<Self, PageError> {
        let blocks =
            serde_json::from_str(&dto.blocks_json).map_err(|_| PageError::InvalidBlocks)?;
        Ok(Self {
            url: dto.url,
            display_url: dto.display_url,
            title: dto.title,
            site_name: dto.site_name,
            description: dto.description,
            hash: dto.hash,
            has_instant_view: dto.has_instant_view,
            part: dto.part,
            rtl: dto.rtl,
            not_modified: dto.not_modified,
            blocks,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedLink {
    Anchor(String),
    Page { url: String, anchor: Option<String> },
    External(String),
}

fn decode_anchor(raw: &str) -> String {
    if raw.contains('\0') {
        return raw.to_owned();
    }
    let form_encoded = raw.replace('+', " ");
    gtk::glib::uri_unescape_string(&form_encoded, gtk::glib::GStr::NONE)
        .map(|value| value.to_string())
        .unwrap_or_else(|| raw.to_owned())
}

fn safe_uri(uri: &str) -> bool {
    let uri = uri.trim();
    if uri.is_empty()
        || uri.contains('\0')
        || uri.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    let Some((scheme, target)) = uri.split_once(':') else {
        return false;
    };
    if target.is_empty() {
        return false;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "https" | "http" => gtk::glib::Uri::parse(uri, gtk::glib::UriFlags::NONE)
            .ok()
            .and_then(|parsed| parsed.host())
            .is_some_and(|host| !host.is_empty()),
        "tg" => target
            .strip_prefix("//")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with(['/', '?', '#'])),
        "mailto" | "tel" | "geo" => true,
        _ => false,
    }
}

fn absolute_page_uri(current_url: &str, href: &str) -> Option<String> {
    if current_url.contains('\0') || href.contains('\0') {
        return None;
    }
    let absolute = if gtk::glib::Uri::parse_scheme(href).is_some() {
        href.to_owned()
    } else {
        gtk::glib::Uri::resolve_relative(
            (!current_url.is_empty()).then_some(current_url),
            href,
            gtk::glib::UriFlags::NONE,
        )
        .ok()?
        .to_string()
    };
    safe_uri(&absolute).then_some(absolute).filter(|uri| {
        matches!(
            gtk::glib::Uri::parse_scheme(uri)
                .map(|scheme| scheme.to_ascii_lowercase())
                .as_deref(),
            Some("http" | "https")
        )
    })
}

fn resolve_link(current_url: &str, href: &str) -> Option<ResolvedLink> {
    let trimmed = href.trim();
    if trimmed.is_empty() || trimmed.contains('\0') || current_url.contains('\0') {
        return None;
    }
    if let Some(fragment) = trimmed.strip_prefix('#') {
        let anchor = decode_anchor(fragment);
        return (!anchor.trim().is_empty()).then_some(ResolvedLink::Anchor(anchor));
    }
    let hash = trimmed.rfind('#');
    let base = hash.map_or(trimmed, |index| &trimmed[..index]);
    let fragment = hash.map(|index| decode_anchor(&trimmed[index + 1..]));
    let current = current_url.trim().to_ascii_lowercase();
    let base_lower = base.to_ascii_lowercase();
    let same_page = fragment.as_ref().is_some_and(|name| !name.is_empty())
        && (base.is_empty()
            || !current.is_empty()
                && (current.contains(&base_lower)
                    || base_lower.contains(&current)
                    || current.split('#').next() == Some(base_lower.as_str())));
    if same_page {
        return fragment.map(ResolvedLink::Anchor);
    }
    if matches!(
        gtk::glib::Uri::parse_scheme(trimmed)
            .map(|scheme| scheme.to_ascii_lowercase())
            .as_deref(),
        Some("mailto" | "tel" | "tg" | "geo")
    ) {
        return safe_uri(trimmed).then(|| ResolvedLink::External(trimmed.to_owned()));
    }
    absolute_page_uri(current_url, base).map(|url| ResolvedLink::Page {
        url,
        anchor: fragment,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ReadingEstimate {
    words: usize,
    han: usize,
    kana: usize,
    hangul: usize,
    minutes: usize,
}

fn reading_estimate(text: &str) -> ReadingEstimate {
    if text.trim().is_empty() {
        return ReadingEstimate::default();
    }
    let mut estimate = ReadingEstimate::default();
    let mut in_word = false;
    for ch in text.chars() {
        let code = ch as u32;
        let han = matches!(code, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2fa1f | 0x30000..=0x323af);
        let kana = matches!(code, 0x3040..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9d);
        let hangul = matches!(code, 0x1100..=0x11ff | 0x3130..=0x318f | 0xa960..=0xa97f | 0xac00..=0xd7af | 0xd7b0..=0xd7ff);
        if han {
            estimate.han += 1;
            in_word = false;
        } else if kana {
            estimate.kana += 1;
            in_word = false;
        } else if hangul {
            estimate.hangul += 1;
            in_word = false;
        } else if ch.is_alphanumeric() {
            if !in_word {
                estimate.words += 1;
            }
            in_word = true;
        } else {
            in_word = false;
        }
    }
    let units = estimate.words as f64 / 200.0
        + estimate.han as f64 / 300.0
        + estimate.kana as f64 / 400.0
        + estimate.hangul as f64 / 350.0;
    estimate.minutes = (units.ceil() as usize).max(1);
    estimate
}

fn append_plain_text(blocks: &[BlockDto], out: &mut Vec<String>, depth: usize) {
    if depth > MAX_NESTING {
        return;
    }
    for block in blocks {
        match block.kind.as_str() {
            "title" | "subtitle" | "kicker" | "paragraph" | "footer" | "pre" | "heading"
            | "authorDate" | "thinking" => out.push(block.text.text.clone()),
            "quote" => {
                out.push(block.text.text.clone());
                append_plain_text(&block.blocks, out, depth + 1);
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            "list" => {
                for (index, value) in block.items.iter().enumerate() {
                    let Ok(item) = serde_json::from_value::<ListItemDto>(value.clone()) else {
                        continue;
                    };
                    let mut body = vec![item.text.text.clone()];
                    append_plain_text(&item.blocks, &mut body, depth + 1);
                    let body = body
                        .into_iter()
                        .filter(|text| !text.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if body.is_empty() {
                        continue;
                    }
                    let marker = if item.checkbox {
                        if item.checked {
                            "☑".into()
                        } else {
                            "☐".into()
                        }
                    } else if block.ordered {
                        item.number
                            .filter(|number| !number.trim().is_empty())
                            .unwrap_or_else(|| format!("{}.", index + 1))
                    } else {
                        "•".into()
                    };
                    out.push(
                        body.lines()
                            .enumerate()
                            .map(|(line, text)| {
                                if line == 0 {
                                    format!("{marker} {text}")
                                } else {
                                    format!("    {text}")
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
            }
            "table" => {
                if let Some(title) = rich_value(block.title.as_ref()) {
                    out.push(title.text);
                }
                for row in &block.rows {
                    let cells = row
                        .iter()
                        .map(|cell| cell.text.text.replace('\n', " ").trim().to_owned())
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>();
                    if !cells.is_empty() {
                        out.push(cells.join(" | "));
                    }
                }
            }
            "details" => {
                if let Some(title) = rich_value(block.title.as_ref()) {
                    out.push(title.text);
                }
                append_plain_text(&block.blocks, out, depth + 1);
            }
            "photo" | "embed" => {
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            "video" | "audio" | "document" => {
                if let Some(title) = string_value(block.title.as_ref()).or(block.file_name.clone())
                {
                    out.push(title);
                }
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            "cover" => {
                if let Some(block) = &block.block {
                    append_plain_text(std::slice::from_ref(block), out, depth + 1);
                }
            }
            "embedPost" => {
                if let Some(author) = block.author.as_ref().filter(|author| !author.is_empty()) {
                    out.push(author.clone());
                }
                append_plain_text(&block.blocks, out, depth + 1);
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            "collage" | "slideshow" => {
                let items = parse_blocks(&block.items);
                append_plain_text(&items, out, depth + 1);
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            "math" => out.push(block.math_source.clone()),
            "map" => {
                if let Some(caption) = &block.caption {
                    out.push(caption.text.text.clone());
                }
            }
            _ => {}
        }
    }
}

fn plain_text(blocks: &[BlockDto]) -> String {
    let mut parts = Vec::new();
    append_plain_text(blocks, &mut parts, 0);
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_owned()
}

fn rich_value(value: Option<&serde_json::Value>) -> Option<RichTextDto> {
    let rich = serde_json::from_value::<RichTextDto>(value?.clone()).ok()?;
    (!rich.text.trim().is_empty()).then_some(rich)
}

fn string_value(value: Option<&serde_json::Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

fn parse_blocks(values: &[serde_json::Value]) -> Vec<BlockDto> {
    values
        .iter()
        .map(|value| {
            serde_json::from_value(value.clone()).unwrap_or_else(|_| BlockDto {
                kind: "unsupported".into(),
                ..BlockDto::default()
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlacedCell {
    row: usize,
    column: usize,
    cell: TableCellDto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TableLayout {
    rows: usize,
    columns: usize,
    cells: Vec<PlacedCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TableError {
    TooLarge,
}

fn place_table(rows: &[Vec<TableCellDto>]) -> Result<TableLayout, TableError> {
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut cells = Vec::new();
    fn ensure(occupied: &mut Vec<Vec<bool>>, row: usize, column: usize) -> Result<(), TableError> {
        if row >= MAX_TABLE_ROWS || column >= MAX_TABLE_COLUMNS {
            return Err(TableError::TooLarge);
        }
        while occupied.len() <= row {
            occupied.push(Vec::new());
        }
        while occupied[row].len() <= column {
            occupied[row].push(false);
        }
        Ok(())
    }
    for (row_index, row) in rows.iter().enumerate() {
        let mut column = 0usize;
        for cell in row {
            let colspan = cell.colspan.max(1) as usize;
            let rowspan = cell.rowspan.max(1) as usize;
            loop {
                ensure(&mut occupied, row_index, column)?;
                if !occupied[row_index][column] {
                    break;
                }
                column += 1;
            }
            for row_offset in 0..rowspan {
                for column_offset in 0..colspan {
                    let target_row = row_index
                        .checked_add(row_offset)
                        .ok_or(TableError::TooLarge)?;
                    let target_column = column
                        .checked_add(column_offset)
                        .ok_or(TableError::TooLarge)?;
                    ensure(&mut occupied, target_row, target_column)?;
                    occupied[target_row][target_column] = true;
                }
            }
            cells.push(PlacedCell {
                row: row_index,
                column,
                cell: cell.clone(),
            });
            column = column.checked_add(colspan).ok_or(TableError::TooLarge)?;
        }
    }
    Ok(TableLayout {
        rows: occupied.len(),
        columns: occupied.iter().map(Vec::len).max().unwrap_or(0),
        cells,
    })
}

pub struct WebView {
    i18n: Rc<I18n>,
    page: Option<InstantViewPage>,
    requested_url: Option<String>,
    loading: bool,
    error: Option<String>,
    refetched_partial: bool,
    pending_anchor: RefCell<Option<String>>,
    anchors: RefCell<BTreeMap<String, gtk::Widget>>,
    title: gtk::Label,
    status: gtk::Label,
    spinner: gtk::Spinner,
    retry: gtk::Button,
    open: gtk::Button,
    scroller: gtk::ScrolledWindow,
    content: gtk::Box,
}

impl WebView {
    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }

    fn accept_page(&mut self, dto: InstantViewDto) -> Option<WebOutput> {
        let page = match InstantViewPage::from_dto(dto) {
            Ok(page) => page,
            Err(_) => {
                self.loading = false;
                self.error = Some(self.text("feature-dialog-dialog_instant_view_error"));
                return None;
            }
        };
        self.loading = false;
        if page.not_modified {
            if self.page.is_none() {
                self.error = Some(self.text("feature-dialog-dialog_instant_view_error"));
            }
            return None;
        }
        if !page.has_instant_view {
            self.page = None;
            self.error = Some(self.text("feature-dialog-dialog_instant_view_empty"));
            return None;
        }
        if page.part && !self.refetched_partial && !page.url.trim().is_empty() {
            self.refetched_partial = true;
            self.requested_url = Some(page.url.clone());
            self.loading = true;
            return Some(WebOutput::Command(WebCommand::Page {
                url: page.url,
                hash: 0,
            }));
        }
        self.requested_url = Some(page.url.clone());
        self.page = Some(page);
        self.error = None;
        None
    }

    fn handle(&mut self, input: WebInput) -> Vec<WebOutput> {
        let mut outputs = Vec::new();
        match input {
            WebInput::Loading => {
                self.loading = true;
                self.error = None;
            }
            WebInput::Page(dto) => outputs.extend(self.accept_page(dto)),
            WebInput::Response(response) => match response {
                WebResponse::Page(page) => outputs.extend(self.accept_page(*page)),
                WebResponse::Preview(Some(preview)) => {
                    self.requested_url = Some(preview.requested_url.clone());
                    outputs.extend(self.accept_page(preview.page));
                }
                WebResponse::Preview(None) => {
                    self.loading = false;
                    self.page = None;
                    self.error = Some(self.text("feature-dialog-dialog_instant_view_empty"));
                }
            },
            WebInput::Error(error) => {
                self.loading = false;
                self.error = Some(if error.trim().is_empty() {
                    self.text("feature-dialog-dialog_instant_view_error")
                } else {
                    error
                });
            }
            WebInput::Back => outputs.push(WebOutput::Back),
            WebInput::Retry => {
                if let Some((url, hash)) = self
                    .page
                    .as_ref()
                    .map(|page| (page.url.clone(), page.hash))
                    .or_else(|| self.requested_url.clone().map(|url| (url, 0)))
                    .filter(|(url, _)| absolute_page_uri("", url).is_some())
                {
                    self.loading = true;
                    self.error = None;
                    outputs.push(WebOutput::Command(WebCommand::Page { url, hash }));
                }
            }
            WebInput::OpenOriginal => {
                if let Some(url) = self
                    .page
                    .as_ref()
                    .map(|page| page.url.clone())
                    .or_else(|| self.requested_url.clone())
                    .filter(|url| safe_uri(url))
                {
                    outputs.push(WebOutput::OpenExternal(url));
                }
            }
            WebInput::OpenLink(href) => {
                let current = self.page.as_ref().map_or("", |page| page.url.as_str());
                match resolve_link(current, &href) {
                    Some(ResolvedLink::Anchor(anchor)) => {
                        *self.pending_anchor.borrow_mut() = Some(anchor);
                    }
                    Some(ResolvedLink::Page { url, anchor }) => {
                        *self.pending_anchor.borrow_mut() = anchor;
                        self.requested_url = Some(url.clone());
                        self.refetched_partial = false;
                        self.loading = true;
                        self.error = None;
                        outputs.push(WebOutput::Command(WebCommand::Page { url, hash: 0 }));
                    }
                    Some(ResolvedLink::External(url)) => {
                        outputs.push(WebOutput::OpenExternal(url));
                    }
                    None => {}
                }
            }
            WebInput::OpenMedia(cache_key) => {
                if valid_media_key(&cache_key) {
                    outputs.push(WebOutput::OpenMedia(cache_key));
                }
            }
        }
        outputs
    }
}

fn valid_media_key(key: &str) -> bool {
    let Some((kind, rest)) = key.split_once(':') else {
        return false;
    };
    matches!(kind, "photo" | "doc" | "geo" | "avatar")
        && rest
            .split(':')
            .next()
            .is_some_and(|id| !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()))
        && !key.chars().any(|ch| ch.is_control() || ch.is_whitespace())
}

impl SimpleComponent for WebView {
    type Init = Rc<I18n>;
    type Input = WebInput;
    type Output = WebOutput;
    type Root = gtk::Box;
    type Widgets = gtk::Box;

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        i18n: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let header = adw::HeaderBar::new();
        let back = gtk::Button::with_label(
            &i18n
                .text("feature-dialog-dialog_instant_view_back", &[])
                .unwrap_or_default(),
        );
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(WebInput::Back));
        header.pack_start(&back);
        let title = gtk::Label::new(Some(
            &i18n
                .text("feature-dialog-dialog_instant_view", &[])
                .unwrap_or_default(),
        ));
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        header.set_title_widget(Some(&title));
        let retry = gtk::Button::with_label(
            &i18n
                .text("feature-dialog-dialog_instant_view_retry", &[])
                .unwrap_or_default(),
        );
        let input = sender.input_sender().clone();
        retry.connect_clicked(move |_| input.emit(WebInput::Retry));
        header.pack_end(&retry);
        let open = gtk::Button::with_label(
            &i18n
                .text("feature-dialog-dialog_instant_view_open_original", &[])
                .unwrap_or_default(),
        );
        let input = sender.input_sender().clone();
        open.connect_clicked(move |_| input.emit(WebInput::OpenOriginal));
        header.pack_end(&open);
        root.append(&header);
        let status = gtk::Label::new(None);
        status.set_wrap(true);
        status.set_margin_top(8);
        status.set_margin_bottom(8);
        status.set_visible(false);
        root.append(&status);
        let spinner = gtk::Spinner::new();
        spinner.set_visible(false);
        root.append(&spinner);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
        content.set_margin_start(18);
        content.set_margin_end(18);
        content.set_margin_top(14);
        content.set_margin_bottom(32);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        root.append(&scroller);
        let model = Self {
            i18n,
            page: None,
            requested_url: None,
            loading: false,
            error: None,
            refetched_partial: false,
            pending_anchor: RefCell::new(None),
            anchors: RefCell::new(BTreeMap::new()),
            title,
            status,
            spinner,
            retry,
            open,
            scroller,
            content,
        };
        model.render(&sender);
        ComponentParts {
            model,
            widgets: root,
        }
    }

    fn update(&mut self, input: Self::Input, sender: ComponentSender<Self>) {
        for output in self.handle(input) {
            let _ = sender.output(output);
        }
    }

    fn update_view(&self, _root: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.render(&sender);
        self.scroll_to_pending_anchor();
    }
}

impl WebView {
    fn render(&self, sender: &ComponentSender<Self>) {
        clear_box(&self.content);
        self.anchors.borrow_mut().clear();
        self.spinner.set_visible(self.loading);
        self.spinner.set_spinning(self.loading);
        self.retry
            .set_visible(self.error.is_some() || self.page.is_some());
        let original = self
            .page
            .as_ref()
            .map(|page| page.url.as_str())
            .or(self.requested_url.as_deref())
            .filter(|url| safe_uri(url));
        self.open.set_visible(original.is_some());
        self.open.set_sensitive(original.is_some());
        let Some(page) = &self.page else {
            self.title
                .set_label(&self.text("feature-dialog-dialog_instant_view"));
            let message = if self.loading {
                Some("feature-dialog-dialog_instant_view_loading")
            } else if self.error.is_some() {
                None
            } else {
                Some("feature-dialog-dialog_instant_view_empty")
            };
            if let Some(key) = message {
                self.status.set_label(&self.text(key));
                self.status.set_visible(true);
            } else {
                self.status
                    .set_label(self.error.as_deref().unwrap_or_default());
                self.status.set_visible(true);
            }
            self.scroller.set_visible(false);
            return;
        };
        self.scroller.set_visible(true);
        self.status
            .set_label(self.error.as_deref().unwrap_or_default());
        self.status.set_visible(self.error.is_some());
        self.title.set_label(
            page.site_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .or(page
                    .title
                    .as_deref()
                    .filter(|value| !value.trim().is_empty()))
                .unwrap_or_else(|| display_host(page).unwrap_or("Instant View")),
        );
        self.title.set_tooltip_text(page.description.as_deref());
        self.content.set_direction(if page.rtl {
            gtk::TextDirection::Rtl
        } else {
            gtk::TextDirection::Ltr
        });
        if should_show_page_title(page)
            && let Some(title) = page.title.as_deref()
        {
            append_rich(
                self,
                &self.content,
                &RichTextDto {
                    text: title.to_owned(),
                    entities: Vec::new(),
                },
                TextStyle::Title,
                None,
            );
        }
        let estimate = reading_estimate(&plain_text(&page.blocks));
        if estimate.minutes > 0 {
            let label = gtk::Label::new(Some(
                &self
                    .i18n
                    .text(
                        "feature-dialog-dialog_instant_view_min_read",
                        &[("arg1", &estimate.minutes.to_string())],
                    )
                    .unwrap_or_default(),
            ));
            label.set_xalign(0.0);
            label.add_css_class("dim-label");
            self.content.append(&label);
        }
        self.render_blocks(&self.content, &page.blocks, sender, 0);
    }

    fn render_blocks(
        &self,
        parent: &gtk::Box,
        blocks: &[BlockDto],
        sender: &ComponentSender<Self>,
        depth: usize,
    ) {
        if depth > MAX_NESTING {
            self.unsupported(parent);
            return;
        }
        for block in blocks {
            match block.kind.as_str() {
                "title" | "subtitle" | "kicker" | "paragraph" | "footer" | "heading"
                | "authorDate" | "thinking" => {
                    let style = match block.kind.as_str() {
                        "title" => TextStyle::Title,
                        "subtitle" => TextStyle::Subtitle,
                        "heading" => TextStyle::Heading(block.level.clamp(1, 6) as u8),
                        "kicker" | "authorDate" | "footer" | "thinking" => TextStyle::Caption,
                        _ => TextStyle::Body,
                    };
                    append_rich(self, parent, &block.text, style, Some(sender));
                    if block.kind == "authorDate"
                        && let Some(date) = format_date(block.published_date)
                    {
                        append_label(self, parent, &date, TextStyle::Caption);
                    }
                }
                "pre" => {
                    let scroll = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Automatic)
                        .vscrollbar_policy(gtk::PolicyType::Never)
                        .propagate_natural_width(true)
                        .build();
                    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    body.add_css_class("card");
                    append_rich(self, &body, &block.text, TextStyle::Pre, Some(sender));
                    scroll.set_child(Some(&body));
                    parent.append(&scroll);
                }
                "quote" => {
                    let card = card(gtk::Orientation::Vertical, 8);
                    if block.pull {
                        card.add_css_class("navigation-sidebar");
                    }
                    append_rich(
                        self,
                        &card,
                        &block.text,
                        if block.pull {
                            TextStyle::Pullquote
                        } else {
                            TextStyle::Quote
                        },
                        Some(sender),
                    );
                    self.render_blocks(&card, &block.blocks, sender, depth + 1);
                    if let Some(caption) = &block.caption {
                        self.append_caption(&card, caption, sender);
                    }
                    parent.append(&card);
                }
                "list" => {
                    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    for (index, value) in block.items.iter().enumerate() {
                        let Ok(item) = serde_json::from_value::<ListItemDto>(value.clone()) else {
                            self.unsupported(&list);
                            continue;
                        };
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                        let marker = if item.checkbox {
                            if item.checked { "☑" } else { "☐" }
                        } else if block.ordered {
                            item.number.as_deref().unwrap_or("")
                        } else {
                            "•"
                        };
                        let marker = if marker.is_empty() && block.ordered && !item.checkbox {
                            format!("{}.", index + 1)
                        } else {
                            marker.to_owned()
                        };
                        let label = gtk::Label::new(Some(&marker));
                        label.set_yalign(0.0);
                        row.append(&label);
                        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
                        body.set_hexpand(true);
                        if !item.text.text.is_empty() {
                            append_rich(self, &body, &item.text, TextStyle::Body, Some(sender));
                        }
                        self.render_blocks(&body, &item.blocks, sender, depth + 1);
                        row.append(&body);
                        list.append(&row);
                    }
                    parent.append(&list);
                }
                "table" => self.render_table(parent, block, sender, depth),
                "details" => {
                    let title = rich_value(block.title.as_ref());
                    let expander = gtk::Expander::new(Some(
                        &title
                            .as_ref()
                            .map(|title| title.text.clone())
                            .unwrap_or_default(),
                    ));
                    if let Some(title) = &title {
                        self.register_dto_anchors(title, &expander);
                    }
                    expander.set_expanded(block.open);
                    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
                    body.set_margin_top(8);
                    self.render_blocks(&body, &block.blocks, sender, depth + 1);
                    expander.set_child(Some(&body));
                    parent.append(&expander);
                }
                "photo" => self.render_photo(parent, block, sender),
                "video" | "audio" | "document" => self.render_document(parent, block, sender),
                "cover" => {
                    if let Some(inner) = &block.block {
                        self.render_blocks(parent, std::slice::from_ref(inner), sender, depth + 1);
                    } else {
                        self.unsupported(parent);
                    }
                }
                "embed" => self.render_embed(parent, block, sender),
                "embedPost" => self.render_embed_post(parent, block, sender, depth),
                "collage" | "slideshow" => self.render_media_group(parent, block, sender, depth),
                "channel" => self.render_channel(parent, block, sender),
                "related" => self.render_related(parent, block, sender),
                "map" => self.render_map(parent, block, sender),
                "math" => {
                    let label = append_label(self, parent, &block.math_source, TextStyle::Pre);
                    label.set_selectable(true);
                    label.set_widget_name("instant-view-math-source");
                }
                "anchor" => {
                    let anchor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    anchor.set_height_request(1);
                    parent.append(&anchor);
                    self.register_anchor(&block.anchor_name, &anchor);
                }
                "divider" => parent.append(&gtk::Separator::new(gtk::Orientation::Horizontal)),
                "buttons" => self.render_buttons(parent, block, sender),
                _ => self.unsupported(parent),
            }
        }
    }

    fn render_table(
        &self,
        parent: &gtk::Box,
        block: &BlockDto,
        sender: &ComponentSender<Self>,
        _depth: usize,
    ) {
        if let Some(title) = rich_value(block.title.as_ref()) {
            append_rich(self, parent, &title, TextStyle::Subtitle, Some(sender));
        }
        let layout = match place_table(&block.rows) {
            Ok(layout) if layout.rows > 0 && layout.columns > 0 => layout,
            Ok(_) => return,
            Err(_) => {
                self.unsupported(parent);
                return;
            }
        };
        let grid = gtk::Grid::new();
        grid.set_row_spacing(2);
        grid.set_column_spacing(2);
        if block.bordered {
            grid.add_css_class("card");
        }
        for placed in layout.cells {
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
            cell.set_margin_start(8);
            cell.set_margin_end(8);
            cell.set_margin_top(6);
            cell.set_margin_bottom(6);
            if block.striped && placed.row % 2 == 1 {
                cell.add_css_class("card");
            }
            append_rich(
                self,
                &cell,
                &placed.cell.text,
                if placed.cell.header {
                    TextStyle::TableHeader
                } else {
                    TextStyle::Body
                },
                Some(sender),
            );
            grid.attach(
                &cell,
                placed.column as i32,
                placed.row as i32,
                placed.cell.colspan.max(1),
                placed.cell.rowspan.max(1),
            );
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .child(&grid)
            .build();
        parent.append(&scroll);
    }

    fn render_photo(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let card = media_card();
        append_label(
            self,
            &card,
            &self.text("feature-dialog-dialog_media_photo"),
            TextStyle::Subtitle,
        );
        append_dimensions(self, &card, block.width, block.height);
        if let Some(cache) = block.cache.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, cache, sender));
        } else {
            self.unsupported(&card);
        }
        if let Some(url) = block.url.as_deref().filter(|url| !url.trim().is_empty()) {
            card.append(&link_button(
                self.text("feature-dialog-dialog_instant_view_open_original"),
                url,
                sender,
            ));
        }
        if let Some(caption) = &block.caption {
            self.append_caption(&card, caption, sender);
        }
        parent.append(&card);
    }

    fn render_document(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let card = media_card();
        let kind = match block.kind.as_str() {
            "video" => "feature-dialog-dialog_media_video",
            "audio" => "feature-dialog-dialog_media_audio",
            _ => "feature-dialog-dialog_media_document",
        };
        let title = string_value(block.title.as_ref())
            .or_else(|| block.file_name.clone())
            .unwrap_or_else(|| self.text(kind));
        append_label(self, &card, &title, TextStyle::Subtitle);
        if let Some(performer) = block.performer.as_deref().filter(|value| !value.is_empty()) {
            append_label(self, &card, performer, TextStyle::Caption);
        }
        if let Some(name) = block.file_name.as_deref().filter(|value| !value.is_empty()) {
            append_label(
                self,
                &card,
                &format!("{}: {name}", self.text("core-ui-media_info_name")),
                TextStyle::Caption,
            );
        }
        append_dimensions(self, &card, block.width, block.height);
        if let Some(size) = format_file_size(block.file_size) {
            append_label(
                self,
                &card,
                &format!("{}: {size}", self.text("core-ui-media_info_size")),
                TextStyle::Caption,
            );
        }
        if let Some(duration) = block.duration_seconds.filter(|duration| *duration >= 0) {
            append_label(
                self,
                &card,
                &format!("{}: {duration}", self.text("core-ui-media_info_duration")),
                TextStyle::Caption,
            );
        }
        if let Some(cache) = block.cache.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, cache, sender));
        } else {
            self.unsupported(&card);
        }
        if let Some(caption) = &block.caption {
            self.append_caption(&card, caption, sender);
        }
        parent.append(&card);
    }

    fn render_embed(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let card = media_card();
        append_label(
            self,
            &card,
            &self.text("feature-dialog-dialog_media_link"),
            TextStyle::Subtitle,
        );
        let mut represented = false;
        if let Some(poster) = block.poster.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, poster, sender));
            represented = true;
        }
        if let Some(url) = block.url.as_deref().filter(|url| safe_uri(url)) {
            card.append(&link_button(
                self.text("feature-dialog-dialog_instant_view_open_original"),
                url,
                sender,
            ));
            represented = true;
        }
        if !represented {
            self.unsupported(&card);
        }
        if let Some(caption) = &block.caption {
            self.append_caption(&card, caption, sender);
        }
        parent.append(&card);
    }

    fn render_embed_post(
        &self,
        parent: &gtk::Box,
        block: &BlockDto,
        sender: &ComponentSender<Self>,
        depth: usize,
    ) {
        let card = media_card();
        if let Some(photo) = block.photo.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, photo, sender));
        }
        if let Some(author) = block.author.as_deref().filter(|author| !author.is_empty()) {
            append_label(self, &card, author, TextStyle::Subtitle);
        }
        if let Some(date) = format_date(block.date) {
            append_label(self, &card, &date, TextStyle::Caption);
        }
        self.render_blocks(&card, &block.blocks, sender, depth + 1);
        if let Some(url) = block.url.as_deref().filter(|url| safe_uri(url)) {
            card.append(&link_button(
                self.text("feature-dialog-dialog_instant_view_open_original"),
                url,
                sender,
            ));
        }
        if let Some(caption) = &block.caption {
            self.append_caption(&card, caption, sender);
        }
        parent.append(&card);
    }

    fn render_media_group(
        &self,
        parent: &gtk::Box,
        block: &BlockDto,
        sender: &ComponentSender<Self>,
        depth: usize,
    ) {
        let items = parse_blocks(&block.items);
        if items.is_empty() {
            self.unsupported(parent);
        } else if block.kind == "slideshow" {
            let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
            let stack = gtk::Stack::new();
            stack.set_vexpand(false);
            for (index, item) in items.iter().enumerate() {
                let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
                self.render_blocks(&page, std::slice::from_ref(item), sender, depth + 1);
                stack.add_titled(&page, Some(&index.to_string()), &(index + 1).to_string());
            }
            container.append(&stack);
            let switcher = gtk::StackSwitcher::new();
            switcher.set_stack(Some(&stack));
            container.append(&switcher);
            parent.append(&container);
        } else {
            let grid = gtk::Grid::new();
            grid.set_row_spacing(8);
            grid.set_column_spacing(8);
            for (index, item) in items.iter().enumerate() {
                let cell = gtk::Box::new(gtk::Orientation::Vertical, 8);
                self.render_blocks(&cell, std::slice::from_ref(item), sender, depth + 1);
                grid.attach(&cell, (index % 2) as i32, (index / 2) as i32, 1, 1);
            }
            parent.append(&grid);
        }
        if let Some(caption) = &block.caption {
            self.append_caption(parent, caption, sender);
        }
    }

    fn render_channel(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let card = media_card();
        if let Some(photo) = block.photo.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, photo, sender));
        }
        let title = string_value(block.title.as_ref()).unwrap_or_default();
        append_label(self, &card, &title, TextStyle::Subtitle);
        if let Some(username) = block.username.as_deref().filter(|username| {
            !username.is_empty()
                && username
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        }) {
            append_label(self, &card, &format!("@{username}"), TextStyle::Caption);
            card.append(&link_button(
                self.text("feature-dialog-dialog_instant_view_open_original"),
                &format!("https://t.me/{username}"),
                sender,
            ));
        }
        parent.append(&card);
    }

    fn render_related(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        if let Some(title) = rich_value(block.title.as_ref()) {
            append_rich(self, parent, &title, TextStyle::Subtitle, Some(sender));
        } else {
            append_label(
                self,
                parent,
                &self.text("feature-dialog-dialog_instant_view_related"),
                TextStyle::Subtitle,
            );
        }
        for article in &block.articles {
            let card = media_card();
            if let Some(photo) = article.photo.as_deref().filter(|key| valid_media_key(key)) {
                card.append(&media_button(self, photo, sender));
            }
            let title = article.title.as_deref().unwrap_or(&article.url);
            append_label(self, &card, title, TextStyle::Subtitle);
            let subtitle = [
                article
                    .author
                    .clone()
                    .filter(|value| !value.trim().is_empty()),
                format_date(article.published_date),
                article
                    .description
                    .clone()
                    .filter(|value| !value.trim().is_empty()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            if !subtitle.is_empty() {
                append_label(self, &card, &subtitle, TextStyle::Caption);
            }
            card.append(&link_button(title.to_owned(), &article.url, sender));
            parent.append(&card);
        }
    }

    fn render_map(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let card = media_card();
        append_label(
            self,
            &card,
            &format!("{}, {}", block.lat, block.lng),
            TextStyle::Subtitle,
        );
        if let Some(cache) = block.cache.as_deref().filter(|key| valid_media_key(key)) {
            card.append(&media_button(self, cache, sender));
        }
        let target = format!("geo:{},{}?z={}", block.lat, block.lng, block.zoom);
        if safe_uri(&target) {
            card.append(&link_button(
                self.text("feature-dialog-dialog_instant_view_map"),
                &target,
                sender,
            ));
        }
        if let Some(caption) = &block.caption {
            self.append_caption(&card, caption, sender);
        }
        parent.append(&card);
    }

    fn render_buttons(&self, parent: &gtk::Box, block: &BlockDto, sender: &ComponentSender<Self>) {
        let row = gtk::FlowBox::new();
        row.set_selection_mode(gtk::SelectionMode::None);
        row.set_column_spacing(8);
        row.set_row_spacing(8);
        for value in &block.items {
            let Ok(rich) = serde_json::from_value::<RichTextDto>(value.clone()) else {
                self.unsupported(parent);
                continue;
            };
            let href = parse_rich(&rich).and_then(|rich| first_href(&rich));
            let button = gtk::Button::with_label(&rich.text);
            self.register_dto_anchors(&rich, &button);
            button.set_sensitive(href.is_some());
            if let Some(href) = href {
                let input = sender.input_sender().clone();
                button.connect_clicked(move |_| input.emit(WebInput::OpenLink(href.clone())));
            }
            row.insert(&button, -1);
        }
        parent.append(&row);
    }

    fn append_caption(
        &self,
        parent: &gtk::Box,
        caption: &CaptionDto,
        sender: &ComponentSender<Self>,
    ) {
        if !caption.text.text.is_empty() {
            append_rich(
                self,
                parent,
                &caption.text,
                TextStyle::Caption,
                Some(sender),
            );
        }
        if let Some(credit) = &caption.credit {
            append_rich(self, parent, credit, TextStyle::Credit, Some(sender));
        }
    }

    fn unsupported(&self, parent: &gtk::Box) {
        append_label(
            self,
            parent,
            &self.text("feature-dialog-dialog_instant_view_unsupported"),
            TextStyle::Caption,
        );
    }

    fn register_anchor(&self, name: &str, widget: &impl IsA<gtk::Widget>) {
        let name = name.trim().trim_start_matches('#').to_ascii_lowercase();
        if name.is_empty() {
            return;
        }
        self.anchors
            .borrow_mut()
            .entry(name)
            .or_insert_with(|| widget.clone().upcast());
    }

    fn register_dto_anchors(&self, text: &RichTextDto, widget: &impl IsA<gtk::Widget>) {
        for entity in &text.entities {
            if entity.kind == "anchor"
                && let Some(name) = &entity.url
            {
                self.register_anchor(name, widget);
            }
        }
    }

    fn scroll_to_pending_anchor(&self) {
        let name = self.pending_anchor.borrow().clone();
        let Some(name) = name else {
            return;
        };
        let anchors = self.anchors.borrow();
        let Some(widget) = anchors.get(&name.to_ascii_lowercase()) else {
            return;
        };
        let Some(point) = widget.compute_point(&self.content, &gtk::graphene::Point::new(0.0, 0.0))
        else {
            return;
        };
        let adjustment = self.scroller.vadjustment();
        adjustment.set_value(f64::from(point.y()).max(adjustment.lower()));
        self.pending_anchor.borrow_mut().take();
    }
}

#[derive(Clone, Copy)]
enum TextStyle {
    Body,
    Title,
    Subtitle,
    Heading(u8),
    Caption,
    Credit,
    Pre,
    Quote,
    Pullquote,
    TableHeader,
}

fn parse_rich(dto: &RichTextDto) -> Option<RichText> {
    let entities = dto
        .entities
        .iter()
        .map(|entity| MarkupEntityDto {
            kind: entity.kind.clone(),
            offset: entity.offset,
            length: entity.length,
            extra: entity.url.clone(),
        })
        .collect();
    let document = Document::parse(&dto.text, entities, ParseMode::TelegramEntities).ok()?;
    document.blocks.into_iter().find_map(|block| match block {
        crate::ui::markup::Block::Paragraph(text)
        | crate::ui::markup::Block::Heading { text, .. }
        | crate::ui::markup::Block::Quote { text, .. }
        | crate::ui::markup::Block::Tasks { text, .. }
        | crate::ui::markup::Block::Details { text, .. }
        | crate::ui::markup::Block::Unsupported { text, .. } => Some(text),
        crate::ui::markup::Block::Code(code) => Some(code.text),
        crate::ui::markup::Block::Table { .. } | crate::ui::markup::Block::Rule => None,
    })
}

fn append_rich(
    view: &WebView,
    parent: &gtk::Box,
    dto: &RichTextDto,
    style: TextStyle,
    sender: Option<&ComponentSender<WebView>>,
) {
    let Some(rich) = parse_rich(dto) else {
        view.unsupported(parent);
        return;
    };
    let text = rich_text_view(view, &rich, style, sender);
    for entity in rich.entities() {
        if let EntityKind::Other { kind, extra } = &entity.kind
            && kind == "anchor"
            && let Some(name) = extra
        {
            view.register_anchor(name, &text);
        }
    }
    parent.append(&text);
}

fn rich_text_view(
    page: &WebView,
    rich: &RichText,
    style: TextStyle,
    sender: Option<&ComponentSender<WebView>>,
) -> gtk::TextView {
    let text = rich
        .selected_text(0..rich.text().chars().count(), &[])
        .unwrap_or_default();
    let view = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(if matches!(style, TextStyle::Pre) {
            gtk::WrapMode::None
        } else {
            gtk::WrapMode::WordChar
        })
        .top_margin(0)
        .bottom_margin(0)
        .left_margin(0)
        .right_margin(0)
        .build();
    view.set_direction(if page.page.as_ref().is_some_and(|page| page.rtl) {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
    view.update_property(&[gtk::accessible::Property::Label(&text)]);
    let buffer = view.buffer();
    buffer.set_text(&text);
    for entity in rich.entities() {
        let mut builder = gtk::TextTag::builder();
        builder = match &entity.kind {
            EntityKind::Bold => builder.weight(700),
            EntityKind::Italic => builder.style(gtk::pango::Style::Italic),
            EntityKind::Underline | EntityKind::Link(_) => {
                builder.underline(gtk::pango::Underline::Single)
            }
            EntityKind::Strike => builder.strikethrough(true),
            EntityKind::Code | EntityKind::Pre(_) => builder.family("monospace"),
            EntityKind::Spoiler => continue,
            EntityKind::Other { kind, .. } if kind == "anchor" => {
                builder.underline(gtk::pango::Underline::Single)
            }
            _ => continue,
        };
        let tag = builder.build();
        buffer.tag_table().add(&tag);
        buffer.apply_tag(
            &tag,
            &buffer.iter_at_offset(entity.range.characters.start as i32),
            &buffer.iter_at_offset(entity.range.characters.end as i32),
        );
    }
    let mut block = gtk::TextTag::builder();
    block = match style {
        TextStyle::Title => block.scale(1.55).weight(700),
        TextStyle::Subtitle => block.scale(1.2).weight(600),
        TextStyle::Heading(level) => block
            .scale(match level {
                1 => 1.4,
                2 => 1.25,
                3 => 1.15,
                _ => 1.05,
            })
            .weight(600),
        TextStyle::Caption | TextStyle::Credit => block.scale(0.9),
        TextStyle::Pre => block.family("monospace"),
        TextStyle::Quote | TextStyle::Pullquote => block.style(gtk::pango::Style::Italic),
        TextStyle::TableHeader => block.weight(600),
        TextStyle::Body => block,
    };
    let tag = block.build();
    buffer.tag_table().add(&tag);
    buffer.apply_tag(&tag, &buffer.start_iter(), &buffer.end_iter());
    if matches!(style, TextStyle::Caption | TextStyle::Credit) {
        view.add_css_class("dim-label");
    }
    if matches!(style, TextStyle::Pullquote) {
        view.set_justification(gtk::Justification::Center);
    }
    if let Some(sender) = sender {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        let rich = rich.clone();
        let input = sender.input_sender().clone();
        let text_view = view.clone();
        gesture.connect_released(move |_, presses, x, y| {
            if presses != 1 {
                return;
            }
            let (x, y) =
                text_view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            let Some(iter) = text_view.iter_at_location(x, y) else {
                return;
            };
            if let Some(href) = href_at(&rich, iter.offset().max(0) as usize) {
                input.emit(WebInput::OpenLink(href));
            }
        });
        view.add_controller(gesture);
    }
    view
}

fn href_at(rich: &RichText, character: usize) -> Option<String> {
    if rich.entities().iter().any(|entity| {
        entity.kind == EntityKind::Spoiler && entity.range.characters.contains(&character)
    }) {
        return None;
    }
    if let Some(target) = rich.link_at_character(character, &[]) {
        return match target {
            LinkTarget::Uri(uri) => Some(uri.clone()),
            LinkTarget::Mention(name) => {
                let name = name.trim_start_matches('@');
                (!name.is_empty()
                    && name
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
                .then(|| format!("https://t.me/{name}"))
            }
            LinkTarget::UserId(id) => (!id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()))
                .then(|| format!("tg://user?id={id}")),
            LinkTarget::Hashtag(_) | LinkTarget::Cashtag(_) | LinkTarget::BotCommand(_) => None,
        };
    }
    rich.entities()
        .iter()
        .filter(|entity| entity.range.characters.contains(&character))
        .filter_map(|entity| match &entity.kind {
            EntityKind::Other { kind, extra } if kind == "anchor" => extra.as_ref().map(|name| {
                if name.starts_with('#') {
                    name.clone()
                } else {
                    format!("#{name}")
                }
            }),
            _ => None,
        })
        .next()
}

fn first_href(rich: &RichText) -> Option<String> {
    (0..rich.text().chars().count()).find_map(|character| href_at(rich, character))
}

fn append_label(view: &WebView, parent: &gtk::Box, text: &str, style: TextStyle) -> gtk::Label {
    let rich = RichTextDto {
        text: text.to_owned(),
        entities: Vec::new(),
    };
    let Some(rich) = parse_rich(&rich) else {
        let label = gtk::Label::new(Some(text));
        parent.append(&label);
        return label;
    };
    let text = rich
        .selected_text(0..rich.text().chars().count(), &[])
        .unwrap_or_default();
    let label = gtk::Label::new(Some(&text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    match style {
        TextStyle::Title => label.add_css_class("title-1"),
        TextStyle::Subtitle => label.add_css_class("title-3"),
        TextStyle::Heading(1) => label.add_css_class("title-2"),
        TextStyle::Heading(_) => label.add_css_class("title-3"),
        TextStyle::Caption | TextStyle::Credit => label.add_css_class("dim-label"),
        TextStyle::Pre => label.add_css_class("monospace"),
        TextStyle::Body | TextStyle::Quote | TextStyle::Pullquote | TextStyle::TableHeader => {}
    }
    label.set_direction(if view.page.as_ref().is_some_and(|page| page.rtl) {
        gtk::TextDirection::Rtl
    } else {
        gtk::TextDirection::Ltr
    });
    parent.append(&label);
    label
}

fn card(orientation: gtk::Orientation, spacing: i32) -> gtk::Box {
    let card = gtk::Box::new(orientation, spacing);
    card.add_css_class("card");
    card.set_margin_start(12);
    card.set_margin_end(12);
    card.set_margin_top(10);
    card.set_margin_bottom(10);
    card
}

fn media_card() -> gtk::Box {
    card(gtk::Orientation::Vertical, 6)
}

fn link_button(label: String, href: &str, sender: &ComponentSender<WebView>) -> gtk::Button {
    let button = gtk::Button::with_label(&label);
    let href = href.to_owned();
    let input = sender.input_sender().clone();
    button.connect_clicked(move |_| input.emit(WebInput::OpenLink(href.clone())));
    button
}

fn media_button(page: &WebView, cache_key: &str, sender: &ComponentSender<WebView>) -> gtk::Button {
    let button =
        gtk::Button::with_label(&page.text("feature-dialog-dialog_instant_view_media_open"));
    let cache_key = cache_key.to_owned();
    let input = sender.input_sender().clone();
    button.connect_clicked(move |_| input.emit(WebInput::OpenMedia(cache_key.clone())));
    button
}

fn append_dimensions(view: &WebView, parent: &gtk::Box, width: i32, height: i32) {
    if width <= 0 || height <= 0 {
        return;
    }
    let dimensions = view
        .i18n
        .text(
            "core-ui-media_info_dimensions_value",
            &[("arg1", &width.to_string()), ("arg2", &height.to_string())],
        )
        .unwrap_or_else(|_| format!("{width} × {height}"));
    append_label(view, parent, &dimensions, TextStyle::Caption);
}

fn format_file_size(bytes: Option<i64>) -> Option<String> {
    let bytes = bytes.filter(|bytes| *bytes > 0)?;
    Some(if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    })
}

fn format_date(epoch: Option<i32>) -> Option<String> {
    let epoch = epoch.filter(|epoch| *epoch > 0)?;
    gtk::glib::DateTime::from_unix_utc(i64::from(epoch))
        .ok()?
        .format("%Y-%m-%d")
        .ok()
        .map(|date| date.to_string())
}

fn display_host(page: &InstantViewPage) -> Option<&str> {
    let raw = if page.display_url.trim().is_empty() {
        page.url.as_str()
    } else {
        page.display_url.as_str()
    };
    raw.split_once("://")
        .map_or(raw, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .filter(|host| !host.is_empty())
}

fn should_show_page_title(page: &InstantViewPage) -> bool {
    let Some(title) = page
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    else {
        return false;
    };
    !page.blocks.iter().any(|block| match block.kind.as_str() {
        "title" => block.text.text.trim().eq_ignore_ascii_case(title),
        "heading" if block.level <= 1 => block.text.text.trim().eq_ignore_ascii_case(title),
        "cover" => block.block.as_ref().is_some_and(|inner| {
            matches!(inner.kind.as_str(), "title" | "heading")
                && inner.text.text.trim().eq_ignore_ascii_case(title)
        }),
        _ => false,
    })
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/web_ui.rs"]
pub(crate) mod tests;
