use crate::photosort_core::PhotosortError;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

impl Database {
    /// Connect to the database at the specified path. Run migrations if necessary.
    pub fn new(path: &Path) -> Result<Self, PhotosortError> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?; // Use WAL mode for better concurrency
        conn.pragma_update(None, "foreign_keys", "ON")?; // Enable foreign key constraints

        let migrations = Migrations::new(vec![M::up(
            r#"
            CREATE TABLE IF NOT EXISTS photos (
                id INTEGER PRIMARY KEY,
                filename TEXT NOT NULL,
                relpath TEXT NOT NULL,
                filetype TEXT,
                created_at TEXT NOT NULL,
                hash TEXT UNIQUE NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sidecars (
                id INTEGER PRIMARY KEY,
                photo_id INTEGER NOT NULL,
                filename TEXT NOT NULL,
                relpath TEXT NOT NULL,
                filetype TEXT,
                modified_at TEXT NOT NULL,
                hash TEXT NOT NULL,
                FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE,
                UNIQUE (photo_id, filename)
            );
            "#,
        )]);

        migrations.to_latest(&mut conn)?;

        Ok(Database { conn })
    }

    pub fn connection(&mut self) -> &mut Connection {
        &mut self.conn
    }
}
