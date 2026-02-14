use crate::photosort_core::database::Database;
use crate::photosort_core::error::{CopyFailures, PhotosortError, Result};
use crate::photosort_core::exif::extract_metadata;
use crate::photosort_core::media::{detect_media_type, ExifMetadata, MediaType};
use crate::photosort_core::sidecar::find_sidecars;
use base64::{engine::general_purpose, Engine};
use exiftool::ExifTool;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use time::OffsetDateTime;
use walkdir::WalkDir;

thread_local! {
    static EXIFTOOL: RefCell<Option<ExifTool>> = const { RefCell::new(None) };
}

const DB_FILE_NAME: &str = "library.db";

/// Date format for database storage.
pub const DB_DATE_FORMAT: &[time::format_description::FormatItem] = time::macros::format_description!(
    "[year]:[month]:[day] [hour]:[minute]:[second].[subsecond][offset_hour sign:mandatory]:[offset_minute]"
);

/// Date format for filesystem paths (YYYY/MM-DD).
pub const PATH_DATE_FORMAT: &[time::format_description::FormatItem] =
    time::macros::format_description!("[year]/[month]-[day]");

/// A library of photos and videos.
pub struct Library {
    root: PathBuf,
    db: Database,
}

/// Information about a file to be imported.
#[derive(Debug)]
struct ImportCandidate {
    source_path: PathBuf,
    hash: String,
    media_type: MediaType,
    file_size: u64,
    created_at: OffsetDateTime,
    filename: String,
    filetype: String,
    sidecars: Vec<SidecarCandidate>,
    exif: ExifMetadata,
}

#[derive(Debug)]
struct SidecarCandidate {
    source_path: PathBuf,
    filename: String,
    filetype: String,
    file_size: u64,
    hash: String,
    modified_at: OffsetDateTime,
}

/// File copy operation to be performed.
#[derive(Debug, Clone)]
struct FileCopy {
    source: PathBuf,
    destination: PathBuf,
}

impl Library {
    /// Create a new library at the specified directory.
    pub fn create(dir: &Path) -> Result<Self> {
        if dir.exists() {
            if dir.join(DB_FILE_NAME).exists() {
                return Err(PhotosortError::LibraryExists(dir.to_path_buf()));
            }
        } else {
            fs::create_dir_all(dir)?;
        }

        // Create images and videos subdirectories
        fs::create_dir_all(dir.join("images"))?;
        fs::create_dir_all(dir.join("videos"))?;

        let db_path = dir.join(DB_FILE_NAME);
        let db = Database::new(&db_path)?;

        Ok(Library {
            root: dir.to_path_buf(),
            db,
        })
    }

    /// Open an existing library.
    pub fn open(dir: &Path) -> Result<Self> {
        if !dir.exists() {
            return Err(PhotosortError::LibraryNotFound(dir.to_path_buf()));
        }

        let db_path = dir.join(DB_FILE_NAME);
        if !db_path.exists() {
            return Err(PhotosortError::InvalidLibrary(dir.to_path_buf()));
        }

        let db = Database::new(&db_path)?;

        Ok(Library {
            root: dir.to_path_buf(),
            db,
        })
    }

    /// Get the library root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get a reference to the database.
    pub fn database(&self) -> &Database {
        &self.db
    }

    /// Get a mutable reference to the database.
    pub fn database_mut(&mut self) -> &mut Database {
        &mut self.db
    }

