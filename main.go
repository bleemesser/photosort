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
		fmt.Println(err)
		os.Exit(1)
	}

	fmt.Println(args)
}