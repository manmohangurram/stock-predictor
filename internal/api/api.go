// Package api is the HTTP surface: the same routes and the same JSON the Rust build served.
package api

import (
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"fmt"
	"log/slog"
	"net/http"
	"runtime/debug"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/config"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/tstamp"
)

type ManualRequest struct {
	RunID string
	Cmd   runs.Manual
}

type AppState struct {
	Pool *sql.DB
	Cfg  *config.Config
	// The channel the API uses to ask the loop to run something.
	Manual chan ManualRequest

	// AMFI's scheme file is ~1.5 MB; it is fetched at most every few hours, not per keystroke.
	amfiMu   sync.Mutex
	amfiAt   time.Time
	amfiText string
}

// AMFIText is the cached scheme file, refreshed at most every six hours. The download happens
// outside the lock so one slow request cannot stall every other caller.
func (st *AppState) AMFIText() *string {
	st.amfiMu.Lock()
	if st.amfiText != "" && time.Since(st.amfiAt) < 6*time.Hour {
		text := st.amfiText
		st.amfiMu.Unlock()
		return &text
	}
	st.amfiMu.Unlock()

	rt, err := rules.LoadRuntime(st.Pool)
	if err != nil {
		return nil
	}
	client, err := market.YahooClient(&rt)
	if err != nil {
		return nil
	}
	text, err := market.FetchNAVAll(client)
	if err != nil {
		slog.Warn("AMFI unavailable", "error", err)
		return nil
	}

	st.amfiMu.Lock()
	defer st.amfiMu.Unlock()
	st.amfiText = text
	st.amfiAt = time.Now()
	cached := st.amfiText
	return &cached
}

func NewState(pool *sql.DB, cfg *config.Config) *AppState {
	return &AppState{Pool: pool, Cfg: cfg}
}

type ApiError struct {
	Status  int
	Message string
}

func (e *ApiError) Error() string { return e.Message }

func badRequest(msg string) *ApiError { return &ApiError{Status: http.StatusBadRequest, Message: msg} }
func notFound(msg string) *ApiError   { return &ApiError{Status: http.StatusNotFound, Message: msg} }
func internalErr(err error) *ApiError {
	return &ApiError{Status: http.StatusInternalServerError, Message: err.Error()}
}

func respondErr(c *gin.Context, err error) {
	if apiErr, ok := err.(*ApiError); ok {
		if apiErr.Status >= 500 {
			slog.Error("api request failed", "method", c.Request.Method, "path", c.Request.URL.Path,
				"status", apiErr.Status, "error", apiErr.Message)
		} else {
			slog.Warn("api refused", "method", c.Request.Method, "path", c.Request.URL.Path,
				"status", apiErr.Status, "error", apiErr.Message)
		}
		c.JSON(apiErr.Status, gin.H{"error": apiErr.Message})
		return
	}
	slog.Error("api request failed", "method", c.Request.Method, "path", c.Request.URL.Path,
		"error", err)
	c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
}

