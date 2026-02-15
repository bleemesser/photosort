use anyhow::Result;
use clap::Parser;
use photosort::photosort_core::{Cli, Commands};
use photosort::photosort_core::import::Library;
use simplelog::{CombinedLogger, Config, LevelFilter, SharedLogger, TermLogger, WriteLogger};
use std::fs::File;

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize loggers
    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![TermLogger::new(
        LevelFilter::Warn,
        Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    )];

    if cli.log {
        loggers.push(WriteLogger::new(
            cli.log_level,
            Config::default(),
            File::create("photosort.log")?,
        ));
    }

    CombinedLogger::init(loggers)?;

    match cli.command {
        Commands::Create { library_dir } => {
            Library::create(&library_dir)?;
            println!("Created library at {}", library_dir.display());
            println!("  images/  - for photos");
            println!("  videos/  - for videos");
        }

        Commands::Import {
            source_dir,
            library_dir,
            dry_run,
        } => {
            let mut lib = Library::open(&library_dir)?;
            let stats = lib.import(&source_dir, dry_run)?;

            if !dry_run {
                println!("\nImport complete!");
                println!("  {} images imported", stats.images_imported);
                println!("  {} videos imported", stats.videos_imported);
                println!("  {} sidecars imported", stats.sidecars_imported);
                if stats.duplicates_skipped > 0 {
                    println!("  {} duplicates skipped", stats.duplicates_skipped);
                }
            }
        }

        Commands::Scan { library_dir } => {
            let mut lib = Library::open(&library_dir)?;
            let result = photosort::photosort_core::scan::scan_library(&lib)?;
            photosort::photosort_core::scan::handle_scan_results(&mut lib, &result)?;
        }

        Commands::Search {
            library_dir,
            r#type,
            date,
            ext,
            has_sidecar,
            no_sidecar,
            size,
            camera,
            lens,
            output,
        } => {
            use photosort::photosort_core::search::{search, format_results, SearchQuery};

            let lib = Library::open(&library_dir)?;

            // Build query
            let mut query = SearchQuery {
                media_type: r#type,
                camera,
                lens,
                ..Default::default()
            };

            // Parse date filter
            if let Some(date_str) = date {
                let (start, end) = SearchQuery::parse_date_filter(&date_str);
                query.date_start = start;
                query.date_end = end;
            }

            // Parse extension filter
            if let Some(ext_str) = ext {
                query.extensions = ext_str.split(',').map(|s| s.trim().to_string()).collect();
            }

            // Parse size filter
            if let Some(size_str) = size {
                let (min, max) = SearchQuery::parse_size_filter(&size_str);
                query.min_size = min;
                query.max_size = max;
            }

            // Has sidecar filter
            if has_sidecar {
                query.has_sidecar = Some(true);
            } else if no_sidecar {
                query.has_sidecar = Some(false);
            }

            let results = search(&lib, &query)?;
            println!("{}", format_results(&results, &output));
        }

        Commands::Stats { library_dir } => {
            let lib = Library::open(&library_dir)?;
            let db = lib.database();

            let image_count = db.image_count()?;
            let video_count = db.video_count()?;
            let sidecar_count = db.sidecar_count()?;
            let total_image_size = db.total_image_size()?;
            let total_video_size = db.total_video_size()?;
            let total_sidecar_size = db.total_sidecar_size()?;
            let total_size = total_image_size + total_video_size + total_sidecar_size;

            println!("Library: {}", library_dir.display());
            println!("─────────────────────────────────");
            println!("Images:    {:>8} ({:.1} GB)", image_count, total_image_size as f64 / 1_073_741_824.0);
            println!("Videos:    {:>8} ({:.1} GB)", video_count, total_video_size as f64 / 1_073_741_824.0);
            println!("Sidecars:  {:>8} ({:.1} MB)", sidecar_count, total_sidecar_size as f64 / 1_048_576.0);
            println!("─────────────────────────────────");
            println!("Total:     {:>8} files ({:.1} GB)", image_count + video_count + sidecar_count, total_size as f64 / 1_073_741_824.0);
        }

        Commands::Backup {
            library_dir,
            target_dir,
            dry_run,
        } => {
            use photosort::photosort_core::backup::{backup, files_changed_since_backup};

            let mut lib = Library::open(&library_dir)?;

            // Show files changed since last backup
            let changed = files_changed_since_backup(&lib)?;
            if changed > 0 {
                println!("{} files changed since last backup\n", changed);
            }

            let result = backup(&mut lib, &target_dir, dry_run)?;

            if !dry_run {
                println!("\nBackup complete!");
                println!("  {} files copied", result.files_copied);
                println!("  {} bytes transferred", result.bytes_transferred);
            }
        }

        Commands::Push {
            local_library,
            remote_library,
            dry_run,
        } => {
            use photosort::photosort_core::push::push;

            let mut lib = Library::open(&local_library)?;
            let result = push(&mut lib, &remote_library, dry_run)?;

            if !dry_run {
                println!("\nPush complete!");
                println!("  {} media files pushed", result.files_pushed);
                println!("  {} sidecars pushed", result.sidecars_pushed);
                println!("  {} bytes transferred", result.bytes_transferred);
                if result.conflicts_resolved > 0 {
                    println!("  {} conflicts resolved", result.conflicts_resolved);
                }
                if result.skipped > 0 {
                    println!("  {} skipped", result.skipped);
                }
            }
        }

        Commands::Info {
            library_dir,
            file_path,
        } => {
            let lib = Library::open(&library_dir)?;
            let db = lib.database();

            if let Some(_file) = file_path {
                println!("File info not yet implemented");
                // TODO: Show detailed file info
            } else {
                let image_count = db.image_count()?;
                let video_count = db.video_count()?;
                let sidecar_count = db.sidecar_count()?;

                println!("Library: {}", library_dir.display());
                println!("  {} images, {} videos, {} sidecars", image_count, video_count, sidecar_count);
            }
        }
    }

    Ok(())
}
