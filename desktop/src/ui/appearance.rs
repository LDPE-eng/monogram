use std::{cell::Cell, rc::Rc};

use relm4::{adw, adw::prelude::*, gtk, prelude::*};

use crate::i18n::I18n;
use mono_proto_client::adapter::media::wallpaper::{WallpaperCatalogDto, WallpaperDto};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperFile {
    pub path: String,
    pub pattern: bool,
    pub colors: Vec<i32>,
    pub intensity: Option<i32>,
    pub rotation: i32,
    pub blur: bool,
}

pub const MIN_MESSAGE_TEXT_SIZE: u32 = 12;
pub const MAX_MESSAGE_TEXT_SIZE: u32 = 30;
pub const DEFAULT_MESSAGE_TEXT_SIZE: u32 = 16;
pub const MIN_LINE_SPACING: f32 = 0.9;
pub const MAX_LINE_SPACING: f32 = 1.5;
pub const DEFAULT_LINE_SPACING: f32 = 1.0;
pub const LINE_SPACING_STEP: f32 = 0.05;
pub const MIN_LETTER_SPACING: f32 = 0.0;
pub const MAX_LETTER_SPACING: f32 = 0.08;
pub const DEFAULT_LETTER_SPACING: f32 = 0.0;
pub const LETTER_SPACING_STEP: f32 = 0.01;
pub const MIN_PREVIEW_LINES: u32 = 2;
pub const MAX_PREVIEW_LINES: u32 = 3;
pub const MIN_WALLPAPER_DIM: f32 = 0.0;
pub const MAX_WALLPAPER_DIM: f32 = 0.8;
pub const MIN_WALLPAPER_INTENSITY: i32 = -100;
pub const MAX_WALLPAPER_INTENSITY: i32 = 100;

/// Keys already present in every supported locale and referenced by this screen.
pub const APPEARANCE_I18N_KEYS: &[&str] = &[
    "desktop-appearance",
    "desktop-back",
    "feature-settings-settings_accent",
    "feature-settings-settings_accent_amber",
    "feature-settings-settings_accent_forest",
    "feature-settings-settings_accent_monogram",
    "feature-settings-settings_accent_ocean",
    "feature-settings-settings_accent_rose",
    "feature-settings-settings_accent_sakura",
    "feature-settings-settings_accent_slate",
    "feature-settings-settings_accent_sunset",
    "feature-settings-settings_accent_teal",
    "feature-settings-settings_accent_violet",
    "feature-settings-settings_avatar_profile_tap",
    "feature-settings-settings_avatar_profile_tap_sub",
    "feature-settings-settings_avatar_profile_tap_unavailable",
    "feature-settings-settings_chat_list",
    "feature-settings-settings_folders_at_bottom",
    "feature-settings-settings_folders_at_bottom_sub",
    "feature-settings-settings_input_bar",
    "feature-settings-settings_input_bar_fix_previews",
    "feature-settings-settings_input_bar_fix_previews_sub",
    "feature-settings-settings_input_bar_ios",
    "feature-settings-settings_input_bar_left",
    "feature-settings-settings_input_bar_left_sub",
    "feature-settings-settings_input_bar_material",
    "feature-settings-settings_input_bar_right",
    "feature-settings-settings_input_bar_right_sub",
    "feature-settings-settings_input_bar_send_by_enter",
    "feature-settings-settings_input_bar_send_by_enter_sub",
    "feature-settings-settings_letter_spacing",
    "feature-settings-settings_line_spacing",
    "feature-settings-settings_line_spacing_default",
    "feature-settings-settings_message_text_size",
    "feature-settings-settings_muted_counter",
    "feature-settings-settings_muted_counter_sub",
    "feature-settings-settings_preview_lines",
    "feature-settings-settings_preview_lines_three",
    "feature-settings-settings_preview_lines_three_sub",
    "feature-settings-settings_preview_lines_two",
    "feature-settings-settings_preview_lines_two_sub",
    "feature-settings-settings_show_avatars",
    "feature-settings-settings_show_avatars_sub",
    "feature-settings-settings_show_read_status",
    "feature-settings-settings_show_read_status_sub",
    "feature-settings-settings_slot_attach",
    "feature-settings-settings_slot_emoji",
    "feature-settings-settings_slot_file",
    "feature-settings-settings_slot_gifs",
    "feature-settings-settings_slot_location",
    "feature-settings-settings_slot_none",
    "feature-settings-settings_slot_photos",
    "feature-settings-settings_slot_stickers",
    "feature-settings-settings_theme",
    "feature-settings-settings_theme_dark",
    "feature-settings-settings_theme_light",
    "feature-settings-settings_theme_system",
    "feature-settings-settings_wallpaper",
    "feature-settings-settings_wallpaper_apply",
    "feature-settings-settings_wallpaper_applied_all",
    "feature-settings-settings_wallpaper_blur",
    "feature-settings-settings_wallpaper_device",
    "feature-settings-settings_wallpaper_hero_sub",
    "feature-settings-settings_wallpaper_intensity",
    "feature-settings-settings_wallpaper_none",
    "feature-settings-settings_wallpaper_reset",
    "feature-settings-settings_wallpaper_rotation",
    "feature-settings-settings_wallpaper_sample_in",
    "feature-settings-settings_wallpaper_sample_out",
    "feature-settings-settings_wallpaper_selected",
    "feature-settings-settings_wallpaper_sources",
    "feature-settings-settings_wallpaper_telegram",
];

pub const APPEARANCE_I18N_KEYS_WITH_ARG: &[&str] = &[
    "feature-settings-settings_letter_spacing_value",
    "feature-settings-settings_line_spacing_value",
    "feature-settings-settings_message_text_size_value",
    "feature-settings-settings_wallpaper_dim",
];