    /// Import media from a source directory.
    pub fn import(&mut self, source_dir: &Path, dry_run: bool) -> Result<ImportStats> {
        if !source_dir.exists() || !source_dir.is_dir() {
            return Err(PhotosortError::NotADirectory(source_dir.to_path_buf()));
        }

        log::info!("Phase 1: Scanning source directory {}", source_dir.display());

        // Collect all files
        let files: Vec<PathBuf> = WalkDir::new(source_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect();

        let bar_style = ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap();

        let scan_bar = ProgressBar::new(files.len() as u64).with_style(bar_style.clone());
        scan_bar.set_message("Scanning files");

        // Process files in parallel to extract metadata
        let candidates: Vec<ImportCandidate> = files
            .par_iter()
            .filter_map(|path| {
                let result = process_source_file(path);
                scan_bar.inc(1);
                match result {
                    Ok(Some(candidate)) => Some(candidate),
                    Ok(None) => None, // Not a media file
                    Err(e) => {
                        log::warn!("Error processing {}: {}", path.display(), e);
                        None
                    }
                }
            })
            .collect();

        scan_bar.finish_with_message("Scan complete");

        log::info!("Found {} media files to process", candidates.len());

        // Deduplicate by hash, but handle sidecar conflicts interactively
        let mut unique_by_hash: HashMap<String, ImportCandidate> = HashMap::new();
        let mut duplicates_skipped = 0;

        for candidate in candidates {
            if self.db.hash_exists(&candidate.hash)? {
                duplicates_skipped += 1;
                log::debug!("Skipping duplicate (already in library): {}", candidate.filename);
                continue;
            }

            match unique_by_hash.entry(candidate.hash.clone()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(candidate);
                }
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let existing = e.get_mut();

                    // Check if both have sidecars (potential edit conflict)
                    if !existing.sidecars.is_empty() && !candidate.sidecars.is_empty() {
                        // Both have edits - prompt user
                        println!("\nDuplicate media with different edits detected:");
                        println!("  1. {} ({} sidecars)", existing.filename, existing.sidecars.len());
                        for sc in &existing.sidecars {
                            println!("     - {}", sc.filename);
                        }
                        println!("  2. {} ({} sidecars)", candidate.filename, candidate.sidecars.len());
                        for sc in &candidate.sidecars {
                            println!("     - {}", sc.filename);
                        }
                        println!("\nOptions:");
                        println!("  [1] Keep first only (discard second and its edits)");
                        println!("  [2] Keep second only (discard first and its edits)");
                        println!("  [B] Keep both (import both files with their respective edits)");
                        print!("Choice [1/2/B]: ");
                        io::stdout().flush()?;

                        let mut input = String::new();
                        io::stdin().read_line(&mut input)?;

                        match input.trim().to_uppercase().as_str() {
                            "2" => {
                                // Replace existing with candidate
                                *existing = candidate;
                                log::info!("User chose to keep second file");
                            }
                            "B" => {
                                // Keep both - add candidate as separate entry with modified hash
                                // We use a synthetic hash to keep them separate
                                let synthetic_hash = format!("{}-alt", candidate.hash);
                                let mut alt_candidate = candidate;
                                alt_candidate.hash = synthetic_hash.clone();
                                unique_by_hash.insert(synthetic_hash, alt_candidate);
                                log::info!("User chose to keep both files");
                            }
                            _ => {
                                // Default: keep first (existing), skip candidate
                                duplicates_skipped += 1;
                                log::info!("User chose to keep first file");
                            }
                        }
                    } else if candidate.sidecars.is_empty() {
                        // Candidate has no sidecars, just skip it
                        duplicates_skipped += 1;
                        log::debug!("Skipping duplicate media (no sidecars): {}", candidate.filename);
                    } else {
                        // Existing has no sidecars but candidate does - merge sidecars
                        log::info!(
                            "Adding {} sidecars from {} to {}",
                            candidate.sidecars.len(),
                            candidate.filename,
                            existing.filename
                        );
                        existing.sidecars.extend(candidate.sidecars);
                        duplicates_skipped += 1;
                    }
                }
            }
        }

        let to_import: Vec<ImportCandidate> = unique_by_hash.into_values().collect();
        log::info!(
            "{} unique files to import ({} duplicates skipped)",
            to_import.len(),
            duplicates_skipped
        );

        if dry_run {
            println!("\n[DRY RUN] Would import:");
            let mut images = 0;
            let mut videos = 0;
            for c in &to_import {
                match c.media_type {
                    MediaType::Image => images += 1,
                    MediaType::Video => videos += 1,
                }
            }
            println!("  {} images", images);
            println!("  {} videos", videos);
            println!("  {} sidecars", to_import.iter().map(|c| c.sidecars.len()).sum::<usize>());
            return Ok(ImportStats {
                images_imported: 0,
                videos_imported: 0,
                sidecars_imported: 0,
                duplicates_skipped,
                errors: 0,
            });
        }

