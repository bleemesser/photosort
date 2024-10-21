package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	// "database/sql"

	bar "github.com/schollz/progressbar/v3"

	exif "github.com/barasher/go-exiftool"

	// _ "github.com/glebarez/go-sqlite"

)

func parseArgs() (dir1, dir2 string, action string) {
	// Usage: photosort import <photo_dir> <library_dir>
	// Usage: photosort create <library_dir>
	// Usage: photosort sync <library_dir1> <library_dir2>
	// Usage: photosort update <library_dir>

	if len(os.Args) == 1 {
		fmt.Println("Usage: photosort import <photo_dir> <library_dir>")
		fmt.Println("Usage: photosort create <library_dir>")
		os.Exit(1)
	}

	if os.Args[1] == "import" {
		if len(os.Args) != 4 {
			fmt.Println("Usage: photosort import <photo_dir> <library_dir>")
			os.Exit(1)
		}
		action = "import"
		if _, err := os.Stat(os.Args[2]); os.IsNotExist(err) {
			fmt.Println("Photo directory does not exist")
			os.Exit(1)
		}
		dir1, _ = filepath.Abs(os.Args[2])
		if _, err := os.Stat(os.Args[3]); os.IsNotExist(err) {
			fmt.Println("Library directory does not exist")
			os.Exit(1)
		}
		dir2, _ = filepath.Abs(os.Args[3])
	} else if os.Args[1] == "create" {
		if len(os.Args) != 3 {
			fmt.Println("Usage: photosort create <library_dir>")
			os.Exit(1)
		}
		action = "create"
		dir2, _ = filepath.Abs(os.Args[2])
	} else if os.Args[1] == "sync" {
		if len(os.Args) != 4 {
			fmt.Println("Usage: photosort sync <library_dir1> <library_dir2>")
			os.Exit(1)
		}
		action = "sync"
		if _, err := os.Stat(os.Args[2]); os.IsNotExist(err) {
			fmt.Println("Library directory 1 does not exist")
			os.Exit(1)
		}
		dir1, _ = filepath.Abs(os.Args[2])
		if _, err := os.Stat(os.Args[3]); os.IsNotExist(err) {
			fmt.Println("Library directory 2 does not exist")
			os.Exit(1)
		}
		dir2, _ = filepath.Abs(os.Args[3])
	} else if os.Args[1] == "update" {
		if len(os.Args) != 3 {
			fmt.Println("Usage: photosort update <library_dir>")
			os.Exit(1)
		}
		action = "update"
		dir2, _ = filepath.Abs(os.Args[2])
		if _, err := os.Stat(dir2); os.IsNotExist(err) {
			fmt.Println("Library directory does not exist")
			os.Exit(1)
		}
	} else {
		fmt.Println("Usage: photosort import <photo_dir> <library_dir>")
		fmt.Println("Usage: photosort create <library_dir>")
		os.Exit(1)
	}

	return
}

func findFiles(photoDir string) (filePaths []string) {
	// Walk through the photo directory and find all files that are images.
	// Return a list of the paths to those files.

	err := filepath.Walk(photoDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			fmt.Println("Error walking through directory:", err)
			return err
		}
		if fi, err := os.Stat(path); err == nil && !fi.IsDir() {
			// ignore dotfiles
			if !strings.HasPrefix(fi.Name(), ".") {
				filePaths = append(filePaths, path)
			}

		}

		return nil
	})
	if err != nil {
		fmt.Println("Error walking through directory:", err)
	}
	
	return
}

func getExif(path string, et *exif.Exiftool) (fields map[string]interface{}) {
	// Extract metadata from the file at the given path.
	metadata := et.ExtractMetadata(path)
	for _, value := range metadata {
		if value.Err != nil {
			fmt.Printf("Error concerning %v: %v\n", value.File, value.Err)
			continue
		}
		
		fields = value.Fields
	}

	return
}

type Photo struct{
	Path string `json:"path"`
	Name string `json:"name"`
	CreateDate Date `json:"createDate"`
	FileType string `json:"fileType"`
	Sidecar string `json:"sidecar"`
}

type Date struct{
	Year int `json:"year"`
	Month int `json:"month"`
	Day int `json:"day"`
}

type Library struct{
	Photos []Photo `json:"photos"`
	ModifyDate time.Time `json:"modifyDate"`
	CreateDate time.Time `json:"createDate"`
	Directory string `json:"directory"`
}

// date to string format: yyyy-mm-dd
func (d Date) String() string {
	return fmt.Sprintf("%04d-%02d-%02d", d.Year, d.Month, d.Day)
}

// photo to string format: name date, type
func (p Photo) String() string {
	return fmt.Sprintf("%s %s %s", p.Name, p.CreateDate, p.FileType)
}

// library to string format: created date, modified date, number of photos
func (l Library) String() string {
	return fmt.Sprintf("Library created on %s, last modified on %s, %d photos", l.CreateDate, l.ModifyDate, len(l.Photos))
}

