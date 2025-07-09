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

After installation, you can invoke the program directly.

* **Create a new library**:
    The directory will be created if it does not exist.
    ```bash
    photosort create <path/to/library_dir>
    ```

* **Import photos into a library**:
    Photos and their sidecars will be copied from the source directory into the library.
    ```bash
    photosort import <path/to/your/photos> <path/to/library_dir>
    ```

* **Update a library's database**:
    This command scans the library for file changes and updates the database.
    ```bash
    photosort update <path/to/library_dir>
    ```

* **Sync two libraries**:
    This performs a one-way sync from a source library to a target.
    ```bash
    photosort sync <path/to/source_library> <path/to/target_library>
    ```

* **Get library information**:
    Displays the total number of photos in the library.
    ```bash
    photosort info <path/to/library_dir>
    ```

## Todo
- [ ] Add a startup check and a more user-friendly error message if `exiftool` is not found in the system's `PATH`.