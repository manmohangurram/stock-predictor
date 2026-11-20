// Package auth is the one door into the app: a single admin, a bcrypt password, a short access JWT
// and a rotating refresh token. The signing key is derived from SECRET_KEY, so rotating that secret
// is the emergency lever that ends every session.
package auth

import (
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"strings"
	"time"

	"github.com/golang-jwt/jwt/v5"
	"golang.org/x/crypto/bcrypt"

	"stock-predictor/internal/ids"
	"stock-predictor/internal/tstamp"
)

const (
	AccessLifetime = 15 * time.Minute
	RefreshLife    = 30 * 24 * time.Hour
	bcryptCost     = 12
)

var (
	ErrNoAdmin      = errors.New("no admin yet")
	ErrExists       = errors.New("an admin already exists")
	ErrBadPassword  = errors.New("invalid credentials")
	ErrTokenExpired = errors.New("token expired")
	ErrTokenInvalid = errors.New("token invalid")
	ErrTokenReused  = errors.New("refresh token reused")
	ErrShortPass    = errors.New("use at least 10 characters")
)

type Admin struct {
	ID           string `json:"id"`
	Username     string `json:"username"`
	PasswordHash string `json:"-"`
	CreatedAt    string `json:"created_at"`
}

type Tokens struct {
	Access         string    `json:"access_token"`
	AccessExpires  time.Time `json:"access_expires_at"`
	Refresh        string    `json:"-"`
	RefreshExpires time.Time `json:"-"`
}

// SigningKey turns the one configured secret into a key dedicated to tokens, so the value that
// encrypts stored secrets is never used directly as a JWT key.
func SigningKey(secret string) []byte {
	sum := sha256.Sum256([]byte("jwt:" + secret))
	return sum[:]
}

func NeedsSetup(pool *sql.DB) (bool, error) {
	var n int
	if err := pool.QueryRow("SELECT COUNT(*) FROM admin").Scan(&n); err != nil {
		return false, err
	}
	return n == 0, nil
}

func Current(pool *sql.DB) (*Admin, error) {
	var a Admin
	err := pool.QueryRow("SELECT id, username, password_hash, created_at FROM admin LIMIT 1").
		Scan(&a.ID, &a.Username, &a.PasswordHash, &a.CreatedAt)
	if err == sql.ErrNoRows {
		return nil, ErrNoAdmin
	}
	if err != nil {
		return nil, err
	}
	return &a, nil
}

// Create makes the one admin. The number of admins is checked and enforced by the same statement,
// so two simultaneous setup calls cannot both win.
func Create(pool *sql.DB, username, password string, now time.Time) (*Admin, error) {
	if len(password) < 10 {
		return nil, ErrShortPass
	}
	name := strings.TrimSpace(username)
	if name == "" {
		name = "admin"
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(password), bcryptCost)
	if err != nil {
		return nil, err
	}
	a := &Admin{ID: ids.New(), Username: name, PasswordHash: string(hash), CreatedAt: tstamp.From(now)}
	tx, err := pool.Begin()
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	var n int
	if err := tx.QueryRow("SELECT COUNT(*) FROM admin").Scan(&n); err != nil {
		return nil, err
	}
	if n > 0 {
		return nil, ErrExists
	}
	if _, err := tx.Exec("INSERT INTO admin (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
		a.ID, a.Username, a.PasswordHash, a.CreatedAt); err != nil {
		return nil, err
	}
	if _, err := tx.Exec("UPDATE api_keys SET user_id = ? WHERE user_id IS NULL", a.ID); err != nil {
		return nil, err
	}
	if err := tx.Commit(); err != nil {
		return nil, err
	}
	return a, nil
}

// Verify checks the password. It hashes even when the username is wrong, so the answer takes the
// same time either way.
func Verify(pool *sql.DB, username, password string) (*Admin, error) {
	a, err := Current(pool)
	if err == ErrNoAdmin {
		_, _ = bcrypt.GenerateFromPassword([]byte(password), bcryptCost)
		return nil, ErrBadPassword
	}
	if err != nil {
		return nil, err
	}
	if a.Username != strings.TrimSpace(username) {
		_, _ = bcrypt.GenerateFromPassword([]byte(password), bcryptCost)
		return nil, ErrBadPassword
	}
	if bcrypt.CompareHashAndPassword([]byte(a.PasswordHash), []byte(password)) != nil {
		return nil, ErrBadPassword
	}
	return a, nil
}

func ChangePassword(pool *sql.DB, current, next string, now time.Time) error {
	a, err := Current(pool)
	if err != nil {
		return err
	}
	if bcrypt.CompareHashAndPassword([]byte(a.PasswordHash), []byte(current)) != nil {
		return ErrBadPassword
	}
	if len(next) < 10 {
		return ErrShortPass
	}
	hash, err := bcrypt.GenerateFromPassword([]byte(next), bcryptCost)
	if err != nil {
		return err
	}
	tx, err := pool.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err := tx.Exec("UPDATE admin SET password_hash = ? WHERE id = ?", string(hash), a.ID); err != nil {
		return err
	}
	if _, err := tx.Exec("UPDATE refresh_tokens SET revoked_at = ? WHERE admin_id = ? AND revoked_at IS NULL",
		tstamp.From(now), a.ID); err != nil {
		return err
	}
	return tx.Commit()
}

