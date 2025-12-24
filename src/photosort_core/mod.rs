// Core modules
pub mod cli;
pub mod database;
pub mod error;
pub mod media;
pub mod sidecar;

// Feature modules
pub mod backup;
pub mod exif;
pub mod import;
pub mod push;
pub mod scan;
pub mod search;

// Re-exports for convenience
pub use cli::{Cli, Commands, MediaTypeFilter, OutputFormat};
pub use database::Database;
pub use error::{PhotosortError, Result};
pub use media::{ExifMetadata, Media, MediaType};
pub use sidecar::Sidecar;
