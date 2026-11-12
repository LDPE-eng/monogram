use super::*;

use crate::i18n::{I18n, SUPPORTED_LOCALES};

fn model() -> Appearance {
    Appearance {
        i18n: Rc::new(I18n::new("en").unwrap()),
        state: AppearanceState::default(),
        catalog: Vec::new(),
        catalog_hash: 0,
        catalog_visible: false,
        catalog_loading: false,
        wallpaper_pending: None,
        wallpaper_error: None,
        accent_provider: None,
        typography_provider: None,
        applied_accent_css: None,
        applied_typography_css: None,
        applied_theme: None,
        rendering: Rc::new(Cell::new(false)),
    }
}

fn close_eq(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-4
}

fn close(actual: f32, expected: f32) {
    assert!(close_eq(actual, expected), "{actual} != {expected}");
}

fn wallpaper_dto(id: i64, document_id: Option<i64>) -> WallpaperDto {
    WallpaperDto {
        id,
        access_hash: id + 100,
        slug: format!("wall-{id}"),
        pattern: false,
        dark: false,
        mime_type: document_id.map(|_| "image/png".into()).unwrap_or_default(),
        document_id,
        colors: vec![0x102030, 0x405060],
        intensity: Some(-25),
        rotation: 45,
        blur: true,
        motion: false,
    }
}

fn wallpaper_file(path: &str) -> WallpaperFile {
    WallpaperFile {
        path: path.into(),
        pattern: false,
        colors: Vec::new(),
        intensity: None,
        rotation: 0,
        blur: false,
    }
}

fn solid(color: [u8; 4]) -> RgbaImage {
    RgbaImage::new(1, 1, color.to_vec()).unwrap()
}

fn image_config() -> WallpaperConfig {
    WallpaperConfig {
        mode: WallpaperMode::Image,
        path: Some("offline".into()),
        telegram: None,
        pattern: false,
        colors: Vec::new(),
        dim: 0.0,
        blur: false,
        intensity: 0,
        rotation: 0,
    }
}

#[test]
fn theme_names_round_trip_with_desktop_storage_values() {
    for preference in [
        ThemePreference::System,
        ThemePreference::Light,
        ThemePreference::Dark,
    ] {
        assert_eq!(
            ThemePreference::from_setting(preference.as_str()),
            Some(preference)
        );
    }
    assert_eq!(
        ThemePreference::from_setting("System"),
        Some(ThemePreference::System)
    );
    assert_eq!(
        ThemePreference::from_setting("DARK"),
        Some(ThemePreference::Dark)
    );
    assert_eq!(ThemePreference::from_setting("sepia"), None);
    assert_eq!(ThemePreference::from_index(1), ThemePreference::Light);
    assert_eq!(ThemePreference::from_index(99), ThemePreference::System);
}

#[test]
fn ten_accents_expose_catalog_names_keys_and_colors() {
    assert_eq!(ACCENTS.len(), 10);
    let mut keys = ACCENTS
        .iter()
        .map(|accent| accent.key())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 10);
    for accent in ACCENTS {
        assert_eq!(AccentPreset::from_setting(accent.as_str()), Some(accent));
        let (light, dark) = accent.colors();
        assert!(light.starts_with('#') && light.len() == 7, "{light}");
        assert!(dark.starts_with('#') && dark.len() == 7, "{dark}");
    }
    assert_eq!(
        AccentPreset::from_setting("monogram"),
        Some(AccentPreset::Monogram)
    );
    assert_eq!(AccentPreset::from_setting("neon"), None);
}

#[test]
fn darken_hex_produces_on_primary_for_dark_scheme() {
    assert_eq!(darken_hex("#006A8C", 0.3), "#00202A");
    assert_eq!(darken_hex("#FFFFFF", 0.3), "#4D4D4D");
    assert_eq!(darken_hex("#000000", 0.3), "#000000");
}

