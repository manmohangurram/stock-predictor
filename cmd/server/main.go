package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	debugpkg "runtime/debug"
	"strings"
	"syscall"
	"time"

	"github.com/gin-gonic/gin"

	"stock-predictor/internal/api"
	"stock-predictor/internal/benchmark"
	"stock-predictor/internal/config"
	"stock-predictor/internal/db"
	"stock-predictor/internal/engine"
	"stock-predictor/internal/fundscan"
	"stock-predictor/internal/httpx"
	"stock-predictor/internal/llm"
	"stock-predictor/internal/logging"
	"stock-predictor/internal/market"
	"stock-predictor/internal/models"
	"stock-predictor/internal/notify"
	"stock-predictor/internal/rules"
	"stock-predictor/internal/runs"
	"stock-predictor/internal/schedule"
	"stock-predictor/internal/secrets"
	"stock-predictor/internal/seed"
	"stock-predictor/internal/store"
	"stock-predictor/internal/trend"
)

// The audit of outbound model calls is kept this long; older rows are the bulk of the file.
const auditKeepDays = 90

func main() {
	debug := false
	for _, a := range os.Args[1:] {
		if a == "--debug" || a == "--verbose" || a == "-v" {
			debug = true
		}
	}
	slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelInfo})))
	if err := run(debug); err != nil {
		slog.Error("fatal", "error", err)
		os.Exit(1)
	}
}

