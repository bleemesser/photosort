// photosort/util/library.go
package util

import (
	"database/sql"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "github.com/mattn/go-sqlite3" // Use the cgo-based driver
	bar "github.com/schollz/progressbar/v3"
)

type Library struct {
	db   *sql.DB
	root string
}

// isFilenameBetter determines if newFilename is preferred over oldFilename.
// Prefers non-"copy" versions and then shorter filenames.
func isFilenameBetter(newFilename, oldFilename string) bool {
	newBase := strings.ToLower(strings.TrimSuffix(newFilename, filepath.Ext(newFilename)))
	oldBase := strings.ToLower(strings.TrimSuffix(oldFilename, filepath.Ext(oldFilename)))
	copyPatterns := []string{" copy", " (1)", " (2)", " (3)", "_1", "_2", "_3"}

	newIsLikelyCopy := false
	for _, pattern := range copyPatterns {
		if strings.HasSuffix(newBase, pattern) {
			newIsLikelyCopy = true
			break
		}
	}
	oldIsLikelyCopy := false
	for _, pattern := range copyPatterns {
		if strings.HasSuffix(oldBase, pattern) {
			oldIsLikelyCopy = true
			break
		}
	}

	if oldIsLikelyCopy && !newIsLikelyCopy {
		return true
	}
	if !oldIsLikelyCopy && newIsLikelyCopy {
		return false
	}
	if len(newFilename) < len(oldFilename) {
		return true
	}
	if len(newFilename) > len(oldFilename) {
		return false
	}
	if newFilename < oldFilename {
		return true
	}
	return false
}

// CreateLibrary and OpenLibrary remain the same
func CreateLibrary(dir string) (*Library, error) {
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		if errMk := os.MkdirAll(dir, 0755); errMk != nil {
			return nil, fmt.Errorf("failed to create library directory %s: %w", dir, errMk)
		}
	}
	dbPath := filepath.Join(dir, "library.db")
	if _, err := os.Stat(dbPath); !os.IsNotExist(err) {
		return nil, fmt.Errorf("library database already exists in %s", dir)
	}
	lib := &Library{}
	db, err := sql.Open("sqlite3", dbPath)
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}
	if err = db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("failed to connect to database: %w", err)
	}
	lib.db = db
	lib.root = dir
	// Create tables
	if _, err = lib.db.Exec(`CREATE TABLE IF NOT EXISTS photos (id INTEGER PRIMARY KEY AUTOINCREMENT, filename TEXT NOT NULL, relpath TEXT NOT NULL, filetype TEXT, created TIMESTAMP, hash TEXT UNIQUE NOT NULL)`); err != nil {
		lib.db.Close()
		return nil, fmt.Errorf("failed to create photos table: %w", err)
	}
	if _, err = lib.db.Exec(`CREATE TABLE IF NOT EXISTS sidecars (id INTEGER PRIMARY KEY AUTOINCREMENT, photo_id INTEGER NOT NULL, filename TEXT NOT NULL, relpath TEXT NOT NULL, filetype TEXT, created TIMESTAMP, modified TIMESTAMP, hash TEXT NOT NULL, FOREIGN KEY (photo_id) REFERENCES photos(id) ON DELETE CASCADE, UNIQUE (photo_id, filename))`); err != nil {
		lib.db.Close()
		return nil, fmt.Errorf("failed to create sidecars table: %w", err)
	}
	return lib, nil
}

func OpenLibrary(dir string) (*Library, error) {
	if _, err := os.Stat(dir); os.IsNotExist(err) {
		return nil, fmt.Errorf("library directory does not exist: %s", dir)
	}
	dbPath := filepath.Join(dir, "library.db")
	if _, err := os.Stat(dbPath); os.IsNotExist(err) {
		return nil, fmt.Errorf("library database does not exist in %s", dir)
	}
	lib := &Library{}
	db, err := sql.Open("sqlite3", dbPath)
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}
	if err = db.Ping(); err != nil {
		db.Close()
		return nil, fmt.Errorf("failed to connect to database: %w", err)
	}
	lib.db = db
	lib.root = dir
	return lib, nil
}

func (lib *Library) Close() error {
	if lib.db != nil {
		return lib.db.Close()
	}
	return nil
}

// --- Import function refactored ---
type FileToCopy struct {
	OriginalPath string
	DestPath     string
}

