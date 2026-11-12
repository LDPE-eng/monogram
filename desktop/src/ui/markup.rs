use std::ops::Range;

use monogram_markup::{MarkupBlockDto, MarkupEntityDto};
use relm4::gtk::pango;

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_ENTITIES: usize = 4096;
pub const MAX_BLOCKS: usize = 4096;
pub const MAX_MATH_BYTES: usize = 4096;
pub const MAX_MATH_SPANS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    TextTooLarge,
    TooManyEntities,
    TooManyBlocks,
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for InputError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    TelegramEntities,
    Markdown,
}

/// Character offsets count Unicode scalar values, as GTK TextIter does, not graphemes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRange {
    pub bytes: Range<usize>,
    pub characters: Range<usize>,
    pub utf16: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct TextIndex {
    boundaries: Vec<(usize, usize)>,
}

impl TextIndex {
    pub fn new(text: &str) -> Self {
        let mut units = 0;
        let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
        for (byte, ch) in text.char_indices() {
            boundaries.push((byte, units));
            units += ch.len_utf16();
        }
        boundaries.push((text.len(), units));
        Self { boundaries }
    }

    pub fn utf16_range(&self, offset: i32, length: i32) -> Option<TextRange> {
        if offset < 0 || length <= 0 {
            return None;
        }
        let end = offset.checked_add(length)?;
        let from = self
            .boundaries
            .binary_search_by_key(&(offset as usize), |b| b.1)
            .ok()?;
        let to = self
            .boundaries
            .binary_search_by_key(&(end as usize), |b| b.1)
            .ok()?;
        self.character_range(from..to)
    }

    pub fn character_range(&self, range: Range<usize>) -> Option<TextRange> {
        if range.start > range.end {
            return None;
        }
        let from = self.boundaries.get(range.start)?;
        let to = self.boundaries.get(range.end)?;
        Some(TextRange {
            bytes: from.0..to.0,
            characters: range,
            utf16: from.1..to.1,
        })
    }