func run(debug bool) error {
	args := os.Args[1:]
	flag := func(name string) bool {
		for _, a := range args {
			if a == name {
				return true
			}
		}
		return false
	}
	flagValue := func(name, fallback string) string {
		for i, a := range args {
			if a == name && i+1 < len(args) {
				return args[i+1]
			}
		}
		return fallback
	}

	for _, name := range config.LoadDotenv() {
		slog.Debug("loaded from .env", "name", name)
	}
	cfg, err := config.FromEnv()
	if err != nil {
		return err
	}
	for _, name := range config.RetiredWarnings() {
		slog.Warn("no longer read — set it in Settings instead (Cadence or Network)", "variable", name)
	}
	logFile := flagValue("--log-file", logging.DefaultPath(cfg.DBPath))
	logging.Setup(debug, logFile)
	if bi, ok := debugpkg.ReadBuildInfo(); ok {
		slog.Info("build", "go", bi.GoVersion, "module", bi.Main.Path,
			"version", bi.Main.Version)
	}
	slog.Info("logging", "file", logFile, "debug", debug, "rotation", "10MB, keep 3")
	dbPath := cfg.DBPath
	if legacy := db.LegacySibling(dbPath); legacy != "" {
		if moved, err := db.MigrateLegacyPath(legacy, dbPath); err != nil {
			return err
		} else if moved != "" {
			slog.Info("moved the database from the old name", "from", legacy, "to", moved)
		}
	}
	slog.Info("database", "path", dbPath)
	pool, err := db.ConnectAndMigrate(context.Background(), dbPath)
	if err != nil {
		return err
	}
	slog.Debug("boot", "bind", cfg.Bind, "public_url", cfg.PublicURL,
		"secret_key_from_env", cfg.SecretKey != nil, "args", args)

	key := cfg.SecretKey
	source := "env"
	if key == nil {
		source = "file"
		slog.Warn("SECRET_KEY is not set — secrets are encrypted with the key file beside the database; set SECRET_KEY to keep the key out of the data directory",
			"path", secrets.KeyPath(cfg.DBPath))
		key, err = secrets.LoadOrCreateKey(cfg.DBPath)
		if err != nil {
			return err
		}
	}
	slog.Info("secret key", "source", source, "bytes", len(key))
	secrets.Init(key)
	if n, err := secrets.Migrate(pool); err != nil {
		return err
	} else if n > 0 {
		slog.Info("encrypted stored secrets", "count", n)
	}
	if err := rules.Seed(pool); err != nil {
		return err
	}

	if flag("--fundamentals") {
		symbol := flagValue("--fundamentals", "INFY.NS")
		rt, err := rules.LoadRuntime(pool)
		if err != nil {
			return err
		}
		client, err := httpx.ClientWith(&rt, 20, "Mozilla/5.0")
		if err != nil {
			return err
		}
		f, err := market.FetchFundamentals(client, symbol, time.Now().Year())
		if err != nil {
			return err
		}
		fmt.Printf("%+v\n---\n%s\n", f, f.Summary())
		return nil
	}

	if flag("--seed") {
		rt, err := rules.LoadRuntime(pool)
		if err != nil {
			return err
		}
		client, err := httpx.ClientWith(&rt, 20, "Mozilla/5.0 stock-predictor")
		if err != nil {
			return err
		}
		report, err := seed.Run(pool, client, !flag("--no-positions"))
		if err != nil {
			return err
		}
		fmt.Printf("seed: %+v\n", report)
	}

	if flag("--test-notify") {
		notifier, err := notify.NtfyFromSettings(pool, cfg.PublicURL)
		if err != nil {
			return err
		}
		if notifier == nil {
			return errors.New("no ntfy topic configured — set one in Settings, or NTFY_TOPIC")
		}
		n := notify.Notification{
			Title:     "stock-predictor · test",
			Body:      "If this arrived on your phone, the push channel works.",
			Priority:  notify.PriorityDefault,
			DedupeKey: "test-notify",
		}
		if err := notifier.SendTest(n); err != nil {
			fmt.Println("failed:", err)
		} else {
			fmt.Println("test notification sent")
		}
		return nil
	}

	deps, err := buildDeps(pool, cfg)
	if err != nil {
		return err
	}

	if flag("--ask") {
		symbol := flagValue("--ask", "")
		var id string
		var classRaw, name string
		err := pool.QueryRow(
			"SELECT id, class, name FROM instruments WHERE symbol = ?", symbol,
		).Scan(&id, &classRaw, &name)
		if err != nil {
			return fmt.Errorf("no instrument %s — add it first", symbol)
		}
		class, _ := models.ParseAssetClass(classRaw)
		instrument := models.Instrument{
			ID: id, Symbol: symbol, Class: class, Name: name, Currency: "INR", Active: true,
		}
		line, err := engine.Advise(deps, instrument, time.Now().UTC())
		if err != nil {
			return err
		}
		if line == nil {
			fmt.Println("ADVICE none")
		} else {
			fmt.Println("ADVICE", *line)
		}
		return nil
	}

	if flag("--poll-once") || flag("--predict") {
		if flag("--predict") {
			deps.Force = true
		}
		report, err := engine.PollOnce(deps, time.Now().UTC())
		if err != nil {
			return err
		}
		for _, note := range report.Notes {
			fmt.Println("DECISION", note)
		}
		fmt.Printf("poll: %+v\n", report)
		rt := deps.Runtime
		client, _ := httpx.ClientWith(&rt, 20, "Mozilla/5.0 stock-predictor")
		if client != nil {
			if scan, err := fundscan.Run(pool, &rt, client, time.Now().UTC()); err == nil && scan.Examined > 0 {
				fmt.Printf("fund scan: %+v\n", scan)
			}
		}
		if planned, note, err := engine.PlanTrade(deps, time.Now().UTC(), nil); err != nil {
			fmt.Println("trade plan failed:", err)
		} else if planned {
			fmt.Println("trade plan: stored")
		} else if note != "" {
			fmt.Println("trade plan:", note)
		} else {
			fmt.Println("trade plan: nothing worth doing")
		}
		if !flag("--serve") {
			return nil
		}
	}

	manual := make(chan api.ManualRequest, 8)
	state := api.NewState(pool, cfg)
	state.Manual = manual
	router := api.Router(state)
	spa := spaFallback()
	router.NoRoute(func(c *gin.Context) {
		// An unknown API path is a 404, not the SPA's index page pretending to be JSON.
		if strings.HasPrefix(c.Request.URL.Path, "/api/") {
			c.JSON(http.StatusNotFound, gin.H{"error": "not found"})
			return
		}
		spa(c)
	})

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	go pollLoop(ctx, pool, cfg, manual)

	server := &http.Server{Addr: cfg.Bind, Handler: router}
	errCh := make(chan error, 1)
	go func() {
		slog.Info("listening", "url", "http://"+cfg.Bind)
		if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
	}()

	select {
	case <-ctx.Done():
		slog.Info("shutting down, checkpointing the database")
	case err := <-errCh:
		return err
	}
	shutdownCtx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()
	_ = server.Shutdown(shutdownCtx)
	_, _ = pool.Exec("PRAGMA wal_checkpoint(TRUNCATE)")
	return pool.Close()
}

// The app shell: never cached without revalidation. The bundle is content-addressed and can be
// cached; index.html names a hashed script, so a stale copy is the "hard refresh needed" report.
func spaFallback() gin.HandlerFunc {
	return func(c *gin.Context) {
		path := filepath.Join("web/dist", filepath.Clean("/"+c.Request.URL.Path))
		if info, err := os.Stat(path); err == nil && !info.IsDir() {
			c.File(path)
			return
		}
		html, err := os.ReadFile("web/dist/index.html")
		if err != nil {
			c.String(http.StatusOK, "<h1>Build the front end: cd web && npm run build</h1>")
			return
		}
		c.Header("Cache-Control", "no-cache, must-revalidate")
		c.Data(http.StatusOK, "text/html; charset=utf-8", html)
	}
}

