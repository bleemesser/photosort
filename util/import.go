// photosort/util/import.go
package util

import (
	"crypto/sha256"
	"encoding/base64"
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"strings"
	"time"

	exif "github.com/barasher/go-exiftool"
	bar "github.com/schollz/progressbar/v3"
)

// SourceSidecarInfo holds metadata about a sidecar file from its original location.
type SourceSidecarInfo struct {
	OriginalPath string
	Filename     string
	Filetype     string
	Created      time.Time // Can be photo's creation date or sidecar's own time
	Modified     time.Time
	Hash         string
}

// SourcePhotoInfo holds metadata about a photo file from its original location,
// including its strictly associated sidecars.
type SourcePhotoInfo struct {
	OriginalPath string
	Filename     string
	Filetype     string
	Created      time.Time
	Hash         string
	Sidecars     []SourceSidecarInfo // Sidecars strictly matching this photo's base name
}

func WalkDir(dir string) ([]string, error) {
	var files []string
	err := filepath.Walk(dir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if !info.IsDir() && !strings.HasPrefix(info.Name(), ".") {
			files = append(files, path)
		}
		return nil
	})
	return files, err
}

func GetPhotos(sourceFilePaths []string) []SourcePhotoInfo {
	progressBar := bar.Default(int64(len(sourceFilePaths)), "Scanning source files metadata")
	var allPhotoInfo []SourcePhotoInfo
	et, err := exif.NewExiftool()
	if err != nil {
		log.Printf("Error creating Exiftool helper: %v. EXIF data reading might be affected.", err)
	}
	if et != nil {
		defer et.Close()
	}

	for _, photoOriginalPath := range sourceFilePaths {
		var fields map[string]interface{}
		if et != nil {
			extractedMeta := et.ExtractMetadata(photoOriginalPath)
			if len(extractedMeta) > 0 && extractedMeta[0].Err == nil {
				fields = extractedMeta[0].Fields
			} else if len(extractedMeta) > 0 && extractedMeta[0].Err != nil {
				log.Printf("Warning: Could not get EXIF for %s: %v", photoOriginalPath, extractedMeta[0].Err)
			}
		}

		fileInfo, statErr := os.Stat(photoOriginalPath)
		if statErr != nil {
			log.Printf("Warning: Could not stat file %s: %v. Skipping.", photoOriginalPath, statErr)
			progressBar.Add(1)
			continue
		}

		if fields == nil {
			fields = make(map[string]interface{})
		}
		// Ensure FileName is present, prefer EXIF, fallback to OS filename
		if _, ok := fields["FileName"]; !ok {
			fields["FileName"] = fileInfo.Name()
		}


		// Basic MIME type check, can be expanded
		isImage := false
		if mimeType, ok := fields["MIMEType"].(string); ok {
			if strings.Contains(mimeType, "image") {
				isImage = true
			}
		} else { // Fallback if MIMEType is not in EXIF - very basic check
			ext := strings.ToLower(filepath.Ext(photoOriginalPath))
			imgExts := []string{".jpg", ".jpeg", ".png", ".gif", ".tiff", ".tif", ".nef", ".cr2", ".arw", ".dng", ".heic", ".heif", ".webp"}
			for _, imgExt := range imgExts {
				if ext == imgExt {
					isImage = true
					break
				}
			}
			if !isImage {
                 // log.Printf("Debug: File %s with ext %s not considered image by ext check", photoOriginalPath, ext)
            }
		}

		if !isImage {
			progressBar.Add(1)
			continue
		}

		var date time.Time
		parsedDate := false
		if createdDateStr, ok := fields["CreateDate"].(string); ok {
			date, err = time.Parse("2006:01:02 15:04:05", createdDateStr)
			if err == nil { parsedDate = true }
		}
		if !parsedDate {
			if dateTimeOrigStr, ok := fields["DateTimeOriginal"].(string); ok {
				date, err = time.Parse("2006:01:02 15:04:05", dateTimeOrigStr)
				if err == nil { parsedDate = true }
			}
		}
		if !parsedDate {
			date = fileInfo.ModTime() // Fallback to file modification time
		}

		photoFilename := filepath.Base(photoOriginalPath)
		photoFiletype := strings.ToUpper(strings.TrimPrefix(filepath.Ext(photoFilename), "."))
		photoHash, err := HashFile(photoOriginalPath)
		if err != nil {
			log.Printf("Error: Failed to hash photo %s: %v. Skipping photo.", photoOriginalPath, err)
			progressBar.Add(1)
			continue
		}

		var foundSidecars []SourceSidecarInfo
		sidecarExtensions := []string{".xmp", ".photo-edit"} // Define your sidecar extensions
		photoBaseName := strings.TrimSuffix(photoOriginalPath, filepath.Ext(photoOriginalPath))

		for _, scExt := range sidecarExtensions {
			sidecarOriginalPath := photoBaseName + scExt
			scFileInfo, scStatErr := os.Stat(sidecarOriginalPath)
			if scStatErr == nil { // Sidecar file exists
				scHash, scHashErr := HashFile(sidecarOriginalPath)
				if scHashErr != nil {
					log.Printf("Warning: Failed to hash sidecar %s: %v. Skipping sidecar.", sidecarOriginalPath, scHashErr)
					continue
				}
				foundSidecars = append(foundSidecars, SourceSidecarInfo{
					OriginalPath: sidecarOriginalPath,
					Filename:     filepath.Base(sidecarOriginalPath),
					Filetype:     strings.ToUpper(strings.TrimPrefix(scExt, ".")),
					Created:      date, // Often sidecars share photo's "original" date context
					Modified:     scFileInfo.ModTime(),
					Hash:         scHash,
				})
			}
		}
		
		allPhotoInfo = append(allPhotoInfo, SourcePhotoInfo{
			OriginalPath: photoOriginalPath,
			Filename:     photoFilename,
			Filetype:     photoFiletype,
			Created:      date,
			Hash:         photoHash,
			Sidecars:     foundSidecars,
		})
		progressBar.Add(1)
	}
	progressBar.Finish()
	return allPhotoInfo
}


func HashFile(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()

	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return base64.StdEncoding.EncodeToString(h.Sum(nil)), nil
}

func Copy(src, dst string) error {
	sourceFileStat, err := os.Stat(src)
	if err != nil {
		return err
	}

	if !sourceFileStat.Mode().IsRegular() {
		return fmt.Errorf("%s is not a regular file", src)
	}

	source, err := os.Open(src)
	if err != nil {
		return err
	}
	defer source.Close()

	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		return err
	}

	destination, err := os.Create(dst)
	if err != nil {
		return err
	}
	defer destination.Close()

	_, err = io.Copy(destination, source)
	if err != nil {
		os.Remove(dst)
		return fmt.Errorf("failed to copy content from %s to %s: %w", src, dst, err)
	}
	return nil
}

// Photo and Sidecar structs (if used directly by library.go for DB interaction, keep them)
// Or, library.go can map from SourcePhotoInfo to its internal DB representation.
// For now, these are not directly used by GetPhotos anymore.
type Photo struct {
	ID       int
	Filename string
	Path     string // Path within the library
	Filetype string
	Created  time.Time
	Sidecars []Sidecar // Sidecars associated in the library
	Hash     string
}

type Sidecar struct {
	ID       int
	PhotoID  int
	Filename string
	Path     string // Path within the library
	Filetype string
	Created  time.Time
	Modified time.Time
	Hash     string
}