        // Phase 2: Copy files first
        log::info!("Phase 2: Copying files to library");

        let mut file_copies: Vec<FileCopy> = Vec::new();
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());

        for candidate in &to_import {
            let rel_path = format!(
                "{}/{}",
                candidate.media_type.folder_name(),
                candidate.created_at.format(PATH_DATE_FORMAT).unwrap()
            );
            let dest_dir = self.root.join(&rel_path);
            let dest_path = dest_dir.join(&candidate.filename);

            file_copies.push(FileCopy {
                source: candidate.source_path.clone(),
                destination: dest_path,
            });

            // Add sidecar copies
            for sidecar in &candidate.sidecars {
                let sidecar_dest = dest_dir.join(&sidecar.filename);
                file_copies.push(FileCopy {
                    source: sidecar.source_path.clone(),
                    destination: sidecar_dest,
                });
            }
        }

        // Deduplicate by destination path (handles case where JPG and DNG share sidecars)
        let mut seen_destinations: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut deduped_copies: Vec<FileCopy> = Vec::new();

        for fc in file_copies {
            match seen_destinations.entry(fc.destination.clone()) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(fc.source.clone());
                    deduped_copies.push(fc);
                }
                std::collections::hash_map::Entry::Occupied(e) => {
                    // Same destination from different source - skip (sidecar shared between JPG/DNG)
                    log::debug!(
                        "Skipping duplicate destination {} (already from {})",
                        fc.destination.display(),
                        e.get().display()
                    );
                }
            }
        }

        let file_copies = deduped_copies;

        // Perform copies
        let copy_bar = ProgressBar::new(file_copies.len() as u64).with_style(bar_style.clone());
        copy_bar.set_message("Copying files");

        let copy_failures = Mutex::new(CopyFailures::new());

        file_copies.par_iter().for_each(|fc| {
            // Create parent directory
            if let Some(parent) = fc.destination.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    copy_failures.lock().unwrap().add(
                        fc.source.clone(),
                        fc.destination.clone(),
                        e,
                    );
                    copy_bar.inc(1);
                    return;
                }
            }

            // Copy file
            if let Err(e) = fs::copy(&fc.source, &fc.destination) {
                copy_failures.lock().unwrap().add(
                    fc.source.clone(),
                    fc.destination.clone(),
                    e,
                );
            }
            copy_bar.inc(1);
        });

        copy_bar.finish_with_message("Copy complete");

        let failures = copy_failures.into_inner().unwrap();
        if !failures.is_empty() {
            log::error!("{} files failed to copy", failures.len());
            return Err(PhotosortError::CopyFailed(failures));
        }

        // Phase 3: Update database (only after successful copies)
        log::info!("Phase 3: Updating database");

        let conn = self.db.connection();
        let tx = conn.transaction()?;

        let mut images_imported = 0;
        let mut videos_imported = 0;
        let mut sidecars_imported = 0;

        for candidate in &to_import {
            let rel_path = format!(
                "{}/{}",
                candidate.media_type.folder_name(),
                candidate.created_at.format(PATH_DATE_FORMAT).unwrap()
            );

            let created_at_str = candidate.created_at.format(DB_DATE_FORMAT).unwrap();
            let imported_at_str = now.format(DB_DATE_FORMAT).unwrap();

            tx.execute(
                "INSERT INTO media (hash, filename, relpath, media_type, filetype, file_size, created_at, imported_at,
                                    camera_make, camera_model, lens, focal_length, aperture, shutter_speed, iso, gps_lat, gps_lon)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    candidate.hash,
                    candidate.filename,
                    rel_path,
                    candidate.media_type.as_str(),
                    candidate.filetype,
                    candidate.file_size as i64,
                    created_at_str,
                    imported_at_str,
                    candidate.exif.camera_make,
                    candidate.exif.camera_model,
                    candidate.exif.lens,
                    candidate.exif.focal_length,
                    candidate.exif.aperture,
                    candidate.exif.shutter_speed,
                    candidate.exif.iso,
                    candidate.exif.gps_lat,
                    candidate.exif.gps_lon,
                ],
            )?;

            let media_id = tx.last_insert_rowid();

            match candidate.media_type {
                MediaType::Image => images_imported += 1,
                MediaType::Video => videos_imported += 1,
            }

            // Insert sidecars
            for sidecar in &candidate.sidecars {
                let modified_at_str = sidecar.modified_at.format(DB_DATE_FORMAT).unwrap();
                tx.execute(
                    "INSERT INTO sidecars (media_id, filename, filetype, file_size, hash, modified_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        media_id,
                        sidecar.filename,
                        sidecar.filetype,
                        sidecar.file_size as i64,
                        sidecar.hash,
                        modified_at_str,
                    ],
                )?;
                sidecars_imported += 1;
            }
        }

        tx.commit()?;

        log::info!(
            "Import complete: {} images, {} videos, {} sidecars",
            images_imported,
            videos_imported,
            sidecars_imported
        );

        Ok(ImportStats {
            images_imported,
            videos_imported,
            sidecars_imported,
            duplicates_skipped,
            errors: 0,
        })
    }
}

