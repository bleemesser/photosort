use crate::photosort_core::error::{PhotosortError, Result};
use crate::photosort_core::import::{Library, DB_DATE_FORMAT};
use rusqlite::params;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use time::OffsetDateTime;

/// Result of a push operation.
#[derive(Debug)]
pub struct PushResult {
    pub files_pushed: usize,
    pub sidecars_pushed: usize,
    pub bytes_transferred: u64,
    pub conflicts_resolved: usize,
    pub skipped: usize,
}

/// A detected conflict between local and remote sidecars.
#[derive(Debug)]
pub struct SidecarConflict {
    pub media_hash: String,
    pub media_filename: String,
    pub sidecar_filename: String,
    pub local_modified: String,
    pub local_size: i64,
    pub remote_modified: String,
    pub remote_size: i64,
    pub local_path: PathBuf,
    pub remote_path: PathBuf,
}

/// Resolution choice for a conflict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConflictResolution {
    UseLocal,
    UseRemote,
    Skip,
}

/// Remote library info.
#[derive(Debug)]
pub struct RemoteLibrary {
    /// The path to the remote library (either mounted or ssh:host:path)
    pub path: String,
    /// Whether this is an SSH remote
    pub is_ssh: bool,
    /// The actual filesystem path (for mounted paths) or temp path for DB
    pub local_path: Option<PathBuf>,
}

impl RemoteLibrary {
    /// Parse a remote path string into a RemoteLibrary.
    /// Supports:
    /// - Local/mounted paths: /Volumes/NAS/photos
    /// - SSH paths: user@host:/path/to/library
    pub fn parse(remote_str: &str) -> Result<Self> {
        if remote_str.contains(':') && !remote_str.starts_with('/') {
            // SSH remote: user@host:/path
            Ok(RemoteLibrary {
                path: remote_str.to_string(),
                is_ssh: true,
                local_path: None,
            })
        } else {
            // Local/mounted path
            let path = PathBuf::from(remote_str);
            if !path.exists() {
                return Err(PhotosortError::Library(format!(
                    "Remote path does not exist: {}",
                    remote_str
                )));
            }
            Ok(RemoteLibrary {
                path: remote_str.to_string(),
                is_ssh: false,
                local_path: Some(path),
            })
        }
    }

    /// Check if this is a valid photosort library.
    pub fn is_valid_library(&self) -> Result<bool> {
        if self.is_ssh {
            // For SSH, check if library.db exists on remote
            let output = Command::new("ssh")
                .arg(self.get_ssh_host())
                .arg(format!("test -f {}/library.db && echo yes", self.get_ssh_path()))
                .output()?;
            Ok(output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == "yes")
        } else {
            // For local path, check directly
            let db_path = self.local_path.as_ref().unwrap().join("library.db");
            Ok(db_path.exists())
        }
    }

    /// Get SSH host from path (e.g., "user@host" from "user@host:/path")
    fn get_ssh_host(&self) -> &str {
        self.path.split(':').next().unwrap_or("")
    }

    /// Get SSH path from path (e.g., "/path" from "user@host:/path")
    fn get_ssh_path(&self) -> &str {
        self.path.split(':').nth(1).unwrap_or("")
    }

    /// Get the database for comparison.
    /// For SSH remotes, this copies the DB locally first.
    pub fn get_database_path(&self) -> Result<PathBuf> {
        if self.is_ssh {
            // Copy remote DB to temp location
            let temp_dir = std::env::temp_dir();
            let temp_db = temp_dir.join("photosort_remote_library.db");

            let remote_db = format!("{}:{}/library.db", self.get_ssh_host(), self.get_ssh_path());
            let status = Command::new("scp")
                .arg(&remote_db)
                .arg(&temp_db)
                .status()?;

            if !status.success() {
                return Err(PhotosortError::Remote(
                    "Failed to copy remote database".to_string(),
                ));
            }

            Ok(temp_db)
        } else {
            Ok(self.local_path.as_ref().unwrap().join("library.db"))
        }
    }
}

/// Media info from a library database.
#[derive(Debug)]
struct MediaInfo {
    hash: String,
    filename: String,
    relpath: String,
}

/// Sidecar info from a library database.
#[derive(Debug)]
struct SidecarInfo {
    filename: String,
    modified_at: String,
    file_size: i64,
}

