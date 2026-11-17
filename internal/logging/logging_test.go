package logging

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestTheLogRotatesAndKeepsTheNewestFiles(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "app.log")
	w, err := newRotatingWriter(path, 64, 2)
	if err != nil {
		t.Fatal(err)
	}
	line := []byte(strings.Repeat("x", 40) + "\n")
	for i := 0; i < 5; i++ {
		if _, err := w.Write(line); err != nil {
			t.Fatal(err)
		}
	}
	w.file.Close()
	if _, err := os.Stat(path); err != nil {
		t.Fatal("current log missing")
	}
	if _, err := os.Stat(path + ".1"); err != nil {
		t.Fatal("nothing was rotated")
	}
	if _, err := os.Stat(path + ".3"); err == nil {
		t.Fatal("kept more files than the limit")
	}
}

func TestNoFileEverExceedsTheLimit(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "app.log")
	w, err := newRotatingWriter(path, 64, 3)
	if err != nil {
		t.Fatal(err)
	}
	line := []byte(strings.Repeat("y", 41) + "\n")
	for i := 0; i < 8; i++ {
		if _, err := w.Write(line); err != nil {
			t.Fatal(err)
		}
	}
	w.file.Close()
	matches, _ := filepath.Glob(path + "*")
	for _, name := range matches {
		info, err := os.Stat(name)
		if err != nil {
			t.Fatal(err)
		}
		if info.Size() > 64 {
			t.Fatalf("%s is %d bytes, over the 64-byte test limit", name, info.Size())
		}
	}
}

func TestFilesOlderThanTheKeepCountAreDeletedAtStartup(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "app.log")
	for _, n := range []string{".1", ".2", ".3", ".4", ".5"} {
		if err := os.WriteFile(path+n, []byte("old\n"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	w, err := newRotatingWriter(path, 1<<20, 3)
	if err != nil {
		t.Fatal(err)
	}
	w.file.Close()
	for _, n := range []string{".4", ".5"} {
		if _, err := os.Stat(path + n); err == nil {
			t.Fatalf("%s should have been deleted", n)
		}
	}
	for _, n := range []string{".1", ".2", ".3"} {
		if _, err := os.Stat(path + n); err != nil {
			t.Fatalf("%s should have been kept", n)
		}
	}
}

func TestDefaultPathSitsBesideTheDatabase(t *testing.T) {
	got := DefaultPath(filepath.Join("data", "stock-predictor.db"))
	want := filepath.Join("data", "logs", "app.log")
	if got != want {
		t.Fatalf("%q want %q", got, want)
	}
}
