use crate::photosort_core::error::{PhotosortError, Result};
use crate::photosort_core::media::ExifMetadata;
use exiftool::ExifTool;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// Date format used in EXIF data.
const EXIF_DATE_FORMAT: &[time::format_description::FormatItem] =
    time::macros::format_description!("[year]:[month]:[day] [hour]:[minute]:[second]");

const EXIF_OFFSET_FORMAT: &[time::format_description::FormatItem] =
    time::macros::format_description!("[offset_hour]:[offset_minute]");

/// Raw EXIF data from exiftool using flexible Value types for fields that vary.
#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct RawExifInfo {
    #[serde(rename = "MIMEType", default)]
    mime_type: String,
    #[serde(default)]
    date_time_original: String,
    #[serde(default)]
    create_date: String,
    #[serde(default)]
    offset_time_original: Option<String>,
    #[serde(default)]
    offset_time: Option<String>,
    #[serde(default)]
    make: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "LensModel", default)]
    lens_model: Option<String>,
    #[serde(default)]
    focal_length: Option<Value>,  // Can be string "50 mm" or number
    #[serde(rename = "FNumber", default)]
    f_number: Option<Value>,      // Can be string or number
    #[serde(default)]
    exposure_time: Option<Value>, // Can be string "1/250" or number 0.004
    #[serde(rename = "ISO", default)]
    iso: Option<Value>,           // Can be string or number
    #[serde(rename = "GPSLatitude", default)]
    gps_latitude: Option<Value>,  // Can be string "45 deg 30' 16.91\" N" or number
    #[serde(rename = "GPSLongitude", default)]
    gps_longitude: Option<Value>, // Can be string "122 deg 40' 30.12\" W" or number
}

/// Helper to extract f64 from Value (handles both string and number)
fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            // Try to parse GPS coordinates like "45 deg 30' 16.91\" N"
            if let Some(degrees) = parse_gps_string(s) {
                return Some(degrees);
            }
            // Try simple float parse
            s.trim().parse().ok()
        }
        _ => None,
    }
}

/// Parse GPS string like "45 deg 30' 16.91\" N" to decimal degrees
fn parse_gps_string(s: &str) -> Option<f64> {
    // Handle empty strings
    if s.trim().is_empty() {
        return None;
    }

    // Pattern: "45 deg 30' 16.91\" N" or similar
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let degrees: f64 = parts[0].parse().ok()?;
    let minutes_str = parts[2].trim_end_matches('\'');
    let minutes: f64 = minutes_str.parse().ok()?;
    let seconds_str = parts[3].trim_end_matches('"').trim_end_matches('\'');
    let seconds: f64 = seconds_str.parse().ok()?;
    let direction = parts.get(4).map(|s| s.chars().next()).flatten();

    let mut result = degrees + (minutes / 60.0) + (seconds / 3600.0);

    // South and West are negative
    if direction == Some('S') || direction == Some('W') {
        result = -result;
    }

    Some(result)
}

/// Helper to extract i32 from Value
fn value_to_i32(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n.as_i64().map(|i| i as i32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Helper to extract String from Value (for display-friendly values)
fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                // Format exposure time nicely if it's a decimal
                if f < 1.0 && f > 0.0 {
                    let denom = (1.0 / f).round() as i32;
                    Some(format!("1/{}", denom))
                } else {
                    Some(format!("{}", f))
                }
            } else {
                Some(n.to_string())
            }
        }
        _ => None,
    }
}

/// Result of extracting metadata from a file.
pub struct ExtractedMetadata {
    pub created_at: OffsetDateTime,
    pub exif: ExifMetadata,
}