// Router wires the endpoints. Any path not matched falls through to the SPA's static files,
// which the server mounts as a fallback.
func Router(st *AppState) *gin.Engine {
	r := gin.New()
	r.Use(authGate(st))
	// A panic must leave a stack in the log file, not only on the terminal that happened to be
	// attached.
	r.Use(func(c *gin.Context) {
		defer func() {
			if rec := recover(); rec != nil {
				slog.Error("api panic", "method", c.Request.Method, "path", c.Request.URL.Path,
					"panic", fmt.Sprint(rec), "stack", string(debug.Stack()))
				c.AbortWithStatusJSON(http.StatusInternalServerError,
					gin.H{"error": "internal error"})
			}
		}()
		c.Next()
	})
	// API responses may be reused only after revalidation, so a changed row is never served from
	// a stale copy while the dashboard's ETag still saves the body.
	r.Use(func(c *gin.Context) {
		c.Header("Cache-Control", "no-cache, must-revalidate")
		c.Next()
	})
	// One line per request at debug; 4xx/5xx are always logged so a failing page leaves a trace.
	r.Use(func(c *gin.Context) {
		start := time.Now()
		c.Next()
		status := c.Writer.Status()
		ms := time.Since(start).Milliseconds()
		attrs := []any{"method", c.Request.Method, "path", c.Request.URL.Path,
			"query", c.Request.URL.RawQuery, "status", status, "ms", ms, "bytes", c.Writer.Size()}
		switch {
		case status >= 500:
			slog.Error("api request", attrs...)
		case status >= 400 && status != http.StatusNotModified:
			slog.Warn("api request", attrs...)
		case ms > 1500:
			slog.Warn("slow api request", attrs...)
		default:
			slog.Debug("api request", attrs...)
		}
	})
	r.POST("/api/auth/setup", func(c *gin.Context) { authSetup(c, st) })
	r.POST("/api/auth/login", func(c *gin.Context) { authLogin(c, st) })
	r.POST("/api/auth/refresh", func(c *gin.Context) { authRefresh(c, st) })
	r.POST("/api/auth/logout", func(c *gin.Context) { authLogout(c, st) })
	r.GET("/api/auth/status", func(c *gin.Context) { authStatus(c, st) })
	r.GET("/api/auth/me", func(c *gin.Context) { authMe(c, st) })
	r.POST("/api/auth/password", func(c *gin.Context) { authPassword(c, st) })

	r.GET("/api/health", func(c *gin.Context) { c.JSON(http.StatusOK, gin.H{"ok": true}) })

	r.GET("/api/dashboard", func(c *gin.Context) { dashboardHandler(c, st) })
	r.GET("/api/settings", func(c *gin.Context) { getSettings(c, st) })
	r.PUT("/api/settings", func(c *gin.Context) { putSettings(c, st) })
	r.POST("/api/settings/proxy/test", func(c *gin.Context) { testProxy(c, st) })

	r.GET("/api/instruments/:symbol", func(c *gin.Context) { instrumentDetail(c, st) })
	r.GET("/api/instruments/:symbol/analysis", func(c *gin.Context) { analysisHandler(c, st) })
	r.PUT("/api/instruments/:symbol/target-profit", func(c *gin.Context) { setTargetProfit(c, st) })
	r.DELETE("/api/instruments/:symbol", func(c *gin.Context) { archiveInstrument(c, st) })
	r.POST("/api/instruments/:symbol/restore", func(c *gin.Context) { restoreInstrument(c, st) })
	r.GET("/api/instruments/:symbol/trades", func(c *gin.Context) { instrumentTrades(c, st) })
	r.POST("/api/instruments/:symbol/trades", func(c *gin.Context) { recordTrade(c, st) })
	r.GET("/api/archived", func(c *gin.Context) { archivedInstruments(c, st) })

	r.GET("/api/trades", func(c *gin.Context) { allTrades(c, st) })
	r.PATCH("/api/trades/:id", func(c *gin.Context) { editTrade(c, st) })
	r.DELETE("/api/trades/:id", func(c *gin.Context) { deleteTrade(c, st) })

	r.POST("/api/positions", func(c *gin.Context) { postPosition(c, st) })
	r.PATCH("/api/positions/:id", func(c *gin.Context) { patchPosition(c, st) })
	r.POST("/api/ipo", func(c *gin.Context) { postIPO(c, st) })

	r.POST("/api/signals/:id/approve", func(c *gin.Context) { approveSignal(c, st) })
	r.POST("/api/signals/:id/reject", func(c *gin.Context) { rejectSignal(c, st) })

	r.GET("/api/runs", func(c *gin.Context) { listRuns(c, st) })
	r.POST("/api/runs", func(c *gin.Context) { startRun(c, st) })
	r.POST("/api/notify/test", func(c *gin.Context) { testNotify(c, st) })

	r.GET("/api/search", func(c *gin.Context) { searchInstruments(c, st) })
	r.POST("/api/instruments", func(c *gin.Context) { addInstrument(c, st) })
	r.POST("/api/instruments/:symbol/refresh", func(c *gin.Context) { refreshInstrument(c, st) })
	r.GET("/api/calls", func(c *gin.Context) { calls(c, st) })
	r.GET("/api/calls/facets", func(c *gin.Context) { callFacets(c, st) })
	r.GET("/api/calls/:id", func(c *gin.Context) { callDetail(c, st) })
	r.GET("/api/market", func(c *gin.Context) { marketState(c, st) })
	r.GET("/api/schedule", func(c *gin.Context) { scheduleView(c, st) })
	r.GET("/api/keys", func(c *gin.Context) { listKeys(c, st) })
	r.POST("/api/keys", func(c *gin.Context) { createKey(c, st) })
	r.DELETE("/api/keys/:id", func(c *gin.Context) { revokeKey(c, st) })

	r.Any("/mcp", gin.WrapH(MCPHandler(st, r)))
	return r
}

