use super::*;

fn entity(kind: &str, offset: i32, length: i32, extra: Option<&str>) -> MarkupEntityDto {
    MarkupEntityDto {
        kind: kind.into(),
        offset,
        length,
        extra: extra.map(str::to_owned),
    }
}

fn block(kind: &str, text: &str, entities: Vec<MarkupEntityDto>) -> MarkupBlockDto {
    MarkupBlockDto {
        kind: kind.into(),
        text: text.into(),
        entities,
        language: None,
        level: 0,
        headers: Vec::new(),
        rows: Vec::new(),
    }
}

fn paragraph(document: &Document) -> &RichText {
    let Block::Paragraph(text) = &document.blocks[0] else {
        panic!("expected paragraph")
    };
    text
}

#[test]
fn utf16_byte_and_gtk_character_offsets_agree_after_emoji() {
    let index = TextIndex::new("a😀e\u{301}👩\u{200d}💻z");
    assert_eq!(
        index.utf16_range(1, 2),
        Some(TextRange {
            bytes: 1..5,
            characters: 1..2,
            utf16: 1..3,
        })
    );
    assert_eq!(
        index.utf16_range(5, 5),
        Some(TextRange {
            bytes: 8..19,
            characters: 4..7,
            utf16: 5..10,
        })
    );
    assert_eq!(index.byte_range(8..19), index.character_range(4..7));
    assert_eq!(index.character_range(8..8).unwrap().bytes, 20..20);
}

#[test]
fn invalid_surrogate_boundaries_and_overflow_are_rejected_not_rounded() {
    let index = TextIndex::new("😀x");
    for (offset, length) in [
        (-1, 2),
        (0, 1),
        (1, 1),
        (1, 2),
        (0, 4),
        (3, 1),
        (0, 0),
        (0, -1),
        (i32::MAX, 1),
    ] {
        assert_eq!(index.utf16_range(offset, length), None, "{offset}:{length}");
    }
    assert!(index.byte_range(1..4).is_none());
    assert!(index.character_range(Range { start: 2, end: 1 }).is_none());
    let rich = RichText::new(
        "😀x".into(),
        vec![
            entity("bold", 0, 1, None),
            entity("italic", 1, 1, None),
            entity("bold", 0, 2, None),
        ],
        true,
    );
    assert_eq!(rich.rejected_entities(), 2);
    assert_eq!(rich.entities().len(), 1);
}

#[test]
fn every_scalar_boundary_round_trips_in_mixed_unicode() {
    let text = "Русский 😀\n日本語 e\u{301}👨\u{200d}👩\u{200d}👧 🇷🇺";
    let index = TextIndex::new(text);
    for start in 0..=text.chars().count() {
        for end in start..=text.chars().count() {
            let range = index.character_range(start..end).unwrap();
            assert_eq!(index.byte_range(range.bytes.clone()), Some(range.clone()));
            if start != end {
                assert_eq!(
                    index.utf16_range(range.utf16.start as i32, range.utf16.len() as i32),
                    Some(range)
                );
            }
        }
    }
}

#[test]
fn markdown_emoji_entities_reach_real_pango_with_byte_offsets() {
    let document = Document::parse("👋 **bold __斜体__**", vec![], ParseMode::Markdown).unwrap();
    let rich = paragraph(&document);
    assert!(rich.text().starts_with("👋 bold 斜体"));
    let bold = rich
        .entities()
        .iter()
        .find(|e| e.kind == EntityKind::Bold)
        .unwrap();
    assert_eq!(bold.range.bytes, 5..16);
    assert_eq!(bold.range.characters, 2..9);
    let attrs = rich.pango_attributes(&[]);
    let attributes: Vec<_> = attrs.iterator().into_iter().flatten().collect();
    let weight = attributes
        .iter()
        .find(|a| a.type_() == pango::AttrType::Weight)
        .unwrap();
    assert_eq!((weight.start_index(), weight.end_index()), (5, 16));
    assert!(
        !attributes
            .iter()
            .any(|a| a.type_() == pango::AttrType::ForegroundAlpha)
    );
}

