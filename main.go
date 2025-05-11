package main

import (
	"fmt"
	"log"
	"os"

	"github.com/bleemesser/photosort/util"
)

func main() {
	// Configure logger for more structured output
	log.SetFlags(log.Ldate | log.Ltime | log.Lshortfile)

	args, err := util.NewArgs(os.Args)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Argument error: %v\nRun 'photosort help' for usage.\n", err)
		os.Exit(1)
	}

	// fmt.Println(args) // Keep for debugging if needed, or remove for cleaner output

	switch args.Action {
	case "create":
		libPath := args.GetDir(0)
		fmt.Printf("Creating new library at: %s\n", libPath)
		_, err := util.CreateLibrary(libPath)
		if err != nil {
			log.Fatalf("Failed to create library: %v", err)
		}
		fmt.Println("Library created successfully.")
	case "import":
		photoDir := args.GetDir(0)
		libPath := args.GetDir(1)
		fmt.Printf("Importing photos from %s into library %s\n", photoDir, libPath)

		lib, err := util.OpenLibrary(libPath)
		if err != nil {
			log.Fatalf("Failed to open library %s: %v", libPath, err)
		}
		defer func() {
			if err := lib.Close(); err != nil {
				log.Printf("Error closing library %s: %v", libPath, err)
			}
		}()

		err = lib.Import(photoDir, true) // true for doCopy
		if err != nil {
			log.Fatalf("Failed to import photos from %s: %v", photoDir, err)
		}

		count, countErr := lib.GetPhotoCount()
		if countErr != nil {
			log.Printf("Warning: Failed to get photo count from library %s: %v", libPath, countErr)
		} else {
			fmt.Printf("Import complete. Library %s now has %d photos.\n", libPath, count)
		}

	case "update":
		libPath := args.GetDir(0)
		fmt.Printf("Updating library: %s\n", libPath)

		lib, err := util.OpenLibrary(libPath)
		if err != nil {
			log.Fatalf("Failed to open library %s: %v", libPath, err)
		}
		defer func() {
			if err := lib.Close(); err != nil {
				log.Printf("Error closing library %s: %v", libPath, err)
			}
		}()

		err = lib.UpdateDB()
		if err != nil {
			log.Fatalf("Failed to update library %s: %v", libPath, err)
		}

		count, countErr := lib.GetPhotoCount()
		if countErr != nil {
			log.Printf("Warning: Failed to get photo count from library %s: %v", libPath, countErr)
		} else {
			fmt.Printf("Update complete. Library %s now has %d photos.\n", libPath, count)
		}
	case "sync":
		handleSync(args)
	case "debug":
		fmt.Println("Debug action called. Parsed arguments:")
		fmt.Println(args)
		// Add any specific debug logic here if needed.
	default:
		// This case should ideally be caught by NewArgs/validateArgs,
		// but as a fallback:
		if args.Action != "help" { // help action exits on its own
			fmt.Fprintf(os.Stderr, "Unknown action: %s\nRun 'photosort help' for usage.\n", args.Action)
			os.Exit(1)
		}
	}
}

func handleSync(args util.Args) {
	sourceLibPath := args.GetDir(0)
	targetLibPath := args.GetDir(1)

	fmt.Printf("Opening source library: %s\n", sourceLibPath)
	libSource, err := util.OpenLibrary(sourceLibPath)
	if err != nil {
		log.Fatalf("Failed to open source library %s: %v", sourceLibPath, err)
	}
	defer func() {
		if err := libSource.Close(); err != nil {
			log.Printf("Error closing source library %s: %v", sourceLibPath, err)
		}
	}()

	fmt.Printf("Opening target library: %s\n", targetLibPath)
	libTarget, err := util.OpenLibrary(targetLibPath)
	if err != nil {
		log.Fatalf("Failed to open target library %s: %v", targetLibPath, err)
	}
	defer func() {
		if err := libTarget.Close(); err != nil {
			log.Printf("Error closing target library %s: %v", targetLibPath, err)
		}
	}()

	fmt.Println("Updating source library before sync...")
	if err := libSource.UpdateDB(); err != nil {
		log.Fatalf("Failed to update source library %s: %v", sourceLibPath, err)
	}

	fmt.Println("Updating target library before sync...")
	if err := libTarget.UpdateDB(); err != nil {
		log.Fatalf("Failed to update target library %s: %v", targetLibPath, err)
	}

	fmt.Printf("Syncing photos from %s to %s...\n", sourceLibPath, targetLibPath)
	if err := libTarget.SyncFrom(libSource); err != nil { // Sync libSource into libTarget
		log.Fatalf("Failed to sync libraries: %v", err)
	}

	// It's good practice to run UpdateDB on the target again to ensure full consistency
	// especially if SyncFrom might have edge cases or if files were manipulated externally during sync.
	fmt.Println("Updating target library after sync to ensure consistency...")
	if err := libTarget.UpdateDB(); err != nil {
		log.Fatalf("Failed to update target library %s post-sync: %v", targetLibPath, err)
	}

	count, countErr := libTarget.GetPhotoCount()
	if countErr != nil {
		log.Printf("Warning: Failed to get photo count from target library %s post-sync: %v", targetLibPath, countErr)
	} else {
		fmt.Printf("Sync complete. Target library %s now has %d photos.\n", targetLibPath, count)
	}
}