func modeOf(raw string) (models.Mode, *ApiError) {
	switch strings.ToLower(strings.TrimSpace(raw)) {
	case "", "long":
		return models.ModeLong, nil
	case "trade":
		return models.ModeTrade, nil
	}
	return "", badRequest(fmt.Sprintf("mode must be long or trade, got %q", raw))
}

func instrumentIDBySymbol(pool *sql.DB, symbol string) (string, error) {
	var id string
	err := pool.QueryRow("SELECT id FROM instruments WHERE symbol = ?", symbol).Scan(&id)
	if err == sql.ErrNoRows {
		return "", notFound("no instrument " + symbol)
	}
	if err != nil {
		return "", internalErr(err)
	}
	return id, nil
}

func pathID(c *gin.Context) (int64, error) {
	id, err := strconv.ParseInt(c.Param("id"), 10, 64)
	if err != nil {
		return 0, badRequest("id must be a number")
	}
	return id, nil
}

// The tag fingerprints the data, not row counts: approving changes no id, editing a fill changes
// values in place, and archiving flips a flag. Counting rows answered 304 through all three.
func dashboardETag(pool *sql.DB, mode models.Mode) (string, error) {
	var pending, instruments int64
	var signals, trades, ledger, positions, settings string
	err := pool.QueryRow(
		`SELECT (SELECT COALESCE(MAX(id),'') FROM signals WHERE mode = ?) AS signals,
                (SELECT COUNT(*) FROM signals WHERE mode = ? AND status = 'pending') AS pending,
                (SELECT COALESCE(MAX(id),'') FROM trades WHERE mode = ?) AS trades,
                (SELECT COALESCE(MAX(id),'') FROM cash_ledger WHERE mode = ?) AS ledger,
                (SELECT COALESCE(group_concat(id || ':' || qty || ':' || avg_cost || ':'
                         || COALESCE(stop, '') || ':' || COALESCE(target, '') || ':' || state), '')
                   FROM positions WHERE mode = ?) AS positions,
                (SELECT COUNT(*) FROM instruments WHERE active = 1) AS instruments,
                (SELECT COALESCE(group_concat(key || '=' || value), '') FROM settings) AS settings`,
		string(mode), string(mode), string(mode), string(mode), string(mode),
	).Scan(&signals, &pending, &trades, &ledger, &positions, &instruments, &settings)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s-%d-%s-%s-%s-%d-%s-%s",
		signals, pending, trades, ledger, positions, instruments, settings, mode)))
	return `"` + hex.EncodeToString(sum[:]) + `"`, nil
}

var _ = schedule.ISTDay
var _ = time.Now
var _ = rules.DefaultLimits

func tstampNow() string { return tstamp.Now() }
