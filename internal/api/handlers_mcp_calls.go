package api

import (
	"database/sql"
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"
)

// The MCP tool calls: what an assistant asked the book to do, and how it went. The table keeps the
// newest 500 and the prune below deletes the rest, so it cannot grow without bound.

const mcpCallKeep = 500

// pruneMcpCalls keeps the newest 500 rows. Called after each insert: when there are fewer than 500
// the subquery is NULL and `id < NULL` deletes nothing.
func pruneMcpCalls(pool *sql.DB) error {
	_, err := pool.Exec(
		`DELETE FROM mcp_calls
          WHERE id < (SELECT id FROM mcp_calls ORDER BY id DESC LIMIT 1 OFFSET ?)`, mcpCallKeep-1)
	return err
}

type mcpCallRow struct {
	ID       string  `json:"id"`
	TS       string  `json:"ts"`
	KeyName  string  `json:"key_name"`
	KeyID    string  `json:"key_id"`
	Prefix   string  `json:"prefix"`
	Tool     string  `json:"tool"`
	ArgsJSON string  `json:"args_json"`
	Status   string  `json:"status"`
	Error    *string `json:"error"`
	Millis   int64   `json:"ms"`
}

func mcpCalls(c *gin.Context, st *AppState) {
	limit := 200
	if raw := c.Query("limit"); raw != "" {
		if n, err := strconv.Atoi(raw); err == nil {
			limit = n
		}
	}
	if limit < 1 {
		limit = 1
	}
	if limit > mcpCallKeep {
		limit = mcpCallKeep
	}
	if err := pruneMcpCalls(st.Pool); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	rows, err := st.Pool.Query(
		`SELECT c.id, c.ts, c.key_id, COALESCE(k.name, ''), COALESCE(k.prefix, ''),
                c.tool, c.args_json, c.status, c.error, c.ms
           FROM mcp_calls c
           LEFT JOIN api_keys k ON k.id = c.key_id
          ORDER BY c.ts DESC
          LIMIT ?`, limit)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	defer rows.Close()
	out := []mcpCallRow{}
	for rows.Next() {
		var r mcpCallRow
		var errText sql.NullString
		if err := rows.Scan(&r.ID, &r.TS, &r.KeyID, &r.KeyName, &r.Prefix, &r.Tool,
			&r.ArgsJSON, &r.Status, &errText, &r.Millis); err != nil {
			respondErr(c, internalErr(err))
			return
		}
		if errText.Valid {
			r.Error = &errText.String
		}
		out = append(out, r)
	}
	if err := rows.Err(); err != nil {
		respondErr(c, internalErr(err))
		return
	}
	c.JSON(http.StatusOK, gin.H{"calls": out})
}
