package util

import (
	"database/sql"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"time"

	_ "github.com/glebarez/go-sqlite" // SQLite driver
	bar "github.com/schollz/progressbar/v3"
)

type Library struct {
	db   *sql.DB
	root string
}

// CreateLibrary creates a new library in the specified directory
func CreateLibrary(dir string) (*Library, error) {
	// ensure the directory exists, if not create it
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		err := os.MkdirAll(dir, 0755)
		if err != nil {
			return nil, fmt.Errorf("failed to create library directory %s: %w", dir, err)
		}
	}

	dbPath := filepath.Join(dir, "library.db")
	if _, err := os.Stat(dbPath); !os.IsNotExist(err) {
		return nil, fmt.Errorf("library database already exists in %s", dir)
	}

	lib := &Library{}
	db, err := sql.Open("sqlite", dbPath+"?_journal_mode=WAL&_busy_timeout=5000")
	if err != nil {
		return nil, fmt.Errorf("failed to open database connection: %w", err)
	}

	err = db.Ping()
	if err != nil {
		db.Close() // Attempt to close before returning error
		return nil, fmt.Errorf("failed to connect to database: %w", err)
	}

	lib.db = db
	lib.root = dir

	// Create tables
	// Photos table
	_, err = lib.db.Exec(`CREATE TABLE IF NOT EXISTS photos (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		filename TEXT NOT NULL,
		relpath TEXT NOT NULL,
		filetype TEXT,
		created TIMESTAMP,
		hash TEXT UNIQUE NOT NULL
	)`)
	if err != nil {
		lib.db.Close()
		return nil, fmt.Errorf("failed to create photos table: %w", err)
	}

	// Sidecars table - Corrected Schema
	_, err = lib.db.Exec(`CREATE TABLE IF NOT EXISTS sidecars (
		id INTEGER PRIMARY KEY AUTOINCREMENT,
		photo_id INTEGER NOT NULL,
		filename TEXT NOT NULL,
		relpath TEXT NOT NULL,
		filetype TEXT,
		created TIMESTAMP,
		modified TIMESTAMP,
		hash TEXT NOT NULL,
		FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE,
		UNIQUE (photo_id, filename) 
	)`)
	if err != nil {
		lib.db.Close()
		return nil, fmt.Errorf("failed to create sidecars table: %w", err)
	}

	return lib, nil
}

// OpenLibrary opens an existing library
func OpenLibrary(dir string) (*Library, error) {
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		return nil, fmt.Errorf("library directory does not exist: %s", dir)
	}
	dbPath := filepath.Join(dir, "library.db")
	if _, err := os.Stat(dbPath); os.IsNotExist(err) {
		return nil, fmt.Errorf("library database does not exist in %s", dir)
	}

	lib := &Library{}
	db, err := sql.Open("sqlite", dbPath+"?_journal_mode=WAL&_busy_timeout=5000")
	if err != nil {
		return nil, fmt.Errorf("failed to open database connection: %w", err)
	}

	err = db.Ping()
	if err != nil {
		db.Close()
		return nil, fmt.Errorf("failed to connect to database: %w", err)
	}
	lib.db = db
	lib.root = dir
	return lib, nil
}

// Close closes the library's database connection
func (lib *Library) Close() error {
	if lib.db != nil {
		return lib.db.Close()
	}
	return nil
}

// sidecarExists checks if a sidecar with a given photo_id and filename exists.
// It returns exists, current DB hash, current DB ID, and error.
func sidecarExists(tx *sql.Tx, photoID int, filename string) (bool, string, int, error) {
	var currentHash string
	var sidecarID int
	err := tx.QueryRow("SELECT id, hash FROM sidecars WHERE photo_id = ? AND filename = ?", photoID, filename).Scan(&sidecarID, &currentHash)
	if err != nil {
		if err == sql.ErrNoRows {
			return false, "", 0, nil // Does not exist, no error
		}
		return false, "", 0, fmt.Errorf("querying sidecar existence: %w", err) // Other DB error
	}
	return true, currentHash, sidecarID, nil // Exists
}

