package api

import (
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/auth"
)

// The one door into the app. The SPA holds the access token in memory and sends it as a bearer; the
// refresh token lives in an HttpOnly cookie scoped to /api/auth so a script cannot read it.

const refreshCookie = "sp_refresh"

func signingKey(st *AppState) []byte { return auth.SigningKey(string(st.Cfg.SecretKey)) }

func secureCookies(st *AppState) bool { return strings.HasPrefix(st.Cfg.PublicURL, "https://") }

func setRefreshCookie(c *gin.Context, st *AppState, token string, expires time.Time) {
	age := int(time.Until(expires).Seconds())
	if age < 0 {
		age = 0
	}
	c.SetSameSite(http.SameSiteLaxMode)
	c.SetCookie(refreshCookie, token, age, "/api/auth", "", secureCookies(st), true)
}

func clearRefreshCookie(c *gin.Context, st *AppState) {
	c.SetSameSite(http.SameSiteLaxMode)
	c.SetCookie(refreshCookie, "", -1, "/api/auth", "", secureCookies(st), true)
}

func bearerFrom(c *gin.Context) string {
	raw := c.GetHeader("Authorization")
	const prefix = "Bearer "
	if len(raw) > len(prefix) && strings.EqualFold(raw[:len(prefix)], prefix) {
		return strings.TrimSpace(raw[len(prefix):])
	}
	return ""
}

// authGate refuses everything under /api/ except the health probe and the auth endpoints. Static
// files and the SPA shell stay public: the sign-in page is part of the shell.
func authGate(st *AppState) gin.HandlerFunc {
	key := signingKey(st)
	return func(c *gin.Context) {
		path := c.Request.URL.Path
		if !strings.HasPrefix(path, "/api/") || path == "/api/health" || strings.HasPrefix(path, "/api/auth/") {
			c.Next()
			return
		}
		token := bearerFrom(c)
		if token == "" {
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": "sign in"})
			return
		}
		id, err := auth.VerifyAccess(key, token, time.Now().UTC())
		if err != nil {
			reason := "sign in"
			if err == auth.ErrTokenExpired {
				reason = "token expired"
			}
			c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{"error": reason})
			return
		}
		c.Set("admin_id", id)
		c.Next()
	}
}

type authPayload struct {
	Admin        *auth.Admin `json:"admin"`
	AccessToken  string      `json:"access_token"`
	ExpiresAt    time.Time   `json:"expires_at"`
	RefreshToken string      `json:"-"`
	RefreshEnd   time.Time   `json:"-"`
}

func authStatus(c *gin.Context, st *AppState) {
	needs, err := auth.NeedsSetup(st.Pool)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	out := gin.H{"needs_setup": needs}
	if !needs {
		if a, err := auth.Current(st.Pool); err == nil {
			out["username"] = a.Username
		}
		_, err := auth.VerifyAccess(signingKey(st), bearerFrom(c), time.Now().UTC())
		out["signed_in"] = err == nil
	}
	c.JSON(http.StatusOK, out)
}

func authSetup(c *gin.Context, st *AppState) {
	var body struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	a, err := auth.Create(st.Pool, body.Username, body.Password, time.Now().UTC())
	if err == auth.ErrExists {
		respondErr(c, &ApiError{Status: http.StatusConflict, Message: "an admin already exists"})
		return
	}
	if err != nil {
		respondErr(c, badRequest(err.Error()))
		return
	}
	tokens, err := auth.Issue(st.Pool, signingKey(st), a, c.Request.UserAgent(), time.Now().UTC())
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	setRefreshCookie(c, st, tokens.Refresh, tokens.RefreshExpires)
	c.JSON(http.StatusCreated, authPayload{Admin: a, AccessToken: tokens.Access, ExpiresAt: tokens.AccessExpires})
}

