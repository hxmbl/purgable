package main

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
)

const target = "PURGABLE"

// Stats tracks what happened during a run.
type Stats struct {
	Found   int
	Deleted int
	Skipped int
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
// paths of every regular file whose basename is exactly "PURGABLE".
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
			matches = append(matches, path)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	return matches, nil
}

// Purge finds PURGABLE files under root, asks for confirmation on each (via
// in), and deletes confirmed files using remove. It returns a summary.
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
		fmt.Fprintln(out, "No PURGABLE files found.")
		return stats, nil
	}

	reader := bufio.NewReader(in)
	for _, p := range matches {
		fmt.Fprintf(out, "Delete %s? [y/N] ", p)
		line, rerr := reader.ReadString('\n')
		if rerr != nil && rerr != io.EOF {
			return stats, fmt.Errorf("reading input: %w", rerr)
		}
		answer := strings.ToLower(strings.TrimSpace(line))
		if answer == "y" {
			if derr := remove(p); derr != nil {
				fmt.Fprintf(out, "  failed to delete %s: %v\n", p, derr)
				stats.Skipped++
			} else {
				stats.Deleted++
			}
		} else {
			stats.Skipped++
		}
	}
	return stats, nil
}