// Import imports photos from a directory into the library
func (lib *Library) Import(dir string, doCopy bool) error {
	tx, err := lib.db.Begin()
	if err != nil {
		return fmt.Errorf("failed to begin transaction: %w", err)
	}
	defer tx.Rollback() // Rollback if not committed

	files, err := WalkDir(dir)
	if err != nil {
		return fmt.Errorf("failed to walk directory %s: %w", dir, err)
	}

	photosToProcess := GetPhotos(files)
	progressBar := bar.Default(int64(len(photosToProcess)), "Importing photos")

	for _, photo := range photosToProcess {
		var existingPhotoID int
		// var existingPhotoHash string // Not strictly needed here as photo.Hash is the source of truth for comparison

		// Check if photo with this hash already exists
		err := tx.QueryRow("SELECT id FROM photos WHERE hash = ?", photo.Hash).Scan(&existingPhotoID)
		if err != nil && err != sql.ErrNoRows {
			return fmt.Errorf("querying existing photo by hash %s: %w", photo.Hash, err)
		}

		if err == sql.ErrNoRows { // Photo does not exist, insert it
			photoDateDir := photo.Created.Format("2006/01-02") // Relative path for organization
			newPhotoPathInLib := filepath.Join(lib.root, photoDateDir, photo.Filename)

			if doCopy {
				if err := Copy(photo.Path, newPhotoPathInLib); err != nil {
					log.Printf("Warning: Failed to copy photo %s to %s: %v. Skipping photo.", photo.Path, newPhotoPathInLib, err)
					progressBar.Add(1)
					continue
				}
			}

			result, execErr := tx.Exec("INSERT INTO photos (filename, relpath, filetype, created, hash) VALUES (?, ?, ?, ?, ?)",
				photo.Filename, photoDateDir, photo.Filetype, photo.Created, photo.Hash)
			if execErr != nil {
				return fmt.Errorf("inserting photo %s: %w", photo.Filename, execErr)
			}
			id, _ := result.LastInsertId()
			photo.ID = int(id)
		} else { // Photo with this hash already exists
			photo.ID = existingPhotoID
		}

		// Process sidecars for the photo (whether it's new or existing)
		for _, sidecar := range photo.Sidecars {
			photoDateDir := photo.Created.Format("2006/01-02") // Sidecars go into same date folder as photo
			newSidecarPathInLib := filepath.Join(lib.root, photoDateDir, sidecar.Filename)

			exists, currentDbHash, sidecarDbID, checkErr := sidecarExists(tx, photo.ID, sidecar.Filename)
			if checkErr != nil {
				return fmt.Errorf("checking sidecar %s for photo ID %d: %w", sidecar.Filename, photo.ID, checkErr)
			}

			if exists { // Sidecar with this name exists for this photo
				if currentDbHash != sidecar.Hash { // Hash differs, sidecar has been updated
					if doCopy {
						if err := Copy(sidecar.Path, newSidecarPathInLib); err != nil {
							log.Printf("Warning: Failed to copy updated sidecar %s: %v. Skipping sidecar update.", sidecar.Path, err)
							continue
						}
					}
					// Update existing sidecar record
					_, updateErr := tx.Exec("UPDATE sidecars SET hash = ?, modified = ?, relpath = ?, filetype = ?, created = ? WHERE id = ?",
						sidecar.Hash, sidecar.Modified, photoDateDir, sidecar.Filetype, sidecar.Created, sidecarDbID)
					if updateErr != nil {
						return fmt.Errorf("updating sidecar %s in db: %w", sidecar.Filename, updateErr)
					}
				}
			} else { // Sidecar does not exist for this photo, insert it
				if doCopy {
					if err := Copy(sidecar.Path, newSidecarPathInLib); err != nil {
						log.Printf("Warning: Failed to copy new sidecar %s: %v. Skipping sidecar insert.", sidecar.Path, err)
						continue
					}
				}
				// The INSERT will now allow duplicate hashes (from other photos)
				// but UNIQUE (photo_id, filename) prevents this specific photo from having two sidecars with same name.
				_, insertErr := tx.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)",
					photo.ID, sidecar.Filename, photoDateDir, sidecar.Filetype, sidecar.Created, sidecar.Modified, sidecar.Hash)
				if insertErr != nil {
					// This is where your original error occurred.
					// With UNIQUE on hash removed, this specific error type (UNIQUE constraint on hash) should not happen.
					// Other errors (like UNIQUE on photo_id, filename if logic was flawed) could still occur.
					return fmt.Errorf("inserting new sidecar %s (photo_id: %d, hash: %s): %w", sidecar.Filename, photo.ID, sidecar.Hash, insertErr)
				}
			}
		}
		progressBar.Add(1)
	}
	progressBar.Finish() // Ensure progress bar finishes
	return tx.Commit()
}