func (lib *Library) Import(sourceDir string, doCopy bool) error {
	// Phase 1: Collect all source photo metadata and decide winners for each hash
	log.Println("Phase 1: Scanning source files and selecting candidates...")
	sourceFilePaths, err := WalkDir(sourceDir)
	if err != nil {
		return fmt.Errorf("failed to walk source directory %s: %w", sourceDir, err)
	}
	allSourcePhotoInfo := GetPhotos(sourceFilePaths) // From util/import.go

	hashToWinnerPhotoMeta := make(map[string]SourcePhotoInfo)
	for _, currentMeta := range allSourcePhotoInfo {
		if winnerMeta, exists := hashToWinnerPhotoMeta[currentMeta.Hash]; exists {
			if isFilenameBetter(currentMeta.Filename, winnerMeta.Filename) {
				hashToWinnerPhotoMeta[currentMeta.Hash] = currentMeta
			}
		} else {
			hashToWinnerPhotoMeta[currentMeta.Hash] = currentMeta
		}
	}
	log.Printf("Phase 1: Completed. Selected %d unique photos for processing.", len(hashToWinnerPhotoMeta))

	// Phase 2: Update database
	log.Println("Phase 2: Updating database...")
	tx, err := lib.db.Begin()
	if err != nil {
		return fmt.Errorf("failed to begin database transaction: %w", err)
	}
	defer tx.Rollback() // Rollback if not committed

	var filesToCopy []FileToCopy // Collect files that need copying for Phase 3

	dbProgressBar := bar.Default(int64(len(hashToWinnerPhotoMeta)), "Finalizing database entries")

	for _, winnerPhotoMeta := range hashToWinnerPhotoMeta {
		var photoID int64
		var existingFilenameDB string
		finalPhotoDestRelPath := winnerPhotoMeta.Created.Format("2006/01-02")

		queryErr := tx.QueryRow("SELECT id, filename FROM photos WHERE hash = ?", winnerPhotoMeta.Hash).Scan(&photoID, &existingFilenameDB)

		if queryErr == sql.ErrNoRows { // New photo, insert it
			res, execErr := tx.Exec("INSERT INTO photos (filename, relpath, filetype, created, hash) VALUES (?, ?, ?, ?, ?)",
				winnerPhotoMeta.Filename, finalPhotoDestRelPath, winnerPhotoMeta.Filetype, winnerPhotoMeta.Created, winnerPhotoMeta.Hash)
			if execErr != nil {
				return fmt.Errorf("inserting photo %s (hash %s): %w", winnerPhotoMeta.Filename, winnerPhotoMeta.Hash, execErr)
			}
			photoID, _ = res.LastInsertId()
			log.Printf("DB: Added new photo '%s' (ID: %d, Hash: %s)", winnerPhotoMeta.Filename, photoID, winnerPhotoMeta.Hash)
			if doCopy {
				filesToCopy = append(filesToCopy, FileToCopy{
					OriginalPath: winnerPhotoMeta.OriginalPath,
					DestPath:     filepath.Join(lib.root, finalPhotoDestRelPath, winnerPhotoMeta.Filename),
				})
			}
		} else if queryErr == nil { // Photo with this hash already exists
			if winnerPhotoMeta.Filename != existingFilenameDB { // Filename preference implies an update
				_, updateErr := tx.Exec("UPDATE photos SET filename = ?, relpath = ? WHERE id = ?",
					winnerPhotoMeta.Filename, finalPhotoDestRelPath, photoID)
				if updateErr != nil {
					return fmt.Errorf("updating photo ID %d to filename %s: %w", photoID, winnerPhotoMeta.Filename, updateErr)
				}

				// Important: Delete old sidecars as the photo identity (filename) changed
				_, deleteErr := tx.Exec("DELETE FROM sidecars WHERE photo_id = ?", photoID)
				if deleteErr != nil {
					return fmt.Errorf("deleting old sidecars for photo ID %d: %w", photoID, deleteErr)
				}
				log.Printf("DB: Updated photo ID %d from '%s' to '%s'. Old sidecars deleted.", photoID, existingFilenameDB, winnerPhotoMeta.Filename)
			} else {
				log.Printf("DB: Photo ID %d ('%s', Hash: %s) already matches preferred version.", photoID, winnerPhotoMeta.Filename, winnerPhotoMeta.Hash)
			}
			if doCopy { // Still need to ensure the winning file is copied, even if DB record didn't change filename
				filesToCopy = append(filesToCopy, FileToCopy{
					OriginalPath: winnerPhotoMeta.OriginalPath, // Original path of the WINNING file
					DestPath:     filepath.Join(lib.root, finalPhotoDestRelPath, winnerPhotoMeta.Filename),
				})
			}
		} else { // Other database error
			return fmt.Errorf("querying photo by hash %s: %w", winnerPhotoMeta.Hash, queryErr)
		}

		// Process sidecars for this definitive photo (photoID) using sidecars from winnerPhotoMeta
		for _, sidecarMeta := range winnerPhotoMeta.Sidecars {
			var existingSidecarID int
			var existingSidecarHash string
			sidecarDestRelPath := finalPhotoDestRelPath // Sidecars go in same relative path as photo

			errSidecar := tx.QueryRow("SELECT id, hash FROM sidecars WHERE photo_id = ? AND filename = ?", photoID, sidecarMeta.Filename).Scan(&existingSidecarID, &existingSidecarHash)

			if errSidecar == sql.ErrNoRows {
				_, execErr := tx.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)",
					photoID, sidecarMeta.Filename, sidecarDestRelPath, sidecarMeta.Filetype, sidecarMeta.Created, sidecarMeta.Modified, sidecarMeta.Hash)
				if execErr != nil {
					return fmt.Errorf("inserting sidecar %s for photo ID %d: %w", sidecarMeta.Filename, photoID, execErr)
				}
				if doCopy {
					filesToCopy = append(filesToCopy, FileToCopy{
						OriginalPath: sidecarMeta.OriginalPath,
						DestPath:     filepath.Join(lib.root, sidecarDestRelPath, sidecarMeta.Filename),
					})
				}
			} else if errSidecar == nil { // Sidecar exists, check if its content (hash) updated
				if existingSidecarHash != sidecarMeta.Hash {
					_, updateErr := tx.Exec("UPDATE sidecars SET hash = ?, modified = ?, relpath = ? WHERE id = ?", // Removed filetype/created as they are less likely to change if filename is same
						sidecarMeta.Hash, sidecarMeta.Modified, sidecarDestRelPath, existingSidecarID)
					if updateErr != nil {
						return fmt.Errorf("updating sidecar ID %d: %w", existingSidecarID, updateErr)
					}
					if doCopy { // Content changed, so re-copy
						filesToCopy = append(filesToCopy, FileToCopy{
							OriginalPath: sidecarMeta.OriginalPath,
							DestPath:     filepath.Join(lib.root, sidecarDestRelPath, sidecarMeta.Filename),
						})
					}
				} else { // Sidecar exists and hash is same, ensure it's on copy list if main photo was new/updated
					if doCopy { // Add to copy list to ensure it exists, Copy func can handle existing files
						filesToCopy = append(filesToCopy, FileToCopy{
							OriginalPath: sidecarMeta.OriginalPath,
							DestPath:     filepath.Join(lib.root, sidecarDestRelPath, sidecarMeta.Filename),
						})
					}
				}
			} else { // Other error checking sidecar
				return fmt.Errorf("querying sidecar %s for photo ID %d: %w", sidecarMeta.Filename, photoID, errSidecar)
			}
		}
		dbProgressBar.Add(1)
	}
	dbProgressBar.Finish()

	if err := tx.Commit(); err != nil {
		return fmt.Errorf("failed to commit database transaction: %w", err)
	}
	log.Println("Phase 2: Database update completed.")

	// Phase 3: Copy files if doCopy is true
	if doCopy {
		log.Println("Phase 3: Copying files to library...")
		// Deduplicate filesToCopy list (in case photo and sidecar point to same original file if logic error elsewhere, or multiple adds)
		// For now, simple copy; Copy function should be idempotent or handle existing files gracefully.
		seenDestPaths := make(map[string]bool)
		uniqueFilesToCopy := []FileToCopy{}
		for _, f := range filesToCopy {
			if !seenDestPaths[f.DestPath] {
				uniqueFilesToCopy = append(uniqueFilesToCopy, f)
				seenDestPaths[f.DestPath] = true
			}
		}

		copyBar := bar.Default(int64(len(uniqueFilesToCopy)), "Copying files")
		for _, f := range uniqueFilesToCopy {
			// Ensure destination directory exists
			if err := os.MkdirAll(filepath.Dir(f.DestPath), 0755); err != nil {
				log.Printf("Warning: Failed to create directory for %s: %v. Skipping copy.", f.DestPath, err)
				copyBar.Add(1)
				continue
			}
			// Check if file already exists at destination and if hash matches (optional optimization)
			// For simplicity, current Copy overwrites.
			if err := Copy(f.OriginalPath, f.DestPath); err != nil {
				log.Printf("Warning: Failed to copy file from %s to %s: %v", f.OriginalPath, f.DestPath, err)
			}
			copyBar.Add(1)
		}
		copyBar.Finish()
		log.Println("Phase 3: File copying completed.")
	} else {
		log.Println("Phase 3: File copying skipped (doCopy is false).")
	}

	return nil
}