/// Extract metadata from a media file using exiftool.
pub fn extract_metadata(exiftool: &mut ExifTool, path: &Path) -> Result<ExtractedMetadata> {
    let raw: RawExifInfo = exiftool.read_metadata(path, &[]).map_err(|e| {
        PhotosortError::MetadataExtraction {
            path: path.to_path_buf(),
            reason: e.to_string(),
        }
    })?;

    // Parse creation date with fallback chain
    let created_at = parse_exif_date(&raw.create_date, raw.offset_time.as_deref())
        .or_else(|_| {
            parse_exif_date(&raw.date_time_original, raw.offset_time_original.as_deref())
        })
        .or_else(|_| {
            // Fallback to file creation time
            std::fs::metadata(path)
                .and_then(|m| m.created())
                .map(OffsetDateTime::from)
                .map_err(|e| PhotosortError::Io(e))
        })
        .unwrap_or_else(|_| {
            log::warn!(
                "Could not determine creation date for {}, using current time",
                path.display()
            );
            OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
        });

    // Extract aperture (f-number)
    let aperture = raw.f_number.as_ref().and_then(|v| {
        match v {
            Value::Number(n) => n.as_f64().map(|f| format!("f/{:.1}", f)),
            Value::String(s) => Some(s.clone()),
            _ => None,
        }
    });

    let exif = ExifMetadata {
        camera_make: raw.make,
        camera_model: raw.model,
        lens: raw.lens_model,
        focal_length: raw.focal_length.as_ref().and_then(value_to_string),
        aperture,
        shutter_speed: raw.exposure_time.as_ref().and_then(value_to_string),
        iso: raw.iso.as_ref().and_then(value_to_i32),
        gps_lat: raw.gps_latitude.as_ref().and_then(value_to_f64),
        gps_lon: raw.gps_longitude.as_ref().and_then(value_to_f64),
    };

    Ok(ExtractedMetadata { created_at, exif })
}

/// Parse an EXIF date string with optional timezone offset.
fn parse_exif_date(date_str: &str, offset_str: Option<&str>) -> Result<OffsetDateTime> {
    if date_str.is_empty() {
        return Err(PhotosortError::InvalidDateFormat("empty date".to_string()));
    }

    let date_time = PrimitiveDateTime::parse(date_str, EXIF_DATE_FORMAT)
        .map_err(|e| PhotosortError::InvalidDateFormat(e.to_string()))?;

    let offset = match offset_str {
        Some(o) if !o.is_empty() => UtcOffset::parse(o, EXIF_OFFSET_FORMAT)
            .unwrap_or_else(|_| get_local_offset()),
        _ => get_local_offset(),
    };

    Ok(date_time.assume_offset(offset))
}

/// Get the local timezone offset, falling back to UTC if unavailable.
fn get_local_offset() -> UtcOffset {
    OffsetDateTime::now_local()
        .map(|dt| dt.offset())
        .unwrap_or(UtcOffset::UTC)
}

/// Check if exiftool is available on the system.
pub fn exiftool_available() -> bool {
    std::process::Command::new("exiftool")
        .arg("-ver")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_exif_date() {
        let date = parse_exif_date("2024:05:21 12:30:00", Some("+09:00"));
        assert!(date.is_ok());
        let dt = date.unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month() as u8, 5);
        assert_eq!(dt.day(), 21);
    }

    #[test]
    fn test_parse_exif_date_without_offset() {
        let date = parse_exif_date("2024:12:25 08:00:00", None);
        assert!(date.is_ok());
    }

    #[test]
    fn test_parse_empty_date() {
        let date = parse_exif_date("", None);
        assert!(date.is_err());
    }

    #[test]
    fn test_parse_gps_string() {
        // Test latitude parsing
        let lat = parse_gps_string("45 deg 30' 16.91\" N");
        assert!(lat.is_some());
        let lat_val = lat.unwrap();
        assert!((lat_val - 45.50469722).abs() < 0.0001);

        // Test longitude parsing with West (negative)
        let lon = parse_gps_string("122 deg 40' 30.12\" W");
        assert!(lon.is_some());
        let lon_val = lon.unwrap();
        assert!(lon_val < 0.0); // West should be negative
        assert!((lon_val - (-122.675033)).abs() < 0.0001);

        // Empty string should return None
        assert!(parse_gps_string("").is_none());
        assert!(parse_gps_string("   ").is_none());
    }

    #[test]
    fn test_value_to_f64() {
        use serde_json::json;

        // Number value
        let num = json!(45.5);
        assert_eq!(value_to_f64(&num), Some(45.5));

        // GPS string value
        let gps_str = json!("45 deg 30' 16.91\" N");
        let result = value_to_f64(&gps_str);
        assert!(result.is_some());
        assert!((result.unwrap() - 45.50469722).abs() < 0.0001);

        // Simple number string
        let num_str = json!("123.45");
        assert_eq!(value_to_f64(&num_str), Some(123.45));
    }

    #[test]
    fn test_value_to_string_exposure() {
        use serde_json::json;

        // Fraction string stays as-is
        let frac = json!("1/250");
        assert_eq!(value_to_string(&frac), Some("1/250".to_string()));

        // Decimal number gets converted to fraction
        let decimal = json!(0.004);
        let result = value_to_string(&decimal).unwrap();
        assert_eq!(result, "1/250");

        // Longer exposure
        let long_exp = json!(0.5);
        let result = value_to_string(&long_exp).unwrap();
        assert_eq!(result, "1/2");
    }
}