#[test]
fn accent_css_targets_libadwaita_variables_for_light_and_dark() {
    let css = accent_css(AccentPreset::Monogram);
    assert!(css.contains("--accent-bg-color: #006A8C"));
    assert!(css.contains("--accent-fg-color: #ffffff"));
    assert!(css.contains("window.dark"));
    assert!(css.contains("--accent-bg-color: #77D1F5"));
    assert!(css.contains("--accent-fg-color: #00202A"));
}

#[test]
fn typography_css_scales_message_text_line_height_and_letter_spacing() {
    let state = AppearanceState {
        message_text_size: 20,
        line_spacing: 1.25,
        letter_spacing: 0.03,
        ..Default::default()
    };
    let css = typography_css(&state);
    assert!(css.contains("font-size: 20pt"));
    assert!(css.contains("line-height: 1.25"));
    assert!(css.contains("letter-spacing: 0.03em"));
}

#[test]
fn clamps_match_android_ranges_and_steps() {
    assert_eq!(clamp_message_text_size(0), 12);
    assert_eq!(clamp_message_text_size(16), 16);
    assert_eq!(clamp_message_text_size(99), 30);
    assert_eq!(clamp_preview_lines(1), 2);
    assert_eq!(clamp_preview_lines(2), 2);
    assert_eq!(clamp_preview_lines(9), 3);
    close(clamp_line_spacing(0.5), 0.9);
    close(clamp_line_spacing(0.93), 0.95);
    close(clamp_line_spacing(1.0), 1.0);
    close(clamp_line_spacing(2.0), 1.5);
    close(clamp_letter_spacing(-0.01), 0.0);
    close(clamp_letter_spacing(0.037), 0.04);
    close(clamp_letter_spacing(0.1), 0.08);
    close(clamp_wallpaper_dim(-0.1), 0.0);
    close(clamp_wallpaper_dim(0.5), 0.5);
    close(clamp_wallpaper_dim(1.0), 0.8);
    assert_eq!(clamp_wallpaper_intensity(-200), -100);
    assert_eq!(clamp_wallpaper_intensity(50), 50);
    assert_eq!(clamp_wallpaper_intensity(200), 100);
    assert_eq!(clamp_wallpaper_rotation(-10), 350);
    assert_eq!(clamp_wallpaper_rotation(370), 10);
    assert_eq!(clamp_wallpaper_rotation(180), 180);
}

#[test]
fn spacing_labels_map_to_percent_and_hundredths() {
    assert_eq!(line_spacing_percent(1.0), 100);
    assert_eq!(line_spacing_percent(1.15), 115);
    assert_eq!(line_spacing_percent(0.9), 90);
    assert_eq!(letter_spacing_hundredths(0.0), 0);
    assert_eq!(letter_spacing_hundredths(0.08), 8);
    assert_eq!(letter_spacing_hundredths(0.03), 3);
}

#[test]
fn composer_slots_and_styles_round_trip() {
    assert_eq!(COMPOSER_SLOTS.len(), 8);
    for slot in COMPOSER_SLOTS {
        assert_eq!(ComposerSlotAction::from_setting(slot.as_str()), Some(slot));
    }
    assert_eq!(
        ComposerSlotAction::from_index(4),
        ComposerSlotAction::Attach
    );
    assert_eq!(ComposerSlotAction::from_index(99), ComposerSlotAction::None);
    assert_eq!(ComposerStyle::from_setting("IOS"), Some(ComposerStyle::Ios));
    assert_eq!(
        ComposerStyle::from_setting("material"),
        Some(ComposerStyle::Material)
    );
    assert_eq!(ComposerStyle::from_index(0), ComposerStyle::Ios);
}

#[test]
fn wallpaper_defaults_match_android() {
    let wallpaper = WallpaperConfig::default();
    assert_eq!(wallpaper.mode, WallpaperMode::Monogram);
    assert_eq!(wallpaper.path, None);
    close(wallpaper.dim, 0.15);
    assert!(!wallpaper.blur);
    assert_eq!(wallpaper.intensity, 50);
    assert_eq!(wallpaper.rotation, 0);
    assert_eq!(
        WallpaperMode::from_setting("none"),
        Some(WallpaperMode::None)
    );
    assert_eq!(
        WallpaperMode::from_setting("IMAGE"),
        Some(WallpaperMode::Image)
    );
    assert_eq!(WallpaperMode::from_setting("grid"), None);
}

