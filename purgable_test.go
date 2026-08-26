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

func writeDirWithContent(t *testing.T, dir string, files map[string]string) {
	t.Helper()
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	for name, content := range files {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
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
	if matches[0] != dir {
		t.Fatalf("expected match to be root dir %s, got %s", dir, matches[0])
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
	// All matches should be directories containing PURGABLE
	for _, m := range matches {
		if _, err := os.Stat(filepath.Join(m, "PURGABLE")); err != nil {
			t.Fatalf("match %s does not contain PURGABLE file", m)
		}
	}
}

func TestTargetIsContainingDirectory(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeFile(t, filepath.Join(sub, "PURGABLE"))

	matches, err := Find(dir, os.Stderr)
	if err != nil {
		t.Fatal(err)
	}
	if len(matches) != 1 {
		t.Fatalf("expected 1 match, got %d", len(matches))
	}
	if matches[0] != sub {
		t.Fatalf("expected target to be containing dir %s, got %s", sub, matches[0])
	}
}

func TestDeleteActionRemovesContainingDirectory(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeDirWithContent(t, sub, map[string]string{
		"PURGABLE": "",
		"data.txt": "secret",
		"other":    "file",
	})

	// Input: "d\n" for delete
	stats, err := Purge(dir, strings.NewReader("d\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 1 || stats.Deleted != 1 || stats.Shredded != 0 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	// The containing directory should be gone
	if _, err := os.Stat(sub); !os.IsNotExist(err) {
		t.Fatalf("expected containing directory %s to be deleted", sub)
	}
	// The PURGABLE marker should also be gone (as part of the directory)
	if _, err := os.Stat(filepath.Join(sub, "PURGABLE")); !os.IsNotExist(err) {
		t.Fatalf("expected PURGABLE marker to be deleted with directory")
	}
}

func TestSkipActionLeavesEverythingUntouched(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeDirWithContent(t, sub, map[string]string{
		"PURGABLE": "",
		"data.txt": "secret",
	})

	// Input: "k\n" for skip
	stats, err := Purge(dir, strings.NewReader("k\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 1 || stats.Deleted != 0 || stats.Shredded != 0 || stats.Skipped != 1 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	// Everything should remain
	if _, err := os.Stat(sub); err != nil {
		t.Fatalf("expected containing directory %s to remain: %v", sub, err)
	}
	if _, err := os.Stat(filepath.Join(sub, "PURGABLE")); err != nil {
		t.Fatalf("expected PURGABLE marker to remain: %v", err)
	}
	if _, err := os.Stat(filepath.Join(sub, "data.txt")); err != nil {
		t.Fatalf("expected data.txt to remain: %v", err)
	}
}

func TestShredBehavior(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeDirWithContent(t, sub, map[string]string{
		"PURGABLE": "",
		"data.txt": "secret content to shred",
	})

	// Input: "s\n" for shred
	stats, err := Purge(dir, strings.NewReader("s\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 1 || stats.Deleted != 0 || stats.Shredded != 1 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	// The directory should still exist (only contents shredded)
	if _, err := os.Stat(sub); err != nil {
		t.Fatalf("expected containing directory %s to remain after shred: %v", sub, err)
	}
	// The PURGABLE marker should remain (marker is never independently targeted)
	if _, err := os.Stat(filepath.Join(sub, "PURGABLE")); err != nil {
		t.Fatalf("expected PURGABLE marker to remain after shred: %v", err)
	}
	// The data file should be gone (shredded)
	if _, err := os.Stat(filepath.Join(sub, "data.txt")); !os.IsNotExist(err) {
		t.Fatalf("expected data.txt to be shredded (removed)")
	}
}

func TestDeleteAll(t *testing.T) {
	dir := t.TempDir()
	sub1 := filepath.Join(dir, "sub1")
	sub2 := filepath.Join(dir, "sub2")
	writeDirWithContent(t, sub1, map[string]string{"PURGABLE": "", "a": "1"})
	writeDirWithContent(t, sub2, map[string]string{"PURGABLE": "", "b": "2"})

	// Input: "d-ALL\n" for delete all
	stats, err := Purge(dir, strings.NewReader("d-ALL\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 2 || stats.Deleted != 2 || stats.Shredded != 0 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	if _, err := os.Stat(sub1); !os.IsNotExist(err) {
		t.Fatalf("expected sub1 deleted")
	}
	if _, err := os.Stat(sub2); !os.IsNotExist(err) {
		t.Fatalf("expected sub2 deleted")
	}
}

func TestShredAll(t *testing.T) {
	dir := t.TempDir()
	sub1 := filepath.Join(dir, "sub1")
	sub2 := filepath.Join(dir, "sub2")
	writeDirWithContent(t, sub1, map[string]string{"PURGABLE": "", "a": "1"})
	writeDirWithContent(t, sub2, map[string]string{"PURGABLE": "", "b": "2"})

	// Input: "s-ALL\n" for shred all
	stats, err := Purge(dir, strings.NewReader("s-ALL\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 2 || stats.Deleted != 0 || stats.Shredded != 2 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	// Directories should remain, contents shredded
	if _, err := os.Stat(sub1); err != nil {
		t.Fatalf("expected sub1 to remain after shred")
	}
	if _, err := os.Stat(sub2); err != nil {
		t.Fatalf("expected sub2 to remain after shred")
	}
	// PURGABLE markers should remain
	if _, err := os.Stat(filepath.Join(sub1, "PURGABLE")); err != nil {
		t.Fatalf("expected PURGABLE in sub1 to remain after shred")
	}
	if _, err := os.Stat(filepath.Join(sub2, "PURGABLE")); err != nil {
		t.Fatalf("expected PURGABLE in sub2 to remain after shred")
	}
	// Contents should be gone
	if _, err := os.Stat(filepath.Join(sub1, "a")); !os.IsNotExist(err) {
		t.Fatalf("expected a shredded")
	}
	if _, err := os.Stat(filepath.Join(sub2, "b")); !os.IsNotExist(err) {
		t.Fatalf("expected b shredded")
	}
}

func TestSkipAll(t *testing.T) {
	dir := t.TempDir()
	sub1 := filepath.Join(dir, "sub1")
	sub2 := filepath.Join(dir, "sub2")
	writeDirWithContent(t, sub1, map[string]string{"PURGABLE": "", "a": "1"})
	writeDirWithContent(t, sub2, map[string]string{"PURGABLE": "", "b": "2"})

	// Input: "k-ALL\n" for skip all
	stats, err := Purge(dir, strings.NewReader("k-ALL\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 2 || stats.Deleted != 0 || stats.Shredded != 0 || stats.Skipped != 2 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	if _, err := os.Stat(sub1); err != nil {
		t.Fatalf("expected sub1 to remain")
	}
	if _, err := os.Stat(sub2); err != nil {
		t.Fatalf("expected sub2 to remain")
	}
}

func TestExitBehavior(t *testing.T) {
	dir := t.TempDir()
	sub1 := filepath.Join(dir, "sub1")
	sub2 := filepath.Join(dir, "sub2")
	writeDirWithContent(t, sub1, map[string]string{"PURGABLE": "", "a": "1"})
	writeDirWithContent(t, sub2, map[string]string{"PURGABLE": "", "b": "2"})

	// Input: "e\n" for exit
	stats, err := Purge(dir, strings.NewReader("e\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 2 || stats.Deleted != 0 || stats.Shredded != 0 || stats.Skipped != 0 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	// Both should remain untouched
	if _, err := os.Stat(sub1); err != nil {
		t.Fatalf("expected sub1 to remain after exit")
	}
	if _, err := os.Stat(sub2); err != nil {
		t.Fatalf("expected sub2 to remain after exit")
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

func TestFilesystemErrors(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "PURGABLE"))
	// Create a directory we can't read - create PURGABLE first, then remove perms
	noRead := filepath.Join(dir, "noread")
	if err := os.Mkdir(noRead, 0o755); err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(noRead, "PURGABLE"))
	// Now make it unreadable
	if err := os.Chmod(noRead, 0o000); err != nil {
		t.Fatal(err)
	}
	defer os.Chmod(noRead, 0o755) // cleanup

	matches, err := Find(dir, os.Stderr)
	// Should still find the accessible ones, just warn about the unreadable
	if err != nil {
		t.Fatalf("Find should not return error on unreadable dir: %v", err)
	}
	// At minimum the root PURGABLE should be found
	found := false
	for _, m := range matches {
		if m == dir {
			found = true
			break
		}
	}
	if !found {
		t.Fatalf("expected root dir in matches: %v", matches)
	}
}

func TestMarkerNeverIndependentlyTargeted(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeDirWithContent(t, sub, map[string]string{
		"PURGABLE": "",
		"data.txt": "secret",
	})

	// Test delete - marker should not be passed to remove
	var err error
	_, err = Purge(dir, strings.NewReader("d\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	// The whole directory is gone, so marker is gone too
	if _, err := os.Stat(sub); !os.IsNotExist(err) {
		t.Fatalf("expected containing directory deleted")
	}

	// Test shred - marker should remain
	sub2 := filepath.Join(dir, "sub2")
	writeDirWithContent(t, sub2, map[string]string{
		"PURGABLE": "",
		"data.txt": "secret",
	})
	_, err = Purge(dir, strings.NewReader("s\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(sub2, "PURGABLE")); err != nil {
		t.Fatalf("expected PURGABLE marker to remain after shred")
	}
	if _, err := os.Stat(filepath.Join(sub2, "data.txt")); !os.IsNotExist(err) {
		t.Fatalf("expected data.txt to be shredded")
	}
}

func TestDefaultSkipOnEmptyInput(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "sub")
	writeDirWithContent(t, sub, map[string]string{"PURGABLE": "", "data.txt": "secret"})

	// Empty input (just newline) defaults to skip
	stats, err := Purge(dir, strings.NewReader("\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Deleted != 0 || stats.Shredded != 0 || stats.Skipped != 1 {
		t.Fatalf("expected skip on empty input: %+v", stats)
	}
	if _, err := os.Stat(sub); err != nil {
		t.Fatalf("expected directory to remain on empty input")
	}
}

func TestMultipleChoices(t *testing.T) {
	dir := t.TempDir()
	sub1 := filepath.Join(dir, "sub1")
	sub2 := filepath.Join(dir, "sub2")
	sub3 := filepath.Join(dir, "sub3")
	writeDirWithContent(t, sub1, map[string]string{"PURGABLE": "", "a": "1"})
	writeDirWithContent(t, sub2, map[string]string{"PURGABLE": "", "b": "2"})
	writeDirWithContent(t, sub3, map[string]string{"PURGABLE": "", "c": "3"})

	// d for first, s for second, k for third
	stats, err := Purge(dir, strings.NewReader("d\ns\nk\n"), os.Stdout, os.Stderr, os.Remove)
	if err != nil {
		t.Fatal(err)
	}
	if stats.Found != 3 || stats.Deleted != 1 || stats.Shredded != 1 || stats.Skipped != 1 {
		t.Fatalf("unexpected stats: %+v", stats)
	}
	if _, err := os.Stat(sub1); !os.IsNotExist(err) {
		t.Fatalf("expected sub1 deleted")
	}
	if _, err := os.Stat(sub2); err != nil {
		t.Fatalf("expected sub2 to remain after shred")
	}
	if _, err := os.Stat(filepath.Join(sub2, "b")); !os.IsNotExist(err) {
		t.Fatalf("expected sub2 contents shredded")
	}
	if _, err := os.Stat(sub3); err != nil {
		t.Fatalf("expected sub3 to remain after skip")
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

func TestParseAction(t *testing.T) {
	tests := []struct {
		input   string
		want    Action
		wantAll bool
		wantOk  bool
	}{
		{"d", ActionDelete, false, true},
		{"s", ActionShred, false, true},
		{"k", ActionSkip, false, true},
		{"e", ActionExit, false, true},
		{"d-all", ActionDelete, true, true},
		{"s-all", ActionShred, true, true},
		{"k-all", ActionSkip, true, true},
		{"e-all", ActionExit, true, true},
		{"x", 0, false, false},
		{"", 0, false, false},
	}
	for _, tt := range tests {
		t.Run(tt.input, func(t *testing.T) {
			got, gotAll, gotOk := parseAction(tt.input)
			if got != tt.want || gotAll != tt.wantAll || gotOk != tt.wantOk {
				t.Errorf("parseAction(%q) = (%v, %v, %v), want (%v, %v, %v)",
					tt.input, got, gotAll, gotOk, tt.want, tt.wantAll, tt.wantOk)
			}
		})
	}
}

func TestValidateRoot(t *testing.T) {
	t.Run("missing directory", func(t *testing.T) {
		_, err := ValidateRoot(filepath.Join(t.TempDir(), "nope"))
		if err == nil {
			t.Fatal("expected error for missing directory")
		}
	})

	t.Run("file not directory", func(t *testing.T) {
		dir := t.TempDir()
		f := filepath.Join(dir, "file")
		if err := os.WriteFile(f, []byte("x"), 0o644); err != nil {
			t.Fatal(err)
		}
		_, err := ValidateRoot(f)
		if err == nil {
			t.Fatal("expected error for file")
		}
	})

	t.Run("valid directory", func(t *testing.T) {
		dir := t.TempDir()
		info, err := ValidateRoot(dir)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !info.IsDir() {
			t.Fatal("expected directory info")
		}
	})
}

func TestShredFileDisappeared(t *testing.T) {
	dir := t.TempDir()
	f := filepath.Join(dir, "vanish.txt")
	if err := os.WriteFile(f, []byte("secret"), 0o644); err != nil {
		t.Fatal(err)
	}
	// Remove before shred
	os.Remove(f)
	if err := shredFile(f); err != nil {
		t.Fatalf("shredFile should handle missing file gracefully: %v", err)
	}
}