pub const APPEARANCE_REQUIRED_I18N_KEYS: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::System => 0,
            Self::Light => 1,
            Self::Dark => 2,
        }
    }

    fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Light,
            2 => Self::Dark,
            _ => Self::System,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::System => "feature-settings-settings_theme_system",
            Self::Light => "feature-settings-settings_theme_light",
            Self::Dark => "feature-settings-settings_theme_dark",
        }
    }

    fn apply(self) {
        adw::StyleManager::default().set_color_scheme(match self {
            Self::System => adw::ColorScheme::Default,
            Self::Light => adw::ColorScheme::ForceLight,
            Self::Dark => adw::ColorScheme::ForceDark,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentPreset {
    Monogram,
    Sakura,
    Ocean,
    Forest,
    Sunset,
    Violet,
    Amber,
    Rose,
    Slate,
    Teal,
}

pub const ACCENTS: [AccentPreset; 10] = [
    AccentPreset::Monogram,
    AccentPreset::Sakura,
    AccentPreset::Ocean,
    AccentPreset::Forest,
    AccentPreset::Sunset,
    AccentPreset::Violet,
    AccentPreset::Amber,
    AccentPreset::Rose,
    AccentPreset::Slate,
    AccentPreset::Teal,
];

impl AccentPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Monogram => "Monogram",
            Self::Sakura => "Sakura",
            Self::Ocean => "Ocean",
            Self::Forest => "Forest",
            Self::Sunset => "Sunset",
            Self::Violet => "Violet",
            Self::Amber => "Amber",
            Self::Rose => "Rose",
            Self::Slate => "Slate",
            Self::Teal => "Teal",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        ACCENTS
            .into_iter()
            .find(|accent| accent.as_str().eq_ignore_ascii_case(value))
    }

    fn index(self) -> u32 {
        ACCENTS
            .iter()
            .position(|accent| *accent == self)
            .unwrap_or(0) as u32
    }

    fn from_index(index: u32) -> Self {
        ACCENTS
            .get(index as usize)
            .copied()
            .unwrap_or(Self::Monogram)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Monogram => "feature-settings-settings_accent_monogram",
            Self::Sakura => "feature-settings-settings_accent_sakura",
            Self::Ocean => "feature-settings-settings_accent_ocean",
            Self::Forest => "feature-settings-settings_accent_forest",
            Self::Sunset => "feature-settings-settings_accent_sunset",
            Self::Violet => "feature-settings-settings_accent_violet",
            Self::Amber => "feature-settings-settings_accent_amber",
            Self::Rose => "feature-settings-settings_accent_rose",
            Self::Slate => "feature-settings-settings_accent_slate",
            Self::Teal => "feature-settings-settings_accent_teal",
        }
    }

    fn colors(self) -> (&'static str, &'static str) {
        match self {
            Self::Monogram => ("#006A8C", "#77D1F5"),
            Self::Sakura => ("#984061", "#FFB0CB"),
            Self::Ocean => ("#006780", "#65D5F3"),
            Self::Forest => ("#356A3D", "#9BD5A0"),
            Self::Sunset => ("#9A4526", "#FFB59A"),
            Self::Violet => ("#6750A4", "#D0BCFF"),
            Self::Amber => ("#785900", "#FABD00"),
            Self::Rose => ("#8C4A62", "#FFB1C8"),
            Self::Slate => ("#515E7D", "#B9C6E9"),
            Self::Teal => ("#006A65", "#4DDAD3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperMode {
    Monogram,
    None,
    Image,
}

impl WallpaperMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Monogram => "Monogram",
            Self::None => "None",
            Self::Image => "Image",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            value if value.eq_ignore_ascii_case("Monogram") => Some(Self::Monogram),
            value if value.eq_ignore_ascii_case("None") => Some(Self::None),
            value if value.eq_ignore_ascii_case("Image") => Some(Self::Image),
            _ => None,
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::Monogram => 0,
            Self::None => 1,
            Self::Image => 2,
        }
    }

    fn from_index(index: u32) -> Self {
        match index {
            1 => Self::None,
            2 => Self::Image,
            _ => Self::Monogram,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerStyle {
    Ios,
    Material,
}

impl ComposerStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "IOS",
            Self::Material => "Material",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            value if value.eq_ignore_ascii_case("IOS") => Some(Self::Ios),
            value if value.eq_ignore_ascii_case("Material") => Some(Self::Material),
            _ => None,
        }
    }

    fn index(self) -> u32 {
        match self {
            Self::Ios => 0,
            Self::Material => 1,
        }
    }

    fn from_index(index: u32) -> Self {
        if index == 0 {
            Self::Ios
        } else {
            Self::Material
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerSlotAction {
    None,
    Emoji,
    Stickers,
    Gifs,
    Attach,
    Photos,
    File,
    Location,
}

pub const COMPOSER_SLOTS: [ComposerSlotAction; 8] = [
    ComposerSlotAction::None,
    ComposerSlotAction::Emoji,
    ComposerSlotAction::Stickers,
    ComposerSlotAction::Gifs,
    ComposerSlotAction::Attach,
    ComposerSlotAction::Photos,
    ComposerSlotAction::File,
    ComposerSlotAction::Location,
];

impl ComposerSlotAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Emoji => "Emoji",
            Self::Stickers => "Stickers",
            Self::Gifs => "Gifs",
            Self::Attach => "Attach",
            Self::Photos => "Photos",
            Self::File => "File",
            Self::Location => "Location",
        }
    }

    pub fn from_setting(value: &str) -> Option<Self> {
        COMPOSER_SLOTS
            .into_iter()
            .find(|slot| slot.as_str().eq_ignore_ascii_case(value))
    }

    fn index(self) -> u32 {
        COMPOSER_SLOTS
            .iter()
            .position(|slot| *slot == self)
            .unwrap_or(0) as u32
    }

    fn from_index(index: u32) -> Self {
        COMPOSER_SLOTS
            .get(index as usize)
            .copied()
            .unwrap_or(Self::None)
    }

    fn key(self) -> &'static str {
        match self {
            Self::None => "feature-settings-settings_slot_none",
            Self::Emoji => "feature-settings-settings_slot_emoji",
            Self::Stickers => "feature-settings-settings_slot_stickers",
            Self::Gifs => "feature-settings-settings_slot_gifs",
            Self::Attach => "feature-settings-settings_slot_attach",
            Self::Photos => "feature-settings-settings_slot_photos",
            Self::File => "feature-settings-settings_slot_file",
            Self::Location => "feature-settings-settings_slot_location",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramWallpaper {
    pub id: i64,
    pub access_hash: i64,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WallpaperConfig {
    pub mode: WallpaperMode,
    pub path: Option<String>,
    pub telegram: Option<TelegramWallpaper>,
    pub pattern: bool,
    pub colors: Vec<i32>,
    pub dim: f32,
    pub blur: bool,
    pub intensity: i32,
    pub rotation: i32,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            mode: WallpaperMode::Monogram,
            path: None,
            telegram: None,
            pattern: true,
            colors: Vec::new(),
            dim: 0.15,
            blur: false,
            intensity: 50,
            rotation: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceState {
    pub theme: ThemePreference,
    pub accent: AccentPreset,
    pub message_text_size: u32,
    pub line_spacing: f32,
    pub letter_spacing: f32,
    pub preview_lines: u32,
    pub show_chat_avatars: bool,
    pub open_profile_on_avatar_tap: bool,
    pub show_read_status: bool,
    pub show_muted_counter: bool,
    pub folders_at_bottom: bool,
    pub composer_style: ComposerStyle,
    pub left_composer_action: ComposerSlotAction,
    pub right_composer_action: ComposerSlotAction,
    pub send_by_enter: bool,
    pub fix_link_previews: bool,
    pub wallpaper: WallpaperConfig,
}

impl Default for AppearanceState {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            accent: AccentPreset::Monogram,
            message_text_size: DEFAULT_MESSAGE_TEXT_SIZE,
            line_spacing: DEFAULT_LINE_SPACING,
            letter_spacing: DEFAULT_LETTER_SPACING,
            preview_lines: MIN_PREVIEW_LINES,
            show_chat_avatars: true,
            open_profile_on_avatar_tap: true,
            show_read_status: true,
            show_muted_counter: false,
            folders_at_bottom: false,
            composer_style: ComposerStyle::Material,
            left_composer_action: ComposerSlotAction::Attach,
            right_composer_action: ComposerSlotAction::Emoji,
            send_by_enter: false,
            fix_link_previews: true,
            wallpaper: WallpaperConfig::default(),
        }
    }
}

impl AppearanceState {
    pub fn preview_text_lines(&self) -> u32 {
        (self.preview_lines - 1).max(1)
    }
}

pub fn clamp_message_text_size(raw: u32) -> u32 {
    raw.clamp(MIN_MESSAGE_TEXT_SIZE, MAX_MESSAGE_TEXT_SIZE)
}

pub fn clamp_preview_lines(raw: u32) -> u32 {
    raw.clamp(MIN_PREVIEW_LINES, MAX_PREVIEW_LINES)
}

pub fn clamp_line_spacing(raw: f32) -> f32 {
    let clamped = raw.clamp(MIN_LINE_SPACING, MAX_LINE_SPACING);
    (clamped * 20.0).round() / 20.0
}

pub fn clamp_letter_spacing(raw: f32) -> f32 {
    let clamped = raw.clamp(MIN_LETTER_SPACING, MAX_LETTER_SPACING);
    (clamped * 100.0).round() / 100.0
}

pub fn clamp_wallpaper_dim(raw: f32) -> f32 {
    raw.clamp(MIN_WALLPAPER_DIM, MAX_WALLPAPER_DIM)
}

pub fn clamp_wallpaper_intensity(raw: i32) -> i32 {
    raw.clamp(MIN_WALLPAPER_INTENSITY, MAX_WALLPAPER_INTENSITY)
}

pub fn clamp_wallpaper_rotation(raw: i32) -> i32 {
    raw.rem_euclid(360)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (width > 0 && height > 0 && pixels.len() == expected).then_some(Self {
            width,
            height,
            pixels,
        })
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * self.width + x) * 4) as usize;
        self.pixels[offset..offset + 4]
            .try_into()
            .expect("RGBA pixel")
    }

    fn set_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        let offset = ((y * self.width + x) * 4) as usize;
        self.pixels[offset..offset + 4].copy_from_slice(&color);
    }
}

