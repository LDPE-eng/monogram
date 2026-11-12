use std::path::Path;

use serde::{Deserialize, Serialize};

use super::*;
use crate::domain::{APP_PREFERENCES_VERSION, PreferenceDocument, PreferenceNamespace};

pub const WALLPAPER_MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const WALLPAPER_MAX_DIMENSION: u32 = 2560;
pub const WALLPAPER_PATTERN_MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WallpaperSource {
    Default,
    None,
    Local { path: String },
    Telegram { id: i64, access_hash: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearancePreferences {
    pub source: WallpaperSource,
    pub dim: bool,
    pub blur: bool,
    pub intensity: i32,
    pub rotation: i32,
    pub hash: i64,
}

impl Default for AppearancePreferences {
    fn default() -> Self {
        Self {
            source: WallpaperSource::Default,
            dim: false,
            blur: false,
            intensity: 0,
            rotation: 0,
            hash: 0,
        }
    }
}

impl AppearancePreferences {
    pub fn validate(&self) -> Result<(), ClientError> {
        if !(-100..=100).contains(&self.intensity) || !(0..=360).contains(&self.rotation) {
            return Err(ClientError::InvalidRequest);
        }
        match &self.source {
            WallpaperSource::Default | WallpaperSource::None => {}
            WallpaperSource::Local { path } if !path.trim().is_empty() => {}
            WallpaperSource::Telegram { id, .. } if *id != 0 => {}
            _ => return Err(ClientError::InvalidRequest),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppearanceCommand {
    GetWallpapers {
        hash: i64,
    },
    DownloadWallpaper {
        id: i64,
        access_hash: i64,
        pattern: bool,
    },
    ImportLocalWallpaper {
        path: String,
        pattern: bool,
    },
    GetPreferences,
    SetPreferences(AppearancePreferences),
}

#[derive(Clone, Debug)]
pub enum AppearanceResponse {
    Wallpapers(Box<proto::WallpaperCatalogDto>),
    Wallpaper(WallpaperFile),
    Preferences(AppearancePreferences),
}

#[derive(Clone, Debug)]
pub struct WallpaperFile {
    pub path: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub pattern: bool,
    pub colors: Vec<i32>,
    pub intensity: Option<i32>,
    pub rotation: i32,
    pub blur: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedWallpaper {
    id: i64,
    access_hash: i64,
    slug: String,
    pattern: bool,
    dark: bool,
    mime_type: String,
    document_id: Option<i64>,
    colors: Vec<i32>,
    intensity: Option<i32>,
    rotation: i32,
    blur: bool,
    motion: bool,
}

impl From<&proto::adapter::media::wallpaper::WallpaperDto> for CachedWallpaper {
    fn from(wallpaper: &proto::adapter::media::wallpaper::WallpaperDto) -> Self {
        Self {
            id: wallpaper.id,
            access_hash: wallpaper.access_hash,
            slug: wallpaper.slug.clone(),
            pattern: wallpaper.pattern,
            dark: wallpaper.dark,
            mime_type: wallpaper.mime_type.clone(),
            document_id: wallpaper.document_id,
            colors: wallpaper.colors.clone(),
            intensity: wallpaper.intensity,
            rotation: wallpaper.rotation,
            blur: wallpaper.blur,
            motion: wallpaper.motion,
        }
    }
}

impl From<CachedWallpaper> for proto::adapter::media::wallpaper::WallpaperDto {
    fn from(wallpaper: CachedWallpaper) -> Self {
        Self {
            id: wallpaper.id,
            access_hash: wallpaper.access_hash,
            slug: wallpaper.slug,
            pattern: wallpaper.pattern,
            dark: wallpaper.dark,
            mime_type: wallpaper.mime_type,
            document_id: wallpaper.document_id,
            colors: wallpaper.colors,
            intensity: wallpaper.intensity,
            rotation: wallpaper.rotation,
            blur: wallpaper.blur,
            motion: wallpaper.motion,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedCatalog {
    hash: i64,
    wallpapers: Vec<CachedWallpaper>,
}

fn read_catalog_cache(dir: &Path) -> Option<proto::WallpaperCatalogDto> {
    let bytes = std::fs::read(dir.join("catalog.json")).ok()?;
    let catalog: CachedCatalog = serde_json::from_slice(&bytes).ok()?;
    Some(proto::WallpaperCatalogDto {
        hash: catalog.hash,
        not_modified: true,
        wallpapers: catalog.wallpapers.into_iter().map(Into::into).collect(),
    })
}

fn write_catalog_cache(
    dir: &Path,
    catalog: &proto::WallpaperCatalogDto,
) -> Result<(), ClientError> {
    std::fs::create_dir_all(dir).map_err(|_| ClientError::Storage)?;
    let cached = CachedCatalog {
        hash: catalog.hash,
        wallpapers: catalog.wallpapers.iter().map(Into::into).collect(),
    };
    let bytes = serde_json::to_vec(&cached).map_err(|_| ClientError::Storage)?;
    let part = dir.join("catalog.json.part");
    std::fs::write(&part, bytes).map_err(|_| ClientError::Storage)?;
    std::fs::rename(part, dir.join("catalog.json")).map_err(|_| ClientError::Storage)
}

fn merge_catalog(
    response: proto::WallpaperCatalogDto,
    cached: Option<proto::WallpaperCatalogDto>,
) -> proto::WallpaperCatalogDto {
    if response.not_modified
        && let Some(mut cached) = cached
    {
        cached.hash = response.hash;
        return cached;
    }
    response
}

fn cached_wallpaper(
    dir: &Path,
    id: i64,
    pattern: bool,
) -> Result<Option<WallpaperFile>, ClientError> {
    for extension in [".jpg", ".png", ".webp", ".gif", ".bmp"] {
        let path = dir.join(format!("telegram-{id}{extension}"));
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Ok((mime, width, height)) = validate_wallpaper_bytes(&bytes, pattern) {
            return Ok(Some(WallpaperFile {
                path: path.to_string_lossy().into_owned(),
                mime_type: mime.into(),
                width,
                height,
                pattern,
                colors: Vec::new(),
                intensity: None,
                rotation: 0,
                blur: false,
            }));
        }
    }
    Ok(None)
}

fn store_telegram_wallpaper(
    dir: &Path,
    id: i64,
    pattern: bool,
    bytes: &[u8],
) -> Result<WallpaperFile, ClientError> {
    let (mime, width, height) = validate_wallpaper_bytes(bytes, pattern)?;
    std::fs::create_dir_all(dir).map_err(|_| ClientError::Storage)?;
    let destination = dir.join(format!("telegram-{id}{}", extension_for(mime)));
    std::fs::write(&destination, bytes).map_err(|_| ClientError::Storage)?;
    Ok(WallpaperFile {
        path: destination.to_string_lossy().into_owned(),
        mime_type: mime.into(),
        width,
        height,
        pattern,
        colors: Vec::new(),
        intensity: None,
        rotation: 0,
        blur: false,
    })
}

impl Runtime {
    pub(super) fn execute_appearance(
        &self,
        command: AppearanceCommand,
    ) -> Result<AppearanceResponse, ClientError> {
        match command {
            AppearanceCommand::GetWallpapers { hash } => self.wallpaper_catalog(hash),
            AppearanceCommand::DownloadWallpaper {
                id,
                access_hash,
                pattern,
            } => self.wallpaper_download(id, access_hash, pattern),
            AppearanceCommand::ImportLocalWallpaper { path, pattern } => {
                self.wallpaper_import(path, pattern)
            }
            AppearanceCommand::GetPreferences => self.appearance_preferences(),
            AppearanceCommand::SetPreferences(preferences) => {
                self.set_appearance_preferences(preferences)
            }
        }
    }

    fn wallpaper_catalog(&self, hash: i64) -> Result<AppearanceResponse, ClientError> {
        let dir = self.wallpaper_dir();
        let cached = read_catalog_cache(&dir);
        match proto::extras::get_wallpapers(self.authorized()?, hash) {
            Ok(response) => {
                let not_modified = response.not_modified;
                let catalog = merge_catalog(response, cached);
                if !not_modified {
                    write_catalog_cache(&dir, &catalog)?;
                }
                Ok(AppearanceResponse::Wallpapers(Box::new(catalog)))
            }
            Err(error) => {
                let error = ClientError::from(error);
                if error == ClientError::Network
                    && let Some(cached) = cached
                {
                    return Ok(AppearanceResponse::Wallpapers(Box::new(cached)));
                }
                Err(error)
            }
        }
    }

    fn wallpaper_download(
        &self,
        id: i64,
        access_hash: i64,
        pattern: bool,
    ) -> Result<AppearanceResponse, ClientError> {
        if id == 0 {
            return Err(ClientError::InvalidRequest);
        }
        let dir = self.wallpaper_dir();
        if let Some(file) = cached_wallpaper(&dir, id, pattern)? {
            return Ok(AppearanceResponse::Wallpaper(file));
        }
        let handle = self.authorized()?;
        std::fs::create_dir_all(&dir).map_err(|_| ClientError::Storage)?;
        let part = dir.join(format!("telegram-{id}.part"));
        let _ = std::fs::remove_file(&part);
        if let Err(error) = proto::media_download::download_wallpaper(
            handle,
            id,
            access_hash,
            part.to_string_lossy().into_owned(),
        ) {
            let _ = std::fs::remove_file(&part);
            return Err(ClientError::from(error));
        }
        let bytes = std::fs::read(&part).map_err(|_| ClientError::Storage)?;
        let _ = std::fs::remove_file(&part);
        let file = store_telegram_wallpaper(&dir, id, pattern, &bytes)?;
        Ok(AppearanceResponse::Wallpaper(file))
    }

    fn wallpaper_import(
        &self,
        path: String,
        pattern: bool,
    ) -> Result<AppearanceResponse, ClientError> {
        let bytes = read_local_wallpaper(Path::new(&path))?;
        let (mime, width, height) = validate_wallpaper_bytes(&bytes, pattern)?;
        let dir = self.wallpaper_dir();
        std::fs::create_dir_all(&dir).map_err(|_| ClientError::Storage)?;
        let destination = dir.join(format!(
            "local-{:016x}{}",
            fnv1a64(&bytes),
            extension_for(mime)
        ));
        std::fs::write(&destination, &bytes).map_err(|_| ClientError::Storage)?;
        Ok(AppearanceResponse::Wallpaper(WallpaperFile {
            path: destination.to_string_lossy().into_owned(),
            mime_type: mime.to_owned(),
            width,
            height,
            pattern,
            colors: Vec::new(),
            intensity: None,
            rotation: 0,
            blur: false,
        }))
    }

    fn appearance_preferences(&self) -> Result<AppearanceResponse, ClientError> {
        let document = self
            .storage
            .preference(PreferenceNamespace::Appearance)
            .map_err(|_| ClientError::Storage)?;
        let preferences = match document {
            Some(document) => {
                serde_json::from_str(&document.payload_json).map_err(|_| ClientError::Storage)?
            }
            None => AppearancePreferences::default(),
        };
        Ok(AppearanceResponse::Preferences(preferences))
    }

    fn set_appearance_preferences(
        &self,
        preferences: AppearancePreferences,
    ) -> Result<AppearanceResponse, ClientError> {
        preferences.validate()?;
        let document = PreferenceDocument {
            version: APP_PREFERENCES_VERSION,
            payload_json: serde_json::to_string(&preferences)
                .map_err(|_| ClientError::InvalidRequest)?,
        };
        self.storage
            .set_preference(PreferenceNamespace::Appearance, &document)
            .map_err(|_| ClientError::Storage)?;
        Ok(AppearanceResponse::Preferences(preferences))
    }

    fn wallpaper_dir(&self) -> PathBuf {
        self.media_staging.join("wallpapers")
    }
}

fn read_local_wallpaper(path: &Path) -> Result<Vec<u8>, ClientError> {
    let metadata = std::fs::metadata(path).map_err(|_| ClientError::InvalidRequest)?;
    if !metadata.is_file() || metadata.len() > WALLPAPER_MAX_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    let bytes = std::fs::read(path).map_err(|_| ClientError::Storage)?;
    if bytes.len() as u64 > WALLPAPER_MAX_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    Ok(bytes)
}

fn validate_wallpaper_bytes(
    bytes: &[u8],
    pattern: bool,
) -> Result<(&'static str, u32, u32), ClientError> {
    if bytes.is_empty() || bytes.len() as u64 > WALLPAPER_MAX_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    let (width, height) = image_dimensions(bytes).ok_or(ClientError::UnsupportedContent)?;
    if width > WALLPAPER_MAX_DIMENSION || height > WALLPAPER_MAX_DIMENSION {
        return Err(ClientError::InvalidRequest);
    }
    if pattern && bytes.len() as u64 > WALLPAPER_PATTERN_MAX_BYTES {
        return Err(ClientError::InvalidRequest);
    }
    Ok((sniff_mime(bytes), width, height))
}

fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return "image/png";
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return "image/jpeg";
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp";
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return "image/gif";
    }
    if bytes.len() >= 2 && &bytes[..2] == b"BM" {
        return "image/bmp";
    }
    "application/octet-stream"
}

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => ".jpg",
        "image/png" => ".png",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        "image/bmp" => ".bmp",
        _ => ".bin",
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return nonzero(width, height);
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return jpeg_dimensions(bytes);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return webp_dimensions(bytes);
    }
    if bytes.len() >= 10 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return nonzero(width, height);
    }
    if bytes.len() >= 26 && &bytes[..2] == b"BM" {
        let width = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        if width > 0 && height != 0 {
            return Some((width as u32, height.unsigned_abs()));
        }
        return None;
    }
    None
}

fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2;
    while offset + 1 < data.len() {
        if data[offset] != 0xFF {
            offset += 1;
            continue;
        }
        let mut marker = data[offset + 1];
        while marker == 0xFF {
            offset += 1;
            if offset + 1 >= data.len() {
                return None;
            }
            marker = data[offset + 1];
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            offset += 2;
            continue;
        }
        if offset + 3 >= data.len() {
            return None;
        }
        let length = ((data[offset + 2] as usize) << 8) | data[offset + 3] as usize;
        if length < 2 {
            return None;
        }
        if is_sof(marker) {
            if offset + 8 >= data.len() {
                return None;
            }
            let height = u16::from_be_bytes([data[offset + 5], data[offset + 6]]) as u32;
            let width = u16::from_be_bytes([data[offset + 7], data[offset + 8]]) as u32;
            return nonzero(width, height);
        }
        offset += 2 + length;
    }
    None
}

fn is_sof(marker: u8) -> bool {
    (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

fn webp_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    match &data[12..16] {
        b"VP8 " => {
            if data.len() < 30 || data[20..23] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u16::from_le_bytes([data[26], data[27]]) & 0x3FFF;
            let height = u16::from_le_bytes([data[28], data[29]]) & 0x3FFF;
            nonzero(width as u32, height as u32)
        }
        b"VP8L" => {
            if data.len() < 25 || data[20] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
            let width = 1 + (bits & 0x3FFF);
            let height = 1 + ((bits >> 14) & 0x3FFF);
            nonzero(width, height)
        }
        b"VP8X" => {
            if data.len() < 30 {
                return None;
            }
            let width = 1 + u32::from_le_bytes([data[24], data[25], data[26], 0]);
            let height = 1 + u32::from_le_bytes([data[27], data[28], data[29], 0]);
            nonzero(width, height)
        }
        _ => None,
    }
}

fn nonzero(width: u32, height: u32) -> Option<(u32, u32)> {
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
#[path = "../../tests/unit/appearance.rs"]
mod tests;
