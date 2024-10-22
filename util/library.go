package util

import (
	// "encoding/json"
	"fmt"
	"os"
	"time"

	// "os/exec"
	"path/filepath"
	// "strconv"
	// "strings"
	// "time"
	"database/sql"

	_ "github.com/glebarez/go-sqlite"
	bar "github.com/schollz/progressbar/v3"
	// exif "github.com/barasher/go-exiftool"
)

// use a sqlite database to create a library that stores
// all the information about the photos
type Library struct {
	db *sql.DB
	root string
}

// CreateLibrary creates a new library in the specified directory
func CreateLibrary(dir string) (*Library, error) {
	// ensure the directory exists, if not create it
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		err := os.MkdirAll(dir, 0755)
		if err != nil {
			return nil, err
		}
	}

	// if the library already exists, return an error
	if _, err := os.Stat(filepath.Join(dir, "library.db")); !os.IsNotExist(err) {
		return nil, fmt.Errorf("library already exists in %s", dir)
	}


	// create a new library
	lib := &Library{}

	// open the database
	db, err := sql.Open("sqlite", filepath.Join(dir, "library.db"))
	if err != nil {
		return nil, err
	}
	err = db.Ping()
	if err != nil {
		return nil, err
	}

	lib.db = db
	lib.root = dir

	// tables:
	// 	- photos: id, filename string, relpath string, filetype string, created timestamp, sidecar relation, hash string
	// 	- sidecars: id, photo_id, filename string, relpath string, filetype string, created timestamp modified timestamp, hash string

	// create the photos table
	_, err = db.Exec(`CREATE TABLE IF NOT EXISTS photos (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		filename TEXT,
		relpath TEXT,
		filetype TEXT,
		created TIMESTAMP,
		hash TEXT UNIQUE
	)`)
	if err != nil {
		return nil, err
	}

	// create the sidecars table
	_, err = db.Exec(`CREATE TABLE IF NOT EXISTS sidecars (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		photo_id INTEGER,
		filename TEXT,
		relpath TEXT,
		filetype TEXT,
		created TIMESTAMP,
		modified TIMESTAMP,
		hash TEXT UNIQUE,
		FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE
	)`)
	if err != nil {
		return nil, err
	}

	return lib, nil
}

func OpenLibrary(dir string) (*Library, error) {
	// ensure the directory exists, if not create it
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		return nil, fmt.Errorf("library directory does not exist in %s", dir)
	}

	// create a new library
	lib := &Library{}

	// open the database
	db, err := sql.Open("sqlite", filepath.Join(dir, "library.db"))
	if err != nil {
		return nil, err
	}
	err = db.Ping()
	if err != nil {
		return nil, err
	}

	lib.db = db
	lib.root = dir

	return lib, nil
}

// if library goes out of scope, close the database
func (lib *Library) Close() {
	lib.db.Close()
}

func (lib *Library) Import(dir string) error {
	files, err := WalkDir(dir)
	if err != nil {
		return err
	}
	photos := GetPhotos(files)
	bar := bar.Default(int64(len(photos)), "Importing photos")
	for _, photo := range photos {
		var existingID int
		// Check if the photo already exists in the database
		err = lib.db.QueryRow("SELECT id FROM photos WHERE hash = ?", photo.Hash).Scan(&existingID)
		if err != nil && err != sql.ErrNoRows {
			return err
		}

		if existingID > 0 {
			// The photo already exists
			photo.ID = existingID

			// Check for existing sidecars
			for _, sidecar := range photo.Sidecars {
				var sidecarExists bool
				err = lib.db.QueryRow("SELECT EXISTS(SELECT 1 FROM sidecars WHERE photo_id = ? AND hash = ?)", photo.ID, sidecar.Hash).Scan(&sidecarExists)
				if err != nil {
					return err
				}

				if !sidecarExists {
					// Sidecar doesn't exist, so copy and insert it
					sidecarDate := photo.Created.Format("2006/01-02/")
					newPath := filepath.Join(lib.root, sidecarDate, sidecar.Filename)
					err = Copy(sidecar.Path, newPath)
					if err != nil {
						return err
					}
					_, err = lib.db.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)", photo.ID, sidecar.Filename, sidecarDate, sidecar.Filetype, sidecar.Created, sidecar.Modified, sidecar.Hash)
					if err != nil {
						return err
					}
				}
			}
		} else {
			// The photo does not exist, so we proceed to copy it and insert it
			photoDate := photo.Created.Format("2006/01-02/")
			newPath := filepath.Join(lib.root, photoDate, photo.Filename)
			err = Copy(photo.Path, newPath)
			if err != nil {
				return err
			}

			// Insert the photo into the database
			result, err := lib.db.Exec("INSERT INTO photos (filename, relpath, filetype, created, hash) VALUES (?, ?, ?, ?, ?)", photo.Filename, photoDate, photo.Filetype, photo.Created, photo.Hash)
			if err != nil {
				return err
			}

			// Get the ID of the photo that was just inserted
			id, err := result.LastInsertId()
			if err != nil {
				return err
			}
			photo.ID = int(id)

			// Insert sidecars if they exist
			for _, sidecar := range photo.Sidecars {
				sidecarDate := photo.Created.Format("2006/01-02/")
				newPath := filepath.Join(lib.root, sidecarDate, sidecar.Filename)
				err = Copy(sidecar.Path, newPath)
				if err != nil {
					return err
				}
				_, err = lib.db.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)", photo.ID, sidecar.Filename, sidecarDate, sidecar.Filetype, sidecar.Created, sidecar.Modified, sidecar.Hash)
				if err != nil {
					return err
				}
			}
		}

		bar.Add(1)
	}
	return nil
}


