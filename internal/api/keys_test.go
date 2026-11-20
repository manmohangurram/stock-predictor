package api

import (
	"database/sql"
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"stock-predictor/internal/apikeys"
)

func TestAKeyIsShownOnceAndWorksUntilItIsRevoked(t *testing.T) {
	pool, r, _ := setup(t)

	code, body := do(t, r, http.MethodPost, "/api/keys", map[string]any{
		"name": "claude desktop", "scope": "read", "days": 30,
	}, nil)
	if code != http.StatusCreated {
		t.Fatalf("%d %s", code, body)
	}
	var created struct {
		Key   apikeys.Key `json:"key"`
		Token string      `json:"token"`
	}
	if err := json.Unmarshal(body, &created); err != nil {
		t.Fatal(err)
	}
	if created.Token == "" || created.Key.ID == "" || created.Key.Prefix != created.Token[:8] {
		t.Fatalf("%+v", created)
	}

	key, err := apikeys.Verify(pool, created.Token, time.Now().UTC())
	if err != nil || key.ID != created.Key.ID || key.Scope != apikeys.ScopeRead {
		t.Fatalf("verify: %+v %v", key, err)
	}

	_, body = do(t, r, http.MethodGet, "/api/keys", nil, nil)
	if string(body) == "" || containsSecret(body, created.Token) {
		t.Fatalf("the list must not carry the token: %s", body)
	}

	code, body = do(t, r, http.MethodDelete, "/api/keys/"+created.Key.ID, nil, nil)
	if code != http.StatusOK {
		t.Fatalf("%d %s", code, body)
	}
	if _, err := apikeys.Verify(pool, created.Token, time.Now().UTC()); err != apikeys.ErrRevoked {
		t.Fatalf("a revoked key verified: %v", err)
	}
}

func TestAnExpiredKeyIsRefused(t *testing.T) {
	pool, _, _ := setup(t)
	token, key, err := apikeys.New(pool, "old", apikeys.ScopeRead, nil, time.Now().UTC())
	if err != nil {
		t.Fatal(err)
	}
	// Push the expiry into the past rather than waiting a day.
	if _, err := pool.Exec("UPDATE api_keys SET expires_at = ? WHERE id = ?",
		"2020-01-01T00:00:00.000000000+00:00", key.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := apikeys.Verify(pool, token, time.Now().UTC()); err != apikeys.ErrExpired {
		t.Fatalf("an expired key verified: %v", err)
	}
	if _, err := apikeys.Verify(pool, "spk_nonsense", time.Now().UTC()); err != apikeys.ErrUnknown {
		t.Fatal("an unknown key was not unknown")
	}
	if _, err := apikeys.Verify(pool, "not-a-key", time.Now().UTC()); err != apikeys.ErrUnknown {
		t.Fatal("a token without the prefix was accepted")
	}
}

func TestAKeyNeedsANameAndOneDayOrNever(t *testing.T) {
	pool, r, _ := setup(t)
	for _, body := range []map[string]any{
		{"name": "  ", "scope": "read"},
		{"name": "bad scope", "scope": "admin"},
		{"name": "zero days", "scope": "read", "days": 0},
	} {
		if code, resp := do(t, r, http.MethodPost, "/api/keys", body, nil); code != http.StatusBadRequest {
			t.Fatalf("%v -> %d %s", body, code, resp)
		}
	}
	if _, err := pool.Exec("DELETE FROM api_keys"); err != nil {
		t.Fatal(err)
	}
	if n := countRows(t, pool, "SELECT COUNT(*) FROM api_keys"); n != 0 {
		t.Fatalf("%d keys written by the refusals", n)
	}
}

func TestAKeyThatNeverExpiresHasNoDate(t *testing.T) {
	pool, _, _ := setup(t)
	token, _, err := apikeys.New(pool, "forever", apikeys.ScopeWrite, nil, time.Now().UTC())
	if err != nil {
		t.Fatal(err)
	}
	var expires *string
	if err := pool.QueryRow("SELECT expires_at FROM api_keys WHERE key_hash IS NOT NULL LIMIT 1").Scan(&expires); err != nil {
		t.Fatal(err)
	}
	if expires != nil {
		t.Fatalf("expires_at is %v, want NULL", *expires)
	}
	if _, err := apikeys.Verify(pool, token, time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
}

func containsSecret(body []byte, token string) bool {
	for i := 0; i+len(token) <= len(body); i++ {
		if string(body[i:i+len(token)]) == token {
			return true
		}
	}
	return false
}

func countRows(t *testing.T, pool *sql.DB, query string) int {
	t.Helper()
	var n int
	if err := pool.QueryRow(query).Scan(&n); err != nil {
		t.Fatal(err)
	}
	return n
}