#[test]
fn server_entities_do_not_reparse_markdown_or_html() {
    let source = "**literal** <details><summary>x</summary>y</details>";
    let document = Document::parse(
        source,
        vec![entity("bold", 2, 7, None)],
        ParseMode::Markdown,
    )
    .unwrap();
    assert_eq!(document.blocks.len(), 1);
    assert_eq!(paragraph(&document).text(), source);
    assert_eq!(paragraph(&document).entities()[0].range.bytes, 2..9);
    let plain = Document::parse("[x] literal $x$", vec![], ParseMode::TelegramEntities).unwrap();
    assert!(matches!(plain.blocks[0], Block::Paragraph(_)));
}

#[test]
fn selection_clips_and_rebases_overlapping_entities_in_all_three_units() {
    let rich = RichText::new(
        "a😀bc界".into(),
        vec![entity("bold", 1, 4, None), entity("italic", 3, 3, None)],
        true,
    );
    assert_eq!(rich.selected_text(2..5, &[]).as_deref(), Some("bc界"));
    let selected = rich.selected_entities(2..5).unwrap();
    assert_eq!(
        selected[0].range,
        TextRange {
            bytes: 0..2,
            characters: 0..2,
            utf16: 0..2
        }
    );
    assert_eq!(
        selected[1].range,
        TextRange {
            bytes: 0..5,
            characters: 0..3,
            utf16: 0..3
        }
    );
    assert_eq!(rich.selected_text(1..2, &[]).as_deref(), Some("😀"));
    assert_eq!(rich.selected_text(5..5, &[]).as_deref(), Some(""));
    assert!(rich.selected_text(0..6, &[]).is_none());
}

#[test]
fn spoiler_selection_and_links_require_explicit_reveal_of_every_covering_spoiler() {
    let rich = RichText::new(
        "😀secret".into(),
        vec![
            entity("spoiler", 0, 8, None),
            entity("spoiler", 2, 6, None),
            entity("text_url", 2, 6, Some("https://example.org/?a=1&b=2")),
        ],
        true,
    );
    assert_eq!(rich.selected_text(0..7, &[]).as_deref(), Some("•••••••"));
    assert_eq!(rich.selected_text(0..7, &[0]).as_deref(), Some("😀••••••"));
    assert_eq!(
        rich.selected_text(0..7, &[0, 1]).as_deref(),
        Some("😀secret")
    );
    assert!(rich.link_at_character(1, &[]).is_none());
    assert!(rich.link_at_character(1, &[0]).is_none());
    assert!(matches!(
        rich.link_at_character(1, &[0, 1]),
        Some(LinkTarget::Uri(_))
    ));
    assert!(rich.link_at_character(7, &[0, 1]).is_none());
    assert!(
        rich.pango_attributes(&[])
            .iterator()
            .attrs()
            .iter()
            .any(|a| a.type_() == pango::AttrType::ForegroundAlpha)
    );
    assert!(
        !rich
            .pango_attributes(&[0, 1])
            .iterator()
            .attrs()
            .iter()
            .any(|a| a.type_() == pango::AttrType::ForegroundAlpha)
    );
}

#[test]
fn dangerous_link_schemes_are_preserved_as_non_actionable_entities() {
    for uri in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "https://",
        "https:///bad",
        "https://x\ny",
        "https://x\u{0}y",
    ] {
        let rich = RichText::new(
            "link".into(),
            vec![entity("text_url", 0, 4, Some(uri))],
            false,
        );
        assert!(rich.link_at_character(0, &[]).is_none(), "{uri}");
        assert!(
            matches!(&rich.entities()[0].kind, EntityKind::Other { kind, extra } if kind == "text_url" && extra.as_deref() == Some(uri))
        );
    }
    for uri in [
        "https://example.org/<&\"",
        "HTTPS://example.org",
        "tg://resolve?domain=test",
        "mailto:test@example.org",
    ] {
        assert!(safe_uri(uri), "{uri}");
    }
}

