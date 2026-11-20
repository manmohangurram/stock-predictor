package api

import (
	"context"
	"database/sql"
	"fmt"
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
)

// The one destructive action in the app: wipe the book. Everything except the admin and the schema
// record goes, so the app is empty but still yours to sign in to.

const resetPhrase = "DELETE EVERYTHING"

// kept across a reset: the door, the migration record, and the session you are using it from.
var resetKeep = map[string]bool{
	"admin":             true,
	"refresh_tokens":    true,
	"schema_migrations": true,
}

func resetBook(c *gin.Context, st *AppState) {
	var body struct {
		Confirm string `json:"confirm"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	if strings.TrimSpace(body.Confirm) != resetPhrase {
		respondErr(c, badRequest(fmt.Sprintf("type %q to confirm", resetPhrase)))
		return
	}
	removed, err := wipe(st.Pool, c.Request.Context())
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, gin.H{"deleted": removed})
}

// wipe clears every table the schema has, except the three above. One connection, so the foreign-key
// pragma applies to the deletes that follow it.
func wipe(pool *sql.DB, ctx context.Context) (map[string]int, error) {
	conn, err := pool.Conn(ctx)
	if err != nil {
		return nil, err
	}
	defer conn.Close()

	rows, err := conn.QueryContext(ctx, "SELECT name FROM sqlite_master WHERE type = 'table'")
	if err != nil {
		return nil, err
	}
	var tables []string
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			rows.Close()
			return nil, err
		}
		if resetKeep[name] || strings.HasPrefix(name, "sqlite_") {
			continue
		}
		tables = append(tables, name)
	}
	rows.Close()
	if err := rows.Err(); err != nil {
		return nil, err
	}

	if _, err := conn.ExecContext(ctx, "PRAGMA foreign_keys = OFF"); err != nil {
		return nil, err
	}
	defer conn.ExecContext(context.Background(), "PRAGMA foreign_keys = ON")

	deleted := map[string]int{}
	tx, err := conn.BeginTx(ctx, nil)
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	for _, name := range tables {
		res, err := tx.ExecContext(ctx, "DELETE FROM "+quoteIdent(name))
		if err != nil {
			return nil, fmt.Errorf("clear %s: %w", name, err)
		}
		n, _ := res.RowsAffected()
		if n > 0 {
			deleted[name] = int(n)
		}
	}
	if err := tx.Commit(); err != nil {
		return nil, err
	}
	return deleted, nil
}

func quoteIdent(name string) string {
	return `"` + strings.ReplaceAll(name, `"`, `""`) + `"`
}
