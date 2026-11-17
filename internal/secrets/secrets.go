package secrets

import (
	"crypto/rand"
	"database/sql"
	"encoding/base64"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sync"

	"stock-predictor/internal/tstamp"

	"golang.org/x/crypto/chacha20poly1305"
)

// Secrets at rest: provider keys, the ntfy token and the proxy password.
//
// The key lives in <db dir>/secret.key, created on first boot. That defends against the
// database file escaping, not against someone who has the whole data directory, and it is
// nothing like authentication.
//
// Values are stored as enc:v1:<base64 nonce || ciphertext>. Anything without that prefix
// is plaintext from an older database and is encrypted in place on boot.

const Prefix = "enc:v1:"

// Settings rows that hold a secret, and must never be stored in the clear.
var SecretKeys = []string{
	"deepseek_key",
	"openai_key",
	"gemini_key",
	"custom_key",
	"ntfy_token",
	"proxy_pass",
}

var (
	mu  sync.Mutex
	key []byte
)

// Called once at boot with SECRET_KEY or the key file's contents.
func Init(k []byte) {
	mu.Lock()
	defer mu.Unlock()
	key = k
}

func processKey() []byte {
	mu.Lock()
	defer mu.Unlock()
	if key == nil {
		k := make([]byte, 32)
		rand.Read(k)
		key = k
	}
	return key
}

func KeyPath(dbPath string) string {
	dir := filepath.Dir(dbPath)
	if dir == "" {
		dir = "."
	}
	return filepath.Join(dir, "secret.key")
}

// Read the key, or create it. 32 random bytes, mode 0600 on unix.
func LoadOrCreateKey(dbPath string) ([]byte, error) {
	path := KeyPath(dbPath)
	if b, err := os.ReadFile(path); err == nil && len(b) >= 32 {
		return b[:32], nil
	}
	k := make([]byte, 32)
	if _, err := rand.Read(k); err != nil {
		return nil, fmt.Errorf("no system randomness: %w", err)
	}
	if dir := filepath.Dir(path); dir != "" && dir != "." {
		os.MkdirAll(dir, 0o755)
	}
	if err := os.WriteFile(path, k, 0o600); err != nil {
		return nil, fmt.Errorf("write %s: %w", path, err)
	}
	return k, nil
}

func EncryptWith(key []byte, plaintext string) (string, error) {
	if len(key) < 32 {
		return "", fmt.Errorf("secret key is too short")
	}
	aead, err := chacha20poly1305.NewX(key[:32])
	if err != nil {
		return "", err
	}
	nonce := make([]byte, aead.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return "", err
	}
	ct := aead.Seal(nil, nonce, []byte(plaintext), nil)
	blob := append(append([]byte{}, nonce...), ct...)
	return Prefix + base64.StdEncoding.EncodeToString(blob), nil
}

// Nil when the value cannot be opened — a wrong key must not stop the app from running.
func DecryptWith(key []byte, stored string) (string, bool) {
	body, ok := bytes_CutPrefix(stored, Prefix)
	if !ok {
		return "", false
	}
	blob, err := base64.StdEncoding.DecodeString(body)
	if err != nil || len(blob) <= 24 || len(key) < 32 {
		return "", false
	}
	aead, err := chacha20poly1305.NewX(key[:32])
	if err != nil {
		return "", false
	}
	plain, err := aead.Open(nil, blob[:24], blob[24:], nil)
	if err != nil {
		return "", false
	}
	return string(plain), true
}

func Encrypt(plaintext string) (string, error) {
	return EncryptWith(processKey(), plaintext)
}

func Decrypt(stored string) (string, bool) {
	return DecryptWith(processKey(), stored)
}

func IsEncrypted(value string) bool {
	return len(value) >= len(Prefix) && value[:len(Prefix)] == Prefix
}

// Encrypt any secret still sitting in the clear. Idempotent; runs on every boot.
func Migrate(db *sql.DB) (int, error) {
	rows, err := db.Query("SELECT key, value FROM settings")
	if err != nil {
		return 0, err
	}
	defer rows.Close()
	type kv struct{ k, v string }
	var pairs []kv
	for rows.Next() {
		var p kv
		if err := rows.Scan(&p.k, &p.v); err != nil {
			return 0, err
		}
		pairs = append(pairs, p)
	}
	if err := rows.Err(); err != nil {
		return 0, err
	}
	changed := 0
	for _, p := range pairs {
		secret := contains(SecretKeys, p.k)
		if !secret || p.v == "" || IsEncrypted(p.v) {
			continue
		}
		sealed, err := Encrypt(p.v)
		if err != nil {
			return changed, err
		}
		if _, err := db.Exec("UPDATE settings SET value = ? WHERE key = ?", sealed, p.k); err != nil {
			return changed, err
		}
		changed++
	}
	return changed, nil
}

// Read a secret, decrypting when needed. Old plaintext rows still work.
func GetSecret(db *sql.DB, name string) (string, error) {
	var value string
	err := db.QueryRow("SELECT value FROM settings WHERE key = ?", name).Scan(&value)
	if err == sql.ErrNoRows {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	if value == "" {
		return "", nil
	}
	if !IsEncrypted(value) {
		return value, nil
	}
	plain, ok := Decrypt(value)
	if !ok || plain == "" {
		slog.Warn("cannot be opened with the current secret key", "key", name)
		return "", nil
	}
	return plain, nil
}

func PutSecret(db *sql.DB, name, plaintext string) error {
	sealed, err := Encrypt(plaintext)
	if err != nil {
		return err
	}
	_, err = db.Exec(
		`INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
		 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at`,
		name, sealed, tstamp.Now(),
	)
	return err
}

// True when a value is stored for this key but cannot be opened — the SECRET_KEY changed.
func Unreadable(db *sql.DB, name string) (bool, error) {
	var value string
	err := db.QueryRow("SELECT value FROM settings WHERE key = ?", name).Scan(&value)
	if err == sql.ErrNoRows {
		return false, nil
	}
	if err != nil {
		return false, err
	}
	return IsEncrypted(value) && func() bool { _, ok := Decrypt(value); return !ok }(), nil
}

func contains(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}

func bytes_CutPrefix(s, prefix string) (string, bool) {
	if len(s) >= len(prefix) && s[:len(prefix)] == prefix {
		return s[len(prefix):], true
	}
	return "", false
}
