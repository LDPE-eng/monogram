use super::*;
use std::path::PathBuf;

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    bytes.extend_from_slice(&13_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes
}

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    vec![
        0xFF,
        0xD8,
        0xFF,
        0xE0,
        0x00,
        0x04,
        0x00,
        0x00,
        0xFF,
        0xC0,
        0x00,
        0x11,
        0x08,
        (height >> 8) as u8,
        height as u8,
        (width >> 8) as u8,
        width as u8,
        0x03,
        0x01,
        0x00,
        0x02,
        0x11,
        0x03,
        0x11,
    ]
}

fn webp_vp8(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0_u8; 30];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[8..12].copy_from_slice(b"WEBP");
    bytes[12..16].copy_from_slice(b"VP8 ");
    bytes[20..23].copy_from_slice(&[0x9D, 0x01, 0x2A]);
    bytes[26..28].copy_from_slice(&(width & 0x3FFF).to_le_bytes());
    bytes[28..30].copy_from_slice(&(height & 0x3FFF).to_le_bytes());
    bytes
}

fn webp_vp8l(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0_u8; 25];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[8..12].copy_from_slice(b"WEBP");
    bytes[12..16].copy_from_slice(b"VP8L");
    bytes[20] = 0x2F;
    let bits = (width - 1) | ((height - 1) << 14);
    bytes[21..25].copy_from_slice(&bits.to_le_bytes());
    bytes
}

fn webp_vp8x(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0_u8; 30];
    bytes[0..4].copy_from_slice(b"RIFF");
    bytes[8..12].copy_from_slice(b"WEBP");
    bytes[12..16].copy_from_slice(b"VP8X");
    bytes[24..27].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    bytes[27..30].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
    bytes
}

fn gif(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GIF89a");
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes
}

fn bmp(width: i32, height: i32) -> Vec<u8> {
    let mut bytes = vec![0_u8; 26];
    bytes[0..2].copy_from_slice(b"BM");
    bytes[18..22].copy_from_slice(&width.to_le_bytes());
    bytes[22..26].copy_from_slice(&height.to_le_bytes());
    bytes
}

#[test]
fn detects_png_dimensions() {
    assert_eq!(image_dimensions(&png(320, 240)), Some((320, 240)));
}

#[test]
fn detects_jpeg_dimensions() {
    assert_eq!(image_dimensions(&jpeg(640, 480)), Some((640, 480)));
}

#[test]
fn detects_webp_lossy_dimensions() {
    assert_eq!(image_dimensions(&webp_vp8(1280, 720)), Some((1280, 720)));
}

#[test]
fn detects_webp_lossless_dimensions() {
    assert_eq!(image_dimensions(&webp_vp8l(64, 32)), Some((64, 32)));
}

#[test]
fn detects_webp_extended_dimensions() {
    assert_eq!(image_dimensions(&webp_vp8x(640, 480)), Some((640, 480)));
}

#[test]
fn detects_gif_dimensions() {
    assert_eq!(image_dimensions(&gif(96, 64)), Some((96, 64)));
}

#[test]
fn detects_bmp_dimensions() {
    assert_eq!(image_dimensions(&bmp(800, 600)), Some((800, 600)));
}

#[test]
fn rejects_unknown_or_truncated_images() {
    assert_eq!(image_dimensions(b"not an image"), None);
    assert_eq!(image_dimensions(&png(10, 10)[..16]), None);
    assert_eq!(image_dimensions(&[0xFF, 0xD8, 0xFF, 0xC0]), None);
}

#[test]
fn sniffs_supported_mime_types() {
    assert_eq!(sniff_mime(&png(1, 1)), "image/png");
    assert_eq!(sniff_mime(&jpeg(1, 1)), "image/jpeg");
    assert_eq!(sniff_mime(&webp_vp8(1, 1)), "image/webp");
    assert_eq!(sniff_mime(&gif(1, 1)), "image/gif");
    assert_eq!(sniff_mime(&bmp(1, 1)), "image/bmp");
    assert_eq!(sniff_mime(b"arbitrary"), "application/octet-stream");
}

#[test]
fn maps_mime_to_cache_extension() {
    assert_eq!(extension_for("image/jpeg"), ".jpg");
    assert_eq!(extension_for("image/png"), ".png");
    assert_eq!(extension_for("image/webp"), ".webp");
    assert_eq!(extension_for("image/gif"), ".gif");
    assert_eq!(extension_for("image/bmp"), ".bmp");
    assert_eq!(extension_for("application/octet-stream"), ".bin");
}

#[test]
fn fnv1a64_is_stable() {
    assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
    assert_eq!(fnv1a64(b"wallpaper"), 0x05d3a45b469151e9);
    assert_ne!(fnv1a64(b"a"), fnv1a64(b"b"));
}

#[test]
fn validates_wallpaper_limits() {
    let image = png(WALLPAPER_MAX_DIMENSION, WALLPAPER_MAX_DIMENSION);
    assert!(validate_wallpaper_bytes(&image, false).is_ok());

    let too_large = {
        let mut bytes = vec![0_u8; (WALLPAPER_MAX_BYTES + 1) as usize];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes
    };
    assert_eq!(
        validate_wallpaper_bytes(&too_large, false),
        Err(ClientError::InvalidRequest)
    );

    let oversized = png(WALLPAPER_MAX_DIMENSION + 1, 10);
    assert_eq!(
        validate_wallpaper_bytes(&oversized, false),
        Err(ClientError::InvalidRequest)
    );

    let pattern = png(64, 64);
    assert!(validate_wallpaper_bytes(&pattern, true).is_ok());
    let mut large_pattern = pattern;
    large_pattern.resize((WALLPAPER_PATTERN_MAX_BYTES + 1) as usize, 0);
    assert_eq!(
        validate_wallpaper_bytes(&large_pattern, true),
        Err(ClientError::InvalidRequest)
    );

    assert_eq!(
        validate_wallpaper_bytes(b"not an image", false),
        Err(ClientError::UnsupportedContent)
    );
    assert_eq!(
        validate_wallpaper_bytes(&[], false),
        Err(ClientError::InvalidRequest)
    );
}

