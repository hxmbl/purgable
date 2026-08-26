package main

import (
	"fmt"
	"os"
)

func usage() {
	fmt.Fprint(os.Stderr, `purgable - find and delete files named exactly "PURGABLE"

Usage:
  purgable <directory>
  purgable --help | -h

Recursively scans <directory> for regular files whose name is exactly
"PURGABLE" (case-sensitive, no extension). For each one found it asks for
confirmation before deleting. Symlinks are never followed.

Options:
  -h, --help   Show this help and exit.

Exit codes:
  0  completed (regardless of how many files were deleted/skipped)
  1  the root directory does not exist or cannot be accessed
  2  invalid usage
`)
}

func main() {
	args := os.Args[1:]
	for _, a := range args {
		if a == "--help" || a == "-h" {
			usage()
			return
		}
	}

	if len(args) != 1 {
		usage()
		os.Exit(2)
	}

	stats, err := Purge(args[0], os.Stdin, os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("\nDone. Found %d, deleted %d, skipped %d.\n",
		stats.Found, stats.Deleted, stats.Skipped)
}
