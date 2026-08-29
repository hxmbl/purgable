package main

import (
	"bufio"
	"crypto/rand"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
)

var shredBufPool = sync.Pool{New: func() any {
	b := make([]byte, 64*1024)
	return &b
}}

const target = "PURGABLE"

// Stats tracks what happened during a run.
type Stats struct {
	Found    int
	Deleted  int
	Shredded int
	Skipped  int
}

// Action represents a user choice for a purgable directory.
type Action int

const (
	ActionDelete Action = iota
	ActionShred
	ActionSkip
	ActionExit
)

// ActionAll applies the same action to all remaining matches without prompting.
type ActionAll struct {
	Action Action
	All    bool
}

// ValidateRoot ensures the supplied root exists, is accessible, and is a
// directory. It returns a clear error otherwise.
func ValidateRoot(root string) (os.FileInfo, error) {
	info, err := os.Stat(root)
	if err != nil {
		return nil, fmt.Errorf("cannot access %q: %w", root, err)
	}
	if !info.IsDir() {
		return nil, fmt.Errorf("%q is not a directory", root)
	}
	return info, nil
}

// Find walks root recursively (without following symlinks) and returns the
// paths of directories containing a regular file named exactly "PURGABLE".
//
// Filesystem errors encountered while traversing are reported to w and do not
// abort the walk.
func Find(root string, w io.Writer) ([]string, error) {
	var matches []string
	err := filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			fmt.Fprintf(w, "warning: skipping %s: %v\n", path, err)
			return nil
		}
		// d.Type() is the Lstat type, so symlinks are never regular files and
		// symlinked directories are never descended into.
		if d.Type().IsRegular() && d.Name() == target {
			matches = append(matches, filepath.Dir(path))
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return matches, nil
}

// Purge finds PURGABLE-marked directories under root, asks for an action on each
// (via in), and performs the action using remove and shred. It returns a summary.
func Purge(root string, in io.Reader, out, warn io.Writer, remove func(string) error) (Stats, error) {
	if _, err := ValidateRoot(root); err != nil {
		return Stats{}, err
	}

	matches, err := Find(root, warn)
	if err != nil {
		return Stats{}, err
	}

	stats := Stats{Found: len(matches)}
	if len(matches) == 0 {
		fmt.Fprintln(out, "No PURGABLE directories found.")
		return stats, nil
	}

	reader := bufio.NewReader(in)
	var defaultAction *ActionAll

	for _, dir := range matches {
		var action Action
		var all bool

		if defaultAction != nil {
			action = defaultAction.Action
			all = defaultAction.All
		} else {
			fmt.Fprintf(out, "Action for %s [d/s/k/e]? ", dir)
			line, rerr := reader.ReadString('\n')
			if rerr != nil && rerr != io.EOF {
				return stats, fmt.Errorf("reading input: %w", rerr)
			}
			ans := strings.ToLower(strings.TrimSpace(line))
			if ans == "" {
				action = ActionSkip
			} else {
				var ok bool
				action, all, ok = parseAction(ans)
				if !ok {
					fmt.Fprintf(out, "  invalid action %q, skipping\n", ans)
					action = ActionSkip
				}
			}
		}

		if all {
			defaultAction = &ActionAll{Action: action, All: true}
		}

		switch action {
		case ActionDelete:
			if derr := removeDir(dir); derr != nil {
				fmt.Fprintf(out, "  failed to delete %s: %v\n", dir, derr)
				stats.Skipped++
			} else {
				stats.Deleted++
			}
		case ActionShred:
			if serr := shredDir(dir); serr != nil {
				fmt.Fprintf(out, "  failed to shred %s: %v\n", dir, serr)
				stats.Skipped++
			} else {
				stats.Shredded++
			}
		case ActionSkip:
			stats.Skipped++
		case ActionExit:
			return stats, nil
		}
	}
	return stats, nil
}

// parseAction parses the user input into an Action and whether -ALL was specified.
// It returns ok=false for unrecognised input.
func parseAction(s string) (action Action, all, ok bool) {
	if strings.HasSuffix(s, "-all") {
		all = true
		s = strings.TrimSuffix(s, "-all")
	}
	switch s {
	case "d":
		return ActionDelete, all, true
	case "s":
		return ActionShred, all, true
	case "k":
		return ActionSkip, all, true
	case "e":
		return ActionExit, all, true
	default:
		return 0, false, false
	}
}

// removeDir removes the directory and all its contents.
func removeDir(path string) error {
	return os.RemoveAll(path)
}

// shredDir securely deletes the contents of the directory.
// Note: This cannot guarantee physical destruction on SSDs, flash storage,
// or filesystems with copy-on-write/snapshots. It overwrites regular files
// with random data before removal. The PURGABLE marker file is never shredded.
func shredDir(path string) error {
	return filepath.WalkDir(path, func(p string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		// Skip the directory itself at the top level
		if p == path {
			return nil
		}
		// Never shred the PURGABLE marker
		if filepath.Base(p) == target {
			return nil
		}
		info, err := d.Info()
		if err != nil {
			return err
		}
		if info.Mode().IsRegular() {
			if err := shredFile(p); err != nil {
				return err
			}
		}
		return nil
	})
}

// shredFile overwrites a regular file with random data then removes it.
func shredFile(path string) error {
	f, err := os.OpenFile(path, os.O_WRONLY, 0)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return err
	}
	defer f.Close()

	size, err := f.Seek(0, io.SeekEnd)
	if err != nil {
		return err
	}
	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return err
	}

	bufp := shredBufPool.Get().(*[]byte)
	buf := *bufp
	written := int64(0)
	for written < size {
		n := int64(len(buf))
		if written+n > size {
			n = size - written
		}
		if _, err := rand.Read(buf[:n]); err != nil {
			shredBufPool.Put(bufp)
			return err
		}
		if _, err := f.Write(buf[:n]); err != nil {
			shredBufPool.Put(bufp)
			return err
		}
		written += n
	}
	shredBufPool.Put(bufp)
	if err := f.Sync(); err != nil {
		return err
	}
	return os.Remove(path)
}