// library UpdateFile method
func (l *Library) WriteToFile() {
	l.ModifyDate = time.Now()
	libraryFile, err := os.Create(filepath.Join(l.Directory, "library.json"))
	if err != nil {
		fmt.Println("Error creating library file:", err)
		os.Exit(1)
	}
	defer libraryFile.Close()

	libraryJSON, err := json.MarshalIndent(l, "", "    ")
	if err != nil {
		fmt.Println("Error encoding library object:", err)
		os.Exit(1)
	}

	_, err = libraryFile.Write(libraryJSON)
	if err != nil {
		fmt.Println("Error writing library object to file:", err)
		os.Exit(1)
	}
}

func (l *Library) ReadFromFile() {
	libraryFile, err := os.Open(filepath.Join(l.Directory, "library.json"))
	if err != nil {
		fmt.Println("Error opening library file:", err)
		os.Exit(1)
	}
	defer libraryFile.Close()

	decoder := json.NewDecoder(libraryFile)
	err = decoder.Decode(l)
	if err != nil {
		fmt.Println("Error decoding library file:", err)
		os.Exit(1)
	}
}

func sameFiles(file1, file2 string) bool {
	// Check if two files are the same by comparing their contents.
	// Return true if they are the same, false otherwise.

	// Get the sizes of the files.
	fi1, err := os.Stat(file1)
	if err != nil {
		fmt.Println("Error getting file info for", file1, ":", err)
		os.Exit(1)
	}
	fi2, err := os.Stat(file2)
	if err != nil {
		fmt.Println("Error getting file info for", file2, ":", err)
		os.Exit(1)
	}
	if fi1.Size() != fi2.Size() {
		return false
	}

	// Compare the contents of the files.
	out, err := exec.Command("cmp", file1, file2).Output()
	if err != nil {
		return false
	}
	if len(out) > 0 {
		return false
	}

	return true
}

// library .AddPhoto method
func (l *Library) AddPhoto(photo Photo, write bool) {
	// copy photo to library directory: year/mm-dd/filename
	srcPath := photo.Path
	newPath := filepath.Join(l.Directory, fmt.Sprintf("%04d", photo.CreateDate.Year), fmt.Sprintf("%02d-%02d", photo.CreateDate.Month, photo.CreateDate.Day), photo.Name)
	relativePath := strings.TrimPrefix(newPath, l.Directory + "/")

	newPhoto := Photo{Path: relativePath, Name: photo.Name, CreateDate: photo.CreateDate, FileType: photo.FileType, Sidecar: ""}

	var newSidecarPath string
	if photo.Sidecar != "" {
		newSidecarPath = strings.TrimSuffix(newPath, filepath.Ext(newPath)) + ".photo-edit"
		newPhoto.Sidecar = strings.TrimPrefix(newSidecarPath, l.Directory + "/")
	}

	// ensure library does not already contain photo
	for _, p := range l.Photos {
		if p.Path == relativePath {
			if sameFiles(srcPath, newPath) {
				if photo.Sidecar != "" {
					err := os.MkdirAll(filepath.Dir(newPath), os.ModePerm)
					if err != nil {
						fmt.Println("Error creating directory for photo:", err)
						os.Exit(1)
					}
					// fmt.Println("cp", photo.Sidecar, newSidecarPath)

					err = exec.Command("cp", photo.Sidecar, newSidecarPath).Run()
					if err != nil {
						fmt.Println("Error copying sidecar file:", err)
						os.Exit(1)
					}
				}
				return
			} else {
				newPath = newPath + "-2"
				photo.Name = photo.Name + "-2"
				photo.Path = strings.TrimPrefix(newPath, l.Directory + "/")
			}

		}
	}

	err := os.MkdirAll(filepath.Dir(newPath), os.ModePerm)
	if err != nil {
		fmt.Println("Error creating directory for photo:", err)
		os.Exit(1)
	}
	// copy photo's file to new path
	err = exec.Command("cp", srcPath, newPath).Run()
	if err != nil {
		fmt.Println("Error copying photo:", err)
		os.Exit(1)
	}
	// copy photo's sidecar file to new path
	if photo.Sidecar != "" {
		// fmt.Println("cp", photo.Sidecar, newSidecarPath)
		err := exec.Command("cp", photo.Sidecar, newSidecarPath).Run()
		if err != nil {
			fmt.Println("Error copying sidecar file:", err)
			os.Exit(1)
		}
	}
	l.Photos = append(l.Photos, newPhoto)

	if write {
		l.WriteToFile()
	}
}

// library .AddPhotos method
func (l *Library) AddPhotos(photos []Photo) {
	bar := bar.Default(int64(len(photos)), "Adding photos")
	for _, photo := range photos {
		l.AddPhoto(photo, false)
		bar.Add(1)
	}
	l.WriteToFile()
}

// library .SyncInto method
func (l *Library) SyncInto(l2 *Library) {
	bar := bar.Default(int64(len(l.Photos)), "Syncing photos")
	for _, photo := range l.Photos {
		photo.Path = filepath.Join(l.Directory, photo.Path)
		if photo.Sidecar != "" {
			photo.Sidecar = filepath.Join(l.Directory, photo.Sidecar)
		}
		l2.AddPhoto(photo, false)
		bar.Add(1)
	}
	l2.WriteToFile()
}


