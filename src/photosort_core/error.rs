use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PhotosortError {
    // Database errors
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Database migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),

    // I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to copy files: {0} files failed")]
    CopyFailed(CopyFailures),

    // Filesystem errors
    #[error("Directory walker error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("Not a directory: {0}")]
    NotADirectory(PathBuf),

    // Library errors
    #[error("Library already exists at {0}")]
    LibraryExists(PathBuf),

    #[error("Library not found at {0}")]
    LibraryNotFound(PathBuf),

    #[error("Invalid library: missing database at {0}")]
    InvalidLibrary(PathBuf),

    // Metadata errors
    #[error("Exiftool error: {0}")]
    Exiftool(String),

    #[error("Date parsing error: {0}")]
    InvalidDateFormat(String),

    #[error("Failed to extract metadata from {path}: {reason}")]
    MetadataExtraction { path: PathBuf, reason: String },

    // User interaction
    #[error("Operation cancelled by user")]
    Cancelled,

    #[error("Conflict detected: {0}")]
    Conflict(String),

    // Remote/backup errors
    #[error("Remote connection failed: {0}")]
    RemoteConnection(String),

    #[error("Remote error: {0}")]
    Remote(String),

    #[error("Library error: {0}")]
    Library(String),

    #[error("rsync error: {0}")]
    Rsync(String),

    // Generic errors
    #[error("Argument error: {0}")]
    Argument(String),

    #[error("{0}")]
    Other(String),
}

/// Details about files that failed to copy.
#[derive(Debug)]
pub struct CopyFailures {
    pub failures: Vec<CopyFailure>,
}

#[derive(Debug)]
pub struct CopyFailure {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub error: std::io::Error,
}

impl std::fmt::Display for CopyFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for failure in &self.failures {
            writeln!(
                f,
                "  {} -> {}: {}",
                failure.source.display(),
                failure.destination.display(),
                failure.error
            )?;
        }
        Ok(())
    }
}

impl CopyFailures {
    pub fn new() -> Self {
        Self { failures: Vec::new() }
    }

    pub fn add(&mut self, source: PathBuf, destination: PathBuf, error: std::io::Error) {
        self.failures.push(CopyFailure {
            source,
            destination,
            error,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn len(&self) -> usize {
        self.failures.len()
    }
}

impl Default for CopyFailures {
    fn default() -> Self {
        Self::new()
    }
}

/// Result type for photosort operations.
pub type Result<T> = std::result::Result<T, PhotosortError>;
