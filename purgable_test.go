package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func writeFile(t *testing.T, path string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestExactFilenameMatching(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "PURGABLE"))
	writeFile(t, filepath.Join(dir, "PURGABLE.txt"))
	writeFile(t, filepath.Join(dir, "purgable"))
	writeFile(t, filepath.Join(dir, "PURGABLE.old"))

	// A symlink named PURGABLE must NOT match.
	if err := os.Symlink(filepath.Join(dir, "purgable"), filepath.Join(dir, "PURGABLE.link")); err == nil {
		// symlink creation may fail on some platforms; ignore.
	}

	matches, err := Find(dir, os.Stderr)
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("expected 1 match, got %d: %v", len(matches), matches)
	}
	if filepath.Base(matches[0]) != "PURGABLE" {
		t.Fatalf("matched wrong file: %s", matches[0])
	}
}

func TestRecursiveDiscovery(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "PURGABLE"))
	writeFile(t, filepath.Join(dir, "a", "PURGABLE"))
	writeFile(t, filepath.Join(dir, "a", "b", "c", "PURGABLE"))
	writeFile(t, filepath.Join(dir, "a", "b", "other.txt"))

	matches, err := Find(dir, os.Stderr)
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 3 {
		t.Fatalf("expected 3 matches, got %d: %v", len(matches), matches)
	}
}

func TestConfirmingDeletion(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "PURGABLE")
	writeFile(t, target)

	stats, err := Purge(dir, strings.NewReader("y\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 1 || stats.Deleted != 1 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	if _, err := os.Stat(target); !os.IsNotExist(err) {
		t.Fatalf("expected file to be deleted")
	}
}

func TestDecliningDeletion(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "PURGABLE")
	writeFile(t, target)

	stats, err := Purge(dir, strings.NewReader("n\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 1 || stats.Deleted != 0 || stats.Skipped != 1 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	if _, err := os.Stat(target); err != nil {
		t.Fatalf("expected file to remain: %v", err)
	}
}

func TestDefaultNoOnEmptyInput(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "PURGABLE")
	writeFile(t, target)

	stats, err := Purge(dir, strings.NewReader("\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Deleted != 0 || stats.Skipped != 1 {
		t.Fatalf("expected skip on empty input: %+v", stats)
	}
}

func TestMultiplePurgableFiles(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "PURGABLE"))
	writeFile(t, filepath.Join(dir, "sub", "PURGABLE"))
	writeFile(t, filepath.Join(dir, "sub", "deep", "PURGABLE"))

	// "y" for first, "n" for second, "y" for third.
	stats, err := Purge(dir, strings.NewReader("y\nn\ny\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 3 || stats.Deleted != 2 || stats.Skipped != 1 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
}

func TestSymlinksNotFollowed(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "PURGABLE"))
	sub := filepath.Join(dir, "sub")
	writeFile(t, filepath.Join(sub, "PURGABLE"))

	// Symlink to a directory containing a PURGABLE: must not be descended.
	if err := os.Symlink(sub, filepath.Join(dir, "link")); err != nil {
		t.Skipf("symlinks unsupported: %v", err)
	}

	matches, err := Find(dir, os.Stderr)
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 2 {
		t.Fatalf("expected 2 matches (symlink dir not followed), got %d: %v", len(matches), matches)
	}
}

func TestMissingRootDirectory(t *testing.T) {
	stats, err := Purge(filepath.Join(t.TempDir(), "does-not-exist"), strings.NewReader(""), os.Stdout, os.Stderr, os.Remove)
	if err == nil {
		t.Fatalf("expected error for missing root, got stats %+v", stats)
	}
	if stats.Found != 0 {
		t.Fatalf("expected no work done, got %+v", stats)
	}
}