// Issue mints the pair. Only the refresh token is stored, and only as a hash.
func Issue(pool *sql.DB, key []byte, a *Admin, userAgent string, now time.Time) (Tokens, error) {
	access, err := IssueAccess(key, a.ID, now)
	if err != nil {
		return Tokens{}, err
	}
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return Tokens{}, err
	}
	refresh := base64.RawURLEncoding.EncodeToString(raw)
	expires := now.Add(RefreshLife)
	if _, err := pool.Exec(
		`INSERT INTO refresh_tokens (id, admin_id, token_hash, created_at, expires_at, user_agent)
         VALUES (?, ?, ?, ?, ?, ?)`,
		ids.New(), a.ID, hashToken(refresh), tstamp.From(now), tstamp.From(expires), userAgent,
	); err != nil {
		return Tokens{}, err
	}
	return Tokens{Access: access, AccessExpires: now.Add(AccessLifetime), Refresh: refresh, RefreshExpires: expires}, nil
}

func IssueAccess(key []byte, adminID string, now time.Time) (string, error) {
	claims := jwt.MapClaims{
		"sub": adminID,
		"iat": now.Unix(),
		"exp": now.Add(AccessLifetime).Unix(),
		"jti": ids.New(),
	}
	return jwt.NewWithClaims(jwt.SigningMethodHS256, claims).SignedString(key)
}

// VerifyAccess returns the admin id, or ErrTokenExpired / ErrTokenInvalid so the client knows
// whether to refresh or to sign in again.
func VerifyAccess(key []byte, token string, now time.Time) (string, error) {
	claims := jwt.MapClaims{}
	parsed, err := jwt.ParseWithClaims(token, claims, func(t *jwt.Token) (any, error) {
		if t.Method != jwt.SigningMethodHS256 {
			return nil, ErrTokenInvalid
		}
		return key, nil
	}, jwt.WithTimeFunc(func() time.Time { return now }))
	if err != nil {
		if errors.Is(err, jwt.ErrTokenExpired) {
			return "", ErrTokenExpired
		}
		return "", ErrTokenInvalid
	}
	if !parsed.Valid {
		return "", ErrTokenInvalid
	}
	sub, _ := claims["sub"].(string)
	if sub == "" {
		return "", ErrTokenInvalid
	}
	return sub, nil
}

// Rotate spends the presented refresh token and returns the replacement. A token that has already
// been spent means the cookie was replayed, so every token of that admin is revoked.
func Rotate(pool *sql.DB, key []byte, presented, userAgent string, now time.Time) (*Admin, Tokens, error) {
	var id, adminID string
	var expires string
	var revoked sql.NullString
	err := pool.QueryRow(
		"SELECT id, admin_id, expires_at, revoked_at FROM refresh_tokens WHERE token_hash = ?",
		hashToken(presented),
	).Scan(&id, &adminID, &expires, &revoked)
	if err == sql.ErrNoRows {
		return nil, Tokens{}, ErrTokenInvalid
	}
	if err != nil {
		return nil, Tokens{}, err
	}
	if revoked.Valid {
		if _, err := pool.Exec("UPDATE refresh_tokens SET revoked_at = ? WHERE admin_id = ? AND revoked_at IS NULL",
			tstamp.From(now), adminID); err != nil {
			return nil, Tokens{}, err
		}
		return nil, Tokens{}, ErrTokenReused
	}
	if until, perr := time.Parse(time.RFC3339, expires); perr == nil && !now.Before(until) {
		return nil, Tokens{}, ErrTokenExpired
	}
	a, err := Current(pool)
	if err != nil {
		return nil, Tokens{}, err
	}
	tokens, err := Issue(pool, key, a, userAgent, now)
	if err != nil {
		return nil, Tokens{}, err
	}
	var replacement string
	_ = pool.QueryRow("SELECT id FROM refresh_tokens WHERE token_hash = ?", hashToken(tokens.Refresh)).Scan(&replacement)
	if _, err := pool.Exec("UPDATE refresh_tokens SET revoked_at = ?, replaced_by = ? WHERE id = ?",
		tstamp.From(now), replacement, id); err != nil {
		return nil, Tokens{}, err
	}
	return a, tokens, nil
}

func Revoke(pool *sql.DB, presented string, now time.Time) error {
	_, err := pool.Exec("UPDATE refresh_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
		tstamp.From(now), hashToken(presented))
	return err
}

func RevokeAll(pool *sql.DB, adminID string, now time.Time) error {
	_, err := pool.Exec("UPDATE refresh_tokens SET revoked_at = ? WHERE admin_id = ? AND revoked_at IS NULL",
		tstamp.From(now), adminID)
	return err
}

func hashToken(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