#[test]
fn noop_inputs_do_not_emit_commands() {
    let mut page = model();
    assert!(page.handle(AppearanceInput::Accent(0)).is_empty());
    assert!(page.handle(AppearanceInput::MessageTextSize(16)).is_empty());
    assert!(page.handle(AppearanceInput::LineSpacing(1.0)).is_empty());
    assert!(page.handle(AppearanceInput::LetterSpacing(0.0)).is_empty());
    assert!(page.handle(AppearanceInput::PreviewLines(2)).is_empty());
    assert!(
        page.handle(AppearanceInput::ShowChatAvatars(true))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::OpenProfileOnAvatarTap(true))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::ShowReadStatus(true))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::ShowMutedCounter(false))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::FoldersAtBottom(false))
            .is_empty()
    );
    assert!(page.handle(AppearanceInput::ComposerStyle(1)).is_empty());
    assert!(
        page.handle(AppearanceInput::LeftComposerAction(4))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::RightComposerAction(1))
            .is_empty()
    );
    assert!(page.handle(AppearanceInput::SendByEnter(false)).is_empty());
    assert!(
        page.handle(AppearanceInput::FixLinkPreviews(true))
            .is_empty()
    );
    assert!(page.handle(AppearanceInput::WallpaperDim(0.15)).is_empty());
    assert!(
        page.handle(AppearanceInput::WallpaperBlur(false))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::WallpaperIntensity(50))
            .is_empty()
    );
    assert!(
        page.handle(AppearanceInput::WallpaperRotation(0))
            .is_empty()
    );
}

#[test]
fn changed_values_emit_exactly_one_worker_command() {
    let mut page = model();
    let outputs = page.handle(AppearanceInput::Accent(3));
    assert_eq!(outputs.len(), 1);
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetAccent(AccentPreset::Forest))
    ));

    let outputs = page.handle(AppearanceInput::MessageTextSize(99));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetMessageTextSize(30))
    ));

    let outputs = page.handle(AppearanceInput::LineSpacing(1.15));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetLineSpacing(value)) if close_eq(value, 1.15)
    ));

    let outputs = page.handle(AppearanceInput::PreviewLines(3));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetPreviewLines(3))
    ));
}

#[test]
fn theme_input_emits_theme_output_and_ignores_same_selection() {
    let mut page = model();
    let outputs = page.handle(AppearanceInput::Theme(1));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Theme(ThemePreference::Light)
    ));
    assert!(page.handle(AppearanceInput::Theme(1)).is_empty());
    let outputs = page.handle(AppearanceInput::Theme(0));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Theme(ThemePreference::System)
    ));
}

#[test]
fn restore_applies_state_without_emitting_commands() {
    let mut page = model();
    let state = AppearanceState {
        accent: AccentPreset::Violet,
        preview_lines: 3,
        ..Default::default()
    };
    assert!(
        page.handle(AppearanceInput::Restore(state.clone()))
            .is_empty()
    );
    assert_eq!(page.state, state);
}

#[test]
fn wallpaper_changes_emit_full_config_commands() {
    let mut page = model();
    let outputs = page.handle(AppearanceInput::WallpaperDim(0.5));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.mode == WallpaperMode::Monogram
                && config.path.is_none()
                && close_eq(config.dim, 0.5)
    ));

    let outputs = page.handle(AppearanceInput::WallpaperPath(Some("/tmp/wall.jpg".into())));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::ImportLocalWallpaper { ref path } if path == "/tmp/wall.jpg"
    ));
    assert_eq!(page.state.wallpaper.mode, WallpaperMode::Monogram);
    let outputs = page.handle(AppearanceInput::WallpaperLoaded(wallpaper_file(
        "/cache/local.png",
    )));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.mode == WallpaperMode::Image
                && config.path.as_deref() == Some("/cache/local.png")
    ));

    let outputs = page.handle(AppearanceInput::WallpaperBlur(true));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.blur
    ));

    let outputs = page.handle(AppearanceInput::WallpaperRotation(370));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.rotation == 10
    ));
}

