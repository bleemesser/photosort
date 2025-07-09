use crate::photosort_core::{
    Database, PhotosortError, SourcePhotoInfo, SourceSidecarInfo, hash_file, process_photo_file,
};
use crossbeam_channel::unbounded;
use exiftool::ExifTool;
use fuzzy_match_flex::partial_ratio;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use rusqlite::{Connection, params};
use simplelog::FormatItem;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
use time::{OffsetDateTime, macros::format_description};
use walkdir::WalkDir;

const DB_FILE_NAME: &'static str = "library.db";
pub const DB_DATE_FORMAT: &[FormatItem] = format_description!(
    "[year]:[month]:[day] [hour]:[minute]:[second].[subsecond][offset_hour sign:mandatory]:[offset_minute]"
);
pub const FP_DATE_FORMAT: &[FormatItem] = format_description!("[year]/[month]-[day]");

pub struct Library {
    root: PathBuf,
    db: Database,
}

impl Library {
    /// Create a new library at the specified directory.
    /// If a library (including db) already exists, return an error.
    /// If the directory does not exist, it will be created.
    pub fn create(dir: &Path) -> Result<Self, PhotosortError> {
        if dir.exists() {
            if dir.join(DB_FILE_NAME).exists() {
                return Err(PhotosortError::LibraryExists(dir.to_path_buf()));
            }
        } else {
            fs::create_dir_all(dir)?;
        }
        let db_path = dir.join(DB_FILE_NAME);
        let db = Database::new(&db_path)?;

        Ok(Library {
            root: dir.to_path_buf(),
            db,
        })
    }

    /// Open an existing library at the specified directory.
    /// If the directory does not exist or the database file is missing, return an error.
    pub fn open(dir: &Path) -> Result<Self, PhotosortError> {
        if !dir.exists() || !dir.join(DB_FILE_NAME).exists() {
            return Err(PhotosortError::LibraryNotFound(dir.to_path_buf()));
        }
        let db_path = dir.join(DB_FILE_NAME);
        let db = Database::new(&db_path)?;

        Ok(Library {
            root: dir.to_path_buf(),
            db,
        })
    }

