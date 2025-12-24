use crate::photosort_core::database::Database;
use crate::photosort_core::error::Result;
use crate::photosort_core::import::{hash_file, Library, DB_DATE_FORMAT};
use crate::photosort_core::media::detect_media_type;
use rusqlite::params;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use walkdir::WalkDir;

/// Result of scanning a library for changes.
#[derive(Debug, Default)]
pub struct ScanResult {
    pub missing_files: Vec<MissingFile>,
    pub new_files: Vec<PathBuf>,
    pub modified_sidecars: Vec<ModifiedSidecar>,
    pub orphaned_sidecars: Vec<OrphanedSidecar>,
}

#[derive(Debug)]
pub struct MissingFile {
    pub id: i64,
    pub filename: String,
    pub relpath: String,
    pub media_type: String,
    pub expected_path: PathBuf,
}

#[derive(Debug)]
pub struct ModifiedSidecar {
    pub id: i64,
    pub media_id: i64,
    pub filename: String,
    pub old_hash: String,
    pub new_hash: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct OrphanedSidecar {
    pub id: i64,
    pub filename: String,
    pub relpath: String,
    pub expected_path: PathBuf,
}

impl ScanResult {
    pub fn is_clean(&self) -> bool {
        self.missing_files.is_empty()
            && self.new_files.is_empty()
            && self.modified_sidecars.is_empty()
            && self.orphaned_sidecars.is_empty()
    }
}

/// Scan a library for filesystem changes.
pub fn scan_library(lib: &Library) -> Result<ScanResult> {
    let root = lib.root();
    let db = lib.database();

    let mut result = ScanResult::default();

    println!("Scanning library for changes...");

    // Phase 1: Check for missing files (in DB but not on disk)
    println!("\nChecking for missing files...");
    result.missing_files = find_missing_files(db, root)?;

    // Phase 2: Check for orphaned sidecars
    println!("Checking for orphaned sidecars...");
    result.orphaned_sidecars = find_orphaned_sidecars(db, root)?;

    // Phase 3: Check for modified sidecars
    println!("Checking for modified sidecars...");
    result.modified_sidecars = find_modified_sidecars(db, root)?;

    // Phase 4: Check for new files (on disk but not in DB)
    println!("Checking for new files...");
    result.new_files = find_new_files(db, root)?;

    Ok(result)
}

/// Find files that are in the database but missing from disk.
fn find_missing_files(db: &Database, root: &Path) -> Result<Vec<MissingFile>> {
    let mut missing = Vec::new();

    let mut stmt = db.connection_ref().prepare(
        "SELECT id, filename, relpath, media_type FROM media"
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    for row in rows {
        let (id, filename, relpath, media_type) = row?;
        let expected_path = root.join(&relpath).join(&filename);

        if !expected_path.exists() {
            missing.push(MissingFile {
                id,
                filename,
                relpath,
                media_type,
                expected_path,
            });
        }
    }

    Ok(missing)
}

/// Find sidecars that are in the database but missing from disk.
fn find_orphaned_sidecars(db: &Database, root: &Path) -> Result<Vec<OrphanedSidecar>> {
    let mut orphaned = Vec::new();

    let mut stmt = db.connection_ref().prepare(
        "SELECT s.id, s.filename, m.relpath
         FROM sidecars s
         JOIN media m ON s.media_id = m.id"
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    for row in rows {
        let (id, filename, relpath) = row?;
        let expected_path = root.join(&relpath).join(&filename);

        if !expected_path.exists() {
            orphaned.push(OrphanedSidecar {
                id,
                filename,
                relpath,
                expected_path,
            });
        }
    }

    Ok(orphaned)
}

/// Find sidecars that have been modified since import.
fn find_modified_sidecars(db: &Database, root: &Path) -> Result<Vec<ModifiedSidecar>> {
    let mut modified = Vec::new();

    let mut stmt = db.connection_ref().prepare(
        "SELECT s.id, s.media_id, s.filename, s.hash, m.relpath
         FROM sidecars s
         JOIN media m ON s.media_id = m.id"
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    for row in rows {
        let (id, media_id, filename, old_hash, relpath) = row?;
        let path = root.join(&relpath).join(&filename);

        if path.exists() {
            if let Ok(new_hash) = hash_file(&path) {
                if new_hash != old_hash {
                    modified.push(ModifiedSidecar {
                        id,
                        media_id,
                        filename,
                        old_hash,
                        new_hash,
                        path,
                    });
                }
            }
        }
    }

    Ok(modified)
}

/// Find files on disk that are not in the database.
fn find_new_files(db: &Database, root: &Path) -> Result<Vec<PathBuf>> {
    // Get all known file paths from DB
    let mut known_paths: HashSet<PathBuf> = HashSet::new();

    // Add media files
    let mut stmt = db.connection_ref().prepare(
        "SELECT relpath, filename FROM media"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (relpath, filename) = row?;
        known_paths.insert(root.join(&relpath).join(&filename));
    }

    // Add sidecar files
    let mut stmt = db.connection_ref().prepare(
        "SELECT m.relpath, s.filename FROM sidecars s JOIN media m ON s.media_id = m.id"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (relpath, filename) = row?;
        known_paths.insert(root.join(&relpath).join(&filename));
    }

    // Scan filesystem
    let mut new_files = Vec::new();
    let images_dir = root.join("images");
    let videos_dir = root.join("videos");

    for dir in [&images_dir, &videos_dir] {
        if !dir.exists() {
            continue;
        }

        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Skip database file
            if path.file_name().map(|f| f == "library.db").unwrap_or(false) {
                continue;
            }

            // Skip if already known
            if known_paths.contains(path) {
                continue;
            }

            // Check if it's a media file
            if detect_media_type(path).is_some() {
                new_files.push(path.to_path_buf());
            }
        }
    }

    Ok(new_files)
}

/// Interactive handler for scan results.
pub fn handle_scan_results(lib: &mut Library, result: &ScanResult) -> Result<()> {
    if result.is_clean() {
        println!("\nLibrary is clean. No changes detected.");
        return Ok(());
    }

    println!("\n─────────────────────────────────");
    println!("Scan Summary:");
    println!("  Missing files:      {}", result.missing_files.len());
    println!("  Orphaned sidecars:  {}", result.orphaned_sidecars.len());
    println!("  Modified sidecars:  {}", result.modified_sidecars.len());
    println!("  New files:          {}", result.new_files.len());
    println!("─────────────────────────────────\n");

    // Handle missing files
    if !result.missing_files.is_empty() {
        handle_missing_files(lib, &result.missing_files)?;
    }

    // Handle orphaned sidecars
    if !result.orphaned_sidecars.is_empty() {
        handle_orphaned_sidecars(lib, &result.orphaned_sidecars)?;
    }

    // Handle modified sidecars
    if !result.modified_sidecars.is_empty() {
        handle_modified_sidecars(lib, &result.modified_sidecars)?;
    }

    // Handle new files
    if !result.new_files.is_empty() {
        handle_new_files(lib, &result.new_files)?;
    }

    Ok(())
}

fn handle_missing_files(lib: &mut Library, missing: &[MissingFile]) -> Result<()> {
    println!("Missing files ({}):", missing.len());
    for (i, f) in missing.iter().enumerate() {
        println!("  {}. {} ({})", i + 1, f.filename, f.media_type);
        println!("     Expected: {}", f.expected_path.display());
    }

    println!("\nThese files are in the database but not on disk.");
    println!("Options:");
    println!("  [R] Remove all from database (they were intentionally deleted)");
    println!("  [S] Select individually which to remove");
    println!("  [I] Ignore (keep database records, maybe you'll restore from backup)");
    print!("\nChoice [R/S/I]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    match input.trim().to_uppercase().as_str() {
        "R" => {
            let conn = lib.database_mut().connection();
            let tx = conn.transaction()?;
            for f in missing {
                tx.execute("DELETE FROM media WHERE id = ?1", params![f.id])?;
            }
            tx.commit()?;
            println!("Removed {} records from database.", missing.len());
        }
        "S" => {
            let conn = lib.database_mut().connection();
            let tx = conn.transaction()?;
            let mut removed = 0;
            for f in missing {
                print!("Remove '{}' from database? [y/N]: ", f.filename);
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                if input.trim().to_lowercase() == "y" {
                    tx.execute("DELETE FROM media WHERE id = ?1", params![f.id])?;
                    removed += 1;
                }
            }
            tx.commit()?;
            println!("Removed {} records from database.", removed);
        }
        _ => {
            println!("Keeping database records unchanged.");
        }
    }

    Ok(())
}

fn handle_orphaned_sidecars(lib: &mut Library, orphaned: &[OrphanedSidecar]) -> Result<()> {
    println!("\nOrphaned sidecars ({}):", orphaned.len());
    for f in orphaned.iter().take(10) {
        println!("  - {}", f.filename);
    }
    if orphaned.len() > 10 {
        println!("  ... and {} more", orphaned.len() - 10);
    }

    print!("\nRemove orphaned sidecar records from database? [Y/n]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "n" {
        let conn = lib.database_mut().connection();
        let tx = conn.transaction()?;
        for f in orphaned {
            tx.execute("DELETE FROM sidecars WHERE id = ?1", params![f.id])?;
        }
        tx.commit()?;
        println!("Removed {} orphaned sidecar records.", orphaned.len());
    }

    Ok(())
}

fn handle_modified_sidecars(lib: &mut Library, modified: &[ModifiedSidecar]) -> Result<()> {
    println!("\nModified sidecars ({}):", modified.len());
    for f in modified.iter().take(10) {
        println!("  - {} (hash changed)", f.filename);
    }
    if modified.len() > 10 {
        println!("  ... and {} more", modified.len() - 10);
    }

    print!("\nUpdate database with new hashes? [Y/n]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if input.trim().to_lowercase() != "n" {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let now_str = now.format(DB_DATE_FORMAT).unwrap();

        let conn = lib.database_mut().connection();
        let tx = conn.transaction()?;
        for f in modified {
            tx.execute(
                "UPDATE sidecars SET hash = ?1, modified_at = ?2 WHERE id = ?3",
                params![f.new_hash, now_str, f.id],
            )?;
        }
        tx.commit()?;
        println!("Updated {} sidecar records.", modified.len());
    }

    Ok(())
}

fn handle_new_files(_lib: &mut Library, new_files: &[PathBuf]) -> Result<()> {
    println!("\nNew files found ({}):", new_files.len());
    for f in new_files.iter().take(10) {
        println!("  - {}", f.display());
    }
    if new_files.len() > 10 {
        println!("  ... and {} more", new_files.len() - 10);
    }

    println!("\nThese files exist on disk but are not in the database.");
    println!("You can import them with: photosort import <library> <library>");
    println!("(Using the library itself as the source will add untracked files)");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_result_is_clean() {
        let result = ScanResult::default();
        assert!(result.is_clean());

        let result_with_missing = ScanResult {
            missing_files: vec![MissingFile {
                id: 1,
                filename: "test.jpg".to_string(),
                relpath: "images/2024/01-01".to_string(),
                media_type: "image".to_string(),
                expected_path: PathBuf::from("/test/images/2024/01-01/test.jpg"),
            }],
            ..Default::default()
        };
        assert!(!result_with_missing.is_clean());
    }
}
