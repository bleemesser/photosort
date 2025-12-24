use std::path::Path;
use std::process::Command;
use time::OffsetDateTime;

/// Represents a media file (image or video) in the library.
#[derive(Debug, Clone)]
pub struct Media {
    pub hash: String,
    pub filename: String,
    pub relpath: String,
    pub media_type: MediaType,
    pub filetype: String,
    pub file_size: u64,
    pub created_at: OffsetDateTime,
    pub imported_at: OffsetDateTime,
    pub exif: Option<ExifMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
}

impl MediaType {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Video => "video",
        }
    }

    pub fn folder_name(&self) -> &'static str {
        match self {
            MediaType::Image => "images",
            MediaType::Video => "videos",
        }
    }
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// EXIF metadata extracted from media files.
#[derive(Debug, Clone, Default)]
pub struct ExifMetadata {
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub focal_length: Option<String>,
    pub aperture: Option<String>,
    pub shutter_speed: Option<String>,
    pub iso: Option<i32>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

/// Image file extensions (lowercase).
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "heic", "heif", "avif",
    // RAW formats
    "raw", "cr2", "cr3", "nef", "orf", "arw", "dng", "sr2", "raf", "rw2", "pef",
];

/// Video file extensions (lowercase) - used as fallback when ffprobe unavailable.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "avi", "mkv", "webm", "mts", "m2ts", "3gp", "wmv", "flv",
];

/// Detect media type from a file path.
/// Uses MIME type detection first, then ffprobe for videos, then falls back to extension.
pub fn detect_media_type(path: &Path) -> Option<MediaType> {
    // First, try extension-based detection (fast path)
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();

        if IMAGE_EXTENSIONS.contains(&ext_lower.as_str()) {
            return Some(MediaType::Image);
        }

        if VIDEO_EXTENSIONS.contains(&ext_lower.as_str()) {
            return Some(MediaType::Video);
        }
    }

    // For unknown extensions, try ffprobe to detect video
    if is_video_ffprobe(path) {
        return Some(MediaType::Video);
    }

    None
}

/// Use ffprobe to detect if a file is a video.
fn is_video_ffprobe(path: &Path) -> bool {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_type",
            "-of", "csv=p=0",
        ])
        .arg(path)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.trim() == "video"
        }
        Err(_) => false, // ffprobe not available or failed
    }
}

/// Check if ffprobe is available on the system.
pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_image_extensions() {
        assert_eq!(detect_media_type(Path::new("photo.jpg")), Some(MediaType::Image));
        assert_eq!(detect_media_type(Path::new("photo.HEIC")), Some(MediaType::Image));
        assert_eq!(detect_media_type(Path::new("photo.cr2")), Some(MediaType::Image));
        assert_eq!(detect_media_type(Path::new("photo.DNG")), Some(MediaType::Image));
    }

    #[test]
    fn test_detect_video_extensions() {
        assert_eq!(detect_media_type(Path::new("video.mp4")), Some(MediaType::Video));
        assert_eq!(detect_media_type(Path::new("video.MOV")), Some(MediaType::Video));
        assert_eq!(detect_media_type(Path::new("video.mkv")), Some(MediaType::Video));
    }

    #[test]
    fn test_detect_unknown_extension() {
        // Unknown extension without ffprobe detection
        assert_eq!(detect_media_type(Path::new("file.xyz")), None);
    }

    #[test]
    fn test_media_type_display() {
        assert_eq!(MediaType::Image.as_str(), "image");
        assert_eq!(MediaType::Video.as_str(), "video");
        assert_eq!(MediaType::Image.folder_name(), "images");
        assert_eq!(MediaType::Video.folder_name(), "videos");
    }
}