#[test]
fn wallpaper_image_mode_requires_an_imported_path() {
    let mut page = model();
    assert!(page.handle(AppearanceInput::WallpaperMode(2)).is_empty());
    assert_eq!(page.state.wallpaper.mode, WallpaperMode::Monogram);

    page.handle(AppearanceInput::WallpaperPath(Some("/tmp/wall.jpg".into())));
    assert!(page.handle(AppearanceInput::WallpaperMode(2)).is_empty());
    let outputs = page.handle(AppearanceInput::WallpaperLoaded(wallpaper_file(
        "/cache/local.png",
    )));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.mode == WallpaperMode::Image
                && config.path.as_deref() == Some("/cache/local.png")
    ));
    assert!(page.handle(AppearanceInput::WallpaperMode(2)).is_empty());
    page.state.wallpaper.colors = vec![0x102030];
    page.state.wallpaper.blur = true;

    let outputs = page.handle(AppearanceInput::WallpaperMode(0));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(ref config))
            if config.mode == WallpaperMode::Monogram && config.path.is_none()
    ));
    assert_eq!(page.state.wallpaper, WallpaperConfig::default());
    assert!(page.state.wallpaper.pattern);
    assert!(page.state.wallpaper.colors.is_empty());
    assert!(!page.state.wallpaper.blur);
}

#[test]
fn browse_telegram_wallpapers_is_forwarded_as_an_output() {
    let mut page = model();
    let outputs = page.handle(AppearanceInput::BrowseTelegramWallpapers);
    assert!(matches!(
        outputs[0],
        AppearanceOutput::BrowseTelegramWallpapers
    ));
}

#[test]
fn catalog_not_modified_keeps_existing_wallpapers_and_updates_hash() {
    let mut page = model();
    page.catalog = vec![wallpaper_dto(7, Some(70))];
    page.handle(AppearanceInput::WallpaperCatalog(Box::new(
        WallpaperCatalogDto {
            hash: 99,
            not_modified: true,
            wallpapers: Vec::new(),
        },
    )));
    assert_eq!(page.catalog_hash, 99);
    assert_eq!(page.catalog.len(), 1);
    assert_eq!(page.catalog[0].id, 7);
    assert!(!page.catalog_loading);
}

#[test]
fn telegram_selection_downloads_then_applies_cached_file_and_metadata() {
    let mut page = model();
    page.catalog = vec![wallpaper_dto(7, Some(70))];
    let original = page.state.wallpaper.clone();
    let outputs = page.handle(AppearanceInput::SelectTelegramWallpaper(0));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::DownloadTelegramWallpaper {
            id: 7,
            access_hash: 107,
            pattern: false,
        }
    ));
    assert_eq!(page.state.wallpaper, original);

    let outputs = page.handle(AppearanceInput::WallpaperLoaded(wallpaper_file(
        "/cache/telegram-7.png",
    )));
    let AppearanceOutput::Command(AppearanceCommand::SetWallpaper(config)) = &outputs[0] else {
        panic!()
    };
    assert_eq!(config.path.as_deref(), Some("/cache/telegram-7.png"));
    assert_eq!(
        config.telegram.as_ref().map(|telegram| telegram.id),
        Some(7)
    );
    assert_eq!(config.colors, vec![0x102030, 0x405060]);
    assert_eq!(config.intensity, -25);
    assert_eq!(config.rotation, 45);
    assert!(config.blur);
}

