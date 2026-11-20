// Package db owns the SQLite connection and the ordered migration list.
package db

import (
	"context"
	"database/sql"
	"embed"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "github.com/glebarez/go-sqlite"

	"stock-predictor/internal/tstamp"
)

//go:embed migrations/*.sql
var migrationFS embed.FS

// Ordered, applied once each, tracked in schema_migrations.
var Migrations []struct{ ID, SQL string }

func init() {
	entries, err := migrationFS.ReadDir("migrations")
	if err != nil {
		panic(err)
	}
	for _, e := range entries {
		if !strings.HasSuffix(e.Name(), ".sql") {
			continue
		}
		body, err := migrationFS.ReadFile("migrations/" + e.Name())
		if err != nil {
			panic(err)
		}
		Migrations = append(Migrations, struct{ ID, SQL string }{strings.TrimSuffix(e.Name(), ".sql"), string(body)})
	}
}

func PathOf(path string) string {
	path = strings.TrimPrefix(path, "sqlite://")
	return strings.TrimPrefix(path, "sqlite:")
}

// The path the database had before the rename, if this one is the renamed one:
// /data/stock-predictor.db -> /data/stock-predicter.db.
func LegacySibling(path string) string {
	p := PathOf(path)
	if filepath.Base(p) != "stock-predictor.db" {
		return ""
	}
	return filepath.Join(filepath.Dir(p), "stock-predicter.db")
}

// Move a database written under an older name to the current one.
//
// Rule: the file with the old name is the real one, so it becomes the file with the new
// name. If something already sits at the new path it is moved aside first, never deleted.
// The WAL is checkpointed before the move: renaming only the .db would silently drop the
// commits still in -wal.
func MigrateLegacyPath(oldPath, newPath string) (string, error) {
	oldPath, newPath = PathOf(oldPath), PathOf(newPath)
	if _, err := os.Stat(oldPath); err != nil {
		return "", nil
	}
	if _, err := os.Stat(newPath); err == nil {
		aside := fmt.Sprintf("%s.bak-%s", newPath, time.Now().UTC().Format("20060102-150405"))
		if err := os.Rename(newPath, aside); err != nil {
			return "", fmt.Errorf("move %s aside: %w", newPath, err)
		}
		slog.Info("the database at the new name is kept as a backup", "at", aside)
	}

	pool, err := Connect(oldPath)
	if err != nil {
		return "", err
	}
	_, _ = pool.Exec("PRAGMA wal_checkpoint(TRUNCATE)")
	pool.Close()

	if dir := filepath.Dir(newPath); dir != "" && dir != "." {
		os.MkdirAll(dir, 0o755)
	}
	if err := os.Rename(oldPath, newPath); err != nil {
		if err := copyFile(oldPath, newPath); err != nil {
			return "", fmt.Errorf("copy the old database: %w", err)
		}
		slog.Warn("copied instead of moved", "left_in_place", oldPath)
	}
	for _, suffix := range []string{"-wal", "-shm"} {
		os.Remove(oldPath + suffix)
	}
	return newPath, nil
}

func copyFile(src, dst string) error {
	in, err := os.ReadFile(src)
	if err != nil {
		return err
	}
	return os.WriteFile(dst, in, 0o644)
}

// Create the parent directory: a missing folder should not look like a corrupt database.
func Connect(path string) (*sql.DB, error) {
	path = PathOf(path)
	if dir := filepath.Dir(path); dir != "" && dir != "." {
		os.MkdirAll(dir, 0o755)
	}
	dsn := "file:" + path + "?_pragma=busy_timeout(5000)&_pragma=foreign_keys(1)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)"
	d, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("connect sqlite: %w", err)
	}
	d.SetMaxOpenConns(5)
	slog.Debug("sqlite open", "path", path, "max_open_conns", 5, "journal_mode", "wal")
	return d, nil
}

// Ordered, applied once each, tracked in schema_migrations. A single idempotent file was
// enough until a migration needed ALTER TABLE, which cannot be re-run.
func Migrate(pool *sql.DB) error {
	if _, err := pool.Exec(
		"CREATE TABLE IF NOT EXISTS schema_migrations (id TEXT PRIMARY KEY, applied_at TEXT NOT NULL)",
	); err != nil {
		return fmt.Errorf("create schema_migrations: %w", err)
	}
	for _, m := range Migrations {
		var done string
		err := pool.QueryRow("SELECT id FROM schema_migrations WHERE id = ?", m.ID).Scan(&done)
		if err == nil {
			continue
		}
		if err != sql.ErrNoRows {
			return err
		}
		if _, err := pool.Exec(m.SQL); err != nil {
			return fmt.Errorf("apply migration %s: %w", m.ID, err)
		}
		slog.Info("migration applied", "id", m.ID)
		if _, err := pool.Exec(
			"INSERT INTO schema_migrations (id, applied_at) VALUES (?, ?)", m.ID, tstamp.Now(),
		); err != nil {
			return err
		}
	}
	_, _ = pool.Exec("PRAGMA optimize")
	return nil
}

func ConnectAndMigrate(ctx context.Context, path string) (*sql.DB, error) {
	pool, err := Connect(path)
	if err != nil {
		return nil, err
	}
	if err := Migrate(pool); err != nil {
		pool.Close()
		return nil, err
	}
	return pool, nil
}
