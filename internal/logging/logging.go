// Package logging owns the process logger: stderr plus a size-rotated file beside the database,
// so a problem can be read back after the fact instead of only while a terminal is open.
package logging

import (
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"sync"
)

const (
	maxSize = 10 << 20 // 10 MB per file, never exceeded: a write that does not fit rotates first
	keep    = 3        // old files kept beside the current one; anything older is deleted
)

// rotatingWriter appends to one file and renames it aside when it grows past maxSize.
type rotatingWriter struct {
	mu    sync.Mutex
	path  string
	file  *os.File
	size  int64
	keep  int
	limit int64
}

func newRotatingWriter(path string, limit int64, keep int) (*rotatingWriter, error) {
	if dir := filepath.Dir(path); dir != "" && dir != "." {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			return nil, err
		}
	}
	f, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	if err != nil {
		return nil, err
	}
	info, _ := f.Stat()
	size := int64(0)
	if info != nil {
		size = info.Size()
	}
	dropOlder(path, keep)
	return &rotatingWriter{path: path, file: f, size: size, keep: keep, limit: limit}, nil
}

// dropOlder removes rotations beyond the keep count, including files left by an older build with
// a larger limit.
func dropOlder(path string, keep int) {
	for i := keep + 1; ; i++ {
		name := fmt.Sprintf("%s.%d", path, i)
		if _, err := os.Stat(name); err != nil {
			return
		}
		_ = os.Remove(name)
	}
}

func (w *rotatingWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.size+int64(len(p)) > w.limit {
		w.rotate()
	}
	n, err := w.file.Write(p)
	w.size += int64(n)
	return n, err
}

func (w *rotatingWriter) rotate() {
	w.file.Close()
	for i := w.keep - 1; i >= 1; i-- {
		_ = os.Rename(fmt.Sprintf("%s.%d", w.path, i), fmt.Sprintf("%s.%d", w.path, i+1))
	}
	_ = os.Rename(w.path, w.path+".1")
	dropOlder(w.path, w.keep)
	f, err := os.OpenFile(w.path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0o644)
	if err != nil {
		w.file = os.Stderr
		w.size = 0
		return
	}
	w.file = f
	w.size = 0
}

// Setup points slog at stderr and the log file. A file that cannot be opened is reported but never
// fatal: the app must still start and log somewhere.
func Setup(debug bool, filePath string) {
	level := slog.LevelInfo
	if debug {
		level = slog.LevelDebug
	}
	writers := []io.Writer{os.Stderr}
	if filePath != "" && filePath != "-" {
		w, err := newRotatingWriter(filePath, maxSize, keep)
		if err != nil {
			fmt.Fprintf(os.Stderr, "logging: cannot open %s: %v (stderr only)\n", filePath, err)
		} else {
			writers = append(writers, w)
		}
	}
	handler := slog.NewTextHandler(io.MultiWriter(writers...), &slog.HandlerOptions{Level: level})
	slog.SetDefault(slog.New(handler))
}

// DefaultPath is the log file beside the database, so it lives with the data it describes.
func DefaultPath(dbPath string) string {
	dir := filepath.Dir(dbPath)
	if dir == "" {
		dir = "."
	}
	return filepath.Join(dir, "logs", "app.log")
}