func (lib *Library) UpdateDB() error {
	// get all photos
	photos, err := lib.GetPhotos()
	if err != nil {
		return err
	}

	// for each photo, check if the file exists
	// if it doesn't, delete the photo from the database
	for _, photo := range photos {
		if _, err := os.Stat(photo.Path); os.IsNotExist(err) {
			_, err := lib.db.Exec("DELETE FROM photos WHERE id = ?", photo.ID)
			if err != nil {
				return err
			}
		}
		if len(photo.Sidecars) > 0 {
			for _, sidecar := range photo.Sidecars {
				if _, err := os.Stat(sidecar.Path); os.IsNotExist(err) {
					_, err := lib.db.Exec("DELETE FROM sidecars WHERE id = ?", sidecar.ID)
					if err != nil {
						return err
					}
				}
			}
		}
	}

	return nil
}

func (lib *Library) GetPhotos() (map[int]Photo, error) {
	rows, err := lib.db.Query("SELECT p.id AS photo_id, p.filename AS photo_filename, p.relpath AS photo_relpath, p.filetype AS photo_filetype, p.created AS photo_created, p.hash AS photo_hash, s.id AS sidecar_id, s.filename AS sidecar_filename, s.relpath AS sidecar_relpath, s.filetype AS sidecar_filetype, s.created AS sidecar_created, s.modified AS sidecar_modified, s.hash AS sidecar_hash FROM photos p LEFT JOIN sidecars s ON p.id = s.photo_id ORDER BY p.created")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	photos := map[int]Photo{}
	for rows.Next() {
		var (
			photoID           int
			photoFilename     string
			photoRelpath      string
			photoFiletype     string
			photoCreated      time.Time
			photoHash         string
			sidecarID         sql.NullInt64
			sidecarFilename   sql.NullString
			sidecarRelpath    sql.NullString
			sidecarFiletype   sql.NullString
			sidecarCreated     sql.NullTime
			sidecarModified    sql.NullTime
			sidecarHash        sql.NullString
		)

		err = rows.Scan(&photoID, &photoFilename, &photoRelpath, &photoFiletype, &photoCreated, &photoHash, &sidecarID, &sidecarFilename, &sidecarRelpath, &sidecarFiletype, &sidecarCreated, &sidecarModified, &sidecarHash)
		if err != nil {
			return nil, err
		}
		// if the photo (by hash) doesn't exist, add it with its sidecar
		photo, ok := photos[photoID]
		if !ok {
			photo = Photo{
				ID:       photoID,
				Filename: photoFilename,
				Path:     filepath.Join(lib.root, photoRelpath, photoFilename),
				Filetype: photoFiletype,
				Created:  photoCreated,
				Hash:     photoHash,
				Sidecars: []Sidecar{},
			}
			photos[photoID] = photo
		}
		if sidecarID.Valid {
			photo.Sidecars = append(photo.Sidecars, Sidecar{
				ID:       int(sidecarID.Int64),
				Filename: sidecarFilename.String,
				Path:     filepath.Join(lib.root, sidecarRelpath.String, sidecarFilename.String),
				Filetype: sidecarFiletype.String,
				Created:  sidecarCreated.Time,
				Modified: sidecarModified.Time,
				Hash:     sidecarHash.String,
			})
			photos[photoID] = photo
		}

				
	}
	return photos, nil
}

func (lib *Library) GetPhotoCount() (int, error) {
	rows, err := lib.db.Query("SELECT COUNT(*) FROM photos")
	if err != nil {
		return 0, err
	}
	defer rows.Close()
	rows.Next()
	var count int
	rows.Scan(&count)
	return count, nil
}