use anyhow::Result;
use clap::Parser;
use photosort::photosort_core::{Cli, Commands, Library, sync};
use simplelog::{CombinedLogger, Config, LevelFilter, SharedLogger, TermLogger, WriteLogger};
use std::fs::File;

fn main() -> Result<()> {
    // Parse CLI arguments first to determine logging setup
    let cli = Cli::parse();

    // Initialize loggers
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![];

    // TermLogger is always added with the 'Warn' level filter.
    loggers.push(TermLogger::new(
        LevelFilter::Warn,
        Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));

    // If the --log argument is passed with a filepath, add a WriteLogger.
    if let Some(log_path) = &cli.log {
        loggers.push(WriteLogger::new(
            cli.log_level, // The level is set by the --log-level argument.
            Config::default(),
            File::create(log_path)?,
        ));
    }

    CombinedLogger::init(loggers)?;

    match cli.commands {
        Commands::Create { library_dir } => {
            Library::create(&library_dir)?;
            println!("Successfully created library at {}", library_dir.display());
        }
        Commands::Import {
            photo_dir,
            library_dir,
        } => {
            let mut lib = Library::open(&library_dir)?;
            lib.import(&photo_dir, true)?;
            let count = lib.get_photo_count()?;
            println!("Import complete. Library now has {} photos.", count);
        }
        Commands::Update { library_dir } => {
            let mut lib = Library::open(&library_dir)?;
            lib.update()?;
            let count = lib.get_photo_count()?;
            println!("Update complete. Library now has {} photos.", count);
        }
        Commands::Sync {
            source_dir,
            target_dir,
        } => {
            let mut source_lib = Library::open(&source_dir)?;
            let mut target_lib = Library::open(&target_dir)?;
            sync(&mut source_lib, &mut target_lib)?;
            let count = target_lib.get_photo_count()?;
            println!("Sync complete. Target library now has {} photos.", count);
        }
        Commands::Info { library_dir } => {
            let mut lib = Library::open(&library_dir)?;
            let count = lib.get_photo_count()?;
            println!("Library at {} has {} photos.", library_dir.display(), count);
        }
    }

    Ok(())
}
