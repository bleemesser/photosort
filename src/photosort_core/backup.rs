use crate::photosort_core::error::{PhotosortError, Result};
use crate::photosort_core::import::{Library, DB_DATE_FORMAT};
use rusqlite::params;
use std::path::Path;
use std::process::Command;
use time::OffsetDateTime;

/// Result of a backup operation.
#[derive(Debug)]
pub struct BackupResult {
    pub files_copied: usize,
    pub bytes_transferred: u64,
    pub backup_id: i64,
}

/// Check if rsync is available on the system.
pub fn rsync_available() -> bool {
    Command::new("rsync")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a backup from a library to a target directory.
pub fn backup(lib: &mut Library, target_dir: &Path, dry_run: bool) -> Result<BackupResult> {
    if !rsync_available() {
        return Err(PhotosortError::Rsync(
            "rsync is not installed or not in PATH".to_string(),
        ));
    }

    let source = lib.root().to_path_buf();

    // Validate target directory
    if !target_dir.exists() {
        std::fs::create_dir_all(target_dir)?;
        println!("Created backup directory: {}", target_dir.display());
    }

    // Record backup start in database
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let now_str = now.format(DB_DATE_FORMAT).unwrap();

    let backup_id = if !dry_run {
        let conn = lib.database_mut().connection();
        conn.execute(
            "INSERT INTO backup_history (target_path, started_at, status) VALUES (?1, ?2, 'running')",
            params![target_dir.to_string_lossy().to_string(), now_str],
        )?;
        conn.last_insert_rowid()
    } else {
        0
    };

    // Build rsync command
    // -a: archive mode (preserves permissions, times, etc.)
    // -v: verbose
    // --delete: delete files in target that don't exist in source
    // --progress: show progress
    // --stats: show transfer stats
    let mut cmd = Command::new("rsync");
    cmd.arg("-av")
        .arg("--delete")
        .arg("--progress")
        .arg("--stats");

    if dry_run {
        cmd.arg("--dry-run");
    }

    // Ensure source path ends with / to copy contents, not the directory itself
    let source_path = format!("{}/", source.display());
    cmd.arg(&source_path).arg(target_dir);

    println!(
        "{}Backing up {} -> {}",
        if dry_run { "[DRY RUN] " } else { "" },
        source.display(),
        target_dir.display()
    );

    let output = cmd.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !dry_run {
            // Mark backup as failed
            let conn = lib.database_mut().connection();
            conn.execute(
                "UPDATE backup_history SET status = 'failed' WHERE id = ?1",
                params![backup_id],
            )?;
        }
        return Err(PhotosortError::Rsync(format!(
            "rsync failed: {}",
            stderr.trim()
        )));
    }

    // Parse rsync stats output
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (files_copied, bytes_transferred) = parse_rsync_stats(&stdout);

    println!("\n{}", stdout);

    if !dry_run {
        // Update backup history
        let completed_at = OffsetDateTime::now_local()
            .unwrap_or_else(|_| OffsetDateTime::now_utc())
            .format(DB_DATE_FORMAT)
            .unwrap();

        let conn = lib.database_mut().connection();
        conn.execute(
            "UPDATE backup_history SET completed_at = ?1, files_copied = ?2, bytes_copied = ?3, status = 'completed' WHERE id = ?4",
            params![completed_at, files_copied as i64, bytes_transferred as i64, backup_id],
        )?;

        // Update backup state for all media
        conn.execute(
            "INSERT OR REPLACE INTO backup_state (media_id, last_backup_id, backed_up_at)
             SELECT id, ?1, ?2 FROM media",
            params![backup_id, completed_at],
        )?;
    }

    Ok(BackupResult {
        files_copied,
        bytes_transferred,
        backup_id,
    })
}

/// Parse rsync stats output to extract file count and bytes transferred.
fn parse_rsync_stats(output: &str) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0u64;

    for line in output.lines() {
        // Look for "Number of files transferred: X" or "Number of regular files transferred: X"
        if line.contains("files transferred:") {
            if let Some(num) = line.split(':').nth(1) {
                files = num.trim().replace(",", "").parse().unwrap_or(0);
            }
        }
        // Look for "Total transferred file size: X bytes"
        if line.contains("Total transferred file size:") {
            if let Some(num) = line.split(':').nth(1) {
                let num = num.trim().split_whitespace().next().unwrap_or("0");
                bytes = num.replace(",", "").parse().unwrap_or(0);
            }
        }
    }

    (files, bytes)
}

/// Get backup history for a library.
pub fn get_backup_history(lib: &Library) -> Result<Vec<BackupHistoryEntry>> {
    let conn = lib.database().connection_ref();
    let mut stmt = conn.prepare(
        "SELECT id, target_path, started_at, completed_at, files_copied, bytes_copied, status
         FROM backup_history
         ORDER BY started_at DESC
         LIMIT 20"
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(BackupHistoryEntry {
            id: row.get(0)?,
            target_path: row.get(1)?,
            started_at: row.get(2)?,
            completed_at: row.get(3)?,
            files_copied: row.get(4)?,
            bytes_copied: row.get(5)?,
            status: row.get(6)?,
        })
    })?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }

    Ok(entries)
}

/// A backup history entry.
#[derive(Debug)]
pub struct BackupHistoryEntry {
    pub id: i64,
    pub target_path: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub files_copied: i64,
    pub bytes_copied: i64,
    pub status: String,
}

/// Get the count of files changed since last backup.
pub fn files_changed_since_backup(lib: &Library) -> Result<i64> {
    let conn = lib.database().connection_ref();

    // Count media files not in backup_state (never backed up)
    // or where the media was imported after the backup
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM media m
         LEFT JOIN backup_state bs ON m.id = bs.media_id
         WHERE bs.media_id IS NULL
            OR m.imported_at > bs.backed_up_at",
        [],
        |row| row.get(0),
    )?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rsync_stats() {
        let output = r#"
Number of files: 100
Number of files transferred: 25
Total file size: 1,234,567 bytes
Total transferred file size: 500,000 bytes
        "#;

        let (files, bytes) = parse_rsync_stats(output);
        assert_eq!(files, 25);
        assert_eq!(bytes, 500_000);
    }

    #[test]
    fn test_rsync_available() {
        // This test just checks the function runs without panic
        let _ = rsync_available();
    }
}