#[test]
fn telegram_entity_targets_and_unknown_metadata_survive_conversion() {
    let rich = RichText::new(
        "@user".into(),
        vec![
            entity("mention", 0, 5, None),
            entity("mention_name", 0, 5, Some("42")),
            entity("custom_emoji", 0, 5, Some("123")),
            entity("future", 0, 5, Some("metadata")),
            entity("strikethrough", 0, 5, None),
        ],
        false,
    );
    assert_eq!(
        rich.entities()[0].kind,
        EntityKind::Link(LinkTarget::Mention("@user".into()))
    );
    assert_eq!(
        rich.entities()[1].kind,
        EntityKind::Link(LinkTarget::UserId("42".into()))
    );
    assert_eq!(
        rich.entities()[2].kind,
        EntityKind::CustomEmoji(Some("123".into()))
    );
    assert_eq!(
        rich.entities()[3].kind,
        EntityKind::Other {
            kind: "future".into(),
            extra: Some("metadata".into())
        }
    );
    assert_eq!(rich.entities()[4].kind, EntityKind::Strike);
    for (kind, body, expected) in [
        ("email", "a@b.org", "mailto:a@b.org"),
        ("phone", "+1 (23)-45", "tel:+12345"),
        ("url", "example.org", "https://example.org"),
    ] {
        let rich = RichText::new(
            body.into(),
            vec![entity(kind, 0, body.encode_utf16().count() as i32, None)],
            false,
        );
        assert_eq!(
            rich.link_at_character(0, &[]),
            Some(&LinkTarget::Uri(expected.into()))
        );
    }
}

#[test]
fn native_blocks_cover_headings_code_quotes_tables_rules_and_details() {
    let source = "# 👋 Title\n\n```rust\nlet emoji = \"😀\";\n```\n\n> quote\n\n| A | B |\n| --- | --- |\n| **one** | 😀 |\n\n---\n\n<details><summary>Summary</summary>**body**</details>\n";
    let document = Document::parse(source, vec![], ParseMode::Markdown).unwrap();
    assert_eq!(document.blocks.len(), 6);
    assert!(
        matches!(&document.blocks[0], Block::Heading { level: 1, text } if text.text() == "👋 Title")
    );
    let Block::Code(code) = &document.blocks[1] else {
        panic!("code")
    };
    assert_eq!(code.language.as_deref(), Some("rust"));
    assert_eq!(code.text.text(), "let emoji = \"😀\";");
    assert!(!code.highlights.is_empty());
    for span in &code.highlights {
        assert_eq!(
            TextIndex::new(code.text.text()).byte_range(span.range.bytes.clone()),
            Some(span.range.clone())
        );
    }
    let attrs = code.pango_attributes(|_| Some((1000, 2000, 3000)));
    assert!(
        attrs
            .iterator()
            .attrs()
            .iter()
            .any(|a| a.type_() == pango::AttrType::Family)
    );
    assert!(
        matches!(&document.blocks[2], Block::Quote { depth: 1, collapsed: false, text } if text.text().trim() == "quote")
    );
    let Block::Table { headers, rows } = &document.blocks[3] else {
        panic!("table")
    };
    assert_eq!(headers[0].text(), "A");
    assert_eq!(rows[0][0].text(), "one");
    assert_eq!(rows[0][0].entities()[0].kind, EntityKind::Bold);
    assert_eq!(rows[0][1].text(), "😀");
    assert!(matches!(document.blocks[4], Block::Rule));
    let Block::Details {
        text,
        summary,
        body,
    } = &document.blocks[5]
    else {
        panic!("details")
    };
    assert_eq!(&text.text()[summary.bytes.clone()], "Summary");
    assert_eq!(&text.text()[body.bytes.clone()], "body");
    assert_eq!(text.entities()[0].kind, EntityKind::Bold);
}

#[test]
fn details_keep_entity_offsets_and_empty_body_is_valid() {
    let document = Document::from_blocks(vec![block(
        "details",
        "😀 title\nsecret",
        vec![entity("spoiler", 9, 6, None)],
    )])
    .unwrap();
    let Block::Details {
        text,
        summary,
        body,
    } = &document.blocks[0]
    else {
        panic!("details")
    };
    assert_eq!(summary.bytes, 0..10);
    assert_eq!(summary.characters, 0..7);
    assert_eq!(body.bytes, 11..17);
    assert_eq!(text.entities()[0].range.bytes, 11..17);
    let document = Document::from_blocks(vec![block("details", "😀", vec![])]).unwrap();
    let Block::Details { summary, body, .. } = &document.blocks[0] else {
        panic!("details")
    };
    assert_eq!(summary.bytes, 0..4);
    assert_eq!(body.bytes, 4..4);
}

