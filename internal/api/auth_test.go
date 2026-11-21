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

func TestTheSignInNameCanBeChangedAndOnlyWithThePassword(t *testing.T) {
	r, _ := freshApp(t)
	if w := post(t, r, "/api/auth/setup", `{"username":"admin","password":"a-long-enough-password"}`); w.Code != http.StatusCreated {
		t.Fatalf("setup %d %s", w.Code, w.Body.String())
	}

	if w := post(t, r, "/api/auth/username", `{"username":"mohan","password":"wrong"}`); w.Code != http.StatusForbidden {
		t.Fatalf("a wrong password renamed the admin: %d %s", w.Code, w.Body.String())
	}
	if w := post(t, r, "/api/auth/username", `{"username":"   ","password":"a-long-enough-password"}`); w.Code != http.StatusBadRequest {
		t.Fatalf("an empty name answered %d %s", w.Code, w.Body.String())
	}

	w := post(t, r, "/api/auth/username", `{"username":" mohan ","password":"a-long-enough-password"}`)
	if w.Code != http.StatusOK {
		t.Fatalf("%d %s", w.Code, w.Body.String())
	}
	var renamed struct {
		Username string `json:"username"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &renamed); err != nil {
		t.Fatal(err)
	}
	if renamed.Username != "mohan" {
		t.Fatalf("the name came back as %q", renamed.Username)
	}

	if w := post(t, r, "/api/auth/login", `{"username":"admin","password":"a-long-enough-password"}`); w.Code != http.StatusUnauthorized {
		t.Fatalf("the old name still signs in: %d", w.Code)
	}
	if w := post(t, r, "/api/auth/login", `{"username":"mohan","password":"a-long-enough-password"}`); w.Code != http.StatusOK {
		t.Fatalf("the new name does not sign in: %d %s", w.Code, w.Body.String())
	}
}

func TestSessionsListTheBrowsersAndOneSwitchEndsThemAll(t *testing.T) {
	r, _ := freshApp(t)
	if w := post(t, r, "/api/auth/setup", `{"username":"admin","password":"a-long-enough-password"}`); w.Code != http.StatusCreated {
		t.Fatalf("setup %d %s", w.Code, w.Body.String())
	}

	login := func(agent string) (token, refresh string) {
		req := httptest.NewRequest(http.MethodPost, "/api/auth/login",
			strings.NewReader(`{"username":"admin","password":"a-long-enough-password"}`))
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("User-Agent", agent)
		w := httptest.NewRecorder()
		r.ServeHTTP(w, req)
		if w.Code != http.StatusOK {
			t.Fatalf("login %s: %d %s", agent, w.Code, w.Body.String())
		}
		var out struct {
			AccessToken string `json:"access_token"`
		}
		if err := json.Unmarshal(w.Body.Bytes(), &out); err != nil {
			t.Fatal(err)
		}
		var chosen *http.Cookie
		legacyRetired := false
		for _, c := range w.Result().Cookies() {
			if c.Path == "/api" && c.Value != "" {
				chosen = c
			}
			if c.Path == "/api/auth" && c.MaxAge < 0 {
				legacyRetired = true
			}
		}
		if chosen == nil {
			t.Fatal("login set no refresh cookie on /api")
		}
		if !legacyRetired {
			t.Fatal("login did not retire the old /api/auth cookie")
		}
		return out.AccessToken, chosen.Value
	}

	token, refresh := login("Mozilla/5.0 (Windows) Chrome")
	login("Mozilla/5.0 (Android) Chrome")

	withCookie := map[string]string{
		"Authorization": "Bearer " + token,
		"Cookie":        "sp_refresh=" + refresh,
	}
	code, body := do(t, r, http.MethodGet, "/api/sessions", nil, withCookie)
	if code != http.StatusOK {
		t.Fatalf("sessions %d %s", code, body)
	}
	var list struct {
		Sessions []struct {
			Current   bool   `json:"current"`
			UserAgent string `json:"user_agent"`
		} `json:"sessions"`
	}
	if err := json.Unmarshal(body, &list); err != nil {
		t.Fatal(err)
	}
	if len(list.Sessions) != 3 {
		t.Fatalf("setup and two logins leave three sessions, the list shows %d: %s", len(list.Sessions), body)
	}
	current := 0
	for _, s := range list.Sessions {
		if s.Current {
			current++
		}
	}
	if current != 1 {
		t.Fatalf("exactly one session is this browser, found %d", current)
	}

	revoke := httptest.NewRequest(http.MethodPost, "/api/sessions/revoke-all", nil)
	revoke.Header.Set("Authorization", "Bearer "+token)
	w := httptest.NewRecorder()
	r.ServeHTTP(w, revoke)
	if w.Code != http.StatusOK {
		t.Fatalf("revoke-all %d %s", w.Code, w.Body.String())
	}
	var res struct {
		Revoked   int64 `json:"revoked"`
		SignedOut bool  `json:"signed_out"`
	}
	if err := json.Unmarshal(w.Body.Bytes(), &res); err != nil {
		t.Fatal(err)
	}
	if res.Revoked != 3 || !res.SignedOut {
		t.Fatalf("every session must end and the browser sign out: %+v", res)
	}
	if cleared := w.Header().Values("Set-Cookie"); len(cleared) == 0 {
		t.Fatal("revoke-all left the refresh cookie in place")
	}

	if code, body := do(t, r, http.MethodGet, "/api/sessions", nil,
		map[string]string{"Authorization": "Bearer " + token}); code != http.StatusOK {
		t.Fatalf("sessions after revoke %d %s", code, body)
	} else {
		list.Sessions = nil
		if err := json.Unmarshal(body, &list); err != nil {
			t.Fatal(err)
		}
		if len(list.Sessions) != 0 {
			t.Fatalf("revoked sessions still listed: %s", body)
		}
	}
}