fn wallpaper_color(value: i32) -> [u8; 4] {
    let rgb = value as u32;
    [
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
        255,
    ]
}

fn default_wallpaper_colors() -> Vec<i32> {
    vec![0x006A8C, 0x6750A4, 0x77D1F5]
}

fn gradient_image(colors: &[i32], width: u32, height: u32) -> RgbaImage {
    let colors = if colors.is_empty() {
        default_wallpaper_colors()
    } else {
        colors.to_vec()
    };
    let stops = colors
        .iter()
        .map(|color| wallpaper_color(*color))
        .collect::<Vec<_>>();
    let mut output = RgbaImage::new(width, height, vec![0; width as usize * height as usize * 4])
        .expect("nonzero wallpaper dimensions");
    for y in 0..height {
        for x in 0..width {
            let horizontal = x as f32 / width.saturating_sub(1).max(1) as f32;
            let vertical = y as f32 / height.saturating_sub(1).max(1) as f32;
            let position = (horizontal + vertical) * 0.5 * stops.len().saturating_sub(1) as f32;
            let left = position.floor() as usize;
            let right = (left + 1).min(stops.len() - 1);
            let amount = position - left as f32;
            let mut color = [0; 4];
            for channel in 0..4 {
                color[channel] = (stops[left][channel] as f32
                    + (stops[right][channel] as f32 - stops[left][channel] as f32) * amount)
                    .round() as u8;
            }
            output.set_pixel(x, y, color);
        }
    }
    output
}

fn resized_image(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let mut output = RgbaImage::new(width, height, vec![0; width as usize * height as usize * 4])
        .expect("nonzero wallpaper dimensions");
    for y in 0..height {
        let source_y = ((y as u64 * source.height as u64) / height as u64) as u32;
        for x in 0..width {
            let source_x = ((x as u64 * source.width as u64) / width as u64) as u32;
            output.set_pixel(x, y, source.pixel(source_x, source_y));
        }
    }
    output
}

fn cover_image(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let scale = (width as f32 / source.width as f32).max(height as f32 / source.height as f32);
    let mut output = RgbaImage::new(width, height, vec![0; width as usize * height as usize * 4])
        .expect("nonzero wallpaper dimensions");
    for y in 0..height {
        let source_y = ((y as f32 + 0.5) / scale) as u32;
        for x in 0..width {
            let source_x = ((x as f32 + 0.5) / scale) as u32;
            output.set_pixel(
                x,
                y,
                source.pixel(
                    source_x.min(source.width - 1),
                    source_y.min(source.height - 1),
                ),
            );
        }
    }
    output
}

fn intensity_channel(channel: u8, intensity: i32) -> u8 {
    let channel = channel as i32;
    if intensity >= 0 {
        (channel + (255 - channel) * intensity / 100).clamp(0, 255) as u8
    } else {
        (channel * (100 + intensity) / 100).clamp(0, 255) as u8
    }
}

fn apply_intensity(image: &mut RgbaImage, intensity: i32) {
    for pixel in image.pixels.as_chunks_mut::<4>().0 {
        for channel in &mut pixel[..3] {
            *channel = intensity_channel(*channel, intensity);
        }
    }
}

fn rotate_image(source: &RgbaImage, rotation: i32) -> RgbaImage {
    if rotation == 0 {
        return source.clone();
    }
    let angle = (rotation as f32).to_radians();
    let (sin, cos) = angle.sin_cos();
    let cover_scale = sin.abs() + cos.abs();
    let center_x = source.width as f32 * 0.5;
    let center_y = source.height as f32 * 0.5;
    let mut output = source.clone();
    for y in 0..source.height {
        for x in 0..source.width {
            let dx = (x as f32 + 0.5 - center_x) / cover_scale;
            let dy = (y as f32 + 0.5 - center_y) / cover_scale;
            let source_x = dx * cos + dy * sin + center_x - 0.5;
            let source_y = -dx * sin + dy * cos + center_y - 0.5;
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < source.width as f32
                && source_y < source.height as f32
            {
                output.set_pixel(x, y, source.pixel(source_x as u32, source_y as u32));
            }
        }
    }
    output
}

fn blur_axis(source: &RgbaImage, radius: u32, horizontal: bool) -> RgbaImage {
    let mut output = source.clone();
    let outer = if horizontal {
        source.height
    } else {
        source.width
    };
    let inner = if horizontal {
        source.width
    } else {
        source.height
    };
    let divisor = radius * 2 + 1;
    for fixed in 0..outer {
        let mut sums = [0_u32; 4];
        for offset in -(radius as i64)..=radius as i64 {
            let position = offset.clamp(0, inner as i64 - 1) as u32;
            let pixel = if horizontal {
                source.pixel(position, fixed)
            } else {
                source.pixel(fixed, position)
            };
            for channel in 0..4 {
                sums[channel] += pixel[channel] as u32;
            }
        }
        for position in 0..inner {
            let color = sums.map(|sum| (sum / divisor) as u8);
            if horizontal {
                output.set_pixel(position, fixed, color);
            } else {
                output.set_pixel(fixed, position, color);
            }
            let remove = (position as i64 - radius as i64).clamp(0, inner as i64 - 1) as u32;
            let add = (position as i64 + radius as i64 + 1).clamp(0, inner as i64 - 1) as u32;
            let removed = if horizontal {
                source.pixel(remove, fixed)
            } else {
                source.pixel(fixed, remove)
            };
            let added = if horizontal {
                source.pixel(add, fixed)
            } else {
                source.pixel(fixed, add)
            };
            for channel in 0..4 {
                sums[channel] = sums[channel] + added[channel] as u32 - removed[channel] as u32;
            }
        }
    }
    output
}

fn blur_image(source: &RgbaImage, radius: u32) -> RgbaImage {
    if radius == 0 {
        return source.clone();
    }
    let horizontal = blur_axis(source, radius, true);
    blur_axis(&horizontal, radius, false)
}

fn blend_pixel(background: &mut RgbaImage, x: u32, y: u32, color: [u8; 4], opacity: f32) {
    let old = background.pixel(x, y);
    let alpha = (color[3] as f32 / 255.0 * opacity).clamp(0.0, 1.0);
    let mut result = [0; 4];
    for channel in 0..3 {
        result[channel] =
            (color[channel] as f32 * alpha + old[channel] as f32 * (1.0 - alpha)).round() as u8;
    }
    result[3] = 255;
    background.set_pixel(x, y, result);
}

fn add_procedural_pattern(image: &mut RgbaImage) {
    for y in 0..image.height {
        for x in 0..image.width {
            let diagonal = (x + y) % 48;
            let reverse = (x + image.height - 1 - y) % 48;
            if diagonal < 2 || reverse < 2 {
                blend_pixel(image, x, y, [255, 255, 255, 255], 0.12);
            }
        }
    }
}

fn add_image_pattern(background: &mut RgbaImage, source: &RgbaImage, intensity: i32) {
    let tile_width = source.width.clamp(16, 160);
    let tile_height =
        ((tile_width as u64 * source.height as u64) / source.width as u64).clamp(1, 160) as u32;
    let tile = resized_image(source, tile_width, tile_height);
    let opacity = intensity.unsigned_abs() as f32 / 100.0;
    for y in 0..background.height {
        for x in 0..background.width {
            let color = tile.pixel(x % tile_width, y % tile_height);
            blend_pixel(background, x, y, color, opacity);
        }
    }
}

