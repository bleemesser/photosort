use clap::{Parser, Subcommand, ValueEnum};
use simplelog::LevelFilter;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "A local filesystem-friendly photo and video library manager")]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable file logging to photosort.log
    #[arg(long = "log", global = true)]
    pub log: bool,

    /// Log level for file logging (debug, info, warn, error)
    #[arg(long, default_value_t = LevelFilter::Debug, global = true)]
    pub log_level: LevelFilter,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new photosort library
    Create {
        /// Path for the new library (will be created if it doesn't exist)
        #[arg(required = true)]
        library_dir: PathBuf,
    },

    /// Import photos and videos into a library
    Import {
        /// Directory containing media to import
        #[arg(required = true)]
        source_dir: PathBuf,

        /// Library to import into
        #[arg(required = true)]
        library_dir: PathBuf,

        /// Show what would be imported without making changes
        #[arg(long)]
        dry_run: bool,

        /// Move files instead of copying (deletes originals after successful import)
        #[arg(long)]
        r#move: bool,
    },

    /// Scan library for filesystem changes
    Scan {
        /// Library to scan
        #[arg(required = true)]
        library_dir: PathBuf,
    },

    /// Search for media matching filters
    Search {
        /// Library to search
        #[arg(required = true)]
        library_dir: PathBuf,

        /// Filter by media type
        #[arg(long, value_enum)]
        r#type: Option<MediaTypeFilter>,

        /// Filter by date (YYYY-MM-DD or YYYY-MM-DD..YYYY-MM-DD)
        #[arg(long)]
        date: Option<String>,

        /// Filter by file extension(s), comma-separated (e.g., "jpg,heic,png")
        #[arg(long)]
        ext: Option<String>,

        /// Only show media with sidecars
        #[arg(long)]
        has_sidecar: bool,

        /// Only show media without sidecars
        #[arg(long)]
        no_sidecar: bool,

        /// Filter by file size (e.g., ">10MB", "<1MB", "5MB..50MB")
        #[arg(long)]
        size: Option<String>,

        /// Filter by camera model (substring match)
        #[arg(long)]
        camera: Option<String>,

        /// Filter by lens (substring match)
        #[arg(long)]
        lens: Option<String>,

        /// Output format
        #[arg(long, value_enum, default_value_t = OutputFormat::Paths)]
        output: OutputFormat,
    },

    /// Show library statistics
    Stats {
        /// Library to show stats for
        #[arg(required = true)]
        library_dir: PathBuf,
    },

    /// Backup library to a directory.
    ///
    /// Creates an exact mirror of the source library using rsync --delete.
    /// Files deleted locally will also be deleted in the backup. The target
    /// can be an empty directory or a previous backup — either way it will
    /// be updated to exactly match the source.
    Backup {
        /// Source library
        #[arg(required = true)]
        library_dir: PathBuf,

        /// Backup destination directory
        #[arg(required = true)]
        target_dir: PathBuf,

        /// Show what would be backed up without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Push changes to a different library (one-way sync).
    ///
    /// Additive sync: copies new media and newer sidecars from the local
    /// library to the remote library. Files that exist only on the remote
    /// are preserved — nothing is deleted. When a sidecar has been modified
    /// on both sides, you will be prompted to resolve the conflict
    /// interactively (keep local, keep remote, or skip). The remote must
    /// already be an existing photosort library.
    Push {
        /// Local library (source of truth for new content)
        #[arg(required = true)]
        local_library: PathBuf,

        /// Remote library (e.g., user@nas:/path or /Volumes/NAS/photos)
        #[arg(required = true)]
        remote_library: String,

        /// Show what would be pushed without making changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Display library or file information
    Info {
        /// Library to display info for
        #[arg(required = true)]
        library_dir: PathBuf,

        /// Specific file to show detailed info for (optional)
        file_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, ValueEnum)]
pub enum MediaTypeFilter {
    Image,
    Video,
    All,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    /// One file path per line
    Paths,
    /// JSON output
    Json,
    /// Detailed table format
    Table,
}