func buildDeps(pool *sql.DB, cfg *config.Config) (*engine.EngineDeps, error) {
	rt, err := rules.LoadRuntime(pool)
	if err != nil {
		return nil, err
	}
	key := ""
	if model, err := rules.ModelConfigFor(pool); err == nil {
		key = model.APIKey
	}
	source, err := market.NewYahooSource(&rt)
	if err != nil {
		return nil, err
	}
	transport, err := llm.NewHTTPTransport(key, &rt)
	if err != nil {
		return nil, err
	}
	notifier, err := notify.NtfyFromSettings(pool, cfg.PublicURL)
	if err != nil {
		return nil, err
	}
	if notifier == nil {
		notifier, err = notify.NewNtfy("https://ntfy.sh", "", "", cfg.PublicURL, &rt)
		if err != nil {
			return nil, err
		}
	}
	deps := &engine.EngineDeps{
		Pool: pool, Runtime: rt, Market: source, Transport: transport,
		Notifier: notifier, Rules: rules.DefaultRuleConfig(),
		CandidateBlock: trend.CandidateBlock,
		SweepGlances:   trend.SweepGlances,
		NewsSearch:     market.FetchGoogleNews,
	}
	deps.DailyList = func(pool *sql.DB, m market.MarketSource, now time.Time) (string, error) {
		picks, err := benchmark.DailyList(pool, m, now)
		if err != nil {
			return "", err
		}
		if len(picks) == 0 {
			return "no candidate cleared a positive five-session return today\n", nil
		}
		settings, err := rules.GetSettings(pool)
		if err != nil {
			return "", err
		}
		out := ""
		for _, p := range picks {
			out += p.Line(settings.Currency) + "\n"
		}
		return out, nil
	}
	deps.Fundamentals = func(d *engine.EngineDeps, symbol string, year int) string {
		client, err := httpx.ClientWith(&d.Runtime, 15, "Mozilla/5.0 stock-predictor")
		if err != nil {
			return "unavailable"
		}
		f, err := market.FetchFundamentalsCached(client, symbol, year)
		if err != nil {
			return "unavailable"
		}
		text := f.Summary()
		if strings.TrimSpace(text) == "" {
			return "unavailable"
		}
		return text
	}
	return deps, nil
}

func pollLoop(ctx context.Context, pool *sql.DB, cfg *config.Config, manual <-chan api.ManualRequest) {
	if n, err := runs.CloseOrphans(pool, time.Now().UTC()); err != nil {
		slog.Warn("could not close orphaned runs", "error", err)
	} else if n > 0 {
		slog.Info("closed orphaned runs", "count", n)
	}
	var pending *api.ManualRequest
	for {
		now := time.Now().UTC()
		deps, err := buildDeps(pool, cfg)
		if err != nil {
			slog.Error("clients unavailable, skipping tick", "error", err)
			if !sleepOrManual(ctx, manual, &pending, 60) {
				return
			}
			continue
		}
		slog.Debug("tick start", "manual", pending != nil, "poll_secs", deps.Runtime.PollSecs,
			"off_hours_secs", deps.Runtime.OffHoursSecs, "daily_cap", deps.Runtime.DailyCap,
			"in_session", schedule.InStockWindow(time.Now().UTC()))
		if pending != nil {
			req := *pending
			pending = nil
			slog.Info("manual run starting", "run", req.RunID, "command", req.Cmd)
			runManual(deps, req)
		} else {
			rollMonths(pool)
			report, err := engine.PollOnce(deps, now)
			if err != nil {
				slog.Error("poll failed", "error", err)
			} else {
				slog.Info("poll complete", "asked", report.Asked, "polled", report.Polled,
					"signals", report.Signals, "plans", report.Plans)
			}
			if n, err := llm.PruneCalls(pool, auditKeepDays); err == nil && n > 0 {
				slog.Info("audit rows pruned", "deleted", n)
			}
			if n, err := store.PruneNews(pool, auditKeepDays); err == nil && n > 0 {
				slog.Info("headlines pruned", "deleted", n)
			}
			if n, err := engine.RefreshNews(deps, time.Now().UTC()); err == nil && n > 0 {
				slog.Info("morning news search stored", "headlines", n)
			}
			rt := deps.Runtime
			if client, err := httpx.ClientWith(&rt, 20, "Mozilla/5.0 stock-predictor"); err == nil {
				if scan, err := fundscan.Run(pool, &rt, client, time.Now().UTC()); err == nil && scan.Examined > 0 {
					slog.Info("fund scan", "examined", scan.Examined, "proposed", scan.Proposed)
				}
			}
			if _, _, err := engine.PlanFunds(deps, time.Now().UTC(), nil); err != nil {
				slog.Warn("fund plan failed", "error", err)
			}
			if _, _, err := engine.PlanTrade(deps, time.Now().UTC(), nil); err != nil {
				slog.Warn("trade plan failed", "error", err)
			}
		}
		secs := deps.Runtime.OffHoursSecs
		if schedule.InStockWindow(time.Now().UTC()) {
			secs = deps.Runtime.PollSecs
		}
		if secs < 60 {
			secs = 60
		}
		// Wake on the slot boundary rather than wherever a 15-minute interval lands: the trade
		// plan is due at 09:05, 11:15 and 13:15 IST, not 09:22.
		wait := time.Duration(secs) * time.Second
		reason := "interval"
		if d := schedule.UntilNextSlot(time.Now().UTC()); d > 0 && d < wait {
			wait = d
			reason = "next trade slot"
		}
		slog.Debug("sleeping", "in", wait.Round(time.Second), "reason", reason)
		if !sleepOrManual(ctx, manual, &pending, wait) {
			return
		}
	}
}

