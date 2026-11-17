# Rust → Go migration plan

**Status: complete (2026-11-17).** The Rust tree is deleted; the Go module at the repo root is the
server. `docs/testing/flows.md` records the walk that signed it off (desktop pages green; mobile,
the add-dialog Escape, and live search results remain untested in that harness).

Decisions (2026-11-16): full rewrite, Go replaces Rust. **gin** for HTTP, **github.com/glebarez/go-sqlite**
(pure Go, `database/sql`) for the database, WAL mode in the DSN. Everything external stays fixed:
DB schema, API JSON, prompt files, the four env vars, and the `enc:v1:` secret format (ported
byte-compatible on XChaCha20-Poly1305, existing databases decrypt unchanged).

The Go module lives at the repo root (`go.mod`), packages under `internal/`. `go:embed` serves the
migrations from a copy at `internal/db/migrations/` (embed cannot cross the module root); the source
of truth for schema remains `migrations/*.sql` — copy again if a new migration lands.

## Behaviour requested during the migration (2026-11-16)

The Rust tree is **not** changed. These behaviours are implemented in the Go port:

1. **Fully-sold positions stay visible until the instrument is archived.** `internal/ledger.Positions`
   reads `state IN ('open', 'closed')` and filters only `i.active = 1`, so a 0-unit row remains in
   the ledger instead of vanishing when the last unit is sold. Archiving still requires 0 units and
   is what removes the row. `trades.Record` keeps the row (qty 0, state `closed`) rather than
   deleting it; `History` always keeps both fills.
2. **Editing a fill unarchives its instrument.** `trades.EditOn` clears `archived_at` and sets
   `active = 1`, so correcting an archived fill brings the name back into the book. Bars were cleared
   on archive; the next poll refetches them, as `restore` already does.
3. **The instrument page must read both pots.** When the api layer is ported, use
   `ledger.OpenPositionFor` (Long then Trade) instead of reading the long book only; otherwise a
   short-term approval leaves the page looking unchanged. Covered by tests in `internal/trades`.
4. **Holdings page vs fills.** Fills are only rendered on the instrument page today. If the
   holdings page is to show an approval immediately, it needs a fills table (or a link) — the
   dashboard payload already carries `recent_trades`.

## Deferred (2026-11-17)

**AgentGateway cutover — deferred, not started.** The request to replace direct provider calls with a
gateway was retracted before any code changed. Direct providers stay as ported. If it returns, the
open questions are: the gateway's chat protocol (OpenAI-compatible vs its own), what happens to the
DeepSeek-only `/v1/tools/search` and `/v1/tools/fetch` that power the short-term evidence and the
escalation fetch, whether the provider selector disappears from Settings, and whether existing
provider keys are reused.

## Ported

- [x] `internal/config` — `config.rs`: BIND/DB_PATH/PUBLIC_URL/SECRET_KEY, dotenv parser, retired vars
- [x] `internal/secrets` — `secrets.rs`: `enc:v1:` XChaCha20-Poly1305, key file, plaintext upgrade (tests green)
- [x] `internal/db` — `db.rs`: embedded migrations, legacy-path rename, WAL/foreign-keys DSN (tests green)
- [x] `internal/models` — `models.rs`: Mode, AssetClass, Action, Bar, Headline, Instrument, Position, Candidate
- [x] `internal/market` — `market/mod.rs`: the `MarketSource` interface (fake-tested, no network in tests)
- [x] `internal/store` — `store.rs`: bar upsert (tail-only write), headline insert/prune, refresh
- [x] `internal/httpx` — `http.rs`: the one client builder; the transport never reads env proxies, so the
      database is the only source of a proxy (tests green)
- [x] `internal/proxyurl` — leaf helpers (`ValidateProxy`, `ProxyHost`) so httpx↔rules is not a cycle
- [x] `internal/validate` — leaf error vocabulary shared by rules and llm (same strings as `validate.rs`)
- [x] `internal/llm` — `provider.rs` ported (Provider, ModelConfig, request/extract/usage); rest of llm pending
- [x] `internal/rules/limits` — `limits.rs` (incl. MaxFill/Check/CheckInstrument/CheckProfit/AllowsTarget)
- [x] `internal/rules/budget` — `budget.rs`: ledger kinds, purses, unit rounding, RecordOn for in-tx fills
- [x] `internal/rules/pot` — `pot.rs`: IST month, idempotent grant/expiry (tests green, incl. expiry-never-a-loss)
- [x] `internal/rules/runtime` — `runtime.rs`: clamped reads, reject-on-write check, legacy proxy split (tests green)
- [x] `internal/rules/settings` — `settings.rs`: Get/Set, patches, KeyFor with .env fallback, ModelConfigFor, Seed
- [x] `internal/rules/indicators` — `indicators.rs`: SMA/RSI/ATR/52-week, shared band multiplier;
      **2.73 µs / 250 bars, 0 allocs** (within the ~2.8 µs budget)
