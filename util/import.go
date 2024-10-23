package util

import (
	"crypto/sha256"
	"encoding/base64"
	"io"
	"os"
	"os/exec"
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
		return nil
	}
	defer et.Close()
	for _, file := range files {
		fields := getExif(file, et)
		if fields == nil {
			bar.Add(1)
			continue
		}
		if !strings.Contains(fields["MIMEType"].(string), "image") {
			bar.Add(1)
			continue
		}

		var date time.Time

		if fields["CreateDate"] != nil {
			dateString := fields["CreateDate"].(string)
			date, err = time.Parse("2006:01:02 15:04:05", dateString)
			if err != nil {
				bar.Add(1)
				continue
			}
		} else if fields["DateTimeOriginal"] != nil {
			dateString := fields["DateTimeOriginal"].(string)
			date, err = time.Parse("2006:01:02 15:04:05", dateString)
			if err != nil {
				bar.Add(1)
				continue
			}
		} else {
			date = time.Date(1900, 1, 1, 0, 0, 0, 0, time.UTC)
		}

		sidecarExts := []string{".xmp", ".photo-edit"}
		// sidecarExts := []string{".photo-edit"}

		var sidecars []Sidecar
		for _, ext := range sidecarExts {
			sidecarPath := strings.TrimSuffix(file, filepath.Ext(file)) + ext
			if _, err := os.Stat(sidecarPath); !os.IsNotExist(err) {
				// include file name in hash
				sidecarHash, err := HashFile(sidecarPath, fields["FileName"].(string))
				if err != nil { // TODO: handle error better
					bar.Add(1)
					continue
				}
				sc := Sidecar{Filename: filepath.Base(sidecarPath), Path: sidecarPath, Filetype: strings.ToUpper(filepath.Ext(sidecarPath)[1:]), Created: date, Modified: time.Now(), Hash: sidecarHash}
				sidecars = append(sidecars, sc)
			}
		}

		photoHash, err := HashFile(file)
		if err != nil { // TODO: handle error better
			bar.Add(1)
			continue
		}

		p := Photo{Filename: filepath.Base(file), Path: file, Filetype: strings.ToUpper(filepath.Ext(file)[1:]), Created: date, Sidecars: sidecars, Hash: photoHash}

		photos = append(photos, p)
		bar.Add(1)

	}
	return photos
}

func getExif(path string, et *exif.Exiftool) map[string]interface{} {

	metadata := et.ExtractMetadata(path)
	for _, v := range metadata {
		if v.Err != nil {
			continue
		}
		return v.Fields
	}
	return nil
}

func HashFile(path string, metadata ...string) (string, error) {
	// sha256 hash the file and any given metadata
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()

	h := sha256.New()
	for _, m := range metadata {
		io.WriteString(h, m)
	}
	
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}

	return base64.StdEncoding.EncodeToString(h.Sum(nil)), nil
}

func Copy(src, dst string) error {
	// mkdirs
	err := os.MkdirAll(filepath.Dir(dst), 0755)
	if err != nil {
		return err
	}
	cmd := exec.Command("cp", src, dst)
	err = cmd.Run()
	if err != nil {
		return err
	}
	return nil
}

type Photo struct {
	ID       int
	Filename string
	Path  string
	Filetype string
	Created  time.Time
	Sidecars  []Sidecar
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
	Path  string
	Filetype string
	Created  time.Time
	Modified time.Time
	Hash     string
}

func (s Sidecar) String() string {
	return "Sidecar: " + s.Filename + " (" + s.Created.Format("2006-01-02") + ")"
}



