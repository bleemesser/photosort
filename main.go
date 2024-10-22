package main

import (
	// "encoding/json"
	"fmt"
	"os"
	// "os/exec"
	// "path/filepath"
	// "strconv"
	// "strings"
	// "time"
	// "database/sql"

	// bar "github.com/schollz/progressbar/v3"

	// exif "github.com/barasher/go-exiftool"

	// _ "github.com/glebarez/go-sqlite"

	"github.com/bleemesser/photosort/util"
)

func main() {
	args, err := util.NewArgs(os.Args)
	if err != nil {
		fmt.Println(err, "\nRun 'photosort help' for usage.")
		os.Exit(1)
	}

	fmt.Println(args)
	switch args.Action {
	case "create":
		// Create a new library, which will also create the directory if it doesn't exist
		_, err := util.CreateLibrary(args.GetDir(0))
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		fmt.Println("Library created.")
	case "import":
		// Open existing library
		lib, err := util.OpenLibrary(args.GetDir(1))
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		defer lib.Close()
		// call the import function which will copy all the photos and sidecars and add them to the library
		err = lib.Import(args.GetDir(0))
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		// print log
		count, err := lib.GetPhotoCount()
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		fmt.Println("Library now has " + fmt.Sprint(count) + " photos.")
		
	case "update":
		// Open existing library
		lib, err := util.OpenLibrary(args.GetDir(0))
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		defer lib.Close()
		// call the update function which will update the library if any files have been removed
		err = lib.UpdateDB()
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		// print log
		count, err := lib.GetPhotoCount()
		if err != nil {
			fmt.Println(err)
			os.Exit(1)
		}
		fmt.Println("Library now has " + fmt.Sprint(count) + " photos.")
	}
}