// UpdateDB checks for removed files and updates/adds new ones
func (lib *Library) UpdateDB() error {
	txCull, err := lib.db.Begin()
	if err != nil {
		return fmt.Errorf("UpdateDB: failed to begin culling transaction: %w", err)
	}

	photoRows, err := txCull.Query("SELECT id, relpath, filename FROM photos")
	if err != nil {
		txCull.Rollback()
		return fmt.Errorf("UpdateDB: querying photos for culling: %w", err)
	}

	var photosToCull []struct {
		id   int
		path string
	}
	for photoRows.Next() {
		var id int
		var relpath, filename string
		if err := photoRows.Scan(&id, &relpath, &filename); err != nil {
			photoRows.Close()
			txCull.Rollback()
			return fmt.Errorf("UpdateDB: scanning photo row for culling: %w", err)
		}
		photoPath := filepath.Join(lib.root, relpath, filename)
		photosToCull = append(photosToCull, struct {
			id   int
			path string
		}{id, photoPath})
	}
	photoRows.Close()

	cullBar := bar.Default(int64(len(photosToCull)), "Checking for removed photos")
	for _, p := range photosToCull {
		if _, statErr := os.Stat(p.path); os.IsNotExist(statErr) {
			if _, execErr := txCull.Exec("DELETE FROM photos WHERE id = ?", p.id); execErr != nil {
				txCull.Rollback()
				return fmt.Errorf("UpdateDB: deleting photo ID %d: %w", p.id, execErr)
			}
		}
		cullBar.Add(1)
	}
	cullBar.Finish()

	sidecarRows, err := txCull.Query("SELECT id, relpath, filename, hash FROM sidecars")
	if err != nil {
		txCull.Rollback()
		return fmt.Errorf("UpdateDB: querying sidecars for culling/update: %w", err)
	}

	type sidecarCheckInfo struct {
		id     int
		path   string
		dbHash string
	}
	var sidecarsToCheck []sidecarCheckInfo
	for sidecarRows.Next() {
		var id int
		var relpath, filename, dbHash string
		if err := sidecarRows.Scan(&id, &relpath, &filename, &dbHash); err != nil {
			sidecarRows.Close()
			txCull.Rollback()
			return fmt.Errorf("UpdateDB: scanning sidecar row: %w", err)
		}
		scPath := filepath.Join(lib.root, relpath, filename)
		sidecarsToCheck = append(sidecarsToCheck, sidecarCheckInfo{id, scPath, dbHash})
	}
	sidecarRows.Close()

	sidecarBar := bar.Default(int64(len(sidecarsToCheck)), "Checking for removed/modified sidecars")
	for _, sc := range sidecarsToCheck {
		fileInfo, statErr := os.Stat(sc.path)
		if os.IsNotExist(statErr) {
			if _, execErr := txCull.Exec("DELETE FROM sidecars WHERE id = ?", sc.id); execErr != nil {
				txCull.Rollback()
				return fmt.Errorf("UpdateDB: deleting sidecar ID %d: %w", sc.id, execErr)
			}
		} else if statErr == nil {
			currentFileHash, hashErr := HashFile(sc.path)
			if hashErr != nil {
				log.Printf("Warning: UpdateDB: Could not hash sidecar file %s: %v. Skipping update for this sidecar.", sc.path, hashErr)
			} else if currentFileHash != sc.dbHash {
				_, execErr := txCull.Exec("UPDATE sidecars SET hash = ?, modified = ? WHERE id = ?", currentFileHash, fileInfo.ModTime(), sc.id)
				if execErr != nil {
					txCull.Rollback()
					return fmt.Errorf("UpdateDB: updating sidecar ID %d hash/modtime: %w", sc.id, execErr)
				}
			}
		}
		sidecarBar.Add(1)
	}
	sidecarBar.Finish()

	if err := txCull.Commit(); err != nil {
		return fmt.Errorf("UpdateDB: failed to commit culling transaction: %w", err)
	}

	log.Println("UpdateDB: Scanning library root for new or changed files...")
	if err := lib.Import(lib.root, false); err != nil {
		return fmt.Errorf("UpdateDB: failed during re-import phase: %w", err)
	}

	return nil
}

