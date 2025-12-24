use crate::photosort_core::cli::{MediaTypeFilter, OutputFormat};
use crate::photosort_core::error::Result;
use crate::photosort_core::import::Library;
use serde::Serialize;
use std::path::PathBuf;

/// A search query with filters.
#[derive(Debug, Default)]
pub struct SearchQuery {
    pub media_type: Option<MediaTypeFilter>,
    pub date_start: Option<String>,
    pub date_end: Option<String>,
    pub extensions: Vec<String>,
    pub has_sidecar: Option<bool>,
    pub min_size: Option<i64>,
    pub max_size: Option<i64>,
    pub camera: Option<String>,
    pub lens: Option<String>,
}

/// A search result item.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: i64,
    pub filename: String,
    pub relpath: String,
    pub media_type: String,
    pub filetype: String,
    pub file_size: i64,
    pub created_at: String,
    pub camera_model: Option<String>,
    pub has_sidecar: bool,
    #[serde(skip)]
    pub full_path: PathBuf,
}

impl SearchQuery {
    /// Parse a date filter string like "2024-01-01" or "2024-01-01..2024-12-31"
    pub fn parse_date_filter(date_str: &str) -> (Option<String>, Option<String>) {
        if date_str.contains("..") {
            let parts: Vec<&str> = date_str.split("..").collect();
            if parts.len() == 2 {
                return (
                    Some(parts[0].to_string()),
                    Some(parts[1].to_string()),
                );
            }
        }
        // Single date - search for that exact day
        (Some(date_str.to_string()), Some(date_str.to_string()))
    }

    /// Parse a size filter string like ">10MB", "<1MB", or "5MB..50MB"
    pub fn parse_size_filter(size_str: &str) -> (Option<i64>, Option<i64>) {
        let size_str = size_str.trim();

        // Handle range: "5MB..50MB"
        if size_str.contains("..") {
            let parts: Vec<&str> = size_str.split("..").collect();
            if parts.len() == 2 {
                return (
                    parse_size_value(parts[0]),
                    parse_size_value(parts[1]),
                );
            }
        }

        // Handle comparison operators
        if let Some(rest) = size_str.strip_prefix('>') {
            return (parse_size_value(rest), None);
        }
        if let Some(rest) = size_str.strip_prefix('<') {
            return (None, parse_size_value(rest));
        }

        // Exact size
        let size = parse_size_value(size_str);
        (size, size)
    }
}

/// Parse a size value like "10MB" or "1GB" into bytes.
fn parse_size_value(s: &str) -> Option<i64> {
    let s = s.trim().to_uppercase();

    let (num_part, multiplier) = if let Some(rest) = s.strip_suffix("GB") {
        (rest, 1_073_741_824i64)
    } else if let Some(rest) = s.strip_suffix("MB") {
        (rest, 1_048_576i64)
    } else if let Some(rest) = s.strip_suffix("KB") {
        (rest, 1_024i64)
    } else if let Some(rest) = s.strip_suffix('B') {
        (rest, 1i64)
    } else {
        // Assume bytes if no suffix
        (s.as_str(), 1i64)
    };

    num_part.trim().parse::<i64>().ok().map(|n| n * multiplier)
}

