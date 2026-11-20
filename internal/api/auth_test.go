package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/config"
	"stock-predictor/internal/db"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
)

// freshApp builds a database with no admin, which is what an installed instance looks like.
func freshApp(t *testing.T) (*gin.Engine, *AppState) {
	t.Helper()
	gin.SetMode(gin.TestMode)
	pool, err := db.ConnectAndMigrate(t.Context(), filepath.Join(t.TempDir(), "t.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { pool.Close() })
	if _, err := rules.EnsureMonth(pool, models.ModeLong, "", 100_000, time.Now().UTC()); err != nil {
		t.Fatal(err)
	}
	st := NewState(pool, &config.Config{
		PublicURL: "https://example.in",
		SecretKey: []byte("0123456789abcdef0123456789abcdef"),
	})
	st.Manual = make(chan ManualRequest, 8)
	return Router(st), st
}

func post(t *testing.T, r *gin.Engine, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	return w
}

func TestTheAppStaysClosedUntilTheAdminIsSetUp(t *testing.T) {
	r, _ := freshApp(t)

	req := httptest.NewRequest(http.MethodGet, "/api/auth/status", nil)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	var status struct {
		NeedsSetup bool   `json:"needs_setup"`
		SignedIn   bool   `json:"signed_in"`
		Username   string `json:"username"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	if !status.NeedsSetup || status.SignedIn {
		t.Fatalf("%+v", status)
	}

	req = httptest.NewRequest(http.MethodGet, "/api/dashboard", nil)
	w = httptest.NewRecorder()
	r.ServeHTTP(w, req)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("the book answered %d without a session", w.Code)
	}
	if got := w.Header().Get("WWW-Authenticate"); got != "" && !strings.Contains(got, "Bearer") {
		t.Fatalf("unexpected challenge %q", got)
	}

	w = post(t, r, "/api/auth/login", `{"username":"admin","password":"a-long-enough-password"}`)
	if w.Code != http.StatusUnauthorized {
		t.Fatalf("login before setup answered %d", w.Code)
	}

	w = post(t, r, "/api/auth/setup", `{"username":"admin","password":"a-long-enough-password"}`)
	if w.Code != http.StatusCreated {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	var created struct {
		AccessToken string `json:"access_token"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &created); err != nil {
		t.Fatal(err)
	}
	if created.AccessToken == "" {
		t.Fatal("setup returned no access token")
	}
	if w.Header().Get("Set-Cookie") == "" {
		t.Fatal("setup set no refresh cookie")
	}

	req = httptest.NewRequest(http.MethodGet, "/api/dashboard", nil)
	req.Header.Set("Authorization", "Bearer "+created.AccessToken)
	w = httptest.NewRecorder()
	r.ServeHTTP(w, req)
	if w.Code != http.StatusOK {
		t.Fatalf("the book refused the token setup just issued: %d %s", w.Code, w.Body.String())
	}

	w = post(t, r, "/api/auth/setup", `{"username":"other","password":"another-long-password"}`)
	if w.Code != http.StatusConflict {
		t.Fatalf("a second setup answered %d", w.Code)
	}
}

func TestSignInIssuesAPairAndRefreshRotatesIt(t *testing.T) {
	r, _ := freshApp(t)
	if w := post(t, r, "/api/auth/setup", `{"username":"admin","password":"a-long-enough-password"}`); w.Code != http.StatusCreated {
		t.Fatalf("setup %d %s", w.Code, w.Body.String())
	}

	if w := post(t, r, "/api/auth/login", `{"username":"admin","password":"wrong"}`); w.Code != http.StatusUnauthorized {
		t.Fatalf("a wrong password answered %d", w.Code)
	}

	w := post(t, r, "/api/auth/login", `{"username":"admin","password":"a-long-enough-password"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	cookie := w.Result().Cookies()
	if len(cookie) == 0 || cookie[0].Value == "" {
		t.Fatal("login set no refresh cookie")
	}
	if !cookie[0].HttpOnly {
		t.Fatal("the refresh cookie is readable by script")
	}

	req := httptest.NewRequest(http.MethodPost, "/api/auth/refresh", nil)
	req.AddCookie(cookie[0])
	w2 := httptest.NewRecorder()
	r.ServeHTTP(w2, req)
	if w2.Code != http.StatusOK {
		t.Fatalf("refresh answered %d %s", w2.Code, w2.Body.String())
	}
	var refreshed struct {
		AccessToken string `json:"access_token"`
	}
	if err := json.Unmarshal(w2.Body.Bytes(), &refreshed); err != nil {
		t.Fatal(err)
	}
	if refreshed.AccessToken == "" {
		t.Fatal("refresh returned no access token")
	}

	replay := httptest.NewRequest(http.MethodPost, "/api/auth/refresh", nil)
	replay.AddCookie(cookie[0])
	w3 := httptest.NewRecorder()
	r.ServeHTTP(w3, replay)
	if w3.Code != http.StatusUnauthorized {
		t.Fatalf("a replayed refresh token answered %d", w3.Code)
	}
}