#[test]
fn reads_local_wallpaper_within_limits() {
    let file = scratch_file();
    std::fs::write(&file, png(100, 50)).unwrap();
    assert_eq!(read_local_wallpaper(&file).unwrap(), png(100, 50));

    let oversized = scratch_file();
    let handle = std::fs::File::create(&oversized).unwrap();
    handle.set_len(WALLPAPER_MAX_BYTES + 1).unwrap();
    assert_eq!(
        read_local_wallpaper(&oversized),
        Err(ClientError::InvalidRequest)
    );

    let missing = scratch_file();
    let _ = std::fs::remove_file(&missing);
    assert_eq!(
        read_local_wallpaper(&missing),
        Err(ClientError::InvalidRequest)
    );

    for path in [file, oversized] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn appearance_preferences_default_and_validation() {
    let defaults = AppearancePreferences::default();
    assert_eq!(defaults.source, WallpaperSource::Default);
    assert!(!defaults.dim && !defaults.blur);
    assert_eq!(defaults.intensity, 0);
    assert_eq!(defaults.rotation, 0);
    assert_eq!(defaults.hash, 0);
    assert!(defaults.validate().is_ok());

    let mut invalid = defaults.clone();
    invalid.intensity = 101;
    assert_eq!(invalid.validate(), Err(ClientError::InvalidRequest));

    let mut invalid = defaults.clone();
    invalid.rotation = 361;
    assert_eq!(invalid.validate(), Err(ClientError::InvalidRequest));

    let mut invalid = defaults.clone();
    invalid.source = WallpaperSource::Telegram {
        id: 0,
        access_hash: 1,
    };
    assert_eq!(invalid.validate(), Err(ClientError::InvalidRequest));

    let mut invalid = defaults.clone();
    invalid.source = WallpaperSource::Local { path: "  ".into() };
    assert_eq!(invalid.validate(), Err(ClientError::InvalidRequest));

    let mut valid = defaults;
    valid.source = WallpaperSource::Telegram {
        id: 5,
        access_hash: 7,
    };
    valid.dim = true;
    valid.blur = true;
    valid.intensity = -40;
    valid.rotation = 135;
    valid.hash = 42;
    assert!(valid.validate().is_ok());
}

#[test]
fn appearance_preferences_round_trip_through_json() {
    for source in [
        WallpaperSource::Default,
        WallpaperSource::None,
        WallpaperSource::Local {
            path: "/tmp/wall.png".into(),
        },
        WallpaperSource::Telegram {
            id: 12,
            access_hash: 34,
        },
    ] {
        let preferences = AppearancePreferences {
            source,
            dim: true,
            blur: false,
            intensity: -25,
            rotation: 90,
            hash: 77,
        };
        let json = serde_json::to_string(&preferences).unwrap();
        let decoded: AppearancePreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, preferences);
    }
}

#[test]
fn catalog_cache_round_trips_and_not_modified_preserves_entries_offline() {
    let dir = scratch_file();
    let wallpaper = proto::adapter::media::wallpaper::WallpaperDto {
        id: 7,
        access_hash: 70,
        slug: "cached".into(),
        pattern: true,
        dark: true,
        mime_type: "image/png".into(),
        document_id: Some(700),
        colors: vec![0x102030, 0x405060],
        intensity: Some(-30),
        rotation: 45,
        blur: true,
        motion: false,
    };
    let catalog = proto::WallpaperCatalogDto {
        hash: 12,
        not_modified: false,
        wallpapers: vec![wallpaper],
    };
    write_catalog_cache(&dir, &catalog).unwrap();
    let cached = read_catalog_cache(&dir).unwrap();
    assert_eq!(cached.hash, 12);
    assert!(cached.not_modified);
    assert_eq!(cached.wallpapers.len(), 1);
    assert_eq!(cached.wallpapers[0].slug, "cached");
    assert_eq!(cached.wallpapers[0].colors, vec![0x102030, 0x405060]);
    assert_eq!(cached.wallpapers[0].intensity, Some(-30));

    let merged = merge_catalog(
        proto::WallpaperCatalogDto {
            hash: 13,
            not_modified: true,
            wallpapers: Vec::new(),
        },
        Some(cached),
    );
    assert_eq!(merged.hash, 13);
    assert_eq!(merged.wallpapers.len(), 1);
    assert_eq!(merged.wallpapers[0].id, 7);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn downloaded_wallpaper_is_reused_from_validated_offline_cache() {
    let dir = scratch_file();
    let stored = store_telegram_wallpaper(&dir, 9, false, &png(320, 200)).unwrap();
    assert_eq!(stored.mime_type, "image/png");
    assert_eq!((stored.width, stored.height), (320, 200));
    let cached = cached_wallpaper(&dir, 9, false).unwrap().unwrap();
    assert_eq!(cached.path, stored.path);
    assert_eq!((cached.width, cached.height), (320, 200));

    std::fs::write(dir.join("telegram-10.png"), b"not an image").unwrap();
    assert!(cached_wallpaper(&dir, 10, false).unwrap().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

fn scratch_file() -> PathBuf {
    use rand::{TryRng, rngs::SysRng};
    std::env::temp_dir().join(format!(
        "monogram-appearance-test-{}",
        SysRng.try_next_u64().unwrap()
    ))
}