/// Execute a search query on the library.
pub fn search(lib: &Library, query: &SearchQuery) -> Result<Vec<SearchResult>> {
    let db = lib.database();
    let root = lib.root();

    // Build SQL query
    let mut sql = String::from(
        "SELECT m.id, m.filename, m.relpath, m.media_type, m.filetype, m.file_size,
                m.created_at, m.camera_model,
                (SELECT COUNT(*) FROM sidecars s WHERE s.media_id = m.id) as sidecar_count
         FROM media m
         WHERE 1=1"
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    // Media type filter
    if let Some(ref media_type) = query.media_type {
        match media_type {
            MediaTypeFilter::Image => {
                sql.push_str(" AND m.media_type = ?");
                params.push(Box::new("image".to_string()));
            }
            MediaTypeFilter::Video => {
                sql.push_str(" AND m.media_type = ?");
                params.push(Box::new("video".to_string()));
            }
            MediaTypeFilter::All => {}
        }
    }

    // Date filter - search by date prefix in created_at
    if let Some(ref start) = query.date_start {
        sql.push_str(" AND m.created_at >= ?");
        params.push(Box::new(format!("{}:00:00:00", start)));
    }
    if let Some(ref end) = query.date_end {
        sql.push_str(" AND m.created_at <= ?");
        params.push(Box::new(format!("{}:23:59:59", end)));
    }

    // Extension filter
    if !query.extensions.is_empty() {
        let placeholders: Vec<&str> = query.extensions.iter().map(|_| "?").collect();
        sql.push_str(&format!(" AND UPPER(m.filetype) IN ({})", placeholders.join(",")));
        for ext in &query.extensions {
            params.push(Box::new(ext.to_uppercase()));
        }
    }

    // Size filter
    if let Some(min) = query.min_size {
        sql.push_str(" AND m.file_size >= ?");
        params.push(Box::new(min));
    }
    if let Some(max) = query.max_size {
        sql.push_str(" AND m.file_size <= ?");
        params.push(Box::new(max));
    }

    // Camera filter (substring match)
    if let Some(ref camera) = query.camera {
        sql.push_str(" AND m.camera_model LIKE ?");
        params.push(Box::new(format!("%{}%", camera)));
    }

    // Lens filter (substring match)
    if let Some(ref lens) = query.lens {
        sql.push_str(" AND m.lens LIKE ?");
        params.push(Box::new(format!("%{}%", lens)));
    }

    sql.push_str(" ORDER BY m.created_at DESC");

    // Execute query
    let conn = db.connection_ref();
    let mut stmt = conn.prepare(&sql)?;

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, i64>(8)?,
        ))
    })?;

    let mut results = Vec::new();

    for row in rows {
        let (id, filename, relpath, media_type, filetype, file_size, created_at, camera_model, sidecar_count) = row?;

        let has_sidecar = sidecar_count > 0;

        // Apply sidecar filter
        if let Some(want_sidecar) = query.has_sidecar {
            if want_sidecar != has_sidecar {
                continue;
            }
        }

        let full_path = root.join(&relpath).join(&filename);

        results.push(SearchResult {
            id,
            filename,
            relpath,
            media_type,
            filetype,
            file_size,
            created_at,
            camera_model,
            has_sidecar,
            full_path,
        });
    }

    Ok(results)
}

/// Format search results for output.
pub fn format_results(results: &[SearchResult], format: &OutputFormat) -> String {
    match format {
        OutputFormat::Paths => {
            results.iter()
                .map(|r| r.full_path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        }
        OutputFormat::Json => {
            serde_json::to_string_pretty(results).unwrap_or_else(|_| "[]".to_string())
        }
        OutputFormat::Table => {
            let mut output = String::new();
            output.push_str(&format!("{:<40} {:>10} {:>8} {:>10}\n", "Filename", "Size", "Type", "Date"));
            output.push_str(&format!("{}\n", "─".repeat(72)));
            for r in results {
                let size_str = format_size(r.file_size);
                let date_str = &r.created_at[..10]; // Just YYYY:MM:DD
                output.push_str(&format!(
                    "{:<40} {:>10} {:>8} {:>10}\n",
                    truncate_str(&r.filename, 40),
                    size_str,
                    r.filetype,
                    date_str
                ));
            }
            output.push_str(&format!("\nTotal: {} files", results.len()));
            output
        }
    }
}

fn format_size(bytes: i64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1_024 {
        format!("{:.1} KB", bytes as f64 / 1_024.0)
    } else {
        format!("{} B", bytes)
    }
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_date_filter_single() {
        let (start, end) = SearchQuery::parse_date_filter("2024-01-15");
        assert_eq!(start, Some("2024-01-15".to_string()));
        assert_eq!(end, Some("2024-01-15".to_string()));
    }

    #[test]
    fn test_parse_date_filter_range() {
        let (start, end) = SearchQuery::parse_date_filter("2024-01-01..2024-12-31");
        assert_eq!(start, Some("2024-01-01".to_string()));
        assert_eq!(end, Some("2024-12-31".to_string()));
    }

    #[test]
    fn test_parse_size_filter_gt() {
        let (min, max) = SearchQuery::parse_size_filter(">10MB");
        assert_eq!(min, Some(10_485_760));
        assert_eq!(max, None);
    }

    #[test]
    fn test_parse_size_filter_lt() {
        let (min, max) = SearchQuery::parse_size_filter("<1GB");
        assert_eq!(min, None);
        assert_eq!(max, Some(1_073_741_824));
    }

    #[test]
    fn test_parse_size_filter_range() {
        let (min, max) = SearchQuery::parse_size_filter("5MB..50MB");
        assert_eq!(min, Some(5_242_880));
        assert_eq!(max, Some(52_428_800));
    }

    #[test]
    fn test_parse_size_value() {
        assert_eq!(parse_size_value("100"), Some(100));
        assert_eq!(parse_size_value("1KB"), Some(1024));
        assert_eq!(parse_size_value("10MB"), Some(10_485_760));
        assert_eq!(parse_size_value("1GB"), Some(1_073_741_824));
    }
}