/// Push local library to remote library.
pub fn push(
    lib: &mut Library,
    remote_str: &str,
    dry_run: bool,
) -> Result<PushResult> {
    let remote = RemoteLibrary::parse(remote_str)?;

    // Validate remote is a photosort library
    if !remote.is_valid_library()? {
        return Err(PhotosortError::Library(format!(
            "Remote path is not a valid photosort library: {}",
            remote_str
        )));
    }

    println!(
        "{}Pushing {} -> {}",
        if dry_run { "[DRY RUN] " } else { "" },
        lib.root().display(),
        remote_str
    );

    // Get remote database for comparison
    let remote_db_path = remote.get_database_path()?;
    let remote_conn = rusqlite::Connection::open(&remote_db_path)?;

    // Build maps of what exists in each library
    let local_media = get_media_map(lib.database().connection_ref())?;
    let remote_media = get_media_map(&remote_conn)?;

    let local_sidecars = get_sidecar_map(lib.database().connection_ref())?;
    let remote_sidecars = get_sidecar_map(&remote_conn)?;

    // Categorize files
    let mut new_media: Vec<&MediaInfo> = Vec::new();
    let mut sidecar_updates: Vec<(&str, &SidecarInfo)> = Vec::new();
    let mut conflicts: Vec<SidecarConflict> = Vec::new();

    for (hash, local_info) in &local_media {
        if !remote_media.contains_key(hash) {
            // New media - doesn't exist on remote
            new_media.push(local_info);
        } else {
            // Media exists on both - check sidecars
            if let Some(local_scs) = local_sidecars.get(hash) {
                let remote_scs = remote_sidecars.get(hash);

                for (sc_name, local_sc) in local_scs {
                    if let Some(remote_scs_map) = remote_scs {
                        if let Some(remote_sc) = remote_scs_map.get(sc_name) {
                            // Sidecar exists on both - compare timestamps
                            if local_sc.modified_at > remote_sc.modified_at {
                                // Local is newer - push sidecar
                                sidecar_updates.push((hash.as_str(), local_sc));
                            } else if local_sc.modified_at < remote_sc.modified_at {
                                // Remote is newer - CONFLICT
                                let remote_info = remote_media.get(hash).unwrap();
                                conflicts.push(SidecarConflict {
                                    media_hash: hash.clone(),
                                    media_filename: local_info.filename.clone(),
                                    sidecar_filename: sc_name.clone(),
                                    local_modified: local_sc.modified_at.clone(),
                                    local_size: local_sc.file_size,
                                    remote_modified: remote_sc.modified_at.clone(),
                                    remote_size: remote_sc.file_size,
                                    local_path: lib
                                        .root()
                                        .join(&local_info.relpath)
                                        .join(sc_name),
                                    remote_path: if remote.is_ssh {
                                        PathBuf::from(format!(
                                            "{}/{}",
                                            remote_info.relpath, sc_name
                                        ))
                                    } else {
                                        remote
                                            .local_path
                                            .as_ref()
                                            .unwrap()
                                            .join(&remote_info.relpath)
                                            .join(sc_name)
                                    },
                                });
                            }
                            // Same timestamp - already in sync, skip
                        } else {
                            // Sidecar doesn't exist on remote - push it
                            sidecar_updates.push((hash.as_str(), local_sc));
                        }
                    } else {
                        // No sidecars on remote for this media - push it
                        sidecar_updates.push((hash.as_str(), local_sc));
                    }
                }
            }
        }
    }

    // Report what we found
    println!("\n─────────────────────────────────");
    println!("Push Summary:");
    println!("  New media:        {}", new_media.len());
    println!("  Sidecar updates:  {}", sidecar_updates.len());
    println!("  Conflicts:        {}", conflicts.len());
    println!("─────────────────────────────────\n");

    if new_media.is_empty() && sidecar_updates.is_empty() && conflicts.is_empty() {
        println!("Everything is in sync. Nothing to push.");
        return Ok(PushResult {
            files_pushed: 0,
            sidecars_pushed: 0,
            bytes_transferred: 0,
            conflicts_resolved: 0,
            skipped: 0,
        });
    }

    // Handle conflicts interactively
    let mut conflict_resolutions: HashMap<String, ConflictResolution> = HashMap::new();
    let mut all_local = false;
    let mut all_skip = false;

    if !conflicts.is_empty() && !dry_run {
        println!("Conflicts detected:\n");

        for conflict in &conflicts {
            if all_local {
                conflict_resolutions
                    .insert(conflict.sidecar_filename.clone(), ConflictResolution::UseLocal);
                continue;
            }
            if all_skip {
                conflict_resolutions
                    .insert(conflict.sidecar_filename.clone(), ConflictResolution::Skip);
                continue;
            }

            println!("Conflict for {} ({}):", conflict.media_filename, conflict.sidecar_filename);
            println!(
                "  Local:  modified {} ({} bytes)",
                conflict.local_modified, conflict.local_size
            );
            println!(
                "  Remote: modified {} ({} bytes)",
                conflict.remote_modified, conflict.remote_size
            );
            println!("\n  [L]ocal wins  [R]emote wins  [S]kip  [A]ll-local  [N]one (skip all)");
            print!("  Choice: ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim().to_uppercase().as_str() {
                "L" => {
                    conflict_resolutions
                        .insert(conflict.sidecar_filename.clone(), ConflictResolution::UseLocal);
                }
                "R" => {
                    conflict_resolutions
                        .insert(conflict.sidecar_filename.clone(), ConflictResolution::UseRemote);
                }
                "A" => {
                    all_local = true;
                    conflict_resolutions
                        .insert(conflict.sidecar_filename.clone(), ConflictResolution::UseLocal);
                }
                "N" => {
                    all_skip = true;
                    conflict_resolutions
                        .insert(conflict.sidecar_filename.clone(), ConflictResolution::Skip);
                }
                _ => {
                    conflict_resolutions
                        .insert(conflict.sidecar_filename.clone(), ConflictResolution::Skip);
                }
            }
            println!();
        }
    }

    if dry_run {
        // In dry run mode, just report what would happen
        if !new_media.is_empty() {
            println!("Would push {} new media files:", new_media.len());
            for media in new_media.iter().take(10) {
                println!("  - {}/{}", media.relpath, media.filename);
            }
            if new_media.len() > 10 {
                println!("  ... and {} more", new_media.len() - 10);
            }
        }

        if !sidecar_updates.is_empty() {
            println!("\nWould update {} sidecars:", sidecar_updates.len());
            for (_, sc) in sidecar_updates.iter().take(10) {
                println!("  - {}", sc.filename);
            }
            if sidecar_updates.len() > 10 {
                println!("  ... and {} more", sidecar_updates.len() - 10);
            }
        }

        if !conflicts.is_empty() {
            println!("\n{} conflicts would need resolution", conflicts.len());
        }

        return Ok(PushResult {
            files_pushed: 0,
            sidecars_pushed: 0,
            bytes_transferred: 0,
            conflicts_resolved: 0,
            skipped: 0,
        });
    }

    // Actually push files
    let mut files_pushed = 0;
    let mut sidecars_pushed = 0;
    let mut bytes_transferred = 0u64;
    let mut conflicts_resolved = 0;
    let mut skipped = 0;

    // Push new media files
    for media in &new_media {
        let local_path = lib.root().join(&media.relpath).join(&media.filename);
        let result = push_file(&local_path, &remote, &media.relpath)?;
        if result {
            files_pushed += 1;
            if let Ok(metadata) = std::fs::metadata(&local_path) {
                bytes_transferred += metadata.len();
            }
        }

        // Also push any sidecars for this media
        if let Some(sidecars) = local_sidecars.get(&media.hash) {
            for (sc_name, _) in sidecars {
                let sc_path = lib.root().join(&media.relpath).join(sc_name);
                if sc_path.exists() {
                    let result = push_file(&sc_path, &remote, &media.relpath)?;
                    if result {
                        sidecars_pushed += 1;
                        if let Ok(metadata) = std::fs::metadata(&sc_path) {
                            bytes_transferred += metadata.len();
                        }
                    }
                }
            }
        }
    }

    // Push sidecar updates
    for (hash, sc) in &sidecar_updates {
        if let Some(media) = local_media.get(*hash) {
            let sc_path = lib.root().join(&media.relpath).join(&sc.filename);
            if sc_path.exists() {
                let result = push_file(&sc_path, &remote, &media.relpath)?;
                if result {
                    sidecars_pushed += 1;
                    if let Ok(metadata) = std::fs::metadata(&sc_path) {
                        bytes_transferred += metadata.len();
                    }
                }
            }
        }
    }

    // Handle resolved conflicts
    for conflict in &conflicts {
        if let Some(resolution) = conflict_resolutions.get(&conflict.sidecar_filename) {
            match resolution {
                ConflictResolution::UseLocal => {
                    if let Some(media) = local_media.get(&conflict.media_hash) {
                        let result = push_file(&conflict.local_path, &remote, &media.relpath)?;
                        if result {
                            conflicts_resolved += 1;
                            if let Ok(metadata) = std::fs::metadata(&conflict.local_path) {
                                bytes_transferred += metadata.len();
                            }
                        }
                    }
                }
                ConflictResolution::UseRemote => {
                    // Remote wins - nothing to push
                    skipped += 1;
                }
                ConflictResolution::Skip => {
                    skipped += 1;
                }
            }
        }
    }

    // Record push in history
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let now_str = now.format(DB_DATE_FORMAT).unwrap();

    let conn = lib.database_mut().connection();
    conn.execute(
        "INSERT INTO backup_history (target_path, started_at, completed_at, files_copied, bytes_copied, status)
         VALUES (?1, ?2, ?2, ?3, ?4, 'completed')",
        params![
            remote_str,
            now_str,
            (files_pushed + sidecars_pushed) as i64,
            bytes_transferred as i64
        ],
    )?;

    Ok(PushResult {
        files_pushed,
        sidecars_pushed,
        bytes_transferred,
        conflicts_resolved,
        skipped,
    })
}

