use super::*;
use relm4::{Component, ComponentController};
use std::time::{Duration, Instant};

fn dto(blocks: serde_json::Value) -> InstantViewDto {
    InstantViewDto {
        url: "https://example.org/articles/story".into(),
        display_url: "example.org/articles/story".into(),
        title: Some("Story".into()),
        site_name: Some("Example".into()),
        description: Some("Description".into()),
        webpage_type: Some("article".into()),
        hash: 17,
        has_instant_view: true,
        part: false,
        rtl: false,
        v2: true,
        not_modified: false,
        blocks_json: blocks.to_string(),
    }
}

fn rich(text: &str) -> RichTextDto {
    RichTextDto {
        text: text.into(),
        entities: Vec::new(),
    }
}

#[test]
fn real_compact_dto_covers_nested_content_media_captions_and_math_source() {
    let page = InstantViewPage::from_dto(dto(serde_json::json!([
        {"k":"title","t":"Story","e":[{"k":"bold","o":0,"l":5}]},
        {"k":"heading","l":2,"t":"Section"},
        {"k":"paragraph","t":"Body"},
        {"k":"quote","t":"Quote","caption":{"t":"Caption","credit":{"t":"Credit"}},"blocks":[{"k":"paragraph","t":"Nested"}]},
        {"k":"list","ordered":true,"items":[{"num":"3","t":"Third"},{"checkbox":true,"checked":true,"blocks":[{"k":"paragraph","t":"Done"}]}]},
        {"k":"details","open":true,"title":{"t":"More"},"blocks":[{"k":"paragraph","t":"Hidden"}]},
        {"k":"photo","id":7,"cache":"photo:7","w":640,"h":480,"caption":{"t":"Photo caption","credit":{"t":"Photographer"}}},
        {"k":"document","id":8,"cache":"doc:8","name":"paper.pdf","size":2048},
        {"k":"embed","url":"https://video.example/embed","html":"<script>not executed</script>"},
        {"k":"embedPost","url":"https://example.org/post","author":"Author","date":1700000000,"blocks":[{"k":"paragraph","t":"Post"}]},
        {"k":"collage","items":[{"k":"photo","id":9,"cache":"photo:9"}],"caption":{"t":"Group"}},
        {"k":"related","title":{"t":"Related"},"articles":[{"url":"https://example.org/next","title":"Next"}]},
        {"k":"map","lat":55.75,"lng":37.61,"zoom":10,"cache":"geo:99:10"},
        {"k":"math","src":"x^2 + y^2"},
        {"k":"anchor","n":"section-1"},
        {"k":"unsupported"}
    ])))
    .unwrap();
    assert_eq!(page.blocks.len(), 16);
    assert_eq!(page.blocks[0].text.entities[0].kind, "bold");
    assert_eq!(
        page.blocks[3]
            .caption
            .as_ref()
            .unwrap()
            .credit
            .as_ref()
            .unwrap()
            .text,
        "Credit"
    );
    assert_eq!(
        page.blocks[6].caption.as_ref().unwrap().text.text,
        "Photo caption"
    );
    assert_eq!(page.blocks[7].file_size, Some(2048));
    assert_eq!(page.blocks[13].math_source, "x^2 + y^2");
    let text = plain_text(&page.blocks);
    for expected in [
        "Story",
        "Nested",
        "3 Third",
        "☑ Done",
        "Photo caption",
        "paper.pdf",
        "Post",
        "Group",
        "x^2 + y^2",
    ] {
        assert!(text.contains(expected), "missing {expected:?}");
    }
    assert!(!text.contains("not executed"));
    assert!(!text.contains("Related"));
}

#[test]
fn malformed_blocks_are_an_explicit_page_error() {
    let mut value = dto(serde_json::json!([]));
    value.blocks_json = "not-json".into();
    assert_eq!(
        InstantViewPage::from_dto(value).err(),
        Some(PageError::InvalidBlocks)
    );
}

#[test]
fn links_resolve_internal_anchors_relative_pages_and_safe_external_schemes() {
    let current = "https://example.org/articles/story";
    assert_eq!(
        resolve_link(current, "#Section%201"),
        Some(ResolvedLink::Anchor("Section 1".into()))
    );
    assert_eq!(
        resolve_link(current, "https://example.org/articles/story#two"),
        Some(ResolvedLink::Anchor("two".into()))
    );
    assert_eq!(
        resolve_link(current, "../next#part"),
        Some(ResolvedLink::Page {
            url: "https://example.org/next".into(),
            anchor: Some("part".into()),
        })
    );
    assert_eq!(
        resolve_link(current, "#two+parts"),
        Some(ResolvedLink::Anchor("two parts".into()))
    );
    assert_eq!(
        resolve_link(current, "mailto:editor@example.org"),
        Some(ResolvedLink::External("mailto:editor@example.org".into()))
    );
    assert_eq!(
        resolve_link(current, "tg://resolve?domain=example"),
        Some(ResolvedLink::External("tg://resolve?domain=example".into()))
    );
    assert_eq!(
        resolve_link(current, "geo:55.75,37.61?z=10"),
        Some(ResolvedLink::External("geo:55.75,37.61?z=10".into()))
    );
}