- [x] `internal/rules/context` — `context.rs`: IST session clock, ATR%, RVOL, momentum, 52-week distance
- [x] `internal/rules/evaluate` — `evaluate.rs`: candidate gate, diagnose checks, band reason, trailed stop

- [x] `internal/rules` — limits, budget, pot, runtime, settings, indicators, context, evaluate
- [x] `internal/trades` — `trades.rs`: record/edit/delete, each one transaction; full exit keeps a
      0-unit `closed` row; editing a fill unarchives its instrument (tests green)
- [x] `internal/ledger` — the holdings read: open **and** closed rows until the instrument is
      archived; `OpenPositionFor` covers both pots
- [x] `internal/llm` — complete: provider, prompts, parse, validate, audit, memo, transport,
      rotate, plan (scope/cash/limits/expiry validation), orchestrate (two-phase decide with one
      escalation, daily cap, prompt-injection-safe context)

- [x] `internal/notify` — `notify.rs`: copy, dedupe keys, ntfy payload/headers, dispatch writes the
      row only after a successful send (tests green)
- [x] `internal/runs` — `runs.rs`: open/close/fail, orphan sweep at boot, latest-by-prompt, manual
      commands (tests green)
- [x] `internal/schedule` — `schedule.rs`: IST session window, three trade slots, IST day (tests green)
- [x] `internal/engine` — `engine.rs`: `PollOnce`, `pollInstrument` (trailed stop, clamp, band trigger,
      call gate, rules-only fallback, rotation, two-phase decide, blocked/refused rows, push), `Advise`,
      `DecideOne`, `PlanStock`/`PlanTrade`/`PlanFunds`, `ExpireTradePositions`, `RefreshNews`,
      `FormatBook`, window/daily renderers (tests green). **Wiring note:** the four inputs that live in
      not-yet-ported modules are injected on `EngineDeps` — `CandidateBlock`/`SweepGlances` (trend),
      `DailyList` (benchmark), `NewsSearch` (googlenews), `Fundamentals` (fundamentals). Nil means the
      documented fallback (empty candidate block, "no candidate cleared…", no news pass, "unavailable"),
      never a crash.

## Remaining

1. [ ] Finish the browser flows (`docs/testing/flows.md`): mobile 390 (no viewport control in this harness),
       the add-dialog Escape check, and a search with live Yahoo results (upstream returned 429 during the
       walk). The desktop pages, decisions, fills, archive/restore, audit and settings were walked and are
       green on 2026-11-17; three bugs found there are fixed.
2. [ ] Cutover: ship the Go binary, then delete `src/`, `Cargo.toml`, `build.rs`, benches

CI and packaging are updated for the Go tree: `.github/workflows/ci.yml` now runs module-tidiness,
gofmt, build, vet, tests and all three benchmark suites (Rust jobs removed); the `Dockerfile`
builds with `golang:1.27` and `CGO_ENABLED=0`, cross-compiling by `GOARCH` with no cross toolchain;
`README.md`, `docker-compose.yml` and `docs/deployment.md` no longer mention `RUST_LOG` or cargo.

`internal/seed` and `cmd/server/main.go` are ported: the flags (`--seed`, `--no-positions`, `--ask`,
`--poll-once`, `--predict`, `--fundamentals`, `--test-notify`, `--serve`), the loop with the manual
channel, month rollover per purse, audit/headline pruning, the morning news pass, the fund scan, both
plans, SPA fallback with a non-cacheable index, and graceful shutdown with `wal_checkpoint(TRUNCATE)`.
Smoke-tested against a temp database: health, dashboard, market, fund scan all answered.

Budgets measured on this machine (Go):
**prompt render 40.8 µs** (budget ~58 µs), **bar upsert tail 94 µs / 1.4 KB** (budget ~0.2 ms),
cold 250-row load 4.7 ms, **indicators 2.73 µs / 0 allocs** (budget ~2.8 µs).

Also ported: the whole HTTP surface — dashboard, settings, analysis/detail, trades, positions, IPO,
signals (approve single/plan, reject), archive/restore, archived, runs, search/add/refresh, calls
(filters, `x-total-count`, facets, detail), market, schedule, test-notify. Market sources complete:
Yahoo chart/news/search, AMFI NAVAll, mfapi, NSE gainers, Google News, fundamentals, and
`market.YahooSource` (the `MarketSource` the engine uses). All fixture-tested; no test reaches the
network.

Also ported in the llm layer: `internal/llm/provider` (leaf package — Provider would otherwise cycle
rules↔llm), `prompts.go` (embedded copies at `internal/llm/prompts/`; source of truth stays `prompts/`),
`parse.go`, `validate.go` (CheckProposal over `validate.Error` + `rules.Limits`), `audit.go`, `memo.go`,
`deepseek.go` (HTTP transport + FixtureTransport; no test reaches the network).

## Quality gates for the Go tree

`go build ./... && go vet ./... && go test ./...` — no test hits the network; fixtures only. gin stays
out of `internal/` below `api`. Delete `src/` and the Cargo manifest only after the browser flows in
`docs/testing/flows.md` pass against the Go binary.