// UpdateDB, GetPhotos, GetPhotoCount, SyncFrom need to be reviewed and potentially refactored
// to align with the new SourcePhotoInfo and three-phase import logic,
// especially if they also involve adding or deciding on "winning" files.
// For now, their previous versions are kept below.

// GetPhotos (kept for now, but might need to change if DB representation is preferred)
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
		if err := rows.Scan(&pID, &pFilename, &pRelpath, &pFiletype, &pCreated, &pHash, &sID, &sFilename, &sRelpath, &sFiletype, &sCreated, &sModified, &sHash); err != nil {
			return nil, fmt.Errorf("scanning photo/sidecar row: %w", err)
		}
		photo, ok := photosMap[pID]
		if !ok {
			photo = Photo{ID: pID, Filename: pFilename, Path: filepath.Join(lib.root, pRelpath, pFilename), Filetype: pFiletype, Created: pCreated, Hash: pHash, Sidecars: []Sidecar{}}
		}
		if sID.Valid {
			sidecar := Sidecar{ID: int(sID.Int64), PhotoID: pID, Filename: sFilename.String, Path: filepath.Join(lib.root, sRelpath.String, sFilename.String), Filetype: sFiletype.String, Created: sCreated.Time, Modified: sModified.Time, Hash: sHash.String}
			isDuplicate := false
			for _, sc := range photo.Sidecars {
				if sc.ID == sidecar.ID {
					isDuplicate = true
					break
				}
			}
			if !isDuplicate {
				photo.Sidecars = append(photo.Sidecars, sidecar)
			}
		}
		photosMap[pID] = photo
	}
	if err = rows.Err(); err != nil {
		return nil, fmt.Errorf("iteration error: %w", err)
	}
	return photosMap, nil
}

