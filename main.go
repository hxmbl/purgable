package main

import (
	"fmt"
	"os"
)

func usage() {
	fmt.Fprint(os.Stderr, `purgable - find directories marked with PURGABLE and act on them

Usage:
  purgable <directory>
  purgable --help | -h

Recursively scans <directory> for regular files named exactly "PURGABLE"
(case-sensitive, no extension). Each such file marks its containing directory
as purgable. For each marked directory, the tool presents an action prompt.

The PURGABLE marker itself is NEVER independently deleted, modified, or
shredded. It is removed only as a consequence of its containing directory
being deleted or shredded.

Actions:
  d        Delete the containing directory and everything in it.
  s        Shred (secure-delete) the contents of the containing directory.
           Note: shredding cannot guarantee physical destruction on SSDs,
           flash storage, or filesystems with copy-on-write/snapshots.
  k        Skip this directory and continue scanning.
  e        Exit immediately.

  d-ALL    Delete this and all subsequent PURGABLE directories without prompting.
  s-ALL    Shred this and all subsequent PURGABLE directories without prompting.
  k-ALL    Skip this and all subsequent PURGABLE directories without prompting.
  e-ALL    Exit immediately (equivalent to e).

Options:
  -h, --help   Show this help and exit.

Exit codes:
  0  completed (regardless of actions taken)
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

	fmt.Printf("\nDone. Found %d, deleted %d, shredded %d, skipped %d.\n",
		stats.Found, stats.Deleted, stats.Shredded, stats.Skipped)
}