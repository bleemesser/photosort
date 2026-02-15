# Photosort

A command-line tool to sort, manage, and synchronize libraries of photos and their accompanying sidecar files.

Photosort processes photo metadata using concurrent exiftool processes, helping to speed up imports. It also performs file copies in parallel.

## Installation

### Prerequisites

1.  **Rust Toolchain**: If you don't have Rust installed, you can get it from [rustup.rs](https://rustup.rs/). This will install `rustc`, `cargo`, and `rustup`.

2.  **ExifTool**: Photosort relies on `exiftool` to read metadata from photo files. You must install it and ensure that it's available in your system's `PATH`. It can be installed via package managers like Homebrew or `apt`, or downloaded from the [official website](https://exiftool.org/).

### Steps

1.  **Clone the Repository**:
    ```bash
    git clone https://github.com/bleemesser/photosort.git
    cd photosort
    ``` 
2.  **Install the Binary**:
    Use `cargo install` to compile and install the `photosort` binary into Cargo's  binary directory (`~/.cargo/bin/`).
    ```bash
    cargo install --path .
    ```
    Once this is complete, you can run the program from anywhere by simply typing   `photosort`.   

## Usage

After installation, you can invoke the program directly. All commands support `--log` for file logging and `--log-level` to control verbosity.

* **Create a new library**:
    The directory will be created if it does not exist.
    ```bash
    photosort create <path/to/library_dir>
    ```

* **Import photos and videos into a library**:
    Media and their sidecars will be copied from the source directory into the library.
    ```bash
    photosort import <path/to/source_dir> <path/to/library_dir>
    ```
    Options: `--dry-run` to preview.

* **Scan a library for filesystem changes**:
    Detects files added, removed, or modified on disk and updates the database.
    ```bash
    photosort scan <path/to/library_dir>
    ```

* **Search for media**:
    Find media in a library using filters.
    ```bash
    photosort search <path/to/library_dir> [options]
    ```
    Filters: `--type` (image/video/all), `--date` (YYYY-MM-DD or range), `--ext` (e.g. jpg,heic), `--has-sidecar`, `--no-sidecar`, `--size` (e.g. ">10MB"), `--camera`, `--lens`.
    Output: `--output` (paths/json/table).

* **Show library statistics**:
    ```bash
    photosort stats <path/to/library_dir>
    ```

* **Backup a library**:
    Creates an exact mirror of the library using `rsync --delete`. Files deleted locally will also be deleted in the backup. The target can be an empty directory or a previous backup.
    ```bash
    photosort backup <path/to/library_dir> <path/to/backup_dir>
    ```
    Options: `--dry-run` to preview.

* **Push changes to a remote library**:
    Additive one-way sync — copies new media and newer sidecars to the remote library. Files that exist only on the remote are preserved (nothing is deleted). Sidecar conflicts are resolved interactively. The remote must already be an existing photosort library.
    ```bash
    photosort push <path/to/local_library> <remote>
    ```
    The remote can be a mounted path (e.g. `/Volumes/NAS/photos`) or an SSH path (e.g. `user@nas:/path`).
    Options: `--dry-run` to preview.

* **Display library or file info**:
    ```bash
    photosort info <path/to/library_dir> [file_path]
    ```