func (lib *Library) GetPhotoCount() (int, error) {
	var count int
	if err := lib.db.QueryRow("SELECT COUNT(*) FROM photos").Scan(&count); err != nil {
		return 0, fmt.Errorf("querying photo count: %w", err)
	}
	return count, nil
}

// UpdateDB - Needs review. If it calls Import(lib.root, false), that Import is now the new one.
// The primary goal of UpdateDB was culling and hash-checking files *already in the library structure*.
// This new Import is more for adding from an external source.
// A separate, simpler UpdateDB might be needed for just cleaning library based on files on disk.
func (lib *Library) UpdateDB() error {
	log.Println("UpdateDB: Starting library update process...")
	// Current UpdateDB culls then calls Import(lib.root, false).
	// The culling part is fine.
	// The Import(lib.root, false) will now use the 3-phase logic.
	// With doCopy=false, Phase 3 will effectively just log that copying is skipped.
	// This might be acceptable, or UpdateDB needs a more tailored "rescan-in-place" logic.

	// For now, let's keep original culling logic and let the new Import handle additions/updates.
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
			return fmt.Errorf("UpdateDB: scanning photo for culling: %w", err)
		}
		photosToCull = append(photosToCull, struct {
			id   int
			path string
		}{id, filepath.Join(lib.root, relpath, filename)})
	}
	photoRows.Close()
	cullBarP := bar.Default(int64(len(photosToCull)), "UpdateDB: Culling photos")
	for _, p := range photosToCull {
		if _, statErr := os.Stat(p.path); os.IsNotExist(statErr) {
			if _, execErr := txCull.Exec("DELETE FROM photos WHERE id = ?", p.id); execErr != nil {
				txCull.Rollback()
				return fmt.Errorf("UpdateDB: deleting photo ID %d: %w", p.id, execErr)
			}
		}
		cullBarP.Add(1)
	}
	cullBarP.Finish()

	sidecarRows, err := txCull.Query("SELECT id, relpath, filename, hash FROM sidecars")
	if err != nil {
		txCull.Rollback()
		return fmt.Errorf("UpdateDB: querying sidecars: %w", err)
	}
	type scCheck struct {
		id     int
		path   string
		dbHash string
	}
	var sidecarsToCheck []scCheck
	for sidecarRows.Next() {
		var id int
		var relpath, filename, dbHash string
		if err := sidecarRows.Scan(&id, &relpath, &filename, &dbHash); err != nil {
			sidecarRows.Close()
			txCull.Rollback()
			return fmt.Errorf("UpdateDB: scanning sidecar: %w", err)
		}
		sidecarsToCheck = append(sidecarsToCheck, scCheck{id, filepath.Join(lib.root, relpath, filename), dbHash})
	}
	sidecarRows.Close()
	cullBarS := bar.Default(int64(len(sidecarsToCheck)), "UpdateDB: Culling/Updating sidecars")
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
				log.Printf("Warning: UpdateDB: Could not hash sidecar %s: %v", sc.path, hashErr)
			} else if currentFileHash != sc.dbHash {
				if _, execErr := txCull.Exec("UPDATE sidecars SET hash = ?, modified = ? WHERE id = ?", currentFileHash, fileInfo.ModTime(), sc.id); execErr != nil {
					txCull.Rollback()
					return fmt.Errorf("UpdateDB: updating sidecar ID %d: %w", sc.id, execErr)
				}
			}
		}
		cullBarS.Add(1)
	}
	cullBarS.Finish()
	if err := txCull.Commit(); err != nil {
		return fmt.Errorf("UpdateDB: failed to commit culling: %w", err)
	}
	log.Println("UpdateDB: Culling phase complete.")

	// The Import called by UpdateDB should not try to re-copy files that are already in lib.root.
	// The new Import with doCopy=false will skip Phase 3 copying.
	// It will still rescan metadata from lib.root and update DB if necessary.
	log.Println("UpdateDB: Rescanning library for new/changed metadata (no file copy)...")
	if err := lib.Import(lib.root, false); err != nil { // doCopy is false
		return fmt.Errorf("UpdateDB: failed during library rescan/import phase: %w", err)
	}
	log.Println("UpdateDB: Library update process finished.")
	return nil
}