    /// Import photos from a directory into the library.
    pub fn import(&mut self, photo_dir: &Path, copy_files: bool) -> Result<(), PhotosortError> {
        log::info!("Import Phase 1: Scanning directory {}", photo_dir.display());

        if !photo_dir.exists() || !photo_dir.is_dir() {
            return Err(PhotosortError::Argument(format!(
                "Photo directory '{}' does not exist or is not a directory.",
                photo_dir.display()
            )));
        }

        // Get all files in the photo directory
        let paths: Vec<PathBuf> = WalkDir::new(photo_dir)
            .into_iter()
            .filter_map(Result::ok)
            .map(|entry| entry.into_path())
            .filter(|path| path.is_file())
            .collect();

        let bar_style = ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")
            .unwrap();

        let bar = ProgressBar::new(paths.len() as u64).with_style(bar_style.clone());
        bar.set_message("Scanning metadata");

        let num_workers = num_cpus::get();
        let (job_tx, job_rx) = unbounded::<PathBuf>();
        let (result_tx, result_rx) = unbounded();

        let worker_bar = bar.clone();

        rayon::scope(move |s| {
            s.spawn(move |_| {
                for path in paths {
                    if job_tx.send(path).is_err() {
                        log::error!("Failed to send job to worker channel");
                        break;
                    }
                }
                drop(job_tx);
            });

            for _ in 0..num_workers {
                let job_rx_clone = job_rx.clone();
                let result_tx_clone = result_tx.clone();
                let bar_clone = worker_bar.clone();

                s.spawn(move |_| {
                    if let Ok(mut exiftool) = ExifTool::new() {
                        for path in job_rx_clone {
                            let result = process_photo_file(&mut exiftool, path);
                            bar_clone.inc(1);
                            if result_tx_clone.send(result).is_err() {
                                log::error!("Failed to send result to main thread");
                                break;
                            }
                        }
                    } else {
                        log::error!("A worker failed to initialize ExifTool.");
                    }
                });
            }
        });

        let all_source_photos: Vec<SourcePhotoInfo> = result_rx
            .iter()
            .filter_map(|result| match result {
                Ok(Some(info)) => Some(info),
                Ok(None) => None,
                Err(e) => {
                    log::error!("Error processing file: {}", e);
                    None
                }
            })
            .collect();

        let mut hash_winners: HashMap<String, SourcePhotoInfo> = HashMap::new();
        let mut replace_count = 0;
        for photo_info in all_source_photos {
            let hash = photo_info.hash.clone();
            match hash_winners.entry(hash) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if should_migrate_filename(
                        entry.get().filename.as_str(),
                        photo_info.filename.as_str(),
                    ) {
                        log::debug!(
                            "Replacing existing photo with better filename: {} -> {}",
                            entry.get().filename,
                            photo_info.filename
                        );
                        replace_count += 1;
                        *entry.get_mut() = photo_info;
                    }
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(photo_info);
                }
            }
        }

        bar.finish_with_message("Metadata scan complete");
        log::info!(
            "Phase 1 Complete. Selected {} unique photos for processing, and replacing {} filenames with better ones.",
            hash_winners.len(),
            replace_count
        );

        log::info!("Import Phase 2: Updating database");

        let conn: &mut Connection = self.db.connection();
        let tx = conn.transaction()?;
        let mut files_to_copy: Vec<FileToCopy> = Vec::new();
        let db_bar = ProgressBar::new(hash_winners.len() as u64).with_style(bar_style.clone());
        db_bar.set_message("Updating database");

        for (_, winner) in hash_winners.into_iter() {
            let rel_path_str = winner.created_at.format(FP_DATE_FORMAT).unwrap();

            let existing_photo: Result<(i64, String), _> = tx.query_row(
                "SELECT id, filename FROM photos WHERE hash = ?1",
                params![winner.hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            );

            match existing_photo {
                Ok((photo_id, old_filename)) => {
                    if should_migrate_filename(&old_filename, &winner.filename) {
                        tx.execute(
                            "UPDATE photos SET filename = ?1, relpath = ?2 WHERE id = ?3",
                            params![&winner.filename, &rel_path_str, photo_id],
                        )?;
                        tx.execute(
                            "DELETE FROM sidecars WHERE photo_id = ?1",
                            params![photo_id],
                        )?;
                        log::debug!(
                            "DB: Updated photo ID {} to '{}'",
                            photo_id,
                            &winner.filename
                        );

                        for sidecar in &winner.sidecars {
                            tx.execute("INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                params![photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype, &get_db_date_string(&sidecar.modified_at)?, &sidecar.hash],
                            )?;
                            if copy_files {
                                let destination_path =
                                    self.root.join(&rel_path_str).join(&sidecar.filename);
                                files_to_copy.push(FileToCopy {
                                    original_path: sidecar.original_path.clone(),
                                    destination_path,
                                });
                            }
                        }
                        if copy_files {
                            let destination_path =
                                self.root.join(&rel_path_str).join(&winner.filename);
                            files_to_copy.push(FileToCopy {
                                original_path: winner.original_path,
                                destination_path,
                            });
                        }
                    } else {
                        for sidecar in &winner.sidecars {
                            let mut stmt = tx.prepare(
                                "SELECT hash FROM sidecars WHERE photo_id = ?1 AND filename = ?2",
                            )?;
                            let existing_sidecar_hash: Result<String, _> = stmt
                                .query_row(params![photo_id, &sidecar.filename], |row| row.get(0));

                            match existing_sidecar_hash {
                                Ok(db_hash) if db_hash != sidecar.hash => {
                                    tx.execute("UPDATE sidecars SET hash = ?1, modified_at = ?2 WHERE photo_id = ?3 AND filename = ?4",
                                        params![&sidecar.hash, &get_db_date_string(&sidecar.modified_at)?, photo_id, &sidecar.filename],
                                    )?;
                                    if copy_files {
                                        let destination_path =
                                            self.root.join(&rel_path_str).join(&sidecar.filename);
                                        files_to_copy.push(FileToCopy {
                                            original_path: sidecar.original_path.clone(),
                                            destination_path,
                                        });
                                    }
                                }
                                Err(rusqlite::Error::QueryReturnedNoRows) => {
                                    tx.execute("INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                        params![photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype, &get_db_date_string(&sidecar.modified_at)?, &sidecar.hash],
                                    )?;
                                    if copy_files {
                                        let destination_path =
                                            self.root.join(&rel_path_str).join(&sidecar.filename);
                                        files_to_copy.push(FileToCopy {
                                            original_path: sidecar.original_path.clone(),
                                            destination_path,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    tx.execute(
                        "INSERT INTO photos (filename, relpath, filetype, created_at, hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![&winner.filename, &rel_path_str, &winner.filetype,
                                &get_db_date_string(&winner.created_at)?, &winner.hash]
                    )?;
                    let photo_id = tx.last_insert_rowid();
                    log::debug!(
                        "DB: Added new photo '{}' (ID: {})",
                        &winner.filename,
                        photo_id
                    );
                    if copy_files {
                        let destination_path = self.root.join(&rel_path_str).join(&winner.filename);
                        files_to_copy.push(FileToCopy {
                            original_path: winner.original_path,
                            destination_path,
                        });
                    }

                    for sidecar in &winner.sidecars {
                        tx.execute(
                            "INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype,
                                    &get_db_date_string(&sidecar.modified_at)?, &sidecar.hash]
                        )?;
                        if copy_files {
                            let destination_path =
                                self.root.join(&rel_path_str).join(&sidecar.filename);
                            files_to_copy.push(FileToCopy {
                                original_path: sidecar.original_path.clone(),
                                destination_path,
                            });
                        }
                    }
                }
                Err(e) => return Err(e.into()),
            }
            db_bar.inc(1);
        }

        tx.commit()?;
        db_bar.finish_with_message("Database update complete");
        log::info!("Import Phase 2 Complete");

        if !copy_files {
            log::info!("Import Phase 3 skipped (copy_files is false)");
            return Ok(());
        }

        log::info!("Import Phase 3: Copying files to library");
        let mut unique_copy_map: HashMap<PathBuf, FileToCopy> = HashMap::new();
        for f in files_to_copy {
            unique_copy_map.insert(f.destination_path.clone(), f);
        }
        let unique_files_to_copy: Vec<FileToCopy> = unique_copy_map.into_values().collect();
        let copy_bar = ProgressBar::new(unique_files_to_copy.len() as u64).with_style(bar_style);
        copy_bar.set_message("Copying files");

        unique_files_to_copy.par_iter().for_each(|f| {
            if let Some(parent) = f.destination_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    log::error!("Failed to create directory {}: {}", parent.display(), e);
                    return;
                }
            }
            if let Err(e) = fs::copy(&f.original_path, &f.destination_path) {
                log::warn!(
                    "Failed to copy file from {} to {}: {}",
                    f.original_path.display(),
                    f.destination_path.display(),
                    e
                );
            }
            copy_bar.inc(1);
        });

        copy_bar.finish_with_message("File copying complete");
        log::info!(
            "Import completed successfully. {} files copied to library.",
            unique_files_to_copy.len()
        );

        Ok(())
    }

    /// Update a library to reflect changes in the filesystem.
    pub fn update(&mut self) -> Result<(), PhotosortError> {
        log::info!("Update: Starting library update process...");
        {
            log::info!("UpdateDB: Culling missing files from database...");
            let conn = self.db.connection();
            let tx = conn.transaction()?;
            let mut photos_to_cull = Vec::new();
            {
                let mut stmt = tx.prepare("SELECT id, relpath, filename FROM photos")?;
                let photo_iter = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?, // id
                        self.root
                            .join(row.get::<_, String>(1)?)
                            .join(row.get::<_, String>(2)?), // path
                    ))
                })?;

                for photo_res in photo_iter {
                    let (id, path) = photo_res?;
                    if !path.exists() {
                        photos_to_cull.push(id);
                    }
                }
            }

            log::info!(
                "Update: Found {} photos to cull from database.",
                photos_to_cull.len()
            );

            for id in photos_to_cull.as_slice() {
                tx.execute("DELETE FROM photos WHERE id = ?", params![id])?;
            }

            let mut sidecars_to_cull = Vec::new();
            let mut sidecars_to_update: Vec<(String, OffsetDateTime, i64)> = Vec::new();
            {
                let mut stmt =
                    tx.prepare("SELECT id, relpath, filename, hash, photo_id FROM sidecars")?;
                let sidecar_iter = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?, // id
                        self.root
                            .join(row.get::<_, String>(1)?)
                            .join(row.get::<_, String>(2)?), // path
                        row.get::<_, String>(3)?, // db hash
                        row.get::<_, i64>(4)?, // photo_id
                    ))
                })?;

                for sidecar_res in sidecar_iter {
                    let (id, path, db_hash, photo_id) = sidecar_res?;
                    if !path.exists() || !path.is_file() || photos_to_cull.contains(&photo_id) {
                        sidecars_to_cull.push(id);
                        continue;
                    }

                    if let Ok(file_info) = fs::metadata(&path) {
                        if let Ok(current_hash) = hash_file(&path) {
                            if current_hash != db_hash {
                                let modified_time =
                                    file_info.modified().unwrap_or_else(|_| SystemTime::now());

                                sidecars_to_update.push((
                                    current_hash,
                                    OffsetDateTime::from(modified_time),
                                    id,
                                ));
                            }
                        }
                    }
                }
            }

            log::info!(
                "Update: Found {} sidecars to cull and {} to update.",
                sidecars_to_cull.len(),
                sidecars_to_update.len()
            );

            for id in sidecars_to_cull {
                tx.execute("DELETE FROM sidecars WHERE id = ?", params![id])?;
            }
            for (hash, modified, id) in sidecars_to_update {
                tx.execute(
                    "UPDATE sidecars SET hash = ?1, modified_at = ?2 WHERE id = ?3",
                    params![hash, modified.format(DB_DATE_FORMAT).unwrap(), id],
                )?;
            }
            tx.commit()?;
            log::info!("Update: Culling phase complete.");
        }

        log::info!(
            "Update: Rescanning library for new/changed metadata by calling import (no file copy)..."
        );
        let photo_dir = &mut self.root.clone();
        self.import(photo_dir, false)?;

        log::info!("UpdateDB: Library update process finished.");
        Ok(())
    }

    /// Count the number of photos in the library.
    pub fn get_photo_count(&mut self) -> Result<i64, PhotosortError> {
        let conn = self.db.connection();
        let mut select = conn.prepare("SELECT COUNT(*) FROM photos")?;
        let count = select.query_row([], |row| row.get(0))?;
        Ok(count)
    }

    /// Count the number of sidecars in the library.
    pub fn get_sidecar_count(&mut self) -> Result<i64, PhotosortError> {
        let conn = self.db.connection();
        let mut select = conn.prepare("SELECT COUNT(*) FROM sidecars")?;
        let count = select.query_row([], |row| row.get(0))?;
        Ok(count)
    }

    /// Fetch all photo information (joined with sidecars) from the library.
    fn fetch_photo_info(&mut self) -> Result<Vec<SourcePhotoInfo>, PhotosortError> {
        let conn = self.db.connection();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.filename, p.relpath, p.filetype, p.created_at, p.hash,
                    s.filename, s.relpath, s.filetype, s.modified_at, s.hash
             FROM photos p
             LEFT JOIN sidecars s ON p.id = s.photo_id
             ORDER BY p.id",
        )?;

        let rows = stmt.query_map([], |row| {
            let created_at_str: String = row.get(4)?;
            let created_at = get_db_date_object(&created_at_str).unwrap_or_else(|e| {
                log::error!(
                    "Failed to parse 'created_at' date '{}': {}",
                    created_at_str,
                    e
                );

                OffsetDateTime::now_local().unwrap_or_else(|_| {
                    log::error!("Failed to get local time, using UTC instead.");
                    OffsetDateTime::now_utc()
                })
            });

            let photo = (
                row.get::<_, i64>(0)?, // id
                SourcePhotoInfo {
                    original_path: self
                        .root
                        .join(row.get::<_, String>(2)?)
                        .join(row.get::<_, String>(1)?),
                    filename: row.get(1)?,
                    filetype: row.get(3)?,
                    created_at,
                    hash: row.get(5)?,
                    sidecars: Vec::new(),
                },
            );

            let sidecar_filename: String = row.get(6)?;
            let sidecar = if !sidecar_filename.is_empty() {
                let modified_at_str: String = row.get(9)?;
                let modified_at = get_db_date_object(&modified_at_str).unwrap_or_else(|e| {
                    log::error!(
                        "Failed to parse 'modified_at' date '{}': {}",
                        modified_at_str,
                        e
                    );
                    OffsetDateTime::now_local().unwrap_or_else(|_| {
                        log::error!("Failed to get local time, using UTC instead.");
                        OffsetDateTime::now_utc()
                    })
                });
                Some(SourceSidecarInfo {
                    original_path: self
                        .root
                        .join(row.get::<_, String>(7)?)
                        .join(&sidecar_filename),
                    filename: sidecar_filename,
                    filetype: row.get(8)?,
                    modified_at,
                    hash: row.get(10)?,
                })
            } else {
                None
            };

            Ok((photo, sidecar))
        })?;

        let mut photos_map: HashMap<i64, SourcePhotoInfo> = HashMap::new();
        for row_result in rows {
            let ((id, photo_info), sidecar_info) = row_result?;
            let entry = photos_map.entry(id).or_insert(photo_info);
            if let Some(sidecar) = sidecar_info {
                entry.sidecars.push(sidecar);
            }
        }

        Ok(photos_map.into_values().collect())
    }
}