pub fn render_wallpaper(
    config: &WallpaperConfig,
    source: Option<&RgbaImage>,
    width: u32,
    height: u32,
) -> Option<RgbaImage> {
    if config.mode == WallpaperMode::None || width == 0 || height == 0 {
        return None;
    }
    let mut output = if config.pattern {
        let mut background = gradient_image(&config.colors, width, height);
        if let Some(source) = source {
            add_image_pattern(&mut background, source, config.intensity);
        } else {
            add_procedural_pattern(&mut background);
        }
        background
    } else if let Some(source) = source {
        let mut image = cover_image(source, width, height);
        apply_intensity(&mut image, config.intensity);
        image
    } else {
        gradient_image(&config.colors, width, height)
    };
    output = rotate_image(&output, config.rotation);
    if config.blur {
        output = blur_image(&output, (width.min(height) / 100).max(2));
    }
    let dim = clamp_wallpaper_dim(config.dim);
    for pixel in output.pixels.as_chunks_mut::<4>().0 {
        for channel in &mut pixel[..3] {
            *channel = (*channel as f32 * (1.0 - dim)).round() as u8;
        }
    }
    Some(output)
}

fn load_wallpaper_source(path: &str) -> Option<RgbaImage> {
    let texture = gtk::gdk::Texture::from_filename(path).ok()?;
    let mut downloader = gtk::gdk::TextureDownloader::new(&texture);
    downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    let width = texture.width() as u32;
    let height = texture.height() as u32;
    let row_bytes = width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for row in bytes.as_ref().chunks(stride).take(height as usize) {
        pixels.extend_from_slice(row.get(..row_bytes)?);
    }
    RgbaImage::new(width, height, pixels)
}

pub fn wallpaper_texture(
    config: &WallpaperConfig,
    width: u32,
    height: u32,
) -> Option<gtk::gdk::MemoryTexture> {
    let source = config.path.as_deref().and_then(load_wallpaper_source);
    let image = render_wallpaper(config, source.as_ref(), width, height)?;
    let bytes = gtk::glib::Bytes::from_owned(image.pixels);
    Some(gtk::gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    ))
}

pub fn line_spacing_percent(value: f32) -> u32 {
    (value * 100.0).round() as u32
}

pub fn letter_spacing_hundredths(value: f32) -> u32 {
    (value * 100.0).round() as u32
}

fn darken_hex(hex: &str, factor: f64) -> String {
    let rgb = hex.strip_prefix('#').unwrap_or(hex);
    let value = u32::from_str_radix(rgb, 16).unwrap_or(0);
    let channel = |shift: u32| {
        ((((value >> shift) & 0xff) as f64 * factor)
            .round()
            .clamp(0.0, 255.0)) as u32
    };
    format!("#{:02X}{:02X}{:02X}", channel(16), channel(8), channel(0))
}

fn accent_css(accent: AccentPreset) -> String {
    let (light_bg, dark_bg) = accent.colors();
    let light_fg = "#ffffff";
    let dark_fg = darken_hex(light_bg, 0.3);
    format!(
        "window {{\n  --accent-bg-color: {light_bg};\n  --accent-fg-color: {light_fg};\n  --accent-color: {light_bg};\n}}\n\
         window.dark {{\n  --accent-bg-color: {dark_bg};\n  --accent-fg-color: {dark_fg};\n  --accent-color: {dark_bg};\n}}\n"
    )
}

fn typography_css(state: &AppearanceState) -> String {
    format!(
        ".monogram-message-text {{\n  font-size: {}pt;\n  line-height: {:.2};\n  letter-spacing: {:.2}em;\n}}\n",
        state.message_text_size, state.line_spacing, state.letter_spacing
    )
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum AppearanceCommand {
    SetAccent(AccentPreset),
    SetMessageTextSize(u32),
    SetLineSpacing(f32),
    SetLetterSpacing(f32),
    SetPreviewLines(u32),
    SetShowChatAvatars(bool),
    SetOpenProfileOnAvatarTap(bool),
    SetShowReadStatus(bool),
    SetShowMutedCounter(bool),
    SetFoldersAtBottom(bool),
    SetComposerStyle(ComposerStyle),
    SetLeftComposerAction(ComposerSlotAction),
    SetRightComposerAction(ComposerSlotAction),
    SetSendByEnter(bool),
    SetFixLinkPreviews(bool),
    SetWallpaper(WallpaperConfig),
}

pub enum AppearanceInput {
    SetI18n(Rc<I18n>),
    Restore(AppearanceState),
    Theme(u32),
    Accent(u32),
    MessageTextSize(u32),
    LineSpacing(f32),
    LetterSpacing(f32),
    PreviewLines(u32),
    ShowChatAvatars(bool),
    OpenProfileOnAvatarTap(bool),
    ShowReadStatus(bool),
    ShowMutedCounter(bool),
    FoldersAtBottom(bool),
    ComposerStyle(u32),
    LeftComposerAction(u32),
    RightComposerAction(u32),
    SendByEnter(bool),
    FixLinkPreviews(bool),
    WallpaperMode(u32),
    WallpaperDim(f32),
    WallpaperBlur(bool),
    WallpaperIntensity(i32),
    WallpaperRotation(i32),
    WallpaperPath(Option<String>),
    WallpaperCatalog(Box<WallpaperCatalogDto>),
    WallpaperLoaded(WallpaperFile),
    WallpaperFailed(Option<String>),
    SelectTelegramWallpaper(usize),
    BrowseTelegramWallpapers,
    CloseTelegramWallpapers,
    Back,
}

impl std::fmt::Debug for AppearanceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SetI18n(_) => "SetI18n",
            Self::Restore(_) => "Restore",
            Self::Theme(_) => "Theme",
            Self::Accent(_) => "Accent",
            Self::MessageTextSize(_) => "MessageTextSize",
            Self::LineSpacing(_) => "LineSpacing",
            Self::LetterSpacing(_) => "LetterSpacing",
            Self::PreviewLines(_) => "PreviewLines",
            Self::ShowChatAvatars(_) => "ShowChatAvatars",
            Self::OpenProfileOnAvatarTap(_) => "OpenProfileOnAvatarTap",
            Self::ShowReadStatus(_) => "ShowReadStatus",
            Self::ShowMutedCounter(_) => "ShowMutedCounter",
            Self::FoldersAtBottom(_) => "FoldersAtBottom",
            Self::ComposerStyle(_) => "ComposerStyle",
            Self::LeftComposerAction(_) => "LeftComposerAction",
            Self::RightComposerAction(_) => "RightComposerAction",
            Self::SendByEnter(_) => "SendByEnter",
            Self::FixLinkPreviews(_) => "FixLinkPreviews",
            Self::WallpaperMode(_) => "WallpaperMode",
            Self::WallpaperDim(_) => "WallpaperDim",
            Self::WallpaperBlur(_) => "WallpaperBlur",
            Self::WallpaperIntensity(_) => "WallpaperIntensity",
            Self::WallpaperRotation(_) => "WallpaperRotation",
            Self::WallpaperPath(_) => "WallpaperPath",
            Self::WallpaperCatalog(_) => "WallpaperCatalog",
            Self::WallpaperLoaded(_) => "WallpaperLoaded",
            Self::WallpaperFailed(_) => "WallpaperFailed",
            Self::SelectTelegramWallpaper(_) => "SelectTelegramWallpaper",
            Self::BrowseTelegramWallpapers => "BrowseTelegramWallpapers",
            Self::CloseTelegramWallpapers => "CloseTelegramWallpapers",
            Self::Back => "Back",
        })
    }
}

#[derive(Debug)]
pub enum AppearanceOutput {
    Back,
    Theme(ThemePreference),
    Command(AppearanceCommand),
    BrowseTelegramWallpapers,
    ImportLocalWallpaper {
        path: String,
    },
    DownloadTelegramWallpaper {
        id: i64,
        access_hash: i64,
        pattern: bool,
    },
}