func (l *Library) Update() {
	// go through all photos and ensure they exist. if not, remove them
	bar := bar.Default(int64(len(l.Photos)), "Updating photos")
	newPhotos := l.Photos
	for _, photo := range l.Photos {
		if _, err := os.Stat(filepath.Join(l.Directory, photo.Path)); os.IsNotExist(err) {
			fmt.Println("Removing photo", photo.Name)
			newPhotos = append(newPhotos[:0], newPhotos[1:]...)
		}
		bar.Add(1)
	}
	l.Photos = newPhotos
	l.WriteToFile()
}

func extractPhotos(filePaths []string, et *exif.Exiftool) (photos []Photo) {
	bar := bar.Default(int64(len(filePaths)), "Extracting photos")
	for _, path := range filePaths {
		fields := getExif(path, et)
		if fields == nil {
			bar.Add(1)
			continue
		}
		if !strings.Contains(fields["MIMEType"].(string), "image") {
			bar.Add(1)
			continue
		}
		
		var date Date

		if fields["CreateDate"] != nil {
			dateString := fields["CreateDate"].(string)
			date.Year, _ = strconv.Atoi(dateString[:4])
			date.Month, _ = strconv.Atoi(dateString[5:7])
			date.Day, _ = strconv.Atoi(dateString[8:10])
		} else if fields["DateTimeOriginal"] != nil {
			dateString := fields["DateTimeOriginal"].(string)
			date.Year, _ = strconv.Atoi(dateString[:4])
			date.Month, _ = strconv.Atoi(dateString[5:7])
			date.Day, _ = strconv.Atoi(dateString[8:10])
		} else {
			t := time.Now()
			date.Year = t.Year()
			date.Month = int(t.Month())
			date.Day = t.Day()
		}

		// see if there is a sidecar file
		sidecar := strings.TrimSuffix(path, filepath.Ext(path)) + ".photo-edit"
		if _, err := os.Stat(sidecar); err == nil {
			photo := Photo{Path: path, Name: fields["FileName"].(string), CreateDate: date, FileType: fields["FileType"].(string), Sidecar: sidecar}
			photos = append(photos, photo)
		} else {
			photos = append(photos, Photo{Path: path, Name: fields["FileName"].(string), CreateDate: date, FileType: fields["FileType"].(string), Sidecar: ""})
		}
		bar.Add(1)
	}

	return
}

func doImport(photoDir, libraryDir string) {
	// Find all files in the photo directory.
	filePaths := findFiles(photoDir)

	// Create an exiftool instance to extract metadata from the files.
	et, err := exif.NewExiftool()
	if err != nil {
		fmt.Println("Error creating exiftool:", err)
		os.Exit(1)
	}
	defer et.Close()

	// Extract list of photos from the files with simplified metadata.
	photos := extractPhotos(filePaths, et)
	fmt.Println("Found", len(photos), "photos")

	if _, err := os.Stat(filepath.Join(libraryDir, "library.json")); os.IsNotExist(err) {
		fmt.Println("Library does not exist in", libraryDir, " please create it first")
		os.Exit(1)
	}

	// Read library from file
	library := Library{Directory: libraryDir}
	library.ReadFromFile()

	// Add photos to library
	library.AddPhotos(photos)

	fmt.Println(library)
}

func doCreate(libraryDir string) Library{
	// Create library directory
	err := os.MkdirAll(libraryDir, os.ModePerm)
	if err != nil {
		fmt.Println("Error creating library directory:", err)
		os.Exit(1)
	}

	// Create library file
	libraryFile, err := os.Create(filepath.Join(libraryDir, "library.json"))
	if err != nil {
		fmt.Println("Error creating library file:", err)
		os.Exit(1)
	}
	defer libraryFile.Close()

	// Create library object
	library := Library{}
	library.ModifyDate = time.Now()
	library.CreateDate = time.Now()
	library.Photos = []Photo{}
	library.Directory = libraryDir

	// Write library object to library file as JSON
	library.WriteToFile()

	fmt.Println("Library created in", libraryDir)
	return library
}



func main() {
	// Parse arguments to get photo and library directories.
	dir1, dir2, action := parseArgs()
	if action == "import" {
		fmt.Println("Importing photos from", dir1, "to", dir2)
		doImport(dir1, dir2)
	} else if action == "create" {
		fmt.Println("Creating library in", dir2)
		doCreate(dir2)
	} else if action == "sync" {
		fmt.Println("Syncing libraries in", dir2, "and", dir1)
		library1 := Library{Directory: dir1}
		library1.ReadFromFile()
		library2 := Library{Directory: dir2}
		library2.ReadFromFile()
		library1.Update()
		library2.Update()
		library1.SyncInto(&library2)
	} else if action == "update" {
		library := Library{Directory: dir2}
		library.ReadFromFile()
		library.Update()
	}
}