    pub fn byte_range(&self, range: Range<usize>) -> Option<TextRange> {
        let from = self
            .boundaries
            .binary_search_by_key(&range.start, |b| b.0)
            .ok()?;
        let to = self
            .boundaries
            .binary_search_by_key(&range.end, |b| b.0)
            .ok()?;
        self.character_range(from..to)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Uri(String),
    Mention(String),
    UserId(String),
    Hashtag(String),
    Cashtag(String),
    BotCommand(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Pre(Option<String>),
    Spoiler,
    Link(LinkTarget),
    CustomEmoji(Option<String>),
    Quote { collapsed: bool },
    Other { kind: String, extra: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub range: TextRange,
    pub kind: EntityKind,
}

/// No TeX engine is linked. SourceOnly is deliberately not a rendered formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathStatus {
    SourceOnly,
    SourceTooLarge,
    SpanLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathSource {
    pub range: TextRange,
    pub source: String,
    pub display: bool,
    pub status: MathStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichText {
    text: String,
    entities: Vec<Entity>,
    math: Vec<MathSource>,
    rejected_entities: usize,
}

impl RichText {
    fn new(text: String, entities: Vec<MarkupEntityDto>, math: bool) -> Self {
        let index = TextIndex::new(&text);
        let count = entities.len();
        let entities: Vec<_> = entities
            .into_iter()
            .filter_map(|entity| {
                let range = index.utf16_range(entity.offset, entity.length)?;
                let body = &text[range.bytes.clone()];
                Some(Entity {
                    range,
                    kind: entity_kind(entity.kind, entity.extra, body),
                })
            })
            .collect();
        let rejected_entities = count - entities.len();
        let math = if math {
            monogram_markup::extract_math(text.clone())
                .into_iter()
                .filter_map(|span| {
                    let range = index.utf16_range(span.start, span.end.checked_sub(span.start)?)?;
                    if entities.iter().any(|entity| {
                        matches!(entity.kind, EntityKind::Code | EntityKind::Pre(_))
                            && overlaps(&entity.range.bytes, &range.bytes)
                    }) {
                        return None;
                    }
                    Some((range, span))
                })
                .enumerate()
                .map(|(ordinal, (range, span))| MathSource {
                    status: if span.source.len() > MAX_MATH_BYTES {
                        MathStatus::SourceTooLarge
                    } else if ordinal >= MAX_MATH_SPANS {
                        MathStatus::SpanLimit
                    } else {
                        MathStatus::SourceOnly
                    },
                    range,
                    source: span.source,
                    display: span.display,
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            text,
            entities,
            math,
            rejected_entities,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }
    pub fn math(&self) -> &[MathSource] {
        &self.math
    }
    pub fn rejected_entities(&self) -> usize {
        self.rejected_entities
    }

    fn concealed_at(&self, character: usize, revealed: &[usize]) -> bool {
        self.entities.iter().enumerate().any(|(id, entity)| {
            entity.kind == EntityKind::Spoiler
                && !revealed.contains(&id)
                && entity.range.characters.contains(&character)
        })
    }

    /// Use this for clipboard/accessibility text; raw GTK label selection exposes spoilers.
    pub fn selected_text(&self, characters: Range<usize>, revealed: &[usize]) -> Option<String> {
        let range = TextIndex::new(&self.text).character_range(characters.clone())?;
        Some(
            self.text[range.bytes]
                .chars()
                .enumerate()
                .map(|(offset, ch)| {
                    if self.concealed_at(characters.start + offset, revealed) && !ch.is_whitespace()
                    {
                        '•'
                    } else {
                        ch
                    }
                })
                .collect(),
        )
    }

    pub fn selected_entities(&self, characters: Range<usize>) -> Option<Vec<Entity>> {
        let index = TextIndex::new(&self.text);
        let selected = index.character_range(characters)?;
        Some(
            self.entities
                .iter()
                .filter_map(|entity| {
                    let start = entity.range.characters.start.max(selected.characters.start);
                    let end = entity.range.characters.end.min(selected.characters.end);
                    if start >= end {
                        return None;
                    }
                    let range = index.character_range(start..end)?;
                    Some(Entity {
                        range: TextRange {
                            bytes: range.bytes.start - selected.bytes.start
                                ..range.bytes.end - selected.bytes.start,
                            characters: start - selected.characters.start
                                ..end - selected.characters.start,
                            utf16: range.utf16.start - selected.utf16.start
                                ..range.utf16.end - selected.utf16.start,
                        },
                        kind: entity.kind.clone(),
                    })
                })
                .collect(),
        )
    }

    pub fn link_at_character(&self, character: usize, revealed: &[usize]) -> Option<&LinkTarget> {
        if self.concealed_at(character, revealed) {
            return None;
        }
        self.entities
            .iter()
            .filter(|entity| entity.range.characters.contains(&character))
            .filter_map(|entity| match &entity.kind {
                EntityKind::Link(target) => Some((entity.range.characters.len(), target)),
                _ => None,
            })
            .min_by_key(|(length, _)| *length)
            .map(|(_, target)| target)
    }

    /// Attributes apply to plain text, never to untrusted Pango markup strings.
    pub fn pango_attributes(&self, revealed: &[usize]) -> pango::AttrList {
        let list = pango::AttrList::new();
        for (id, entity) in self.entities.iter().enumerate() {
            let attribute: Option<pango::Attribute> = match &entity.kind {
                EntityKind::Bold => Some(pango::AttrInt::new_weight(pango::Weight::Bold).into()),
                EntityKind::Italic => Some(pango::AttrInt::new_style(pango::Style::Italic).into()),
                EntityKind::Underline | EntityKind::Link(_) => {
                    Some(pango::AttrInt::new_underline(pango::Underline::Single).into())
                }
                EntityKind::Strike => Some(pango::AttrInt::new_strikethrough(true).into()),
                EntityKind::Code | EntityKind::Pre(_) => {
                    Some(pango::AttrString::new_family("monospace").into())
                }
                EntityKind::Spoiler if !revealed.contains(&id) => {
                    Some(pango::AttrInt::new_foreground_alpha(0).into())
                }
                _ => None,
            };
            if let Some(attribute) = attribute {
                insert_attribute(&list, attribute, &entity.range);
            }
        }
        list
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Highlight {
    pub range: TextRange,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    pub text: RichText,
    pub language: Option<String>,
    pub highlights: Vec<Highlight>,
}

impl CodeBlock {
    pub fn pango_attributes(
        &self,
        color: impl Fn(&str) -> Option<(u16, u16, u16)>,
    ) -> pango::AttrList {
        let list = self.text.pango_attributes(&[]);
        for highlight in &self.highlights {
            if let Some((r, g, b)) = color(&highlight.scope) {
                insert_attribute(
                    &list,
                    pango::AttrColor::new_foreground(r, g, b).into(),
                    &highlight.range,
                );
            }
        }
        list
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub checked: bool,
    pub line: TextRange,
    pub content: TextRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(RichText),
    Heading {
        level: u8,
        text: RichText,
    },
    Code(CodeBlock),
    Quote {
        depth: usize,
        collapsed: bool,
        text: RichText,
    },
    Table {
        headers: Vec<RichText>,
        rows: Vec<Vec<RichText>>,
    },
    Tasks {
        text: RichText,
        items: Vec<Task>,
    },
    Rule,
    Details {
        text: RichText,
        summary: TextRange,
        body: TextRange,
    },
    Unsupported {
        kind: String,
        text: RichText,
        extra: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn parse(
        text: &str,
        entities: Vec<MarkupEntityDto>,
        mode: ParseMode,
    ) -> Result<Self, InputError> {
        check_size(text.len(), entities.len())?;
        check_size(entity_bytes(&entities), entities.len())?;
        // Server entities are authoritative, even when the body happens to contain HTML.
        let markdown = mode == ParseMode::Markdown && entities.is_empty();
        let blocks = monogram_markup::render_blocks(text.to_owned(), entities, markdown);
        Self::convert(blocks, markdown)
    }

    /// DTO text is already normalized; unlike Markdown input, table cells remain literal.
    pub fn from_blocks(blocks: Vec<MarkupBlockDto>) -> Result<Self, InputError> {
        Self::convert(blocks, false)
    }

    fn convert(blocks: Vec<MarkupBlockDto>, markdown: bool) -> Result<Self, InputError> {
        if blocks.len() > MAX_BLOCKS {
            return Err(InputError::TooManyBlocks);
        }
        let mut bytes = 0usize;
        let mut metadata_bytes = 0usize;
        let mut entities = 0usize;
        for block in &blocks {
            bytes = bytes
                .saturating_add(block.text.len())
                .saturating_add(block.rows.len());
            metadata_bytes = metadata_bytes
                .saturating_add(block.kind.len())
                .saturating_add(block.language.as_ref().map_or(0, String::len))
                .saturating_add(entity_bytes(&block.entities));
            entities = entities.saturating_add(block.entities.len());
            for cell in block.headers.iter().chain(block.rows.iter().flatten()) {
                bytes = bytes.saturating_add(cell.len()).saturating_add(1);
            }
            check_size(bytes, entities)?;
            check_size(metadata_bytes, entities)?;
        }
        Ok(Self {
            blocks: blocks
                .into_iter()
                .map(|block| convert_block(block, markdown))
                .collect(),
        })
    }
}

fn entity_bytes(entities: &[MarkupEntityDto]) -> usize {
    entities.iter().fold(0usize, |bytes, entity| {
        bytes
            .saturating_add(entity.kind.len())
            .saturating_add(entity.extra.as_ref().map_or(0, String::len))
    })
}

fn check_size(bytes: usize, entities: usize) -> Result<(), InputError> {
    if bytes > MAX_INPUT_BYTES {
        return Err(InputError::TextTooLarge);
    }
    if entities > MAX_ENTITIES {
        return Err(InputError::TooManyEntities);
    }
    Ok(())
}

fn convert_block(mut block: MarkupBlockDto, markdown: bool) -> Block {
    if block.kind == "rule" {
        return Block::Rule;
    }
    if block.kind == "table" {
        let cell = |value| {
            if markdown {
                let parsed = monogram_markup::parse_telegram_markdown(value);
                RichText::new(parsed.text, parsed.entities, true)
            } else {
                RichText::new(value, Vec::new(), true)
            }
        };
        return Block::Table {
            headers: block.headers.into_iter().map(cell).collect(),
            rows: block
                .rows
                .into_iter()
                .map(|row| row.into_iter().map(cell).collect())
                .collect(),
        };
    }
    if block.kind == "details" && markdown && block.entities.is_empty() {
        let parsed = monogram_markup::parse_telegram_markdown(block.text);
        block.text = parsed.text;
        block.entities = parsed.entities;
    }
    if block.kind == "code" {
        let index = TextIndex::new(&block.text);
        let highlights = monogram_markup::highlight_code(
            block.text.clone(),
            block.language.clone().unwrap_or_default(),
        )
        .into_iter()
        .filter_map(|span| {
            Some(Highlight {
                range: index.utf16_range(span.start, span.end.checked_sub(span.start)?)?,
                scope: span.scope,
            })
        })
        .collect();
        let length = block.text.encode_utf16().count() as i32;
        let entities = if length == 0 {
            vec![]
        } else {
            vec![MarkupEntityDto {
                kind: "pre".into(),
                offset: 0,
                length,
                extra: block.language.clone(),
            }]
        };
        return Block::Code(CodeBlock {
            text: RichText::new(block.text, entities, false),
            language: block.language,
            highlights,
        });
    }
    let text = RichText::new(block.text, block.entities, true);
    match block.kind.as_str() {
        "paragraph" => {
            let items = if markdown {
                tasks(&text.text)
            } else {
                Vec::new()
            };
            if items.is_empty() {
                Block::Paragraph(text)
            } else {
                Block::Tasks { text, items }
            }
        }
        "heading" => Block::Heading {
            level: block.level.clamp(1, 6) as u8,
            text,
        },
        "quote" => Block::Quote {
            depth: block.level.max(1) as usize,
            collapsed: block.language.as_deref() == Some("collapsed"),
            text,
        },
        "tasks" => {
            let items = tasks(&text.text);
            Block::Tasks { text, items }
        }
        "details" => {
            let split = text.text.find('\n').unwrap_or(text.text.len());
            let body_start = (split + 1).min(text.text.len());
            let index = TextIndex::new(&text.text);
            let summary = index.byte_range(0..split).unwrap();
            let body = index.byte_range(body_start..text.text.len()).unwrap();
            Block::Details {
                text,
                summary,
                body,
            }
        }
        _ => Block::Unsupported {
            kind: block.kind,
            text,
            extra: block.language,
        },
    }
}

fn tasks(text: &str) -> Vec<Task> {
    let index = TextIndex::new(text);
    let mut start = 0;
    let mut items = Vec::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start();
        let trimmed = trimmed.strip_prefix("• ").unwrap_or(trimmed);
        let marker = ["[x]", "[X]", "[ ]", "☑", "☐"].into_iter().find(|marker| {
            trimmed
                .strip_prefix(marker)
                .is_some_and(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
        });
        if let Some(marker) = marker {
            let content = trimmed[marker.len()..].trim_start();
            let content_start = start + body.len() - content.len();
            items.push(Task {
                checked: matches!(marker, "[x]" | "[X]" | "☑"),
                line: index.byte_range(start..start + body.len()).unwrap(),
                content: index.byte_range(content_start..start + body.len()).unwrap(),
            });
        }
        start += line.len();
    }
    items
}

fn entity_kind(kind: String, extra: Option<String>, body: &str) -> EntityKind {
    match kind.as_str() {
        "bold" => EntityKind::Bold,
        "italic" => EntityKind::Italic,
        "underline" => EntityKind::Underline,
        "strike" | "strikethrough" => EntityKind::Strike,
        "code" => EntityKind::Code,
        "pre" => EntityKind::Pre(extra),
        "spoiler" => EntityKind::Spoiler,
        "custom_emoji" => EntityKind::CustomEmoji(extra),
        "blockquote" | "expandable_blockquote" => EntityKind::Quote {
            collapsed: kind == "expandable_blockquote" || extra.as_deref() == Some("collapsed"),
        },
        "mention" => EntityKind::Link(LinkTarget::Mention(body.into())),
        "mention_name" | "text_mention" => match extra {
            Some(id) => EntityKind::Link(LinkTarget::UserId(id)),
            None => EntityKind::Other { kind, extra: None },
        },
        "hashtag" => EntityKind::Link(LinkTarget::Hashtag(body.into())),
        "cashtag" => EntityKind::Link(LinkTarget::Cashtag(body.into())),
        "bot_command" => EntityKind::Link(LinkTarget::BotCommand(body.into())),
        "url" | "text_url" | "email" | "phone" => {
            let value = match kind.as_str() {
                "text_url" => extra.as_deref().unwrap_or(""),
                _ => body,
            };
            let uri = if kind == "email" {
                format!("mailto:{value}")
            } else if kind == "phone" {
                format!("tel:{}", value.replace([' ', '-', '(', ')'], ""))
            } else if kind == "url" && !value.contains(':') {
                format!("https://{value}")
            } else {
                value.to_owned()
            };
            if safe_uri(&uri) {
                EntityKind::Link(LinkTarget::Uri(uri))
            } else {
                EntityKind::Other { kind, extra }
            }
        }
        _ => EntityKind::Other { kind, extra },
    }
}

fn safe_uri(uri: &str) -> bool {
    if uri.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return false;
    }
    let Some((scheme, target)) = uri.split_once(':') else {
        return false;
    };
    if target.is_empty() {
        return false;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "https" | "http" | "tg" => target
            .strip_prefix("//")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with(['/', '?', '#'])),
        "mailto" | "tel" => true,
        _ => false,
    }
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn insert_attribute(list: &pango::AttrList, mut attribute: pango::Attribute, range: &TextRange) {
    attribute.set_start_index(range.bytes.start as u32);
    attribute.set_end_index(range.bytes.end as u32);
    list.insert(attribute);
}

#[cfg(test)]
#[path = "../../tests/unit/markup.rs"]
mod tests;