#[test]
fn telegram_color_wallpaper_applies_without_a_download() {
    let mut page = model();
    page.catalog = vec![wallpaper_dto(8, None)];
    let outputs = page.handle(AppearanceInput::SelectTelegramWallpaper(0));
    assert!(matches!(
        outputs[0],
        AppearanceOutput::Command(AppearanceCommand::SetWallpaper(_))
    ));
    assert_eq!(page.state.wallpaper.mode, WallpaperMode::Image);
    assert_eq!(page.state.wallpaper.path, None);
    assert_eq!(page.state.wallpaper.colors, vec![0x102030, 0x405060]);
}

#[test]
fn failed_import_preserves_active_wallpaper_and_ignores_late_files() {
    let mut page = model();
    let original = page.state.wallpaper.clone();
    page.handle(AppearanceInput::WallpaperPath(Some("/tmp/wall.png".into())));
    page.handle(AppearanceInput::WallpaperFailed(Some("offline".into())));
    assert_eq!(page.state.wallpaper, original);
    assert_eq!(page.wallpaper_error.as_deref(), Some("offline"));
    assert!(
        page.handle(AppearanceInput::WallpaperLoaded(wallpaper_file(
            "/cache/late.png"
        )))
        .is_empty()
    );
    assert_eq!(page.state.wallpaper, original);
}

#[test]
fn signed_intensity_lightens_and_darkens_actual_pixels() {
    let source = solid([100, 150, 200, 255]);
    let mut config = image_config();
    config.intensity = 50;
    let light = render_wallpaper(&config, Some(&source), 1, 1).unwrap();
    assert_eq!(light.pixel(0, 0), [177, 202, 227, 255]);
    config.intensity = -50;
    let dark = render_wallpaper(&config, Some(&source), 1, 1).unwrap();
    assert_eq!(dark.pixel(0, 0), [50, 75, 100, 255]);
}

#[test]
fn dim_is_exactly_bounded_at_eighty_percent() {
    let source = solid([100, 150, 200, 255]);
    let mut config = image_config();
    config.dim = 0.8;
    let output = render_wallpaper(&config, Some(&source), 1, 1).unwrap();
    assert_eq!(output.pixel(0, 0), [20, 30, 40, 255]);
}

#[test]
fn color_wallpapers_render_a_real_gradient() {
    let mut config = image_config();
    config.path = None;
    config.colors = vec![0x102030, 0x405060];
    let output = render_wallpaper(&config, None, 2, 2).unwrap();
    assert_eq!(output.pixel(0, 0), [0x10, 0x20, 0x30, 255]);
    assert_eq!(output.pixel(1, 1), [0x40, 0x50, 0x60, 255]);
}

#[test]
fn pattern_pixels_repeat_across_the_background() {
    let source = RgbaImage::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    let mut config = image_config();
    config.pattern = true;
    config.colors = vec![0];
    config.intensity = 100;
    let output = render_wallpaper(&config, Some(&source), 32, 8).unwrap();
    assert_eq!(output.pixel(0, 0), [255, 0, 0, 255]);
    assert_eq!(output.pixel(15, 0), [0, 255, 0, 255]);
    assert_eq!(output.pixel(0, 0), output.pixel(16, 0));
    assert_eq!(output.pixel(15, 7), output.pixel(31, 7));
}

#[test]
fn rotation_moves_real_pixels_by_ninety_degrees() {
    let source = RgbaImage::new(
        2,
        2,
        vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    )
    .unwrap();
    let mut config = image_config();
    config.rotation = 90;
    let output = render_wallpaper(&config, Some(&source), 2, 2).unwrap();
    assert_eq!(output.pixel(0, 0), [0, 0, 255, 255]);
    assert_eq!(output.pixel(0, 1), [255, 255, 255, 255]);
    assert_eq!(output.pixel(1, 0), [255, 0, 0, 255]);
    assert_eq!(output.pixel(1, 1), [0, 255, 0, 255]);
}