func (lib *Library) GetPhotos() (map[int]Photo, error) {
	rows, err := lib.db.Query(`
		SELECT 
			p.id AS photo_id, p.filename AS photo_filename, p.relpath AS photo_relpath, 
			p.filetype AS photo_filetype, p.created AS photo_created, p.hash AS photo_hash,
			s.id AS sidecar_id, s.filename AS sidecar_filename, s.relpath AS sidecar_relpath, 
			s.filetype AS sidecar_filetype, s.created AS sidecar_created, 
			s.modified AS sidecar_modified, s.hash AS sidecar_hash
		FROM photos p 
		LEFT JOIN sidecars s ON p.id = s.photo_id 
		ORDER BY p.created, p.filename, s.filename`)
	if err != nil {
		return nil, fmt.Errorf("querying photos and sidecars: %w", err)
	}
	defer rows.Close()

	photosMap := make(map[int]Photo)
	for rows.Next() {
		var pID int
		var pFilename, pRelpath, pFiletype, pHash string
		var pCreated time.Time
		var sID sql.NullInt64
		var sFilename, sRelpath, sFiletype, sHash sql.NullString
		var sCreated, sModified sql.NullTime

		err := rows.Scan(
			&pID, &pFilename, &pRelpath, &pFiletype, &pCreated, &pHash,
			&sID, &sFilename, &sRelpath, &sFiletype, &sCreated, &sModified, &sHash,
		)
		if err != nil {
			return nil, fmt.Errorf("scanning photo/sidecar row: %w", err)
		}

		photo, ok := photosMap[pID]
		if !ok {
			photo = Photo{
				ID:       pID,
				Filename: pFilename,
				Path:     filepath.Join(lib.root, pRelpath, pFilename),
				Filetype: pFiletype,
				Created:  pCreated,
				Hash:     pHash,
				Sidecars: []Sidecar{},
			}
		}

		if sID.Valid {
			sidecar := Sidecar{
				ID:       int(sID.Int64),
				PhotoID:  pID,
				Filename: sFilename.String,
				Path:     filepath.Join(lib.root, sRelpath.String, sFilename.String),
				Filetype: sFiletype.String,
				Created:  sCreated.Time,
				Modified: sModified.Time,
				Hash:     sHash.String,
			}
			isDuplicateSidecar := false
			for _, existingSc := range photo.Sidecars {
				if existingSc.ID == sidecar.ID {
					isDuplicateSidecar = true
					break
				}
			}
			if !isDuplicateSidecar {
				photo.Sidecars = append(photo.Sidecars, sidecar)
			}
		}
		photosMap[pID] = photo
	}
	if err = rows.Err(); err != nil {
		return nil, fmt.Errorf("iteration error over photo/sidecar rows: %w", err)
	}
	return photosMap, nil
}

func (lib *Library) GetPhotoCount() (int, error) {
	var count int
	err := lib.db.QueryRow("SELECT COUNT(*) FROM photos").Scan(&count)
	if err != nil {
		return 0, fmt.Errorf("querying photo count: %w", err)
	}
	return count, nil
}

