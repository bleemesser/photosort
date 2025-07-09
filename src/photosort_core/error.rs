use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PhotosortError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Database migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Directory walker error: {0}")]
    Walkdir(#[from] walkdir::Error),

    #[error("Exiftool error: {0}")]
    Exiftool(String),

    #[error("Argument error: {0}")]
    Argument(String),

    #[error("Library already exists at {0}")]
    LibraryExists(PathBuf),

    #[error("Library not found at {0}")]
    LibraryNotFound(PathBuf),

    #[error("Date parsing error: {0}")]
    InvalidDateFormat(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl PartialEq for PhotosortError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PhotosortError::Database(_), PhotosortError::Database(_)) => true,
            (PhotosortError::Migration(_), PhotosortError::Migration(_)) => true,
            (PhotosortError::Io(_), PhotosortError::Io(_)) => true,
            (PhotosortError::Walkdir(_), PhotosortError::Walkdir(_)) => true,
            (PhotosortError::Exiftool(_), PhotosortError::Exiftool(_)) => true,
            (PhotosortError::Argument(_), PhotosortError::Argument(_)) => true,
            (PhotosortError::LibraryExists(_), PhotosortError::LibraryExists(_)) => true,
            (PhotosortError::LibraryNotFound(_), PhotosortError::LibraryNotFound(_)) => true,
            (PhotosortError::InvalidDateFormat(msg1), PhotosortError::InvalidDateFormat(msg2)) => {
                msg1 == msg2
            }
            (PhotosortError::Unknown(msg1), PhotosortError::Unknown(msg2)) => msg1 == msg2,
            _ => false,
        }
    }
}
