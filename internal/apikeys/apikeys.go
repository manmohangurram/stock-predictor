// Package apikeys mints and checks the bearer tokens an MCP client presents. Only the hash is
// stored: a key is shown once, at creation, and can never be read back.
package apikeys

import (
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"time"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

type Scope string

const (
	ScopeRead  Scope = "read"
	ScopeWrite Scope = "write"
)

// The reasons a token is refused. The caller turns all three into one 401, because a client with a
// bad key must not learn which way it is bad.
var (
	ErrUnknown = errors.New("no such key")
	ErrExpired = errors.New("key expired")
	ErrRevoked = errors.New("key revoked")
)

type Key struct {
	ID         string  `json:"id"`
	Name       string  `json:"name"`
	Scope      Scope   `json:"scope"`
	Prefix     string  `json:"prefix"`
	CreatedAt  string  `json:"created_at"`
	ExpiresAt  *string `json:"expires_at"`
	LastUsedAt *string `json:"last_used_at"`
	RevokedAt  *string `json:"revoked_at"`
}

// CanWrite is the one place the scope rule lives: write implies read, never the other way round.
func (k *Key) CanWrite() bool { return k.Scope == ScopeWrite }

// New mints a key and stores its hash. days nil means it never expires.
func New(pool *sql.DB, name string, scope Scope, days *int, now time.Time) (string, Key, error) {
	clean := strings.TrimSpace(name)
	if clean == "" {
		return "", Key{}, fmt.Errorf("a key needs a name, so you can tell which one to revoke")
	}
	if scope != ScopeRead && scope != ScopeWrite {
		return "", Key{}, fmt.Errorf("scope must be read or write")
	}
	var expires *string
	if days != nil {
		if *days < 1 {
			return "", Key{}, fmt.Errorf("a key lasts at least one day, or never expires")
		}
		when := tstamp.From(now.AddDate(0, 0, *days))
		expires = &when
	}

	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return "", Key{}, fmt.Errorf("no entropy: %w", err)
	}
	token := "spk_" + base64.RawURLEncoding.EncodeToString(raw)
	sum := sha256.Sum256([]byte(token))
	hash := hex.EncodeToString(sum[:])
	prefix := token[:8]

	key := Key{
		ID: ids.New(), Name: clean, Scope: scope, Prefix: prefix,
		CreatedAt: tstamp.From(now), ExpiresAt: expires,
	}
	if _, err := pool.Exec(
		`INSERT INTO api_keys (id, name, scope, key_hash, prefix, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
		key.ID, key.Name, string(key.Scope), hash, key.Prefix, key.CreatedAt, key.ExpiresAt,
	); err != nil {
		return "", Key{}, err
	}
	return token, key, nil
}

// Verify turns a presented token into the key it belongs to, or one of the three reasons.
func Verify(pool *sql.DB, token string, now time.Time) (*Key, error) {
	if !strings.HasPrefix(token, "spk_") {
		return nil, ErrUnknown
	}
	sum := sha256.Sum256([]byte(token))
	var k Key
	var revoked, expires sql.NullString
	err := pool.QueryRow(
		`SELECT id, name, scope, prefix, created_at, expires_at, last_used_at, revoked_at
           FROM api_keys WHERE key_hash = ?`, hex.EncodeToString(sum[:]),
	).Scan(&k.ID, &k.Name, &k.Scope, &k.Prefix, &k.CreatedAt, &expires, &k.LastUsedAt, &revoked)
	if err == sql.ErrNoRows {
		return nil, ErrUnknown
	}
	if err != nil {
		return nil, err
	}
	if revoked.Valid {
		k.RevokedAt = &revoked.String
		return &k, ErrRevoked
	}
	if expires.Valid {
		k.ExpiresAt = &expires.String
		if until, perr := time.Parse(time.RFC3339, expires.String); perr == nil && !now.Before(until) {
			return &k, ErrExpired
		}
	}
	return &k, nil
}

// Touch records that a key was used, at most once a minute: a busy assistant must not turn every
// read into a write on SQLite.
func Touch(pool *sql.DB, key *Key, now time.Time) {
	if key.LastUsedAt != nil {
		if last, err := time.Parse(time.RFC3339, *key.LastUsedAt); err == nil && now.Sub(last) < time.Minute {
			return
		}
	}
	_, _ = pool.Exec("UPDATE api_keys SET last_used_at = ? WHERE id = ?", tstamp.From(now), key.ID)
}

func List(pool *sql.DB) ([]Key, error) {
	rows, err := pool.Query(
		`SELECT id, name, scope, prefix, created_at, expires_at, last_used_at, revoked_at
           FROM api_keys ORDER BY created_at DESC`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []Key{}
	for rows.Next() {
		var k Key
		var expires, revoked sql.NullString
		if err := rows.Scan(&k.ID, &k.Name, &k.Scope, &k.Prefix, &k.CreatedAt, &expires,
			&k.LastUsedAt, &revoked); err != nil {
			return nil, err
		}
		if expires.Valid {
			k.ExpiresAt = &expires.String
		}
		if revoked.Valid {
			k.RevokedAt = &revoked.String
		}
		out = append(out, k)
	}
	return out, rows.Err()
}

// Revoke is a timestamp, never a delete: the audit trail keeps pointing at a key that is gone.
func Revoke(pool *sql.DB, id string, now time.Time) (bool, error) {
	res, err := pool.Exec(
		"UPDATE api_keys SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL", tstamp.From(now), id)
	if err != nil {
		return false, err
	}
	n, _ := res.RowsAffected()
	return n > 0, nil
}
