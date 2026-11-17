package engine

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"stock-predictor/internal/models"
	"stock-predictor/internal/secrets"
	"stock-predictor/internal/tstamp"
)

func getSetting(pool *sql.DB, key string) (*string, error) {
	var v string
	err := pool.QueryRow("SELECT value FROM settings WHERE key = ?", key).Scan(&v)
	if err == sql.ErrNoRows {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &v, nil
}

func putSetting(pool *sql.DB, key, value string, now time.Time) error {
	_, err := pool.Exec(
		`INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
		key, value, tstamp.From(now),
	)
	return err
}

func mustJSON(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		return "{}"
	}
	return string(b)
}

// A CJK character: enough to say the headline is not in English.
func hasIdeographic(s string) bool {
	for _, r := range s {
		if (r >= 0x3040 && r <= 0x30ff) || (r >= 0x3400 && r <= 0x4dbf) ||
			(r >= 0x4e00 && r <= 0x9fff) || (r >= 0xac00 && r <= 0xd7af) ||
			(r >= 0xf900 && r <= 0xfaff) {
			return true
		}
	}
	return false
}

func lower(s string) string                  { return strings.ToLower(s) }
func trimSpace(s string) string              { return strings.TrimSpace(s) }
func contains(s, sub string) bool            { return strings.Contains(s, sub) }
func join(parts []string, sep string) string { return strings.Join(parts, sep) }

func firstWord(s string) string {
	fields := strings.Fields(s)
	if len(fields) == 0 {
		return s
	}
	return fields[0]
}

func firstToken(s string) string {
	fields := strings.Fields(s)
	if len(fields) == 0 {
		return ""
	}
	return fields[0]
}

func containsStr(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}

func fmtPtr(v *float64) string {
	if v == nil {
		return "—"
	}
	return fmt.Sprintf("%.2f", *v)
}

func fmtOrNone(v *float64) string {
	if v == nil {
		return "none"
	}
	return fmt.Sprintf("%.2f", *v)
}

func samePtr(a, b *float64) bool {
	if a == nil || b == nil {
		return a == nil && b == nil
	}
	return *a == *b
}

func heldQty(p *models.Position) float64 {
	if p == nil {
		return 0
	}
	return p.Qty
}

func unreadableKey(pool *sql.DB, name string) (bool, error) {
	return secrets.Unreadable(pool, name)
}
