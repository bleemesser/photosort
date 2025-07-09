use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock, time::SystemTime,
};

use base64::{Engine, engine::general_purpose};
use sha2::{Digest, Sha256};
use simplelog::FormatItem;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset, macros::format_description};

use crate::photosort_core::{
    ExifInfo, PhotosortError, SourcePhotoInfo, SourceSidecarInfo, get_current_time,
    get_db_date_string, get_local_tz,
};

/// Fallback image file extensions.
static IMG_EXTS: OnceLock<Vec<&'static str>> = OnceLock::new();

// Supported filecar extensions
static SIDE_EXTS: OnceLock<Vec<&'static str>> = OnceLock::new();

pub const EXIF_DATE_FORMAT: &[FormatItem] = format_description!(
    "[year]:[month]:[day] [hour]:[minute]:[second]"
);

pub const EXIF_OFFSET_FORMAT: &[FormatItem] = format_description!(
    "[offset_hour]:[offset_minute]"
);

/// Calculate the SHA256 hash of a file at the given path and returns it as base64.
pub fn hash_file(path: &Path) -> Result<String, io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let hash = hasher.finalize();
    let hash_base64 = general_purpose::STANDARD.encode(hash);

    Ok(hash_base64)
}

fn get_exif_date_object(date_str: &str, offset_str: Option<&str>) -> Result<OffsetDateTime, PhotosortError> {
    let date_time = PrimitiveDateTime::parse(date_str, EXIF_DATE_FORMAT)
        .map_err(|e| PhotosortError::InvalidDateFormat(e.to_string()))?;
    let offset = match offset_str {
        Some(o) => UtcOffset::parse(o, EXIF_OFFSET_FORMAT)
        .map_err(|e| PhotosortError::InvalidDateFormat(e.to_string()))?,
        None => get_local_tz()
    };
    Ok(date_time.assume_offset(offset))
}

/// Determines whether a file is an image based on mime type or extension.
fn is_image(exif: &ExifInfo, path: &Path) -> bool {
    if exif.mime_type.starts_with("image/") {
        return true;
    }

    let extensions = IMG_EXTS.get_or_init(|| {
        vec![
            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "webp", "heic", "heif", "avif", "raw",
            "cr2", "nef", "orf", "arw", "dng", "sr2",
        ]
    });

    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        return extensions.contains(&ext.to_lowercase().as_str());
    }

    false
}

/// Scan a file and, if it is a photo, construct a `SourcePhotoInfo` object.
pub fn process_photo_file(
    exiftool: &mut exiftool::ExifTool,
    path: PathBuf,
) -> Result<Option<SourcePhotoInfo>, PhotosortError> {
    let exif: ExifInfo = match exiftool.read_metadata(&path, &[]) {
        Ok(info) => info,
        Err(e) => {
            log::warn!("Failed to read metadata for {}: {}", path.display(), e);
            return Ok(None);
        }
    };

    if !is_image(&exif, &path) {
        log::debug!("Skipping non-image file: {}", path.display());
        return Ok(None);
    }

    let file_info = fs::metadata(&path).map_err(PhotosortError::Io)?;
    log::debug!(
        "Processing photo file: {} (size: {}, exif date: {})",
        path.display(),
        file_info.len(),
        &exif.create_date
    );
    log::debug!("Full exif info: {:?}", exif);

    let created_at: OffsetDateTime = get_exif_date_object(&exif.create_date, exif.offset_time.as_deref()).unwrap_or_else(|e1| {
        log::warn!(
            "Failed to parse create_date for {}: {}. Falling back to date_time_original.",
            path.display(),
            e1
        );
        get_exif_date_object(&exif.date_time_original, exif.offset_time_original.as_deref()).unwrap_or_else(|e2| {
            log::warn!(
                "Failed to parse date_time_original for {}: {}. Using file creation time.",
                path.display(),
                e2
            );
            file_info
                .created()
                .ok()
                .map(OffsetDateTime::from)
                .unwrap_or_else(|| {
                    log::warn!(
                        "Failed to get file creation time for {}. Using current time.",
                        path.display(),
                    );
                    get_current_time()
                })
                .to_offset(get_local_tz())
        })
    });

    log::debug!(
        "Photo [{}] determined to be created at {}",
        path.display(),
        get_db_date_string(&created_at).unwrap()
    );

    let hash = hash_file(&path).map_err(|e| PhotosortError::Io(e))?;

    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let filetype = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_uppercase();

    let mut sidecars = Vec::new();

    let sidecar_extensions = SIDE_EXTS.get_or_init(|| vec!["xmp", "photo-edit", "on1", "aae"]);

    let base_name = path.with_extension("");
    let base_name_str = base_name.to_str().unwrap_or_default();
    let base_path_no_ext = if base_name_str.ends_with('.') {
        PathBuf::from(&base_name_str[..base_name_str.len() - 1])
    } else {
        PathBuf::from(base_name_str)
    };

    for ext in sidecar_extensions {
        let sidecar_path = base_path_no_ext.with_extension(ext);

        if sidecar_path.exists() && sidecar_path.is_file() {
            if let Ok(sidecar_file_info) = fs::metadata(&sidecar_path) {
                match hash_file(&sidecar_path) {
                    Ok(sidecar_hash) => {
                        let sidecar_info = SourceSidecarInfo {
                            original_path: sidecar_path.clone(),
                            filename: sidecar_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            filetype: sidecar_path
                                .extension()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_uppercase(),
                            modified_at: OffsetDateTime::from(sidecar_file_info.modified().unwrap_or_else(|e| {
                                log::warn!(
                                    "Failed to get modified time for sidecar {}: {}. Using current time.",
                                    sidecar_path.display(),
                                    e
                                );
                                SystemTime::now()
                            })).to_offset(get_local_tz()),
                            hash: sidecar_hash,
                        };
                        sidecars.push(sidecar_info);
                    }
                    Err(e) => {
                        log::warn!("Could not hash sidecar {}: {}", sidecar_path.display(), e)
                    }
                }
            }
        }
    }

    Ok(Some(SourcePhotoInfo {
        original_path: path,
        filename,
        filetype,
        created_at,
        hash,
        sidecars,
    }))
}