/// Get a map of hash -> MediaInfo from a database connection.
fn get_media_map(conn: &rusqlite::Connection) -> Result<HashMap<String, MediaInfo>> {
    let mut map = HashMap::new();
    let mut stmt = conn.prepare("SELECT hash, filename, relpath FROM media")?;

    let rows = stmt.query_map([], |row| {
        Ok(MediaInfo {
            hash: row.get(0)?,
            filename: row.get(1)?,
            relpath: row.get(2)?,
        })
    })?;

    for row in rows {
        let info = row?;
        map.insert(info.hash.clone(), info);
    }

    Ok(map)
}

/// Get a map of media_hash -> (sidecar_filename -> SidecarInfo) from a database connection.
fn get_sidecar_map(
    conn: &rusqlite::Connection,
) -> Result<HashMap<String, HashMap<String, SidecarInfo>>> {
    let mut map: HashMap<String, HashMap<String, SidecarInfo>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT m.hash, s.filename, s.modified_at, s.file_size
         FROM sidecars s
         JOIN media m ON s.media_id = m.id",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    for row in rows {
        let (hash, filename, modified_at, file_size) = row?;
        map.entry(hash).or_default().insert(
            filename.clone(),
            SidecarInfo {
                filename,
                modified_at,
                file_size,
            },
        );
    }

    Ok(map)
}

