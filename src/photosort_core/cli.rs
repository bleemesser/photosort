use clap::{Parser, Subcommand};
use simplelog::LevelFilter;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub commands: Commands,

    /// Path to the log file. If set, logs will be written to this file.
    #[arg(long = "log")]
    pub log: Option<PathBuf>,

    /// Log level for the file logger (debug, info, warn, error).
    #[arg(long, default_value_t = LevelFilter::Debug)]
    pub log_level: LevelFilter,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new photosort library (and create a directory if it doesn't exist)
    Create {
        /// Name of the new library
        #[arg(required = true)]
        library_dir: PathBuf,
    },
    /// Import photos from a directory into a library
    Import {
        /// Directory containing photos to import
        #[arg(required = true)]
        photo_dir: PathBuf,

        /// Directory of the library to import into
        #[arg(required = true)]
        library_dir: PathBuf,
    },
    /// Update the library's photo DB to reflect untracked file changes
    Update {
        /// Directory of the library to update
        #[arg(required = true)]
        library_dir: PathBuf,
    },
    /// Sync photos from one library to another (uni-directional)
    Sync {
        /// Source library directory to sync from
        #[arg(required = true)]
        source_dir: PathBuf,

        /// Target library directory to sync to
        #[arg(required = true)]
        target_dir: PathBuf,
    },
    Info {
        /// Directory of the library to display info for
        #[arg(required = true)]
        library_dir: PathBuf,
    },
}