#[derive(Clone, Debug)]
enum WallpaperPending {
    Local,
    Telegram(WallpaperDto),
}

pub struct Appearance {
    i18n: Rc<I18n>,
    state: AppearanceState,
    catalog: Vec<WallpaperDto>,
    catalog_hash: i64,
    catalog_visible: bool,
    catalog_loading: bool,
    wallpaper_pending: Option<WallpaperPending>,
    wallpaper_error: Option<String>,
    accent_provider: Option<gtk::CssProvider>,
    typography_provider: Option<gtk::CssProvider>,
    applied_accent_css: Option<String>,
    applied_typography_css: Option<String>,
    applied_theme: Option<ThemePreference>,
    rendering: Rc<Cell<bool>>,
}

struct ScaleSpec {
    title: String,
    value_label: String,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
}

fn local_wallpaper_config(current: &WallpaperConfig, file: &WallpaperFile) -> WallpaperConfig {
    WallpaperConfig {
        mode: WallpaperMode::Image,
        path: Some(file.path.clone()),
        telegram: None,
        pattern: file.pattern,
        colors: file.colors.clone(),
        dim: current.dim,
        blur: file.blur,
        intensity: file.intensity.unwrap_or(if file.pattern { 50 } else { 0 }),
        rotation: clamp_wallpaper_rotation(file.rotation),
    }
}

fn telegram_wallpaper_config(
    current: &WallpaperConfig,
    wallpaper: &WallpaperDto,
    file: Option<&WallpaperFile>,
) -> WallpaperConfig {
    WallpaperConfig {
        mode: WallpaperMode::Image,
        path: file.map(|file| file.path.clone()),
        telegram: Some(TelegramWallpaper {
            id: wallpaper.id,
            access_hash: wallpaper.access_hash,
            slug: wallpaper.slug.clone(),
        }),
        pattern: wallpaper.pattern,
        colors: wallpaper.colors.clone(),
        dim: current.dim,
        blur: wallpaper.blur,
        intensity: wallpaper
            .intensity
            .unwrap_or(if wallpaper.pattern { 50 } else { 0 }),
        rotation: clamp_wallpaper_rotation(wallpaper.rotation),
    }
}