pub fn get_db_date_object(date_string: &str) -> Result<OffsetDateTime, PhotosortError> {
    // Parse a date string in the format "YYYY/MM-DD HH:MM:SS.sss+00:00"
    OffsetDateTime::parse(date_string, DB_DATE_FORMAT)
        .map_err(|e| PhotosortError::InvalidDateFormat(e.to_string()))
}

pub fn get_db_date_string(date: &OffsetDateTime) -> Result<String, PhotosortError> {
    // Format the date to "YYYY/MM-DD HH:MM:SS.sss+00:00"
    date.format(DB_DATE_FORMAT)
        .map_err(|e| PhotosortError::InvalidDateFormat(e.to_string()))
}

pub fn get_local_tz() -> time::UtcOffset {
    OffsetDateTime::now_local()
        .map(|dt| dt.offset())
        .unwrap_or_else(|_| {
            log::error!("Failed to get local time, using UTC instead.");
            time::UtcOffset::UTC
        })
}

/// Get the current local time, falling back to UTC if local time cannot be determined.
pub fn get_current_time() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| {
        log::error!("Failed to get local time, using UTC instead.");
        OffsetDateTime::now_utc()
    })
}

pub fn sync(source_lib: &mut Library, target_lib: &mut Library) -> Result<(), PhotosortError> {
    log::info!("Sync Phase 1: Updating source and target libraries...");
    source_lib.update()?;
    target_lib.update()?;
    log::info!("Sync Phase 1: Complete.");

    log::info!("Sync Phase 2: Getting all photo info from source library...");
    let source_photos = source_lib.fetch_photo_info()?;
    log::info!(
        "Sync Phase 2: Found {} photos in source library.",
        source_photos.len()
    );

    log::info!("Sync Phase 3: Syncing to target library database...");
    let target_conn = target_lib.db.connection();
    let tx = target_conn.transaction()?;
    let mut files_to_copy: Vec<FileToCopy> = Vec::new();

    let bar_style = ProgressStyle::default_bar()
        .template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap();
    let sync_bar = ProgressBar::new(source_photos.len() as u64).with_style(bar_style.clone());
    sync_bar.set_message("Syncing database");

    for source_photo in source_photos {
        let rel_path_str = source_photo.created_at.format(FP_DATE_FORMAT).unwrap();
        let existing_target_photo: Result<(i64, String), _> = tx.query_row(
            "SELECT id, filename FROM photos WHERE hash = ?1",
            params![source_photo.hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        match existing_target_photo {
            Ok((target_photo_id, target_filename)) => {
                if should_migrate_filename(&target_filename, &source_photo.filename) {
                    log::debug!(
                        "DB: Updating existing photo ID {} from '{}' to '{}'",
                        target_photo_id,
                        target_filename,
                        source_photo.filename
                    );
                    tx.execute(
                        "UPDATE photos SET filename = ?1, relpath = ?2 WHERE id = ?3",
                        params![&source_photo.filename, &rel_path_str, target_photo_id],
                    )?;
                    tx.execute(
                        "DELETE FROM sidecars WHERE photo_id = ?1",
                        params![target_photo_id],
                    )?;

                    files_to_copy.push(FileToCopy {
                        original_path: source_photo.original_path.clone(),
                        destination_path: target_lib
                            .root
                            .join(&rel_path_str)
                            .join(&source_photo.filename),
                    });

                    for sidecar in &source_photo.sidecars {
                        tx.execute("INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![target_photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype, &sidecar.modified_at.format(DB_DATE_FORMAT).unwrap(), &sidecar.hash],
                        )?;
                        files_to_copy.push(FileToCopy {
                            original_path: sidecar.original_path.clone(),
                            destination_path: target_lib
                                .root
                                .join(&rel_path_str)
                                .join(&sidecar.filename),
                        });
                    }
                } else {
                    for sidecar in &source_photo.sidecars {
                        let existing_sidecar: Result<String, _> = tx.query_row(
                            "SELECT hash FROM sidecars WHERE photo_id = ?1 AND filename = ?2",
                            params![target_photo_id, &sidecar.filename],
                            |row| row.get(0),
                        );
                        match existing_sidecar {
                            Ok(db_hash) if db_hash != sidecar.hash => {
                                tx.execute("UPDATE sidecars SET hash = ?1, modified_at = ?2 WHERE photo_id = ?3 AND filename = ?4",
                                    params![&sidecar.hash, &sidecar.modified_at.format(DB_DATE_FORMAT).unwrap(), target_photo_id, &sidecar.filename],
                                )?;
                                files_to_copy.push(FileToCopy {
                                    original_path: sidecar.original_path.clone(),
                                    destination_path: target_lib
                                        .root
                                        .join(&rel_path_str)
                                        .join(&sidecar.filename),
                                });
                            }
                            Err(rusqlite::Error::QueryReturnedNoRows) => {
                                tx.execute("INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                    params![target_photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype, &sidecar.modified_at.format(DB_DATE_FORMAT).unwrap(), &sidecar.hash],
                                )?;
                                files_to_copy.push(FileToCopy {
                                    original_path: sidecar.original_path.clone(),
                                    destination_path: target_lib
                                        .root
                                        .join(&rel_path_str)
                                        .join(&sidecar.filename),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                tx.execute(
                    "INSERT INTO photos (filename, relpath, filetype, created_at, hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![&source_photo.filename, &rel_path_str, &source_photo.filetype, &source_photo.created_at.format(DB_DATE_FORMAT).unwrap(), &source_photo.hash],
                )?;
                let photo_id = tx.last_insert_rowid();
                files_to_copy.push(FileToCopy {
                    original_path: source_photo.original_path.clone(),
                    destination_path: target_lib
                        .root
                        .join(&rel_path_str)
                        .join(&source_photo.filename),
                });

                for sidecar in &source_photo.sidecars {
                    tx.execute(
                        "INSERT INTO sidecars (photo_id, filename, relpath, filetype, modified_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![photo_id, &sidecar.filename, &rel_path_str, &sidecar.filetype, &sidecar.modified_at.format(DB_DATE_FORMAT).unwrap(), &sidecar.hash],
                    )?;
                    files_to_copy.push(FileToCopy {
                        original_path: sidecar.original_path.clone(),
                        destination_path: target_lib
                            .root
                            .join(&rel_path_str)
                            .join(&sidecar.filename),
                    });
                }
            }
            Err(e) => return Err(e.into()),
        }
        sync_bar.inc(1);
    }

    tx.commit()?;
    sync_bar.finish_with_message("Database sync complete");
    log::info!("Sync Phase 3: Complete.");

    log::info!("Sync Phase 4: Copying new/updated files to target library...");
    let mut unique_copy_map: HashMap<PathBuf, FileToCopy> = HashMap::new();
    for f in files_to_copy {
        unique_copy_map.insert(f.destination_path.clone(), f);
    }
    let unique_files_to_copy: Vec<FileToCopy> = unique_copy_map.into_values().collect();
    let copy_bar = ProgressBar::new(unique_files_to_copy.len() as u64).with_style(bar_style);
    copy_bar.set_message("Copying files");

    unique_files_to_copy.par_iter().for_each(|f| {
        if let Some(parent) = f.destination_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                log::error!("Failed to create directory {}: {}", parent.display(), e);
                return;
            }
        }
        if let Err(e) = fs::copy(&f.original_path, &f.destination_path) {
            log::warn!(
                "Failed to copy file from {} to {}: {}",
                f.original_path.display(),
                f.destination_path.display(),
                e
            );
        }
        copy_bar.inc(1);
    });

    copy_bar.finish_with_message("File copying complete");
    log::info!(
        "Sync completed successfully. {} files copied to target library.",
        unique_files_to_copy.len()
    );

    Ok(())
}

/// Determine if a new filename is better than an old one for migration purposes.
/// Returns true if the new filename should be used instead of the old one.
fn should_migrate_filename(old_filename: &str, new_filename: &str) -> bool {
    if new_filename.is_empty() {
        return false;
    }
    if old_filename.is_empty() {
        return true;
    }

    if old_filename == new_filename {
        // If the filenames are exactly the same, no need to migrate.
        return false;
    }

    // if the new filename is a substring of the old filename, it is better
    if old_filename.contains(new_filename) {
        return true;
    }

    // if the new filename is similar but shorter than the old filename, it is better
    let ratio = partial_ratio(
        new_filename.replace("-", " ").as_str(),
        old_filename.replace("-", " ").as_str(),
        None,
    );
    println!(
        "Comparing '{}' to '{}', ratio: {}",
        new_filename, old_filename, ratio
    );
    if ratio >= 0.7 {
        return new_filename.len() < old_filename.len();
    }

    // Otherwise, the filenames are divergent so migrating to the new filename is unnecessary.
    false
}

#[derive(Clone, Debug)]
struct FileToCopy {
    original_path: PathBuf,
    destination_path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs;

    #[test]
    fn test_is_filename_better() {
        assert!(!should_migrate_filename("photo.jpg", "photo.jpg")); // same filename, no migration
        assert!(!should_migrate_filename("photo.jpg", "photo-2.jpg")); // do want to rename copies
        assert!(should_migrate_filename("photo-2.jpg", "photo.jpg")); // don't want to rename originals
        assert!(should_migrate_filename("", "photo.jpg")); // empty new filename is not better
        assert!(!should_migrate_filename("photo.jpg", "")); // something is better than nothing
        assert!(!should_migrate_filename("", "")); // wtf, better not trigger any migrations
        assert!(!should_migrate_filename("photo-2023.jpg", "photo-2022.jpg")); // very similar, don't bother
        assert!(!should_migrate_filename("photo-2022.jpg", "photo-2023.jpg")); // very similar, don't bother
    }

    #[test]
    fn test_dt_format() {
        let date = "2024:05:21 12:46:20.865+09:00";
        let parsed_date = get_db_date_object(date);
        assert!(
            parsed_date.is_ok(),
            "Failed to parse date: {:?}",
            parsed_date.err()
        );
        let date_str = get_db_date_string(&parsed_date.unwrap());
        assert!(date_str.is_ok());
        assert_eq!(date_str.unwrap(), date);
    }

    #[test]
    fn test_library_create_and_open() {
        let temp_dir = assert_fs::TempDir::new().unwrap();
        let library_path = temp_dir.path().join("test_library");

        // Create a new library
        let library = Library::create(&library_path);
        assert!(library.is_ok());
        assert!(library_path.exists());
        assert!(library_path.join(DB_FILE_NAME).exists());

        // Try to create the same library again
        let duplicate_library = Library::create(&library_path);
        assert!(duplicate_library.is_err());

        // Open the existing library
        let opened_library = Library::open(&library_path);
        assert!(opened_library.is_ok());
    }
}
