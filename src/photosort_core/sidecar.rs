use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// Sidecar file extensions (lowercase).
/// These files are associated with a parent media file and should move/rename together.
pub const SIDECAR_EXTENSIONS: &[&str] = &[
    "xmp",         // Adobe XMP sidecar
    "photo-edit",  // Photomator edit file
    "on1",         // ON1 Photo RAW
    "aae",         // Apple photo adjustments
    "pp3",         // RawTherapee
    "dop",         // DxO PhotoLab
];

/// Information about a sidecar file.
#[derive(Debug, Clone)]
pub struct Sidecar {
    pub filename: String,
    pub filetype: String,
    pub file_size: u64,
    pub hash: String,
    pub modified_at: OffsetDateTime,
    /// Full path to the sidecar file (used during import).
    pub source_path: Option<PathBuf>,
}

/// Find all sidecar files associated with a media file.
/// Sidecars have the same base name but different extensions.
///
/// Example: For "photo.jpg", finds "photo.xmp", "photo.photo-edit", etc.
pub fn find_sidecars(media_path: &Path) -> Vec<PathBuf> {
    let mut sidecars = Vec::new();

    // Get base name without extension
    let Some(parent) = media_path.parent() else {
        return sidecars;
    };

    let Some(stem) = media_path.file_stem().and_then(|s| s.to_str()) else {
        return sidecars;
    };

    // Check for each sidecar extension
    for ext in SIDECAR_EXTENSIONS {
        let sidecar_path = parent.join(format!("{}.{}", stem, ext));
        if sidecar_path.exists() && sidecar_path.is_file() {
            sidecars.push(sidecar_path);
        }
    }

    sidecars
}

/// Check if a file is a sidecar based on its extension.
pub fn is_sidecar(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SIDECAR_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Get the expected sidecar filename for a media file and sidecar extension.
///
/// Example: get_sidecar_filename("photo.jpg", "xmp") -> "photo.xmp"
pub fn get_sidecar_filename(media_filename: &str, sidecar_ext: &str) -> Option<String> {
    let path = Path::new(media_filename);
    let stem = path.file_stem()?.to_str()?;
    Some(format!("{}.{}", stem, sidecar_ext))
}

/// Rename a sidecar file to match a new media filename.
///
/// Example: If media "IMG_001.jpg" is renamed to "vacation.jpg",
/// its sidecar "IMG_001.xmp" should become "vacation.xmp"
pub fn rename_sidecar_for_media(
    old_sidecar_filename: &str,
    new_media_filename: &str,
) -> Option<String> {
    let sidecar_path = Path::new(old_sidecar_filename);
    let sidecar_ext = sidecar_path.extension()?.to_str()?;

    let new_media_path = Path::new(new_media_filename);
    let new_stem = new_media_path.file_stem()?.to_str()?;

    Some(format!("{}.{}", new_stem, sidecar_ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sidecar() {
        assert!(is_sidecar(Path::new("photo.xmp")));
        assert!(is_sidecar(Path::new("photo.photo-edit")));
        assert!(is_sidecar(Path::new("PHOTO.XMP"))); // case insensitive
        assert!(!is_sidecar(Path::new("photo.jpg")));
        assert!(!is_sidecar(Path::new("photo.mp4")));
    }

    #[test]
    fn test_get_sidecar_filename() {
        assert_eq!(
            get_sidecar_filename("photo.jpg", "xmp"),
            Some("photo.xmp".to_string())
        );
        assert_eq!(
            get_sidecar_filename("IMG_0001.HEIC", "photo-edit"),
            Some("IMG_0001.photo-edit".to_string())
        );
    }

    #[test]
    fn test_rename_sidecar_for_media() {
        assert_eq!(
            rename_sidecar_for_media("IMG_001.xmp", "vacation.jpg"),
            Some("vacation.xmp".to_string())
        );
        assert_eq!(
            rename_sidecar_for_media("old_name.photo-edit", "new_name.heic"),
            Some("new_name.photo-edit".to_string())
        );
    }
}