// SyncFrom - This function will require a similar three-phase refactor
// to correctly decide on "winning" files from the sourceLib and then copy them.
// The current implementation might lead to issues similar to the old Import.
// For now, it's kept as is but marked for future refactoring.
func (lib *Library) SyncFrom(sourceLib *Library) error {
	log.Println("WARNING: SyncFrom function has not been fully updated to the new three-phase logic and may exhibit previous file handling bugs. Refactoring needed.")
	tx, err := lib.db.Begin()
	if err != nil {
		return fmt.Errorf("SyncFrom: failed to begin transaction: %w", err)
	}
	defer tx.Rollback()

	photosFromSource, err := sourceLib.GetPhotos()
	if err != nil {
		return fmt.Errorf("SyncFrom: failed to get photos from source: %w", err)
	}
	syncBar := bar.Default(int64(len(photosFromSource)), "Syncing photos (legacy method)")

	for _, sourcePhoto := range photosFromSource {
		var targetPhotoID int
		var targetFilenameDB string
		queryErr := tx.QueryRow("SELECT id, filename FROM photos WHERE hash = ?", sourcePhoto.Hash).Scan(&targetPhotoID, &targetFilenameDB)
		photoIDForSidecarProcessing := 0
		finalTargetFilename := ""

		if queryErr == sql.ErrNoRows {
			targetPhotoDateDir := sourcePhoto.Created.Format("2006/01-02")
			targetPhotoPath := filepath.Join(lib.root, targetPhotoDateDir, sourcePhoto.Filename) // Filename from sourcePhoto
			if err := Copy(sourcePhoto.Path, targetPhotoPath); err != nil {                      // Problem: sourcePhoto.Path here is library path
				log.Printf("Warning: SyncFrom (legacy): Failed to copy photo %s: %v.", sourcePhoto.Path, err)
				syncBar.Add(1)
				continue
			}
			res, execErr := tx.Exec("INSERT INTO photos (filename, relpath, filetype, created, hash) VALUES (?, ?, ?, ?, ?)",
				sourcePhoto.Filename, targetPhotoDateDir, sourcePhoto.Filetype, sourcePhoto.Created, sourcePhoto.Hash)
			if execErr != nil {
				return fmt.Errorf("SyncFrom (legacy): inserting photo %s: %w", sourcePhoto.Filename, execErr)
			}
			id, _ := res.LastInsertId()
			photoIDForSidecarProcessing = int(id)
			finalTargetFilename = sourcePhoto.Filename
		} else if queryErr == nil {
			photoIDForSidecarProcessing = targetPhotoID // Use existing ID
			if isFilenameBetter(sourcePhoto.Filename, targetFilenameDB) {
				targetPhotoDateDir := sourcePhoto.Created.Format("2006/01-02")
				// Copy the better file version before updating DB
				targetPhotoPath := filepath.Join(lib.root, targetPhotoDateDir, sourcePhoto.Filename)
				if err := Copy(sourcePhoto.Path, targetPhotoPath); err != nil {
					log.Printf("Warning: SyncFrom (legacy): Failed to copy preferred photo file %s: %v.", sourcePhoto.Path, err) // Continue with DB update?
				}

				_, updateErr := tx.Exec("UPDATE photos SET filename = ?, relpath = ? WHERE id = ?", sourcePhoto.Filename, targetPhotoDateDir, targetPhotoID)
				if updateErr != nil {
					return fmt.Errorf("SyncFrom (legacy): updating target photo ID %d: %w", targetPhotoID, updateErr)
				}
				_, deleteErr := tx.Exec("DELETE FROM sidecars WHERE photo_id = ?", targetPhotoID)
				if deleteErr != nil {
					return fmt.Errorf("SyncFrom (legacy): deleting old sidecars for ID %d: %w", targetPhotoID, deleteErr)
				}
				finalTargetFilename = sourcePhoto.Filename
			} else {
				finalTargetFilename = targetFilenameDB // Keep existing target filename
			}
		} else {
			return fmt.Errorf("SyncFrom (legacy): querying target for photo hash %s: %w", sourcePhoto.Hash, queryErr)
		}

		// Simplified sidecar sync for legacy version
		if photoIDForSidecarProcessing > 0 {
			targetSidecarDateDir := sourcePhoto.Created.Format("2006/01-02")
			for _, sidecarToSync := range sourcePhoto.Sidecars {
				targetSidecarPath := filepath.Join(lib.root, targetSidecarDateDir, sidecarToSync.Filename)
				// Check if sidecar exists for this photo_id and filename
				var tempSID int
				errSC := tx.QueryRow("SELECT id FROM sidecars WHERE photo_id = ? AND filename = ?", photoIDForSidecarProcessing, sidecarToSync.Filename).Scan(&tempSID)
				if errSC == sql.ErrNoRows { // Insert
					if errCopySC := Copy(sidecarToSync.Path, targetSidecarPath); errCopySC != nil {
						log.Printf("Warning: SyncFrom (legacy): Failed to copy new sidecar %s: %v", sidecarToSync.Path, errCopySC)
						continue
					}
					_, insErr := tx.Exec("INSERT INTO sidecars (photo_id, filename, relpath, filetype, created, modified, hash) VALUES (?, ?, ?, ?, ?, ?, ?)",
						photoIDForSidecarProcessing, sidecarToSync.Filename, targetSidecarDateDir, sidecarToSync.Filetype, sidecarToSync.Created, sidecarToSync.Modified, sidecarToSync.Hash)
					if insErr != nil {
						return fmt.Errorf("SyncFrom (legacy): inserting sidecar %s for photo %s: %w", sidecarToSync.Filename, finalTargetFilename, insErr)
					}
				} else if errSC == nil { // Exists, potentially update hash/content
					var currentTargetSCHash string
					_ = tx.QueryRow("SELECT hash FROM sidecars WHERE id = ?", tempSID).Scan(&currentTargetSCHash) // Error check omitted for brevity
					if currentTargetSCHash != sidecarToSync.Hash {
						if errCopySC := Copy(sidecarToSync.Path, targetSidecarPath); errCopySC != nil {
							log.Printf("Warning: SyncFrom (legacy): Failed to copy updated sidecar %s: %v", sidecarToSync.Path, errCopySC)
							continue
						}
						_, updErr := tx.Exec("UPDATE sidecars SET hash=?, modified=? WHERE id=?", sidecarToSync.Hash, sidecarToSync.Modified, tempSID)
						if updErr != nil {
							return fmt.Errorf("SyncFrom (legacy): updating sidecar %s for photo %s: %w", sidecarToSync.Filename, finalTargetFilename, updErr)
						}
					}
				} // else other DB error on sidecar check
			}
		}
		syncBar.Add(1)
	}
	syncBar.Finish()
	return tx.Commit()
}