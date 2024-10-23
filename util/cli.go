package util

import (
	"errors"
	"fmt"
	"os"

	"path/filepath"
	"regexp"
	"strings"
)

type Args struct {
	Action string
	flags  map[string]string
	dirs   []string
}

func (a Args) String() string {
	return fmt.Sprintf("Action: %s\nFlags: %v\nDirs: %v", a.Action, a.flags, a.dirs)
}

func (a Args) GetFlag(key string) string {
	return a.flags[key]
}

func (a Args) GetDir(index int) string {
	return a.dirs[index]
}

func (a Args) GetDirs() []string {
	return a.dirs
}

func NewArgs(args []string) (Args, error) {
	a := formatArgs(args)
	a, err := validateArgs(a)
	if err != nil {
		return Args{}, err
	}
	return a, nil
}

// ./[program] [action] [--flag1=value1] [--flag2=value2] [dir1] [dir2] ... [dirN]
func formatArgs(args []string) Args {
	var a Args
	a.flags = make(map[string]string)

	if len(args) > 1 {
		a.Action = args[1]
	}
	if len(args) > 2 {
		match := regexp.MustCompile("--.*=.*")
		for _, arg := range args[2:] {
			if match.MatchString(arg) {
				split := strings.Split(arg, "=")
				// combine split 1:n into a single string
				for i := 2; i < len(split); i++ {
					split[1] += "=" + split[i]
				}
				a.flags[split[0][2:]] = split[1]
			} else {
				a.dirs = append(a.dirs, arg)
			}
		}
	}


	return a
}

// actions: import, create, update, sync
// requirements: import: 2 existing directories
//               create: 1 directory
//               update: 1 existing directory
//               sync: 2 existing directories
// this function will also convert relative paths to absolute paths
func validateArgs(a Args) (Args, error) {
	var e string
	if a.Action == "" {
		e = "No action specified"

	}
	
	switch a.Action {
	case "import":
		if len(a.dirs) != 2 {
			e = "Incorrect number of directories for import"
			break
		}
		for i, dir := range a.dirs {
			if _, err := os.Stat(dir); os.IsNotExist(err) {
				e = "Directory " + dir + " does not exist"
				break
			}
			a.dirs[i], _ = filepath.Abs(dir)
		}
	case "create":
		if len(a.dirs) != 1 {
			e = "Incorrect number of directories for create"
			break
		}
		a.dirs[0], _ = filepath.Abs(a.dirs[0])
	case "update":
		if len(a.dirs) != 1 {
			e = "Incorrect number of directories for update"
			break
		}
		if _, err := os.Stat(a.dirs[0]); os.IsNotExist(err) {
			e = "Directory " + a.dirs[0] + " does not exist"
		}
		a.dirs[0], _ = filepath.Abs(a.dirs[0])
	case "sync":
		if len(a.dirs) != 2 {
			e = "Incorrect number of directories for sync"
			break
		}
		for i, dir := range a.dirs {
			if _, err := os.Stat(dir); os.IsNotExist(err) {
				e = "Directory " + dir + " does not exist"
				break
			}
			a.dirs[i], _ = filepath.Abs(dir)
		}
	case "help":
		fmt.Println("Usage: photosort import <photo_dir> <library_dir>")
		fmt.Println("Usage: photosort create <library_dir>")
		fmt.Println("Usage: photosort sync <library_dir1> <library_dir2>")
		fmt.Println("Usage: photosort update <library_dir>")
		os.Exit(0)
	case "debug":
		
	default:
		e = "Invalid action specified"
	}

	if e != "" {
		return Args{}, errors.New(e)
	}
	return a, nil
}