/// Push a single file to the remote.
fn push_file(local_path: &Path, remote: &RemoteLibrary, relpath: &str) -> Result<bool> {
    if !local_path.exists() {
        return Ok(false);
    }

    if remote.is_ssh {
        // Use rsync for SSH
        let remote_dir = format!(
            "{}:{}/{}",
            remote.path.split(':').next().unwrap_or(""),
            remote.path.split(':').nth(1).unwrap_or(""),
            relpath
        );

        // Ensure remote directory exists
        let ssh_host = remote.path.split(':').next().unwrap_or("");
        let ssh_path = remote.path.split(':').nth(1).unwrap_or("");
        Command::new("ssh")
            .arg(ssh_host)
            .arg(format!("mkdir -p {}/{}", ssh_path, relpath))
            .status()?;

        let status = Command::new("rsync")
            .arg("-av")
            .arg(local_path)
            .arg(&remote_dir)
            .status()?;

        Ok(status.success())
    } else {
        // Direct file copy for mounted paths
        let remote_path = remote.local_path.as_ref().unwrap().join(relpath);
        std::fs::create_dir_all(&remote_path)?;

        let dest = remote_path.join(local_path.file_name().unwrap());
        std::fs::copy(local_path, &dest)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_local_path() {
        // This test just validates the parsing logic
        let result = RemoteLibrary::parse("/some/nonexistent/path");
        assert!(result.is_err()); // Should fail because path doesn't exist
    }

    #[test]
    fn test_parse_ssh_path() {
        let result = RemoteLibrary::parse("user@host:/path/to/library").unwrap();
        assert!(result.is_ssh);
        assert_eq!(result.path, "user@host:/path/to/library");
    }
}