func (lib *Library) SyncFrom(sourceLib *Library) error {
	tx, err := lib.db.Begin()
	if err != nil {
		return fmt.Errorf("SyncFrom: failed to begin transaction: %w", err)
	}
	defer tx.Rollback()

	photosFromSource, err := sourceLib.GetPhotos()
	if err != nil {
		return fmt.Errorf("SyncFrom: failed to get photos from source library %s: %w", sourceLib.root, err)
	}

	syncBar := bar.Default(int64(len(photosFromSource)), "Syncing photos")

	for _, sourcePhoto := range photosFromSource {
		var targetPhotoID int
		err := tx.QueryRow("SELECT id FROM photos WHERE hash = ?", sourcePhoto.Hash).Scan(&targetPhotoID)

		if err == sql.ErrNoRows {
			targetPhotoDateDir := sourcePhoto.Created.Format("2006/01-02")
			targetPhotoPath := filepath.Join(lib.root, targetPhotoDateDir, sourcePhoto.Filename)

			if err := Copy(sourcePhoto.Path, targetPhotoPath); err != nil {
				log.Printf("Warning: SyncFrom: Failed to copy photo %s to %s: %v. Skipping photo.", sourcePhoto.Path, targetPhotoPath, err)
				syncBar.Add(1)
				continue
			}

			result, execErr := tx.Exec("INSERT INTO photos (filename, relpath, filetype, created, hash) VALUES (?, ?, ?, ?, ?)",
				sourcePhoto.Filename, targetPhotoDateDir, sourcePhoto.Filetype, sourcePhoto.Created, sourcePhoto.Hash)
			if execErr != nil {
				return fmt.Errorf("SyncFrom: inserting photo %s: %w", sourcePhoto.Filename, execErr)
			}
			id, _ := result.LastInsertId()
			targetPhotoID = int(id)
		} else if err != nil {
			return fmt.Errorf("SyncFrom: querying target for photo hash %s: %w", sourcePhoto.Hash, err)
		}

		for _, sourceSidecar := range sourcePhoto.Sidecars {
			targetSidecarDateDir := sourcePhoto.Created.Format("2006/01-02")
			targetSidecarPath := filepath.Join(lib.root, targetSidecarDateDir, sourceSidecar.Filename)

			existsInTarget, currentTargetDbHash, targetSidecarDbID, checkErr := sidecarExists(tx, targetPhotoID, sourceSidecar.Filename)
			if checkErr != nil {
				return fmt.Errorf("SyncFrom: checking sidecar %s in target for photo ID %d: %w", sourceSidecar.Filename, targetPhotoID, checkErr)
			}

			if existsInTarget {
				if currentTargetDbHash != sourceSidecar.Hash {
					if err := Copy(sourceSidecar.Path, targetSidecarPath); err != nil {
						log.Printf("Warning: SyncFrom: Failed to copy updated sidecar %s: %v. Skipping sidecar update.", sourceSidecar.Path, err)
						continue
					}
					_, updateErr := tx.Exec("UPDATE sidecars SET hash = ?, modified = ?, relpath = ?, filetype = ?, created = ? WHERE id = ?",
						sourceSidecar.Hash, sourceSidecar.Modified, targetSidecarDateDir, sourceSidecar.Filetype, sourceSidecar.Created, targetSidecarDbID)
					if updateErr != nil {
						return fmt.Errorf("SyncFrom: updating sidecar %s in target DB: %w", sourceSidecar.Filename, updateErr)
					}
				}
			} else {
				if err := Copy(sourceSidecar.Path, targetSidecarPath); err != nil {
					log.Printf("Warning: SyncFrom: Failed to copy new sidecar %s: %v. Skipping sidecar insert.", sourceSidecar.Path, err)
					continue
				}
				_, insertErr := tx.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)",
					targetPhotoID, sourceSidecar.Filename, targetSidecarDateDir, sourceSidecar.Filetype, sourceSidecar.Created, sourceSidecar.Modified, sourceSidecar.Hash)
				if insertErr != nil {
					return fmt.Errorf("SyncFrom: inserting new sidecar %s into target DB: %w", sourceSidecar.Filename, insertErr)
				}
			}
		}
		syncBar.Add(1)
	}
	syncBar.Finish()
	return tx.Commit()
}
