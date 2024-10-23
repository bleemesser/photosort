# Photosort
A CLI tool to sort and sync libraries of photos and their accompanying sidecar files.
## Installation
- Clone the repository to a directory of your choice.
- To build the program, you will need to have Go installed on your system. You can download it [here](https://golang.org/dl/).
- Once you have Go installed, you can build the program by running the following command from within the project directory:
```go build -o photosort```
- Photosort depends on exiftool to read metadata from photos. It must be installed and in your path. It is available in homebrew and apt, and can be downloaded from the [official website](https://exiftool.org/).

## Usage
Photosort is a command line tool. It is run by executing the binary file with the appropriate arguments. The following is a list of all available arguments:
- create: `./photosort create <dir>` - Create a new photosort library in the specified directory. The directory will be created if it does not exist.
- import: `./photosort import <photo_dir> <library_dir>` - Import photos from the specified directory into the library. The photos and sidecars will be copied into the library directory and organized by date.
- update: `./photosort update <library_dir>` - Update the library's database with any photo removals or additions, and any sidecar changes.
- sync: `./photosort sync <src_lib_dir> <target_lib_dir>` - Sync changes from the source library to the target library. This will copy any new photos and sidecars from the source library to the target library, and update all sidecars in the target library. No deletions will be made.