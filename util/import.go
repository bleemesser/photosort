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

func GetPhotos(files []string) []Photo {
	bar := bar.Default(int64(len(files)), "Acquiring file metadata")
	var photos []Photo
	et, err := exif.NewExiftool()
	if err != nil {
		log.Printf("Error creating Exiftool helper: %v. EXIF data will not be read.", err)
		// Allow continuing without exiftool, but some features might be limited.
		// Alternatively, return nil or an error: return nil
	}
	if et != nil {
		defer et.Close()
	}

	for _, file := range files {
		var fields map[string]interface{}
		if et != nil {
			extractedMeta := et.ExtractMetadata(file)
			if len(extractedMeta) > 0 && extractedMeta[0].Err == nil {
				fields = extractedMeta[0].Fields
			} else if len(extractedMeta) > 0 && extractedMeta[0].Err != nil {
				log.Printf("Warning: Could not get EXIF for %s: %v", file, extractedMeta[0].Err)
			}
		}

		if fields == nil { // Fallback or if exiftool failed for this file
			fields = make(map[string]interface{}) // Ensure fields is not nil
			fileInfo, statErr := os.Stat(file)
			if statErr == nil {
				fields["FileName"] = fileInfo.Name()
				// Attempt to get MIME type through other means if needed, or skip if critical
			} else {
				log.Printf("Warning: Could not stat file %s: %v. Skipping.", file, statErr)
				bar.Add(1)
				continue
			}
		}

		if fields["MIMEType"] == nil {
			// Potentially try http.DetectContentType here if MIMEType is critical
			// For now, if EXIF didn't yield it, we might skip or make a guess based on extension
			log.Printf("Warning: MIMEType missing for %s. Skipping.", file)
			bar.Add(1)
			continue
		}
		if !strings.Contains(fields["MIMEType"].(string), "image") {
			bar.Add(1)
			continue
		}

		var date time.Time
		parsedDate := false

		if fields["CreateDate"] != nil {
			dateString, ok := fields["CreateDate"].(string)
			if ok {
				date, err = time.Parse("2006:01:02 15:04:05", dateString)
				if err == nil {
					parsedDate = true
				}
			}
		}
		if !parsedDate && fields["DateTimeOriginal"] != nil {
			dateString, ok := fields["DateTimeOriginal"].(string)
			if ok {
				date, err = time.Parse("2006:01:02 15:04:05", dateString)
				if err == nil {
					parsedDate = true
				}
			}
		}

		if !parsedDate {
			fileInfo, statErr := os.Stat(file)
			if statErr == nil {
				date = fileInfo.ModTime() // Fallback to file modification time
				log.Printf("Warning: No EXIF date for %s, using file modification time: %s", file, date.Format(time.RFC3339))
			} else {
				date = time.Date(1900, 1, 1, 0, 0, 0, 0, time.UTC) // Absolute fallback
				log.Printf("Warning: No EXIF date and could not stat file %s, using default date 1900-01-01", file)
			}
		}

		sidecarExts := []string{".xmp", ".photo-edit"}
		var sidecars []Sidecar
		for _, ext := range sidecarExts {
			sidecarPath := strings.TrimSuffix(file, filepath.Ext(file)) + ext
			sidecarInfo, err := os.Stat(sidecarPath)
			if err == nil { // Sidecar file exists
				sidecarHash, err := HashFile(sidecarPath) // Hash sidecar based on its own content only
				if err != nil {
					log.Printf("Warning: Failed to hash sidecar %s for photo %s: %v. Skipping sidecar.", sidecarPath, file, err)
					// bar.Add(1) // Don't add to bar here, main photo progress is separate
					continue
				}
				sidecarModifiedTime := sidecarInfo.ModTime()
				// For Created, ideally, it would be the sidecar's creation, but that's hard.
				// Using the photo's date or sidecar's mod time are options.
				// Let's use the photo's date for consistency with original logic unless specific sidecar created date is found.
				sc := Sidecar{
					Filename: filepath.Base(sidecarPath),
					Path:     sidecarPath,
					Filetype: strings.ToUpper(strings.TrimPrefix(filepath.Ext(sidecarPath), ".")),
					Created:  date, // Or sidecarInfo.ModTime() if preferred for 'Created' too
					Modified: sidecarModifiedTime,
					Hash:     sidecarHash,
				}
				sidecars = append(sidecars, sc)
			}
		}

		photoHash, err := HashFile(file)
		if err != nil {
			log.Printf("Error: Failed to hash photo %s: %v. Skipping photo.", file, err)
			bar.Add(1)
			continue
		}

		p := Photo{
			Filename: filepath.Base(file),
			Path:     file,
			Filetype: strings.ToUpper(strings.TrimPrefix(filepath.Ext(file), ".")),
			Created:  date,
			Sidecars: sidecars,
			Hash:     photoHash,
		}

		photos = append(photos, p)
		bar.Add(1)
	}
	return photos
}

// getExif is now effectively inlined into GetPhotos or handled by its error checks.
// If exiftool is not found or fails, GetPhotos attempts to proceed with minimal info or skips.

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
		// If copy fails, attempt to remove the potentially partially written destination file.
		os.Remove(dst)
		return fmt.Errorf("failed to copy content from %s to %s: %w", src, dst, err)
	}
	return nil
}

type Photo struct {
	ID       int
	Filename string
	Path     string
	Filetype string
	Created  time.Time
	Sidecars []Sidecar
	Hash     string
}

func (p Photo) String() string {
	out := "Photo: " + p.Filename + " (" + p.Created.Format("2006-01-02") + ")"
	for _, s := range p.Sidecars {
		out += "\n\t" + s.String()
	}
	return out
}

type Sidecar struct {
	ID       int
	PhotoID  int
	Filename string
	Path     string
	Filetype string
	Created  time.Time
	Modified time.Time
	Hash     string
}

func (s Sidecar) String() string {
	return "Sidecar: " + s.Filename + " (Modified: " + s.Modified.Format("2006-01-02 15:04:05") + ")"
}