#[test]
fn tasks_retain_check_state_entities_and_unicode_selection_ranges() {
    let document =
        Document::parse("- [x] **😀 done**\n- [ ] next", vec![], ParseMode::Markdown).unwrap();
    let Block::Tasks { text, items } = &document.blocks[0] else {
        panic!("tasks: {:?}", document.blocks)
    };
    assert_eq!(items.len(), 2);
    assert!(items[0].checked);
    assert!(!items[1].checked);
    assert_eq!(&text.text()[items[0].content.bytes.clone()], "😀 done");
    assert_eq!(&text.text()[items[1].content.bytes.clone()], "next");
    assert!(text.entities().iter().any(|e| e.kind == EntityKind::Bold));
}

#[test]
fn quote_depth_collapsing_and_unsupported_blocks_are_explicit() {
    let mut quote = block("quote", "nested", vec![]);
    quote.level = 3;
    quote.language = Some("collapsed".into());
    let document = Document::from_blocks(vec![
        quote,
        block("photo", "", vec![]),
        block("heading", "title", vec![]),
    ])
    .unwrap();
    assert!(matches!(
        document.blocks[0],
        Block::Quote {
            depth: 3,
            collapsed: true,
            ..
        }
    ));
    assert!(matches!(&document.blocks[1], Block::Unsupported { kind, .. } if kind == "photo"));
    assert!(matches!(
        document.blocks[2],
        Block::Heading { level: 1, .. }
    ));
}

#[test]
fn math_is_source_only_utf16_correct_and_excludes_code_entities() {
    let document =
        Document::parse("😀 $x^2$ and $$y$$", vec![], ParseMode::TelegramEntities).unwrap();
    let text = paragraph(&document);
    assert_eq!(text.math().len(), 2);
    assert_eq!(text.math()[0].range.bytes, 5..10);
    assert_eq!(text.math()[0].range.characters, 2..7);
    assert_eq!(text.math()[0].range.utf16, 3..8);
    assert_eq!(text.math()[0].source, "x^2");
    assert_eq!(text.math()[0].status, MathStatus::SourceOnly);
    assert!(!text.math()[0].display);
    assert!(text.math()[1].display);
    let document = Document::parse(
        "$x$ $y$",
        vec![entity("code", 0, 3, None)],
        ParseMode::TelegramEntities,
    )
    .unwrap();
    assert_eq!(paragraph(&document).math().len(), 1);
    assert_eq!(paragraph(&document).math()[0].source, "y");
    let document = Document::parse("```text\n$x$\n```", vec![], ParseMode::Markdown).unwrap();
    let Block::Code(code) = &document.blocks[0] else {
        panic!("code")
    };
    assert!(code.text.math().is_empty());
}

#[test]
fn formula_size_and_count_limits_never_claim_rendering() {
    let raw = format!("${}$", "x".repeat(MAX_MATH_BYTES + 1));
    let document = Document::parse(&raw, vec![], ParseMode::TelegramEntities).unwrap();
    assert_eq!(
        paragraph(&document).math()[0].status,
        MathStatus::SourceTooLarge
    );
    let raw = "$x$ ".repeat(MAX_MATH_SPANS + 1);
    let document = Document::parse(&raw, vec![], ParseMode::TelegramEntities).unwrap();
    assert_eq!(
        paragraph(&document).math()[MAX_MATH_SPANS].status,
        MathStatus::SpanLimit
    );
    assert!(
        paragraph(&document).math()[..MAX_MATH_SPANS]
            .iter()
            .all(|s| s.status == MathStatus::SourceOnly)
    );
}

