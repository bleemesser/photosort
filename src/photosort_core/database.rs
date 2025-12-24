use crate::photosort_core::error::Result;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;

const SCHEMA_VERSION: i32 = 2;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Connect to the database at the specified path. Run migrations if necessary.
    pub fn new(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;

        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Enable foreign key constraints
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let migrations = Migrations::new(vec![
            // Migration 1: Initial schema (v2)
            M::up(
                r#"
                -- Main media table (photos and videos)
                CREATE TABLE IF NOT EXISTS media (
                    id INTEGER PRIMARY KEY,
                    hash TEXT UNIQUE NOT NULL,
                    filename TEXT NOT NULL,
                    relpath TEXT NOT NULL,
                    media_type TEXT NOT NULL CHECK (media_type IN ('image', 'video')),
                    filetype TEXT NOT NULL,
                    file_size INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    imported_at TEXT NOT NULL,

                    -- EXIF metadata (nullable)
                    camera_make TEXT,
                    camera_model TEXT,
                    lens TEXT,
                    focal_length TEXT,
                    aperture TEXT,
                    shutter_speed TEXT,
                    iso INTEGER,
                    gps_lat REAL,
                    gps_lon REAL
                );

                -- Sidecars linked to media
                CREATE TABLE IF NOT EXISTS sidecars (
                    id INTEGER PRIMARY KEY,
                    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
                    filename TEXT NOT NULL,
                    filetype TEXT NOT NULL,
                    file_size INTEGER NOT NULL,
                    hash TEXT NOT NULL,
                    modified_at TEXT NOT NULL,
                    UNIQUE(media_id, filename)
                );

                -- Backup tracking
                CREATE TABLE IF NOT EXISTS backup_history (
                    id INTEGER PRIMARY KEY,
                    target_path TEXT NOT NULL,
                    started_at TEXT NOT NULL,
                    completed_at TEXT,
                    files_copied INTEGER DEFAULT 0,
                    bytes_copied INTEGER DEFAULT 0,
                    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed'))
                );

                CREATE TABLE IF NOT EXISTS backup_state (
                    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
                    last_backup_id INTEGER REFERENCES backup_history(id),
                    backed_up_at TEXT
                );

                -- Indexes for search performance
                CREATE INDEX IF NOT EXISTS idx_media_type ON media(media_type);
                CREATE INDEX IF NOT EXISTS idx_media_created ON media(created_at);
                CREATE INDEX IF NOT EXISTS idx_media_filetype ON media(filetype);
                CREATE INDEX IF NOT EXISTS idx_media_camera ON media(camera_model);
                CREATE INDEX IF NOT EXISTS idx_media_file_size ON media(file_size);
                CREATE INDEX IF NOT EXISTS idx_media_hash ON media(hash);
                "#,
            ),
        ]);

        migrations.to_latest(&mut conn)?;

        Ok(Database { conn })
    }

    /// Get a mutable reference to the database connection.
    pub fn connection(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Get an immutable reference to the database connection.
    pub fn connection_ref(&self) -> &Connection {
        &self.conn
    }

    /// Get the current schema version.
    pub fn schema_version(&self) -> Result<i32> {
        let version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        Ok(version)
    }

    /// Get the count of media items in the library.
    pub fn media_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM media", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Get the count of images in the library.
    pub fn image_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE media_type = 'image'",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get the count of videos in the library.
    pub fn video_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE media_type = 'video'",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get the count of sidecars in the library.
    pub fn sidecar_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sidecars", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Get the total size of all media files in bytes.
    pub fn total_media_size(&self) -> Result<i64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM media",
            [],
            |row| row.get(0),
        )?;
        Ok(size)
    }

    /// Get the total size of all image files in bytes.
    pub fn total_image_size(&self) -> Result<i64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM media WHERE media_type = 'image'",
            [],
            |row| row.get(0),
        )?;
        Ok(size)
    }

    /// Get the total size of all video files in bytes.
    pub fn total_video_size(&self) -> Result<i64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM media WHERE media_type = 'video'",
            [],
            |row| row.get(0),
        )?;
        Ok(size)
    }

    /// Get the total size of all sidecar files in bytes.
    pub fn total_sidecar_size(&self) -> Result<i64> {
        let size: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(file_size), 0) FROM sidecars",
            [],
            |row| row.get(0),
        )?;
        Ok(size)
    }

    /// Check if a hash exists in the database.
    pub fn hash_exists(&self, hash: &str) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM media WHERE hash = ?1",
            [hash],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get media ID by hash.
    pub fn get_media_id_by_hash(&self, hash: &str) -> Result<Option<i64>> {
        let result = self.conn.query_row(
            "SELECT id FROM media WHERE hash = ?1",
            [hash],
            |row| row.get(0),
        );
        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;

    #[test]
    fn test_database_creation() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = Database::new(&db_path);
        assert!(db.is_ok());

        let db = db.unwrap();
        assert_eq!(db.media_count().unwrap(), 0);
        assert_eq!(db.sidecar_count().unwrap(), 0);
    }

    #[test]
    fn test_hash_exists() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let db = Database::new(&db_path).unwrap();
        assert!(!db.hash_exists("nonexistent").unwrap());
    }
}