#[test]
fn unsafe_links_and_media_keys_never_become_outputs() {
    for uri in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "data:text/html,x",
        "https://",
        "https:///bad",
        "https://example.org/a b",
        "https://example.org/\u{0}",
    ] {
        assert!(!safe_uri(uri), "{uri}");
        assert_eq!(resolve_link("https://example.org", uri), None, "{uri}");
    }
    for uri in [
        "HTTPS://example.org",
        "http://localhost:8080/path",
        "tg://user?id=42",
        "mailto:test@example.org",
        "tel:+12345",
        "geo:1,2?z=3",
    ] {
        assert!(safe_uri(uri), "{uri}");
    }
    for key in ["photo:7", "doc:8", "geo:99:10", "avatar:42"] {
        assert!(valid_media_key(key), "{key}");
    }
    for key in [
        "",
        "photo",
        "photo:",
        "photo:x",
        "file:7",
        "photo:7 x",
        "photo:-1",
    ] {
        assert!(!valid_media_key(key), "{key}");
    }
}

#[test]
fn rich_links_use_markup_ranges_and_preserve_internal_anchors() {
    let rich = RichTextDto {
        text: "😀link anchor".into(),
        entities: vec![
            EntityDto {
                kind: "text_url".into(),
                offset: 2,
                length: 4,
                url: Some("https://example.org/?a=1&b=2".into()),
            },
            EntityDto {
                kind: "anchor".into(),
                offset: 7,
                length: 6,
                url: Some("#target".into()),
            },
        ],
    };
    let parsed = parse_rich(&rich).unwrap();
    assert_eq!(parsed.entities()[0].range.characters, 1..5);
    assert_eq!(
        href_at(&parsed, 1).as_deref(),
        Some("https://example.org/?a=1&b=2")
    );
    assert_eq!(href_at(&parsed, 6).as_deref(), Some("#target"));
    assert_eq!(
        first_href(&parsed).as_deref(),
        Some("https://example.org/?a=1&b=2")
    );
}

#[test]
fn spoiler_links_are_not_actionable_before_reveal() {
    let parsed = parse_rich(&RichTextDto {
        text: "secret".into(),
        entities: vec![
            EntityDto {
                kind: "spoiler".into(),
                offset: 0,
                length: 6,
                url: None,
            },
            EntityDto {
                kind: "text_url".into(),
                offset: 0,
                length: 6,
                url: Some("https://example.org".into()),
            },
        ],
    })
    .unwrap();
    assert_eq!(href_at(&parsed, 0), None);
}

#[test]
fn table_layout_respects_rowspan_colspan_and_occupancy() {
    let rows = vec![
        vec![
            TableCellDto {
                text: rich("A"),
                colspan: 2,
                rowspan: 2,
                ..TableCellDto::default()
            },
            TableCellDto {
                text: rich("B"),
                ..TableCellDto::default()
            },
        ],
        vec![TableCellDto {
            text: rich("C"),
            ..TableCellDto::default()
        }],
    ];
    let layout = place_table(&rows).unwrap();
    assert_eq!((layout.rows, layout.columns), (2, 3));
    assert_eq!(
        layout
            .cells
            .iter()
            .map(|cell| (cell.row, cell.column))
            .collect::<Vec<_>>(),
        vec![(0, 0), (0, 2), (1, 2)]
    );
    let oversized = vec![vec![TableCellDto {
        colspan: (MAX_TABLE_COLUMNS + 1) as i32,
        ..TableCellDto::default()
    }]];
    assert_eq!(place_table(&oversized), Err(TableError::TooLarge));
}

#[test]
fn reading_estimate_matches_android_rates_for_latin_and_cjk() {
    assert_eq!(reading_estimate("  \n"), ReadingEstimate::default());
    assert_eq!(reading_estimate(&"word ".repeat(200)).minutes, 1);
    assert_eq!(reading_estimate(&"word ".repeat(201)).minutes, 2);
    let cjk = reading_estimate(&format!(
        "{}{}{}{}",
        "漢".repeat(300),
        "か".repeat(400),
        "한".repeat(350),
        "word ".repeat(200)
    ));
    assert_eq!(
        (cjk.han, cjk.kana, cjk.hangul, cjk.words),
        (300, 400, 350, 200)
    );
    assert_eq!(cjk.minutes, 4);
    let mixed = reading_estimate("😀 one два 3");
    assert_eq!(mixed.words, 3);
}