#[test]
fn input_limits_are_explicit_errors_and_do_not_silently_drop_formatting() {
    assert_eq!(
        Document::parse(
            &"😀".repeat(MAX_INPUT_BYTES / 4 + 1),
            vec![],
            ParseMode::Markdown
        ),
        Err(InputError::TextTooLarge)
    );
    assert!(
        Document::parse(
            &"x".repeat(MAX_INPUT_BYTES),
            vec![],
            ParseMode::TelegramEntities
        )
        .is_ok()
    );
    assert_eq!(
        Document::parse(
            "x",
            vec![entity("bold", 0, 1, None); MAX_ENTITIES + 1],
            ParseMode::TelegramEntities
        ),
        Err(InputError::TooManyEntities)
    );
    assert_eq!(
        Document::from_blocks(vec![block("rule", "", vec![]); MAX_BLOCKS + 1]),
        Err(InputError::TooManyBlocks)
    );
    let mut table = block("table", "", vec![]);
    table.headers = vec!["x".repeat(MAX_INPUT_BYTES + 1)];
    assert_eq!(
        Document::from_blocks(vec![table]),
        Err(InputError::TextTooLarge)
    );
    let oversized = "x".repeat(MAX_INPUT_BYTES + 1);
    assert_eq!(
        Document::parse(
            "x",
            vec![entity("text_url", 0, 1, Some(&oversized))],
            ParseMode::TelegramEntities
        ),
        Err(InputError::TextTooLarge)
    );
    let mut code = block("code", "x", vec![]);
    code.language = Some(oversized);
    assert_eq!(
        Document::from_blocks(vec![code]),
        Err(InputError::TextTooLarge)
    );
    let mut table = block("table", "", vec![]);
    table.rows = vec![vec![]; MAX_INPUT_BYTES + 1];
    assert_eq!(
        Document::from_blocks(vec![table]),
        Err(InputError::TextTooLarge)
    );
}

#[test]
fn server_table_cells_remain_literal_while_markdown_tables_parse_inline_styles() {
    let mut table = block("table", "", vec![]);
    table.headers = vec!["**literal**".into()];
    table.rows = vec![vec!["😀 __literal__".into()]];
    let document = Document::from_blocks(vec![table]).unwrap();
    let Block::Table { headers, rows } = &document.blocks[0] else {
        panic!("table")
    };
    assert_eq!(headers[0].text(), "**literal**");
    assert_eq!(rows[0][0].text(), "😀 __literal__");
    assert!(headers[0].entities().is_empty());
    let source = "table";
    let document = Document::parse(
        source,
        vec![entity(
            "table",
            0,
            5,
            Some("**literal**\u{1e}😀 __literal__"),
        )],
        ParseMode::Markdown,
    )
    .unwrap();
    let Block::Table { headers, rows } = &document.blocks[0] else {
        panic!("server table")
    };
    assert_eq!(headers[0].text(), "**literal**");
    assert_eq!(rows[0][0].text(), "😀 __literal__");
}

#[test]
fn raw_pango_markup_is_text_not_interpreted_attributes() {
    let source = "<span foreground=\"red\">😀 & text</span>";
    let document = Document::parse(source, vec![], ParseMode::TelegramEntities).unwrap();
    let rich = paragraph(&document);
    assert_eq!(rich.text(), source);
    assert!(rich.pango_attributes(&[]).iterator().attrs().is_empty());
}

#[test]
fn native_entity_block_splitting_rebases_emoji_ranges() {
    let source = "😀 before\n😀 code\n😀 after";
    let document = Document::parse(
        source,
        vec![
            entity("pre", 10, 7, Some("text")),
            entity("bold", 18, 8, None),
        ],
        ParseMode::TelegramEntities,
    )
    .unwrap();
    assert_eq!(document.blocks.len(), 3);
    assert!(matches!(&document.blocks[0], Block::Paragraph(text) if text.text() == "😀 before"));
    assert!(matches!(&document.blocks[1], Block::Code(code) if code.text.text() == "😀 code"));
    let Block::Paragraph(text) = &document.blocks[2] else {
        panic!("paragraph")
    };
    assert_eq!(text.text(), "😀 after");
    assert_eq!(
        text.entities()[0].range,
        TextRange {
            bytes: 0..10,
            characters: 0..7,
            utf16: 0..8
        }
    );
}