#[test]
fn blur_changes_sharp_pixels_and_none_disables_rendering() {
    let mut pixels = vec![0; 3 * 3 * 4];
    pixels[16..20].copy_from_slice(&[255, 255, 255, 255]);
    let source = RgbaImage::new(3, 3, pixels).unwrap();
    let mut config = image_config();
    config.blur = true;
    let output = render_wallpaper(&config, Some(&source), 3, 3).unwrap();
    assert!(output.pixel(1, 1)[0] > 0 && output.pixel(1, 1)[0] < 255);
    config.mode = WallpaperMode::None;
    assert!(render_wallpaper(&config, Some(&source), 3, 3).is_none());
}

#[test]
fn every_existing_i18n_key_is_present_in_all_locales() {
    for locale in SUPPORTED_LOCALES {
        let i18n = I18n::new(locale).unwrap();
        for key in APPEARANCE_I18N_KEYS {
            let value = i18n.text(key, &[]).unwrap();
            assert!(!value.is_empty(), "{locale}: {key}");
        }
        for key in APPEARANCE_I18N_KEYS_WITH_ARG {
            let value = i18n.text(key, &[("arg1", "1")]).unwrap();
            assert!(!value.is_empty(), "{locale}: {key}");
        }
    }
}

#[test]
fn i18n_key_lists_are_disjoint_and_use_the_settings_prefix() {
    let mut keys = Vec::new();
    keys.extend_from_slice(APPEARANCE_I18N_KEYS);
    keys.extend_from_slice(APPEARANCE_I18N_KEYS_WITH_ARG);
    keys.extend_from_slice(APPEARANCE_REQUIRED_I18N_KEYS);
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        keys.len(),
        "keys must be declared exactly once"
    );
    for key in APPEARANCE_REQUIRED_I18N_KEYS {
        assert!(
            key.starts_with("feature-settings-settings_"),
            "{key} must follow the feature-settings resource naming"
        );
    }
}

fn find_widget(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn drain_gtk() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..10 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

pub(crate) fn wallpaper_scenario() {
    let page = Appearance::builder().launch(Rc::new(I18n::new("en").unwrap()));
    let window = adw::Window::new();
    window.set_default_size(720, 900);
    window.set_content(Some(page.widget()));
    window.present();
    drain_gtk();
    page.emit(AppearanceInput::BrowseTelegramWallpapers);
    page.emit(AppearanceInput::WallpaperCatalog(Box::new(
        WallpaperCatalogDto {
            hash: 11,
            not_modified: false,
            wallpapers: vec![wallpaper_dto(1, Some(10)), wallpaper_dto(2, None)],
        },
    )));
    drain_gtk();
    assert!(page.model().catalog_visible);
    assert_eq!(page.model().catalog_hash, 11);
    assert!(find_widget(page.widget().upcast_ref(), "wallpaper-grid").is_some());
    assert!(find_widget(page.widget().upcast_ref(), "wallpaper-preview").is_some());

    page.emit(AppearanceInput::SelectTelegramWallpaper(1));
    drain_gtk();
    assert_eq!(
        page.model()
            .state
            .wallpaper
            .telegram
            .as_ref()
            .map(|item| item.id),
        Some(2)
    );

    let path =
        std::env::temp_dir().join(format!("monogram-wallpaper-gtk-{}.png", std::process::id()));
    let bytes = gtk::glib::Bytes::from_owned(vec![120, 40, 200, 255]);
    let texture = gtk::gdk::MemoryTexture::new(1, 1, gtk::gdk::MemoryFormat::R8g8b8a8, &bytes, 4);
    texture.save_to_png(&path).unwrap();
    let path_string = path.to_string_lossy().into_owned();
    page.emit(AppearanceInput::WallpaperPath(Some(path_string.clone())));
    page.emit(AppearanceInput::WallpaperLoaded(wallpaper_file(
        &path_string,
    )));
    drain_gtk();
    assert_eq!(
        page.model().state.wallpaper.path.as_deref(),
        Some(path_string.as_str())
    );
    assert!(wallpaper_texture(&page.model().state.wallpaper, 32, 32).is_some());
    assert!(find_widget(page.widget().upcast_ref(), "wallpaper-preview").is_some());
    let _ = std::fs::remove_file(path);
}