/// Process a source file and return import candidate if it's a media file.
fn process_source_file(path: &Path) -> Result<Option<ImportCandidate>> {
    // Detect media type
    let media_type = match detect_media_type(path) {
        Some(mt) => mt,
        None => return Ok(None), // Not a media file
    };

    // Get file info
    let metadata = fs::metadata(path)?;
    let file_size = metadata.len();

    // Calculate hash
    let hash = hash_file(path)?;

    // Extract EXIF metadata using thread-local ExifTool instance
    let extracted = EXIFTOOL.with(|cell| {
        let mut exiftool_opt = cell.borrow_mut();
        if exiftool_opt.is_none() {
            *exiftool_opt = ExifTool::new().ok();
        }
        match exiftool_opt.as_mut() {
            Some(exiftool) => extract_metadata(exiftool, path).unwrap_or_else(|e| {
                log::warn!("Failed to extract metadata from {}: {}", path.display(), e);
                crate::photosort_core::exif::ExtractedMetadata {
                    created_at: OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()),
                    exif: Default::default(),
                } // TODO: never gets called because exiftool doesn't really fail
                // but exiftool may not find creation date fields correctly and we now
                // have 2 fallbacks total
            }),
            None => {
                log::warn!("ExifTool not available for {}", path.display());
                crate::photosort_core::exif::ExtractedMetadata {
                    created_at: OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()),
                    exif: Default::default(),
                }
            }
        }
    });

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

    // Find sidecars
    let sidecar_paths = find_sidecars(path);
    let mut sidecars = Vec::new();

    for sidecar_path in sidecar_paths {
        if let Ok(sc) = process_sidecar(&sidecar_path) {
            sidecars.push(sc);
        }
    }

    Ok(Some(ImportCandidate {
        source_path: path.to_path_buf(),
        hash,
        media_type,
        file_size,
        created_at: extracted.created_at,
        filename,
        filetype,
        sidecars,
        exif: extracted.exif,
    }))
}

/// Process a sidecar file.
fn process_sidecar(path: &Path) -> Result<SidecarCandidate> {
    let metadata = fs::metadata(path)?;
    let file_size = metadata.len();
    let modified_at = metadata
        .modified()
        .map(OffsetDateTime::from)
        .unwrap_or_else(|_| OffsetDateTime::now_utc());

    let hash = hash_file(path)?;

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

    Ok(SidecarCandidate {
        source_path: path.to_path_buf(),
        filename,
        filetype,
        file_size,
        hash,
        modified_at,
    })
}

/// Calculate SHA256 hash of a file, returned as base64.
pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let hash = hasher.finalize();
    Ok(general_purpose::STANDARD.encode(hash))
}

/// Statistics from an import operation.
#[derive(Debug, Default)]
pub struct ImportStats {
    pub images_imported: usize,
    pub videos_imported: usize,
    pub sidecars_imported: usize,
    pub duplicates_skipped: usize,
    pub errors: usize,
}

impl std::fmt::Display for ImportStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} images, {} videos, {} sidecars imported ({} duplicates skipped)",
            self.images_imported,
            self.videos_imported,
            self.sidecars_imported,
            self.duplicates_skipped
        )
    }
}