impl Appearance {
    fn new(i18n: Rc<I18n>) -> Self {
        let theme = match adw::StyleManager::default().color_scheme() {
            adw::ColorScheme::ForceLight | adw::ColorScheme::PreferLight => ThemePreference::Light,
            adw::ColorScheme::ForceDark | adw::ColorScheme::PreferDark => ThemePreference::Dark,
            _ => ThemePreference::System,
        };
        let state = AppearanceState {
            theme,
            ..Default::default()
        };
        Self {
            i18n,
            state,
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

    fn text(&self, key: &str) -> String {
        self.i18n.text(key, &[]).unwrap_or_else(|_| key.into())
    }

    fn text_arg(&self, key: &str, value: &str) -> String {
        self.i18n
            .text(key, &[("arg1", value)])
            .unwrap_or_else(|_| key.into())
    }

    fn message_text_size_label(&self) -> String {
        self.text_arg(
            "feature-settings-settings_message_text_size_value",
            &self.state.message_text_size.to_string(),
        )
    }

    fn line_spacing_label(&self) -> String {
        let percent = line_spacing_percent(self.state.line_spacing);
        if percent == 100 {
            self.text("feature-settings-settings_line_spacing_default")
        } else {
            self.text_arg(
                "feature-settings-settings_line_spacing_value",
                &percent.to_string(),
            )
        }
    }

    fn letter_spacing_label(&self) -> String {
        let hundredths = letter_spacing_hundredths(self.state.letter_spacing);
        if hundredths == 0 {
            self.text("feature-settings-settings_line_spacing_default")
        } else {
            self.text_arg(
                "feature-settings-settings_letter_spacing_value",
                &hundredths.to_string(),
            )
        }
    }

    fn handle(&mut self, message: AppearanceInput) -> Vec<AppearanceOutput> {
        let output = match message {
            AppearanceInput::SetI18n(i18n) => {
                self.i18n = i18n;
                None
            }
            AppearanceInput::Restore(state) => {
                self.state = state;
                None
            }
            AppearanceInput::Back => Some(AppearanceOutput::Back),
            AppearanceInput::CloseTelegramWallpapers => {
                self.catalog_visible = false;
                None
            }
            AppearanceInput::BrowseTelegramWallpapers => {
                self.catalog_visible = true;
                if self.catalog_loading {
                    None
                } else {
                    self.catalog_loading = true;
                    self.wallpaper_error = None;
                    Some(AppearanceOutput::BrowseTelegramWallpapers)
                }
            }
            AppearanceInput::WallpaperCatalog(response) => {
                self.catalog_loading = false;
                self.catalog_hash = response.hash;
                if !response.not_modified || self.catalog.is_empty() {
                    self.catalog = response.wallpapers;
                }
                self.wallpaper_error = None;
                None
            }
            AppearanceInput::WallpaperFailed(error) => {
                self.catalog_loading = false;
                self.wallpaper_pending = None;
                self.wallpaper_error = error;
                None
            }
            AppearanceInput::SelectTelegramWallpaper(index) => {
                let Some(wallpaper) = self.catalog.get(index).cloned() else {
                    return Vec::new();
                };
                if self.wallpaper_pending.is_some() {
                    return Vec::new();
                }
                self.wallpaper_error = None;
                if wallpaper.document_id.is_some() {
                    self.wallpaper_pending = Some(WallpaperPending::Telegram(wallpaper.clone()));
                    Some(AppearanceOutput::DownloadTelegramWallpaper {
                        id: wallpaper.id,
                        access_hash: wallpaper.access_hash,
                        pattern: wallpaper.pattern,
                    })
                } else {
                    self.wallpaper_commit(|config| {
                        telegram_wallpaper_config(config, &wallpaper, None)
                    })
                }
            }
            AppearanceInput::WallpaperLoaded(file) => {
                let Some(pending) = self.wallpaper_pending.take() else {
                    return Vec::new();
                };
                self.wallpaper_error = None;
                match pending {
                    WallpaperPending::Local => {
                        self.wallpaper_commit(|config| local_wallpaper_config(config, &file))
                    }
                    WallpaperPending::Telegram(wallpaper) => self.wallpaper_commit(|config| {
                        telegram_wallpaper_config(config, &wallpaper, Some(&file))
                    }),
                }
            }
            AppearanceInput::Theme(index) => {
                let preference = ThemePreference::from_index(index);
                (self.state.theme != preference).then(|| {
                    self.state.theme = preference;
                    AppearanceOutput::Theme(preference)
                })
            }
            AppearanceInput::Accent(index) => {
                let accent = AccentPreset::from_index(index);
                (self.state.accent != accent).then(|| {
                    self.state.accent = accent;
                    AppearanceOutput::Command(AppearanceCommand::SetAccent(accent))
                })
            }
            AppearanceInput::MessageTextSize(size) => {
                let size = clamp_message_text_size(size);
                (self.state.message_text_size != size).then(|| {
                    self.state.message_text_size = size;
                    AppearanceOutput::Command(AppearanceCommand::SetMessageTextSize(size))
                })
            }
            AppearanceInput::LineSpacing(value) => {
                let value = clamp_line_spacing(value);
                (self.state.line_spacing != value).then(|| {
                    self.state.line_spacing = value;
                    AppearanceOutput::Command(AppearanceCommand::SetLineSpacing(value))
                })
            }
            AppearanceInput::LetterSpacing(value) => {
                let value = clamp_letter_spacing(value);
                (self.state.letter_spacing != value).then(|| {
                    self.state.letter_spacing = value;
                    AppearanceOutput::Command(AppearanceCommand::SetLetterSpacing(value))
                })
            }
            AppearanceInput::PreviewLines(lines) => {
                let lines = clamp_preview_lines(lines);
                (self.state.preview_lines != lines).then(|| {
                    self.state.preview_lines = lines;
                    AppearanceOutput::Command(AppearanceCommand::SetPreviewLines(lines))
                })
            }
            AppearanceInput::ShowChatAvatars(enabled) => (self.state.show_chat_avatars != enabled)
                .then(|| {
                    self.state.show_chat_avatars = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetShowChatAvatars(enabled))
                }),
            AppearanceInput::OpenProfileOnAvatarTap(enabled) => {
                (self.state.open_profile_on_avatar_tap != enabled).then(|| {
                    self.state.open_profile_on_avatar_tap = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetOpenProfileOnAvatarTap(enabled))
                })
            }
            AppearanceInput::ShowReadStatus(enabled) => (self.state.show_read_status != enabled)
                .then(|| {
                    self.state.show_read_status = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetShowReadStatus(enabled))
                }),
            AppearanceInput::ShowMutedCounter(enabled) => {
                (self.state.show_muted_counter != enabled).then(|| {
                    self.state.show_muted_counter = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetShowMutedCounter(enabled))
                })
            }
            AppearanceInput::FoldersAtBottom(enabled) => (self.state.folders_at_bottom != enabled)
                .then(|| {
                    self.state.folders_at_bottom = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetFoldersAtBottom(enabled))
                }),
            AppearanceInput::ComposerStyle(index) => {
                let style = ComposerStyle::from_index(index);
                (self.state.composer_style != style).then(|| {
                    self.state.composer_style = style;
                    AppearanceOutput::Command(AppearanceCommand::SetComposerStyle(style))
                })
            }
            AppearanceInput::LeftComposerAction(index) => {
                let action = ComposerSlotAction::from_index(index);
                (self.state.left_composer_action != action).then(|| {
                    self.state.left_composer_action = action;
                    AppearanceOutput::Command(AppearanceCommand::SetLeftComposerAction(action))
                })
            }
            AppearanceInput::RightComposerAction(index) => {
                let action = ComposerSlotAction::from_index(index);
                (self.state.right_composer_action != action).then(|| {
                    self.state.right_composer_action = action;
                    AppearanceOutput::Command(AppearanceCommand::SetRightComposerAction(action))
                })
            }
            AppearanceInput::SendByEnter(enabled) => {
                (self.state.send_by_enter != enabled).then(|| {
                    self.state.send_by_enter = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetSendByEnter(enabled))
                })
            }
            AppearanceInput::FixLinkPreviews(enabled) => (self.state.fix_link_previews != enabled)
                .then(|| {
                    self.state.fix_link_previews = enabled;
                    AppearanceOutput::Command(AppearanceCommand::SetFixLinkPreviews(enabled))
                }),
            AppearanceInput::WallpaperMode(index) => {
                let mode = WallpaperMode::from_index(index);
                if mode == WallpaperMode::Image
                    && self.state.wallpaper.path.is_none()
                    && self.state.wallpaper.telegram.is_none()
                {
                    return Vec::new();
                }
                if mode == WallpaperMode::Monogram {
                    self.wallpaper_commit(|_| WallpaperConfig::default())
                } else {
                    self.wallpaper_commit(|config| WallpaperConfig {
                        mode,
                        path: if mode == WallpaperMode::Image {
                            config.path.clone()
                        } else {
                            None
                        },
                        telegram: if mode == WallpaperMode::Image {
                            config.telegram.clone()
                        } else {
                            None
                        },
                        ..config.clone()
                    })
                }
            }
            AppearanceInput::WallpaperDim(value) => {
                let value = clamp_wallpaper_dim(value);
                self.wallpaper_commit(|config| WallpaperConfig {
                    dim: value,
                    ..config.clone()
                })
            }
            AppearanceInput::WallpaperBlur(enabled) => {
                self.wallpaper_commit(|config| WallpaperConfig {
                    blur: enabled,
                    ..config.clone()
                })
            }
            AppearanceInput::WallpaperIntensity(value) => {
                let value = clamp_wallpaper_intensity(value);
                self.wallpaper_commit(|config| WallpaperConfig {
                    intensity: value,
                    ..config.clone()
                })
            }
            AppearanceInput::WallpaperRotation(value) => {
                let value = clamp_wallpaper_rotation(value);
                self.wallpaper_commit(|config| WallpaperConfig {
                    rotation: value,
                    ..config.clone()
                })
            }
            AppearanceInput::WallpaperPath(path) => match path {
                Some(path) if self.wallpaper_pending.is_none() => {
                    self.wallpaper_pending = Some(WallpaperPending::Local);
                    self.wallpaper_error = None;
                    Some(AppearanceOutput::ImportLocalWallpaper { path })
                }
                _ => None,
            },
        };
        output.into_iter().collect()
    }

    fn wallpaper_commit(
        &mut self,
        update: impl FnOnce(&WallpaperConfig) -> WallpaperConfig,
    ) -> Option<AppearanceOutput> {
        let next = update(&self.state.wallpaper);
        (next != self.state.wallpaper).then(|| {
            self.state.wallpaper = next.clone();
            AppearanceOutput::Command(AppearanceCommand::SetWallpaper(next))
        })
    }

    fn apply_css(&mut self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        if self.applied_theme != Some(self.state.theme) {
            self.state.theme.apply();
            self.applied_theme = Some(self.state.theme);
        }
        let accent = accent_css(self.state.accent);
        if self.applied_accent_css.as_deref() != Some(accent.as_str()) {
            self.replace_provider(&display, accent, true);
        }
        let typography = typography_css(&self.state);
        if self.applied_typography_css.as_deref() != Some(typography.as_str()) {
            self.replace_provider(&display, typography, false);
        }
    }

    fn replace_provider(&mut self, display: &gtk::gdk::Display, css: String, accent: bool) {
        let slot = if accent {
            &mut self.accent_provider
        } else {
            &mut self.typography_provider
        };
        if let Some(provider) = slot.take() {
            gtk::style_context_remove_provider_for_display(display, &provider);
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&css);
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        *slot = Some(provider);
        if accent {
            self.applied_accent_css = Some(css);
        } else {
            self.applied_typography_css = Some(css);
        }
    }

    fn render(&self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        self.rendering.set(true);
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some(
            &self.text("desktop-appearance"),
        ))));
        let back = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        back.connect_clicked(move |_| input.emit(AppearanceInput::Back));
        header.pack_start(&back);
        root.append(&header);

        let page = adw::PreferencesPage::builder().vexpand(true).build();
        page.add(&self.wallpaper_group(sender));
        page.add(&self.theme_group(sender));
        page.add(&self.typography_group(sender));
        page.add(&self.chat_list_group(sender));
        page.add(&self.composer_group(sender));
        root.append(&page);
        self.rendering.set(false);
    }

    fn wallpaper_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_wallpaper"))
            .description(self.text("feature-settings-settings_wallpaper_hero_sub"))
            .build();
        if self.state.wallpaper.mode == WallpaperMode::Image {
            let blur = self.switch_row(
                sender,
                &self.text("feature-settings-settings_wallpaper_blur"),
                None,
                self.state.wallpaper.blur,
                true,
                AppearanceInput::WallpaperBlur,
            );
            blur.set_widget_name("wallpaper-blur");
            group.add(&blur);
        }
        group.add(&self.wallpaper_preview());

        let monogram = self.mode_check(
            sender,
            &self.text("feature-settings-settings_wallpaper_reset"),
            WallpaperMode::Monogram,
            None,
        );
        group.add(&monogram);
        let none = self.mode_check(
            sender,
            &self.text("feature-settings-settings_wallpaper_none"),
            WallpaperMode::None,
            Some(&monogram),
        );
        group.add(&none);
        if self.state.wallpaper.path.is_some() || self.state.wallpaper.telegram.is_some() {
            let selected = self.mode_check(
                sender,
                &self.text("feature-settings-settings_wallpaper_selected"),
                WallpaperMode::Image,
                Some(&monogram),
            );
            group.add(&selected);
        }

        let title = self.text("feature-settings-settings_wallpaper_device");
        let choose = gtk::Button::with_label(&title);
        choose.set_widget_name("wallpaper-import");
        choose.set_sensitive(self.wallpaper_pending.is_none());
        let input = sender.input_sender().clone();
        choose.connect_clicked(move |choose| {
            let Some(parent) = root_window(choose) else {
                return;
            };
            Self::open_wallpaper_dialog(title.clone(), parent, input.clone());
        });
        group.add(&choose);

        let telegram =
            gtk::Button::with_label(&self.text("feature-settings-settings_wallpaper_telegram"));
        telegram.set_sensitive(!self.catalog_loading && self.wallpaper_pending.is_none());
        let input = sender.input_sender().clone();
        telegram.connect_clicked(move |_| input.emit(AppearanceInput::BrowseTelegramWallpapers));
        group.add(&telegram);
        group.add(&self.wallpaper_catalog(sender));

        if self.state.wallpaper.mode == WallpaperMode::Image {
            let dim_percent = (self.state.wallpaper.dim * 100.0).round() as u32;
            group.add(&self.scale_row(
                sender,
                ScaleSpec {
                    title: self.text_arg(
                        "feature-settings-settings_wallpaper_dim",
                        &dim_percent.to_string(),
                    ),
                    value_label: String::new(),
                    value: self.state.wallpaper.dim as f64,
                    min: MIN_WALLPAPER_DIM as f64,
                    max: MAX_WALLPAPER_DIM as f64,
                    step: 0.05,
                },
                |value| AppearanceInput::WallpaperDim(value as f32),
            ));
            group.add(&self.scale_row(
                sender,
                ScaleSpec {
                    title: self.text("feature-settings-settings_wallpaper_intensity"),
                    value_label: self.state.wallpaper.intensity.to_string(),
                    value: self.state.wallpaper.intensity as f64,
                    min: MIN_WALLPAPER_INTENSITY as f64,
                    max: MAX_WALLPAPER_INTENSITY as f64,
                    step: 5.0,
                },
                |value| AppearanceInput::WallpaperIntensity(value as i32),
            ));
            group.add(&self.scale_row(
                sender,
                ScaleSpec {
                    title: self.text("feature-settings-settings_wallpaper_rotation"),
                    value_label: self.state.wallpaper.rotation.to_string(),
                    value: self.state.wallpaper.rotation as f64,
                    min: 0.0,
                    max: 360.0,
                    step: 15.0,
                },
                |value| AppearanceInput::WallpaperRotation(value as i32),
            ));
        }
        group
    }

    fn wallpaper_preview(&self) -> gtk::Widget {
        let overlay = gtk::Overlay::new();
        overlay.set_widget_name("wallpaper-preview");
        overlay.set_size_request(-1, 160);
        if let Some(texture) = wallpaper_texture(&self.state.wallpaper, 480, 180) {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Cover);
            overlay.set_child(Some(&picture));
        } else {
            overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
        }
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 8);
        preview.set_valign(gtk::Align::Center);
        preview.set_margin_top(8);
        preview.set_margin_bottom(8);
        preview.set_margin_start(8);
        preview.set_margin_end(8);
        let incoming = gtk::Label::new(Some(
            &self.text("feature-settings-settings_wallpaper_sample_in"),
        ));
        incoming.add_css_class("card");
        incoming.set_wrap(true);
        incoming.set_xalign(0.0);
        preview.append(&incoming);
        let outgoing = gtk::Label::new(Some(
            &self.text("feature-settings-settings_wallpaper_sample_out"),
        ));
        outgoing.add_css_class("card");
        outgoing.set_wrap(true);
        outgoing.set_halign(gtk::Align::End);
        preview.append(&outgoing);
        overlay.add_overlay(&preview);
        overlay.upcast()
    }

    fn wallpaper_catalog(&self, sender: &ComponentSender<Self>) -> gtk::Widget {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 8);
        container.set_widget_name("wallpaper-catalog");
        container.set_visible(self.catalog_visible);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some(
            &self.text("feature-settings-settings_wallpaper_sources"),
        ));
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let spinner = gtk::Spinner::builder()
            .spinning(self.catalog_loading || self.wallpaper_pending.is_some())
            .visible(self.catalog_loading || self.wallpaper_pending.is_some())
            .build();
        header.append(&spinner);
        let close = gtk::Button::with_label(&self.text("desktop-back"));
        let input = sender.input_sender().clone();
        close.connect_clicked(move |_| input.emit(AppearanceInput::CloseTelegramWallpapers));
        header.append(&close);
        container.append(&header);
        if let Some(error) = &self.wallpaper_error {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = gtk::Label::new(Some(error));
            label.set_hexpand(true);
            label.set_wrap(true);
            row.append(&label);
            let retry = gtk::Button::with_label(&self.text("desktop-retry"));
            let input = sender.input_sender().clone();
            retry.connect_clicked(move |_| input.emit(AppearanceInput::BrowseTelegramWallpapers));
            row.append(&retry);
            container.append(&row);
        }
        let grid = gtk::Grid::builder()
            .column_spacing(8)
            .row_spacing(8)
            .build();
        grid.set_widget_name("wallpaper-grid");
        for (index, wallpaper) in self.catalog.iter().enumerate() {
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let config = telegram_wallpaper_config(&WallpaperConfig::default(), wallpaper, None);
            if let Some(texture) = wallpaper_texture(&config, 120, 80) {
                let picture = gtk::Picture::for_paintable(&texture);
                picture.set_size_request(120, 80);
                picture.set_content_fit(gtk::ContentFit::Cover);
                content.append(&picture);
            }
            let title = if wallpaper.slug.is_empty() {
                wallpaper.id.to_string()
            } else {
                wallpaper.slug.clone()
            };
            content.append(&gtk::Label::new(Some(&title)));
            let button = gtk::Button::new();
            button.set_sensitive(self.wallpaper_pending.is_none());
            button.set_child(Some(&content));
            button.set_widget_name(&format!("wallpaper-{}", wallpaper.id));
            let input = sender.input_sender().clone();
            button.connect_clicked(move |_| {
                input.emit(AppearanceInput::SelectTelegramWallpaper(index));
            });
            grid.attach(&button, (index % 3) as i32, (index / 3) as i32, 1, 1);
        }
        container.append(&grid);
        container.upcast()
    }

    fn theme_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_theme"))
            .build();
        let themes = string_list(
            &[
                ThemePreference::System,
                ThemePreference::Light,
                ThemePreference::Dark,
            ]
            .map(|preference| self.text(preference.key())),
        );
        let theme = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_theme"))
            .model(&themes)
            .selected(self.state.theme.index())
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        theme.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::Theme(row.selected()));
            }
        });
        group.add(&theme);

        let accents = string_list(
            &ACCENTS
                .iter()
                .map(|accent| self.text(accent.key()))
                .collect::<Vec<_>>(),
        );
        let accent = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_accent"))
            .model(&accents)
            .selected(self.state.accent.index())
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        accent.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::Accent(row.selected()));
            }
        });
        group.add(&accent);
        group
    }

    fn typography_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_message_text_size"))
            .build();
        group.add(&self.scale_row(
            sender,
            ScaleSpec {
                title: self.text("feature-settings-settings_message_text_size"),
                value_label: self.message_text_size_label(),
                value: self.state.message_text_size as f64,
                min: MIN_MESSAGE_TEXT_SIZE as f64,
                max: MAX_MESSAGE_TEXT_SIZE as f64,
                step: 1.0,
            },
            |value| AppearanceInput::MessageTextSize(value.round() as u32),
        ));
        group.add(&self.scale_row(
            sender,
            ScaleSpec {
                title: self.text("feature-settings-settings_line_spacing"),
                value_label: self.line_spacing_label(),
                value: self.state.line_spacing as f64,
                min: MIN_LINE_SPACING as f64,
                max: MAX_LINE_SPACING as f64,
                step: LINE_SPACING_STEP as f64,
            },
            |value| AppearanceInput::LineSpacing(value as f32),
        ));
        group.add(&self.scale_row(
            sender,
            ScaleSpec {
                title: self.text("feature-settings-settings_letter_spacing"),
                value_label: self.letter_spacing_label(),
                value: self.state.letter_spacing as f64,
                min: MIN_LETTER_SPACING as f64,
                max: MAX_LETTER_SPACING as f64,
                step: LETTER_SPACING_STEP as f64,
            },
            |value| AppearanceInput::LetterSpacing(value as f32),
        ));
        group
    }

    fn chat_list_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_chat_list"))
            .build();
        let lines = string_list(&[
            self.text("feature-settings-settings_preview_lines_two"),
            self.text("feature-settings-settings_preview_lines_three"),
        ]);
        let preview = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_preview_lines"))
            .model(&lines)
            .selected(self.state.preview_lines - MIN_PREVIEW_LINES)
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        preview.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::PreviewLines(
                    row.selected() + MIN_PREVIEW_LINES,
                ));
            }
        });
        group.add(&preview);
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_show_avatars"),
            Some(self.text("feature-settings-settings_show_avatars_sub")),
            self.state.show_chat_avatars,
            true,
            AppearanceInput::ShowChatAvatars,
        ));
        let avatar_tap_sub = if self.state.show_chat_avatars {
            self.text("feature-settings-settings_avatar_profile_tap_sub")
        } else {
            self.text("feature-settings-settings_avatar_profile_tap_unavailable")
        };
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_avatar_profile_tap"),
            Some(avatar_tap_sub),
            self.state.open_profile_on_avatar_tap,
            self.state.show_chat_avatars,
            AppearanceInput::OpenProfileOnAvatarTap,
        ));
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_show_read_status"),
            Some(self.text("feature-settings-settings_show_read_status_sub")),
            self.state.show_read_status,
            true,
            AppearanceInput::ShowReadStatus,
        ));
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_muted_counter"),
            Some(self.text("feature-settings-settings_muted_counter_sub")),
            self.state.show_muted_counter,
            true,
            AppearanceInput::ShowMutedCounter,
        ));
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_folders_at_bottom"),
            Some(self.text("feature-settings-settings_folders_at_bottom_sub")),
            self.state.folders_at_bottom,
            true,
            AppearanceInput::FoldersAtBottom,
        ));
        group
    }

    fn composer_group(&self, sender: &ComponentSender<Self>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(self.text("feature-settings-settings_input_bar"))
            .build();
        let styles = string_list(&[
            self.text("feature-settings-settings_input_bar_ios"),
            self.text("feature-settings-settings_input_bar_material"),
        ]);
        let style = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_input_bar"))
            .model(&styles)
            .selected(self.state.composer_style.index())
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        style.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::ComposerStyle(row.selected()));
            }
        });
        group.add(&style);

        let slots = string_list(
            &COMPOSER_SLOTS
                .iter()
                .map(|slot| self.text(slot.key()))
                .collect::<Vec<_>>(),
        );
        let left = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_input_bar_left"))
            .subtitle(self.text("feature-settings-settings_input_bar_left_sub"))
            .model(&slots)
            .selected(self.state.left_composer_action.index())
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        left.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::LeftComposerAction(row.selected()));
            }
        });
        group.add(&left);
        let right = adw::ComboRow::builder()
            .title(self.text("feature-settings-settings_input_bar_right"))
            .subtitle(self.text("feature-settings-settings_input_bar_right_sub"))
            .model(&slots)
            .selected(self.state.right_composer_action.index())
            .build();
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        right.connect_selected_notify(move |row| {
            if !rendering.get() {
                input.emit(AppearanceInput::RightComposerAction(row.selected()));
            }
        });
        group.add(&right);
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_input_bar_send_by_enter"),
            Some(self.text("feature-settings-settings_input_bar_send_by_enter_sub")),
            self.state.send_by_enter,
            true,
            AppearanceInput::SendByEnter,
        ));
        group.add(&self.switch_row(
            sender,
            &self.text("feature-settings-settings_input_bar_fix_previews"),
            Some(self.text("feature-settings-settings_input_bar_fix_previews_sub")),
            self.state.fix_link_previews,
            true,
            AppearanceInput::FixLinkPreviews,
        ));
        group
    }

    fn switch_row(
        &self,
        sender: &ComponentSender<Self>,
        title: &str,
        subtitle: Option<String>,
        active: bool,
        sensitive: bool,
        to_input: impl Fn(bool) -> AppearanceInput + 'static,
    ) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder()
            .title(title)
            .active(active)
            .sensitive(sensitive)
            .build();
        if let Some(subtitle) = subtitle {
            row.set_subtitle(&subtitle);
        }
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        row.connect_active_notify(move |_| {
            if !rendering.get() {
                input.emit(to_input(!active));
            }
        });
        row
    }

    fn scale_row(
        &self,
        sender: &ComponentSender<Self>,
        spec: ScaleSpec,
        to_input: impl Fn(f64) -> AppearanceInput + 'static,
    ) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = gtk::Label::new(Some(&spec.title));
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let value_label = gtk::Label::new(Some(&spec.value_label));
        value_label.add_css_class("dim-label");
        header.append(&value_label);
        row.append(&header);
        let scale =
            gtk::Scale::with_range(gtk::Orientation::Horizontal, spec.min, spec.max, spec.step);
        scale.set_value(spec.value);
        scale.set_draw_value(false);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        scale.connect_value_changed(move |scale| {
            if !rendering.get() {
                input.emit(to_input(scale.value()));
            }
        });
        row.append(&scale);
        row
    }

    fn mode_check(
        &self,
        sender: &ComponentSender<Self>,
        label: &str,
        mode: WallpaperMode,
        group: Option<&gtk::CheckButton>,
    ) -> gtk::CheckButton {
        let check = gtk::CheckButton::with_label(label);
        if let Some(group) = group {
            check.set_group(Some(group));
        }
        check.set_active(self.state.wallpaper.mode == mode);
        let input = sender.input_sender().clone();
        let rendering = self.rendering.clone();
        let index = mode.index();
        check.connect_toggled(move |check| {
            if check.is_active() && !rendering.get() {
                input.emit(AppearanceInput::WallpaperMode(index));
            }
        });
        check
    }

    fn open_wallpaper_dialog(
        title: String,
        parent: gtk::Window,
        input: relm4::Sender<AppearanceInput>,
    ) {
        let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("image/*");
        let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));
        dialog.open(Some(&parent), gtk::gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file
                .path()
                .and_then(|path| path.into_os_string().into_string().ok())
            else {
                return;
            };
            input.emit(AppearanceInput::WallpaperPath(Some(path)));
        });
    }
}

fn root_window(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.root()?.downcast::<gtk::Window>().ok()
}

fn string_list(labels: &[String]) -> gtk::StringList {
    gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())
}

impl SimpleComponent for Appearance {
    type Init = Rc<I18n>;
    type Input = AppearanceInput;
    type Output = AppearanceOutput;
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
        let model = Self::new(i18n);
        model.render(&root, &sender);
        ComponentParts {
            model,
            widgets: root,
        }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        let outputs = self.handle(message);
        for output in outputs {
            let _ = sender.output(output);
        }
        self.apply_css();
    }

    fn update_view(&self, root: &mut Self::Widgets, sender: ComponentSender<Self>) {
        self.render(root, &sender);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/appearance_ui.rs"]
pub(crate) mod tests;
