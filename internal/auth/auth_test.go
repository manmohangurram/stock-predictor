package auth

import (
	"database/sql"
	"path/filepath"
	"testing"
	"time"

	"stock-predictor/internal/db"
)

func newPool(t *testing.T) *sql.DB {
	t.Helper()
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	return pool
}

func TestTheAdminIsCreatedOnceAndBcryptHoldsThePassword(t *testing.T) {
	pool := newPool(t)
	now := time.Now().UTC()

	if needs, err := NeedsSetup(pool); err != nil || !needs {
		t.Fatalf("needs setup %v %v", needs, err)
	}
	a, err := Create(pool, "mohan", "a-long-enough-password", now)
	if err != nil {
		t.Fatal(err)
	}
	if a.PasswordHash == "" || a.PasswordHash == "a-long-enough-password" {
		t.Fatal("the password was stored as typed")
	}
	if _, err := Create(pool, "other", "another-long-password", now); err != ErrExists {
		t.Fatalf("a second admin was created: %v", err)
	}
	if _, err := Verify(pool, "mohan", "a-long-enough-password"); err != nil {
		t.Fatal(err)
	}
	if _, err := Verify(pool, "mohan", "wrong"); err != ErrBadPassword {
		t.Fatalf("%v", err)
	}
	if _, err := Verify(pool, "someone", "a-long-enough-password"); err != ErrBadPassword {
		t.Fatalf("%v", err)
	}
}

func TestAShortPasswordIsRefused(t *testing.T) {
	pool := newPool(t)
	if _, err := Create(pool, "x", "short", time.Now().UTC()); err != ErrShortPass {
		t.Fatalf("%v", err)
	}
	if needs, _ := NeedsSetup(pool); !needs {
		t.Fatal("a refused password still created the admin")
	}
}

func TestTheAccessTokenExpiresAndIsSigned(t *testing.T) {
	pool := newPool(t)
	now := time.Now().UTC()
	a, err := Create(pool, "admin", "a-long-enough-password", now)
	if err != nil {
		t.Fatal(err)
	}
	key := SigningKey("secret")

	access, err := IssueAccess(key, a.ID, now)
	if err != nil {
		t.Fatal(err)
	}
	id, err := VerifyAccess(key, access, now.Add(time.Minute))
	if err != nil || id != a.ID {
		t.Fatalf("%v %v", id, err)
	}
	if _, err := VerifyAccess(key, access, now.Add(AccessLifetime+time.Second)); err != ErrTokenExpired {
		t.Fatalf("%v", err)
	}
	if _, err := VerifyAccess(SigningKey("another secret"), access, now); err != ErrTokenInvalid {
		t.Fatalf("a token signed elsewhere verified: %v", err)
	}
}

func TestARefreshTokenWorksOnceAndItsReplayIsRefused(t *testing.T) {
	pool := newPool(t)
	now := time.Now().UTC()
	a, err := Create(pool, "admin", "a-long-enough-password", now)
	if err != nil {
		t.Fatal(err)
	}
	key := SigningKey("secret")
	first, err := Issue(pool, key, a, "test", now)
	if err != nil {
		t.Fatal(err)
	}

	_, second, err := Rotate(pool, key, first.Refresh, "test", now.Add(time.Minute))
	if err != nil {
		t.Fatal(err)
	}
	if second.Refresh == first.Refresh {
		t.Fatal("the refresh token was not rotated")
	}
	if _, _, err := Rotate(pool, key, first.Refresh, "test", now.Add(2*time.Minute)); err != ErrTokenReused {
		t.Fatalf("the spent token worked again: %v", err)
	}
	if _, _, err := Rotate(pool, key, second.Refresh, "test", now.Add(3*time.Minute)); err == nil {
		t.Fatal("after reuse the whole chain should be revoked")
	}
}

func TestChangingThePasswordEndsEverySession(t *testing.T) {
	pool := newPool(t)
	now := time.Now().UTC()
	a, err := Create(pool, "admin", "a-long-enough-password", now)
	if err != nil {
		t.Fatal(err)
	}
	key := SigningKey("secret")
	tokens, err := Issue(pool, key, a, "test", now)
	if err != nil {
		t.Fatal(err)
	}
	if err := ChangePassword(pool, "a-long-enough-password", "a-different-long-one", now); err != nil {
		t.Fatal(err)
	}
	if _, err := Verify(pool, "admin", "a-different-long-one"); err != nil {
		t.Fatal(err)
	}
	if _, _, err := Rotate(pool, key, tokens.Refresh, "test", now.Add(time.Minute)); err == nil {
		t.Fatal("the old session survived the password change")
	}
	if err := ChangePassword(pool, "wrong", "another-long-one", now); err != ErrBadPassword {
		t.Fatalf("%v", err)
	}
}