// loginLimiter is a small in-memory brake: a wrong password costs a pause, and an address that keeps
// guessing is refused for a while. Restarting the app clears it, which is fine for one admin.
type loginLimiter struct {
	mu      sync.Mutex
	counts  map[string]int
	resetAt time.Time
}

var limiter = &loginLimiter{counts: map[string]int{}}

func (l *loginLimiter) blocked(ip string, now time.Time) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	if now.After(l.resetAt) {
		l.counts = map[string]int{}
		l.resetAt = now.Add(15 * time.Minute)
	}
	return l.counts[ip] >= 30
}

func (l *loginLimiter) fail(ip string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.counts[ip]++
}

func authLogin(c *gin.Context, st *AppState) {
	var body struct {
		Username string `json:"username"`
		Password string `json:"password"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	now := time.Now().UTC()
	if limiter.blocked(c.ClientIP(), now) {
		respondErr(c, &ApiError{Status: http.StatusTooManyRequests, Message: "too many attempts, try again later"})
		return
	}
	a, err := auth.Verify(st.Pool, body.Username, body.Password)
	if err != nil {
		limiter.fail(c.ClientIP())
		time.Sleep(400 * time.Millisecond)
		respondErr(c, &ApiError{Status: http.StatusUnauthorized, Message: "invalid credentials"})
		return
	}
	tokens, err := auth.Issue(st.Pool, signingKey(st), a, c.Request.UserAgent(), now)
	if err != nil {
		respondErr(c, internalErr(err))
		return
	}
	setRefreshCookie(c, st, tokens.Refresh, tokens.RefreshExpires)
	c.JSON(http.StatusOK, authPayload{Admin: a, AccessToken: tokens.Access, ExpiresAt: tokens.AccessExpires})
}

func authRefresh(c *gin.Context, st *AppState) {
	presented, err := c.Cookie(refreshCookie)
	if err != nil || presented == "" {
		respondErr(c, &ApiError{Status: http.StatusUnauthorized, Message: "sign in"})
		return
	}
	a, tokens, err := auth.Rotate(st.Pool, signingKey(st), presented, c.Request.UserAgent(), time.Now().UTC())
	if err != nil {
		clearRefreshCookie(c, st)
		respondErr(c, &ApiError{Status: http.StatusUnauthorized, Message: "sign in"})
		return
	}
	setRefreshCookie(c, st, tokens.Refresh, tokens.RefreshExpires)
	c.JSON(http.StatusOK, authPayload{Admin: a, AccessToken: tokens.Access, ExpiresAt: tokens.AccessExpires})
}

func authLogout(c *gin.Context, st *AppState) {
	if presented, err := c.Cookie(refreshCookie); err == nil && presented != "" {
		_ = auth.Revoke(st.Pool, presented, time.Now().UTC())
	}
	clearRefreshCookie(c, st)
	c.JSON(http.StatusOK, gin.H{"signed_out": true})
}

func authMe(c *gin.Context, st *AppState) {
	a, err := auth.Current(st.Pool)
	if err != nil {
		respondErr(c, notFound("no admin"))
		return
	}
	c.JSON(http.StatusOK, gin.H{"username": a.Username})
}

func authPassword(c *gin.Context, st *AppState) {
	var body struct {
		Current string `json:"current"`
		Next    string `json:"next"`
	}
	if err := c.ShouldBindJSON(&body); err != nil {
		respondErr(c, badRequest("invalid body"))
		return
	}
	err := auth.ChangePassword(st.Pool, body.Current, body.Next, time.Now().UTC())
	switch err {
	case nil:
		clearRefreshCookie(c, st)
		c.JSON(http.StatusOK, gin.H{"changed": true, "signed_out": true})
	case auth.ErrBadPassword:
		respondErr(c, &ApiError{Status: http.StatusForbidden, Message: "current password is wrong"})
	case auth.ErrShortPass:
		respondErr(c, badRequest(err.Error()))
	default:
		respondErr(c, internalErr(err))
	}
}