// sleepOrManual waits for the next tick or a request from the API, whichever comes first.
func sleepOrManual(ctx context.Context, manual <-chan api.ManualRequest, pending **api.ManualRequest, d time.Duration) bool {
	timer := time.NewTimer(d)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return false
	case req := <-manual:
		*pending = &req
		return true
	case <-timer.C:
		return true
	}
}

func runManual(deps *engine.EngineDeps, req api.ManualRequest) {
	now := time.Now().UTC()
	var summary string
	var err error
	switch req.Cmd.Kind {
	case runs.ManualPoll:
		var report engine.PollReport
		report, err = engine.PollOnce(deps, now)
		summary = fmt.Sprintf("%d of %d instruments asked · %d decisions · %d plans",
			report.Asked, report.Polled, report.Signals, report.Plans)
	case runs.ManualPlanStock:
		var planned bool
		var note string
		planned, note, err = engine.PlanStock(deps, now, &req.RunID)
		summary = planRunSummary(planned, note, "a stock plan is in the ledger")
	case runs.ManualPlanTrade:
		var planned bool
		var note string
		planned, note, err = engine.PlanTrade(deps, now, &req.RunID)
		summary = planRunSummary(planned, note, "a trade plan is in the ledger")
	case runs.ManualPlanFund:
		var planned bool
		var note string
		planned, note, err = engine.PlanFunds(deps, now, &req.RunID)
		summary = planRunSummary(planned, note, "a fund plan is in the ledger")
	case runs.ManualDecide:
		var line *string
		line, err = engine.DecideOne(deps, req.Cmd.Symbol, now)
		if line != nil {
			summary = *line
		} else {
			summary = "no decision — nothing to act on"
		}
	}
	if err != nil {
		slog.Warn("manual run failed", "error", err)
		_ = runs.Fail(deps.Pool, req.RunID, err.Error())
		return
	}
	_ = runs.Finish(deps.Pool, req.RunID, summary, nil)
}

// A manual plan says what it did, and when nothing happened it says why: a run that only reports
// "nothing worth doing" cannot be told apart from a skipped one on the audit page.
func planRunSummary(planned bool, note, stored string) string {
	switch {
	case planned:
		return stored
	case note != "":
		return note
	}
	return "nothing worth doing — no plan"
}

// rollMonths is the month boundary, once per purse: stocks, mutual funds, short-term.
func rollMonths(pool *sql.DB) {
	now := time.Now().UTC()
	for _, purse := range []struct {
		mode   models.Mode
		bucket string
		mf     bool
	}{
		{models.ModeLong, "", false},
		{models.ModeLong, "mf", true},
		{models.ModeTrade, "", false},
	} {
		var allowance float64
		var err error
		if purse.mf {
			allowance, err = rules.MFAllowance(pool)
		} else {
			allowance, err = rules.Allowance(pool, purse.mode)
		}
		if err != nil {
			slog.Error("allowance unreadable", "error", err)
			continue
		}
		report, err := rules.EnsureMonth(pool, purse.mode, purse.bucket, allowance, now)
		if err != nil {
			slog.Error("month reset failed", "error", err)
			continue
		}
		if report.Granted > 0 || report.Expired > 0 {
			label := purse.bucket
			if label == "" {
				label = "main"
			}
			slog.Info("month rolled over", "month", report.Month, "mode", purse.mode,
				"purse", label, "expired", report.Expired, "granted", report.Granted)
		}
	}
}