#[test]
fn page_title_and_file_size_follow_android_rules() {
    let mut page = InstantViewPage::from_dto(dto(serde_json::json!([
        {"k":"paragraph","t":"Body"}
    ])))
    .unwrap();
    assert!(should_show_page_title(&page));
    page.blocks.insert(
        0,
        BlockDto {
            kind: "title".into(),
            text: rich("story"),
            ..BlockDto::default()
        },
    );
    assert!(!should_show_page_title(&page));
    assert_eq!(format_file_size(None), None);
    assert_eq!(format_file_size(Some(0)), None);
    assert_eq!(format_file_size(Some(1000)).as_deref(), Some("1000 B"));
    assert_eq!(format_file_size(Some(2048)).as_deref(), Some("2 KB"));
    assert_eq!(
        format_file_size(Some(3 * 1024 * 1024)).as_deref(),
        Some("3 MB")
    );
}

#[test]
fn rtl_and_page_metadata_survive_real_dto_conversion() {
    let mut value = dto(serde_json::json!([{"k":"paragraph","t":"مرحبا"}]));
    value.rtl = true;
    let page = InstantViewPage::from_dto(value).unwrap();
    assert!(page.rtl);
    assert_eq!(display_host(&page), Some("example.org"));
    assert_eq!(page.hash, 17);
}

#[test]
fn debug_messages_redact_page_error_link_and_media_content() {
    let secret = "private article target";
    let messages = [
        format!("{:?}", WebInput::Error(secret.into())),
        format!("{:?}", WebInput::OpenLink(secret.into())),
        format!("{:?}", WebInput::OpenMedia(secret.into())),
        format!("{:?}", WebOutput::OpenExternal(secret.into())),
        format!("{:?}", WebOutput::OpenMedia(secret.into())),
    ];
    assert!(messages.iter().all(|message| !message.contains(secret)));
}

#[test]
fn instant_view_localization_keys_exist_in_every_language() {
    for locale in crate::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in [
            "feature-dialog-dialog_instant_view",
            "feature-dialog-dialog_instant_view_back",
            "feature-dialog-dialog_instant_view_retry",
            "feature-dialog-dialog_instant_view_open_original",
            "feature-dialog-dialog_instant_view_loading",
            "feature-dialog-dialog_instant_view_empty",
            "feature-dialog-dialog_instant_view_error",
            "feature-dialog-dialog_instant_view_unsupported",
            "feature-dialog-dialog_instant_view_related",
            "feature-dialog-dialog_instant_view_map",
            "feature-dialog-dialog_instant_view_media_open",
            "feature-dialog-dialog_media_photo",
            "feature-dialog-dialog_media_video",
            "feature-dialog-dialog_media_audio",
            "feature-dialog-dialog_media_document",
            "feature-dialog-dialog_media_link",
            "core-ui-media_info_name",
            "core-ui-media_info_size",
            "core-ui-media_info_duration",
        ] {
            assert!(i18n.text(key, &[]).is_ok(), "{locale}: {key}");
        }
        for key in [
            "feature-dialog-dialog_instant_view_min_read",
            "core-ui-media_info_dimensions_value",
        ] {
            assert!(
                i18n.text(key, &[("arg1", "1"), ("arg2", "2")]).is_ok(),
                "{locale}: {key}"
            );
        }
    }
}

pub(crate) fn instant_view_scenario() {
    let view = WebView::builder().launch(std::rc::Rc::new(crate::i18n::I18n::new("en").unwrap()));
    let context = relm4::gtk::glib::MainContext::default();
    view.emit(WebInput::Page(dto(serde_json::json!([
        {"k":"title","t":"Scenario"},
        {"k":"paragraph","t":"Body paragraph"},
        {"k":"list","items":[{"t":"First"},{"t":"Second"}]},
        {"k":"anchor","n":"target"},
        {"k":"buttons","items":[{"t":"Jump","e":[{"k":"anchor","o":0,"l":4,"u":"#target"}]}]}
    ]))));
    let deadline = Instant::now() + Duration::from_secs(5);
    while view.model().page.is_none() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        view.model().page.is_some(),
        "instant view page did not render"
    );
    assert!(
        view.model().content.first_child().is_some(),
        "instant view content is empty"
    );
    assert!(view.model().anchors.borrow().contains_key("target"));
    view.emit(WebInput::OpenLink("#target".into()));
    for _ in 0..5 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(view.model().page.is_some());
    assert!(view.model().pending_anchor.borrow().is_none());
}
