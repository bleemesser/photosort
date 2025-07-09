use serde::Deserialize;
use std::path::PathBuf;
use time::OffsetDateTime;

/// Desired Exiftool metadata fields
#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ExifInfo {
    #[serde(rename = "SourceFile")]
    pub source_file: PathBuf,
    #[serde(rename = "MIMEType", default)]
    pub mime_type: String,
    #[serde(default)]
    pub date_time_original: String,
    #[serde(default)]
    pub create_date: String,
    #[serde(default)]
    pub offset_time_original: Option<String>,
    #[serde(default)]
    pub offset_time: Option<String>,
}

/// Information about a sidecar file associated with a photo.
#[derive(Debug, Clone)]
pub struct SourceSidecarInfo {
    pub original_path: PathBuf,
    pub filename: String,
    pub filetype: String,
    pub modified_at: OffsetDateTime,
    pub hash: String,
}

/// Information about a photo in the source library.
#[derive(Debug, Clone)]
pub struct SourcePhotoInfo {
    pub original_path: PathBuf,
    pub filename: String,
    pub filetype: String,
    pub created_at: OffsetDateTime,
    pub hash: String,
    pub sidecars: Vec<SourceSidecarInfo>,
}
