# stock-predicter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A self-hosted Rust server that polls Yahoo/AMFI every 10 minutes, runs a rules pre-filter plus a two-phase DeepSeek judgement with server-side validation, tracks holdings against a hard cash budget, and pushes a single ntfy notification per actionable change.

**Architecture:** axum serves both JSON and the built Vite bundle from one process. A tokio scheduler ticks every 10 minutes during market hours. Each tick: fetch bars/news → cheap rules candidate → only if a candidate exists, DeepSeek phase 1 (headline ids) → optional phase 2 (fetched article extracts) → validator → persisted signal → deduped notification. sqlite holds instruments, bars, news, signals, positions, cash ledger, IPO reservations, and an append-only `api_calls` audit trail.

**Tech Stack:** Rust (axum, tokio, reqwest, serde, sqlx + sqlite, chrono, sha2, anyhow, thiserror), Vite + TypeScript, DeepSeek chat + `/v1/tools/search` + `/v1/tools/fetch`, ntfy.

**Spec:** `docs/superpowers/specs/2026-10-27-stock-predicter-design.md`

## Global Constraints

- Budget ceiling lives in the `settings` table and is edited from the UI; `BUDGET`/`CURRENCY` env values seed it on first boot only. `available = budget − committed`, where committed includes open stock cost, MF cost, and **cash reserved by pending IPO applications**. A budget cannot be set below `committed`; currency is locked once the ledger has entries.
- No authentication. Bind `127.0.0.1` or LAN only. Never port-forward.
- Stock polling window 09:10–15:35 IST on trading days, every 10 minutes. One daily pass after 19:00 for AMFI NAV + IPO status.
- MF sells are labeled estimates with T+1/T+2 cash arrival. IPO cash is a reservation, not a purchase.
- LLM escalation caps: 1 escalation round (2 chat calls max), 3 headlines per escalation, 4,000 chars per article, 10,000 chars total fetched, daily call cap from config.
- Every DeepSeek chat/search/fetch call is one append-only `api_calls` row. Rows are never updated or deleted.
- Headlines reach the model as `{id, title, publisher, date}` only. Article bodies are stored NULL and fetched only on escalation.
- Validator rejects rather than repairs: stop on wrong side, price outside ATR band, qty exceeding cash, SELL of unheld instrument, invalid `valid_until`, unknown `headline_id`.
- All secrets (`DEEPSEEK_API_KEY`, `NTFY_TOPIC`) come from env, never source.
- No test may hit the network. Fixtures only.
- **Code reading protocol** (see `AGENTS.md`): locate with `semble search`, resolve ranges with
  `read-ranges.py` against `graphify-out/graph.json`, then read only `path:start-end`. Build
  the graph after the first Rust files land. No task may open a whole source file to inspect
  one function.

## Review Focus

- **Yahoo returns a short or empty series** (halted, delisted, single bar): indicators must return an error or `None`, never NaN, and no signal may be emitted from NaN.
- **DeepSeek returns invalid JSON, truncated JSON, or wrong types**: parse failure must degrade to the rules candidate, never panic or emit an unvalidated signal.
- **Fractional MF units**: 3-decimal rounding must never push committed spend above the budget ceiling.
- **The same news URL arriving on repeated polls**: must not be re-scored, re-sent to the LLM, or re-notified.
- **ntfy unreachable or returning non-2xx**: the poll loop must survive and retry on the next tick without duplicating the notification.

---

### Task 1: Crate scaffold, config, and schema

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/config.rs`, `src/db.rs`, `src/models.rs`, `migrations/0001_init.sql`
- Test: `tests/config.rs`

**Interfaces:**
- Consumes: nothing
- Produces: `Config` with fields `seed_budget: f64`, `seed_currency: String`, `poll_secs: u64`, `llm_daily_cap: u32`, `ntfy_url: String`, `ntfy_topic: String`, `deepseek_key: String`; `Config::from_env() -> anyhow::Result<Config>`; `db::connect(path: &str) -> anyhow::Result<sqlx::SqlitePool>`; `db::migrate(&SqlitePool) -> anyhow::Result<()>`; `models::AssetClass { Stock, MutualFund, Ipo }`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn config_defaults_poll_and_cap() {
    let cfg = Config::from_map(&map![
        "BUDGET" => "200000", "DEEPSEEK_API_KEY" => "k",
        "NTFY_TOPIC" => "t", "NTFY_BASE_URL" => "https://ntfy.sh"
    ]).unwrap();
    assert_eq!(cfg.seed_budget, 200_000.0);
    assert_eq!(cfg.seed_currency, "INR");
    assert_eq!(cfg.poll_secs, 600);
    assert_eq!(cfg.llm_daily_cap, 50);
}
```

- [ ] **Step 2: Run it, expect failure**

Run: `cargo test --test config`
Expected: FAIL — `Config` not defined.

- [ ] **Step 3: Implement**

`Config::from_map` reads a `&BTreeMap<String,String>` so tests need no env mutation; `from_env` delegates to it. `BUDGET` and `CURRENCY` are **seed values only** — named `seed_budget` / `seed_currency` in the struct to make that unmissable — written into `settings` on first boot if absent, and never read again afterward. Defaults: `CURRENCY=INR`, `poll_secs=600`, `llm_daily_cap=50`, `NTFY_BASE_URL=https://ntfy.sh`. `migrations/0001_init.sql` creates `settings`, `instruments`, `bars`, `news`, `signals`, `positions`, `cash_ledger`, `ipo_applications`, `notifications`, `api_calls`, `search_results` exactly as named in the spec's data model, plus `CREATE UNIQUE INDEX` on `news.url`, on `notifications.dedupe_key`, and `(instrument_id, ts)` on `bars`.

- [ ] **Step 4: Run tests, expect pass**

Run: `cargo test --test config && cargo test`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git init && git add -A && git commit -m "feat: scaffold crate, config, sqlite schema"
```

---

### Task 2: Yahoo bars and news client (single choke point)

**Files:**
- Create: `src/market/mod.rs`, `src/market/yahoo.rs`, `tests/fixtures/yahoo_chart.json`, `tests/fixtures/yahoo_news.json`
- Test: `tests/yahoo.rs`

**Interfaces:**
- Consumes: `models::Bar`, `models::Headline`
- Produces: `yahoo::fetch_bars(client: &reqwest::Client, symbol: &str) -> Result<Vec<Bar>>`; `yahoo::fetch_news(client: &reqwest::Client, symbol: &str) -> Result<Vec<Headline>>`; `Headline { id: i64, title: String, publisher: String, published_at: String, url: String }` (id assigned later by the caller, `0` from the parser)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn parses_chart_fixture_into_bars() {
    let bars = yahoo::parse_chart(FIXTURE_CHART).unwrap();
    assert_eq!(bars.len(), 5);
    assert!((bars[0].close - 141.2).abs() < 1e-9);
}

#[test]
fn empty_chart_yields_empty_vec_not_error() {
    assert_eq!(yahoo::parse_chart(r#"{"chart":{"result":[]}}"#).unwrap().len(), 0);
}

#[test]
fn news_parser_keeps_headlines_and_never_a_body() {
    let h = yahoo::parse_news(FIXTURE_NEWS).unwrap();
    assert_eq!(h[0].title, "Infosys guidance raised");
    assert_eq!(h[0].publisher, "Reuters");
    assert!(h[0].url.starts_with("https://"));
    // Headline carries no body field at all — the type makes this structural.
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test yahoo`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

`parse_chart` reads `chart.result[0].timestamp` and `indicators.quote[0]{open,high,low,close,volume}`, skips any index with a null value, and returns `Ok(vec![])` when `result` is empty or null. `parse_news` targets Yahoo's news JSON, dedupes by url within one response, and maps `providerPublishTime` epoch → RFC3339 string. Live fetch functions are thin wrappers: `https://query1.finance.yahoo.com/v8/finance/chart/{sym}?range=1y&interval=1d`, and the news endpoint — all through these two functions so endpoint drift touches one file. On non-2xx or unparseable body, return an error; never a silent empty success.

- [ ] **Step 4: Run tests**

Run: `cargo test --test yahoo`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: yahoo bars and headline-only news client"
```

---

### Task 3: AMFI NAV ingestion

**Files:**
- Create: `src/market/amfi.rs`, `tests/fixtures/amfi_navall.txt`
- Test: `tests/amfi.rs`

**Interfaces:**
- Consumes: nothing
- Produces: `amfi::parse_nav(text: &str, scheme_code: &str) -> Result<NavRow>` where `NavRow { scheme_code: String, name: String, nav: f64, date: String }`; `amfi::fetch_nav_all(client) -> Result<String>`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn parses_semicolon_separated_nav_row() {
    let row = amfi::parse_nav(FIXTURE_NAVALL, "120503").unwrap();
    assert_eq!(row.scheme_code, "120503");
    assert!((row.nav - 45.6721).abs() < 1e-9);
    assert_eq!(row.date, "2026-10-24");
}

#[test]
fn unknown_scheme_is_an_error_not_a_zero_nav() {
    assert!(amfi::parse_nav(FIXTURE_NAVALL, "000000").is_err());
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test amfi`
Expected: FAIL.

- [ ] **Step 3: Implement**

AMFI `NAVAll.txt` is `;`-delimited with a header line. Match on scheme code, parse NAV, normalise `dd-Mmm-yyyy` to ISO. A missing scheme is an error — a zero NAV silently poisons the budget ledger.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test amfi`
Expected: PASS.

```bash
git add -A && git commit -m "feat: AMFI NAVAll ingestion for mutual funds"
```

---

### Task 4: Indicators with degenerate-input handling

**Files:**
- Create: `src/rules/mod.rs`, `src/rules/indicators.rs`
- Test: `tests/indicators.rs`

**Interfaces:**
- Consumes: `models::Bar`
- Produces: `indicators::compute(bars: &[Bar]) -> Result<Indicators>`; `Indicators { sma50: f64, sma200: f64, rsi14: f64, atr14: f64, high52: f64, low52: f64 }`; `indicators::atr_bands(last_close: f64, atr14: f64) -> Bands` with `Bands { min: f64, max: f64 }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn computes_sma_on_known_series() {
    let i = indicators::compute(&series(&[1.0,2.0,3.0,4.0,5.0], 60)).unwrap();
    assert!((i.sma50 - 3.0).abs() < 1e-9);
}

#[test]
fn too_few_bars_is_an_error_not_nan() {
    let err = indicators::compute(&series(&[10.0, 11.0], 2)).unwrap_err();
    assert!(matches!(err, indicators::IndicatorError::InsufficientBars));
}

#[test]
fn bands_bracket_last_close_by_atr_multiple() {
    let b = indicators::atr_bands(100.0, 2.0);
    assert!(b.min < 100.0 && b.max > 100.0);
    assert!((b.max - b.min - 4.0 * BAND_MULT).abs() < 1e-9);
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test indicators`
Expected: FAIL.

- [ ] **Step 3: Implement**

`compute` requires ≥ 200 bars for SMA200 and returns `InsufficientBars` below that; every output must be finite or the function errors. RSI14 uses Wilder smoothing; ATR14 uses true range mean; `high52`/`low52` over the last 252 bars. `BAND_MULT` is a `pub const f64 = 1.0`, the multiplier the validator uses — one constant, defined once, referenced by both.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test indicators`
Expected: PASS.

```bash
git add -A && git commit -m "feat: indicators with degenerate-series errors and ATR bands"
```

---

### Task 5: Cash ledger and budget gate

**Files:**
- Create: `src/rules/budget.rs`
- Test: `tests/budget.rs`

**Interfaces:**
- Consumes: `db::connect`, `settings::budget`
- Produces: `settings::get(pool) -> Result<Settings>` with `Settings { budget: f64, currency: String, committed: f64, currency_locked: bool }`; `settings::set(pool, patch: SettingsPatch) -> Result<(), SettingsError>`; `SettingsPatch { budget: Option<f64>, currency: Option<String> }`; `SettingsError { BelowCommitted { committed: f64 }, CurrencyLocked, InvalidAmount }`; `budget::available(pool: &SqlitePool) -> Result<f64>`; `budget::committed(pool: &SqlitePool) -> Result<f64>`; `budget::record(pool, entry: NewCashEntry) -> Result<()>`; `budget::reserve_ipo(pool, instrument_id: i64, qty: f64, price: f64) -> Result<()>`; `budget::fits(pool, cost: f64) -> Result<bool>`; `NewCashEntry { kind: CashKind, amount: f64, reference: String }`; `CashKind { Budget, Buy, Sell, Reserve, Release }`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn buy_within_budget_is_allowed() { /* budget 1000, cost 600 → fits == true */ }

#[tokio::test]
async fn buy_exceeding_budget_is_refused() { /* budget 1000, cost 1001 → fits == false */ }

#[tokio::test]
async fn pending_ipo_reservation_blocks_a_stock_buy() {
    // budget 1000, reserve_ipo(500) → available 500, cost 600 → fits == false
}

#[tokio::test]
async fn fractional_mf_rounding_never_exceeds_ceiling() {
    // 3-decimal units of 45.6721 → total committed <= budget
}

#[tokio::test]
async fn budget_below_committed_is_refused_with_the_figure() {
    // committed 1420, set budget 1000 → Err(BelowCommitted { committed: 1420.0 })
}

#[tokio::test]
async fn currency_change_is_refused_once_ledger_has_entries() {
    // one BUY entry exists → set currency "USD" → Err(CurrencyLocked)
}

#[tokio::test]
async fn currency_change_is_allowed_on_a_fresh_ledger() { /* Ok(()) */ }

#[tokio::test]
async fn raising_the_budget_is_reflected_in_available_immediately() {
    // set budget 1000→2000, committed 600 → available 1400
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test budget`
Expected: FAIL.

- [ ] **Step 3: Implement**

`committed` = Σ(open position qty × avg_cost) + Σ(ipo_applications where status='pending' qty × amount). Rounding rule: MF quantity rounds **down** to 3 decimals before costing, so rounding can only under-spend. Every write goes through `record`, which inserts into `cash_ledger` and updates the derived view; nothing else mutates balances. A budget change is itself a `CashKind::Budget` ledger entry, so the history of the ceiling is auditable and `settings.budget` is a projection of it.

`settings::set` validates before writing: a budget below `committed` returns `BelowCommitted` carrying the committed figure (the UI needs it for its message); any currency change when `cash_ledger` has a non-`Budget` row returns `CurrencyLocked`; a non-finite or negative budget returns `InvalidAmount`.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test budget`
Expected: PASS.

```bash
git add -A && git commit -m "feat: cash ledger with IPO reservations and rounding floor"
```

---

### Task 6: Rules engine producing a Candidate

**Files:**
- Create: `src/rules/evaluate.rs`
- Test: `tests/rules.rs`

**Interfaces:**
- Consumes: `Indicators`, `indicators::atr_bands`, `budget::available`, `AssetClass`
- Produces: `evaluate::candidate(input: CandidateInput) -> Option<Candidate>`; `CandidateInput { instrument: &Instrument, bars: &[Bar], ind: &Indicators, position: Option<&Position>, available_cash: f64 }`; `Candidate { instrument_id: i64, action: Action, reasons: Vec<String>, entry: f64, stop: f64, target: f64, qty: i64, timing_note: String }`; `Action { Buy, Sell, Hold, None }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn uptrend_not_overbought_with_cash_emits_buy() { /* close>sma50>sma200, rsi 55 → Some(Buy) */ }

#[test]
fn overbought_rsi_blocks_buy() { /* rsi 78 → None */ }

#[test]
fn held_position_below_stop_emits_sell() { /* position stop 90, close 88 → Some(Sell) */ }

#[test]
fn buy_without_cash_emits_none() { /* available_cash 0 → None */ }

#[test]
fn sell_never_exceeds_held_quantity() {
    // position qty 5 → candidate.qty == 5
}

#[test]
fn mf_ipo_never_emit_an_intraday_price_target() {
    // class MutualFund → timing_note mentions NAV-at-cutoff, not a stop order
    // class Ipo       → timing_note mentions allotment, cash locked
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test rules`
Expected: FAIL.

- [ ] **Step 3: Implement**

Thresholds live in a `RuleConfig` struct with defaults (RSI buy ceiling 70, stop = entry − 2·ATR, target = entry + 3·ATR). `qty` = floor(available_cash / entry) for BUY, held qty for SELL. Candidates carry `reasons: Vec<String>` naming each indicator that fired. Return `None` when no rule fires — that is what keeps the LLM off the wire.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test rules`
Expected: PASS.

```bash
git add -A && git commit -m "feat: rules engine emitting candidates with reasons"
```

---

### Task 7: Prompt templates and context assembly

**Files:**
- Create: `src/llm/mod.rs`, `src/llm/prompts.rs`, `prompts/decide_v1.md`, `prompts/escalate_v1.md`
- Test: `tests/prompts.rs`

**Interfaces:**
- Consumes: `Instrument`, `Indicators`, `Headline`, `Candidate`, `Config`
- Produces: `prompts::render(name: &str, ctx: &PromptContext) -> Result<RenderedPrompt>`; `RenderedPrompt { text: String, hash: String }`; `PromptContext { instrument, ind, headlines: &[Headline], candidate: &Candidate, cash_available: f64, class_rules: &'static str, search_summaries: &[String] }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rendered_prompt_contains_headline_ids_and_no_article_text() {
    let r = prompts::render("decide_v1", &ctx_with_headlines()).unwrap();
    assert!(r.text.contains("[1] Infosys guidance raised — Reuters"));
    assert!(!r.text.contains("BENGALURU")); // body text must never appear
}

#[test]
fn template_hash_is_stable_and_recorded() {
    let a = prompts::render("decide_v1", &ctx_with_headlines()).unwrap();
    assert_eq!(a.hash.len(), 64);
    assert_eq!(a.hash, prompts::template_hash("decide_v1").unwrap());
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test prompts`
Expected: FAIL.

- [ ] **Step 3: Implement**

Templates are `prompts/*.md` embedded with `include_str!`, rendered with simple `{{key}}` substitution (no template crate needed). `decide_v1.md` states the response JSON union (`decide` / `need_more` / `pass`), the exact numeric fields, and the per-class execution rules. `escalate_v1.md` is the phase-2 prompt carrying the fetched extracts. `hash` is SHA-256 of the template file, not the rendered text.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test prompts`
Expected: PASS.

```bash
git add -A && git commit -m "feat: prompt templates with headline-id context and version hash"
```

---

### Task 8: DeepSeek client with append-only audit

**Files:**
- Create: `src/llm/deepseek.rs`, `tests/fixtures/deepseek_decide.json`, `tests/fixtures/deepseek_search.json`, `tests/fixtures/deepseek_fetch.json`
- Test: `tests/deepseek.rs`

**Interfaces:**
- Consumes: `db::SqlitePool`, `Config`
- Produces: `deepseek::chat(pool, cfg, req: ChatRequest) -> Result<ChatReply>`; `ChatRequest { conversation_id: String, round: u8, prompt: String, prompt_template: String, prompt_hash: String }`; `ChatReply { content: String, prompt_tokens: u32, completion_tokens: u32 }`; `deepseek::search(pool, cfg, query: &str) -> Result<Vec<SearchHit>>`; `deepseek::fetch_page(pool, cfg, url: &str) -> Result<String>`; `SearchHit { title: String, url: String, summary: String, date: Option<String> }`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn every_chat_call_writes_one_api_calls_row() { /* count == 1, endpoint == "chat" */ }

#[tokio::test]
async fn malformed_json_body_is_an_error_not_a_panic() { /* returns Err */ }

#[tokio::test]
async fn api_error_row_is_still_written() { /* non-2xx → Err AND row with error set */ }

#[tokio::test]
async fn search_maps_summary_and_date() { /* fixture → SearchHit */ }
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test deepseek`
Expected: FAIL.

- [ ] **Step 3: Implement**

All three endpoints share one `post_json` helper that writes the `api_calls` row **before** returning, success or failure. URLs: `{base}/chat/completions` with `response_format: json_object`, `{base}/v1/tools/search`, `{base}/v1/tools/fetch`. Chat uses `deepseek-chat` from config. Tests inject a `Transport` trait fake so fixtures are served without network.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test deepseek`
Expected: PASS.

```bash
git add -A && git commit -m "feat: deepseek chat/search/fetch client with append-only audit rows"
```

---

### Task 9: Response parsing and the validator

**Files:**
- Create: `src/llm/parse.rs`, `src/llm/validate.rs`
- Test: `tests/validate.rs`

**Interfaces:**
- Consumes: `Indicators`, `Bands`, `budget::fits`
- Produces: `parse::verdict(content: &str) -> Result<LlmVerdict>`; `LlmVerdict { Decide(Proposal), NeedMore(Vec<HeadlineRequest>), Pass { why: String } }`; `Proposal { action: Action, entry: f64, stop: f64, target: f64, valid_until: String, confidence: Confidence, rationale: String, cites: Vec<i64> }`; `validate::check(p: &Proposal, ctx: &ValidateCtx) -> Result<(), ValidationError>`; `ValidateCtx { bands: Bands, available_cash: f64, held_qty: i64, sent_headline_ids: Vec<i64>, today: String }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn stop_above_entry_on_buy_is_rejected() { /* Err(StopOnWrongSide) */ }
#[test] fn entry_outside_atr_band_is_rejected() { /* Err(PriceOutOfBand) */ }
#[test] fn qty_cost_above_available_cash_is_rejected() { /* Err(InsufficientCash) */ }
#[test] fn sell_of_unheld_instrument_is_rejected() { /* Err(NotHeld) */ }
#[test] fn past_valid_until_is_rejected() { /* Err(BadValidity) */ }
#[test] fn cited_headline_id_never_sent_is_rejected() { /* Err(UnknownHeadlineId) */ }
#[test] fn valid_proposal_passes() { /* Ok(()) */ }
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test validate`
Expected: FAIL.

- [ ] **Step 3: Implement**

`parse::verdict` deserialises into a tagged enum; anything that fails the shape returns `Err(ParseError)` — the caller degrades to rules output. `validate::check` applies each rule in order and returns the first failure. `InsufficientCash` uses `budget::fits` so the ledger remains the single source of truth.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test validate`
Expected: PASS.

```bash
git add -A && git commit -m "feat: llm response parsing and strict proposal validator"
```

---

### Task 10: Two-phase orchestration with hard caps

**Files:**
- Create: `src/llm/orchestrate.rs`
- Test: `tests/orchestrate.rs`

**Interfaces:**
- Consumes: Tasks 7–9 clients, `deepseek::fetch_page`, `Config`
- Produces: `orchestrate::run(deps: &LlmDeps, input: CandidateInput, headlines: &[Headline]) -> Result<Decision>`; `Decision { outcome: Outcome, conversation_id: String }`; `Outcome { Llm(Proposal), Rules(Candidate), Pass { why: String } }`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn need_more_triggers_exactly_one_fetch_round() { /* fetch called once, 2 chat rows */ }
#[tokio::test] async fn second_need_more_becomes_pass() { /* 2 chat rows max, Outcome::Pass */ }
#[tokio::test] async fn more_than_three_requests_are_truncated() { /* fetch called 3 times */ }
#[tokio::test] async fn article_bodies_are_truncated_to_4000_chars() { /* prompt length bounded */ }
#[tokio::test] async fn total_fetched_chars_capped_at_10000() {}
#[tokio::test] async fn daily_cap_exhausted_degrades_to_rules() { /* Outcome::Rules */ }
#[tokio::test] async fn invalid_proposal_degrades_to_rules() { /* Outcome::Rules */ }
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test orchestrate`
Expected: FAIL.

- [ ] **Step 3: Implement**

Assign headline ids as `1..=n` in list order before rendering, and keep a `id → url` map so a returned id resolves to a real url and an unknown id is rejected. Count today's `api_calls` rows where `endpoint='chat'` for the daily-cap check. Truncation is by chars, not bytes, and never splits a UTF-8 boundary.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test orchestrate`
Expected: PASS.

```bash
git add -A && git commit -m "feat: two-phase llm orchestration with escalation caps"
```

---

### Task 11: Notifier with dedupe

**Files:**
- Create: `src/notify.rs`
- Test: `tests/notify.rs`

**Interfaces:**
- Consumes: `db::SqlitePool`, `Config`
- Produces: `trait Notifier { async fn send(&self, n: &Notification) -> Result<()>; }`; `Notification { title: String, body: String, priority: Priority, dedupe_key: String }`; `Priority { High, Default }`; `notify::Ntfy::new(cfg)`; `notify::FakeNotifier { sent: Arc<Mutex<Vec<Notification>>> }` used by tests

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn same_dedupe_key_sends_once() { /* 2 sends → 1 recorded */ }
#[tokio::test] async fn sell_uses_high_priority() {}
#[tokio::test] async fn notifier_failure_does_not_panic_the_loop() { /* Err returned, loop continues */ }
#[tokio::test] async fn failed_send_leaves_key_unsent_so_next_tick_retries() {}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test notify`
Expected: FAIL.

- [ ] **Step 3: Implement**

`Ntfy::send` posts the body to `{base}/{topic}` with `Title`, `Priority`, and `Tags` headers. The `notifications` row is inserted **only after** a 2xx, inside the same transaction, so a failed send retries next tick rather than being silently swallowed. `dedupe_key` = `sha256(symbol || action || round(entry, 2))`.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test notify`
Expected: PASS.

```bash
git add -A && git commit -m "feat: ntfy notifier with transactional dedupe"
```

---

### Task 12: Poll engine and scheduler

**Files:**
- Create: `src/engine.rs`, `src/schedule.rs`
- Test: `tests/engine.rs`

**Interfaces:**
- Consumes: Tasks 2–11
- Produces: `engine::poll_once(deps: &EngineDeps, now: DateTime<Utc>) -> Result<PollReport>`; `PollReport { candidates: usize, llm_calls: u32, signals: usize, notifications: usize, errors: Vec<String> }`; `schedule::in_stock_window(now: DateTime<Utc>) -> bool`; `schedule::is_daily_pass(now: DateTime<Utc>) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn one_poll_with_a_candidate_emits_one_signal_and_one_notification() {}
#[tokio::test] async fn second_identical_poll_emits_nothing() {}
#[tokio::test] async fn instrument_with_insufficient_bars_emits_no_signal_and_no_llm_call() {}
#[tokio::test] async fn one_symbol_failing_does_not_stop_the_others() { /* errors recorded, others processed */ }
#[tokio::test] async fn market_window_respects_ist_hours_and_skips_weekends() {}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test engine`
Expected: FAIL.

- [ ] **Step 3: Implement**

`poll_once` is pure over injected fakes (market source, LLM, notifier) so tests need no network. Symbols whose Yahoo news returned fewer than `news_thin_threshold` headlines in the last 7 days (default 3) are prefixed with one cached `deepseek::search` call, stored in `search_results` and reused for the rest of the day. It records a `signals` row only when `(action, rounded entry, rounded stop)` differs from that instrument's latest signal. Errors are collected per instrument into `PollReport.errors` and never abort the loop. The tokio loop calls `poll_once` on a 10-minute interval, gated by `in_stock_window`, plus one 19:00 pass for AMFI/IPO.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test engine`
Expected: PASS.

```bash
git add -A && git commit -m "feat: poll engine with per-instrument isolation and IST scheduling"
```

---

### Task 13: HTTP API

**Files:**
- Create: `src/api.rs`, `src/main.rs`
- Test: `tests/api.rs`

**Interfaces:**
- Consumes: `db`, `budget`, `Config`
- Produces: routes `GET /api/dashboard`, `GET /api/instruments/:symbol`, `GET /api/calls`, `GET /api/settings`, `PUT /api/settings`, `POST /api/positions`, `PATCH /api/positions/:id`, `POST /api/ipo`, `GET /api/health`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn dashboard_reports_available_committed_and_blocked() {}
#[tokio::test] async fn post_position_updates_committed_cash() {}
#[tokio::test] async fn post_ipo_reserves_without_creating_a_position() {}
#[tokio::test] async fn instrument_route_returns_headlines_without_bodies() {}
#[tokio::test] async fn calls_route_returns_audit_rows_newest_first() {}
#[tokio::test] async fn put_settings_below_committed_returns_409_with_committed() {}
#[tokio::test] async fn put_settings_changes_currency_only_when_unlocked() {}
#[tokio::test] async fn put_settings_rejects_infinite_or_negative_budget() {}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test api`
Expected: FAIL.

- [ ] **Step 3: Implement**

Handlers use `axum::extract::State` over a `AppState { pool, cfg }`. Responses are `serde_json` of the same structs the UI consumes. `PUT /api/settings` maps `SettingsError::BelowCommitted` and `CurrencyLocked` to `409 Conflict` with a JSON body carrying the field the UI needs, and `InvalidAmount` to `400`. Serve the built `web/dist` with `tower_http::services::ServeDir` at `/`. Bind address from `BIND` env, default `127.0.0.1:8080`. Request bodies are validated with typed extractors — malformed JSON returns 400, never a panic.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test api`
Expected: PASS.

```bash
git add -A && git commit -m "feat: http api with dashboard, positions, ipo and audit views"
```

---

### Task 14: Vite dashboard implementing the design system

**Files:**
- Read first: `.interface-design/system.md` (binding — tokens, layout, states, copy)
- Create: `web/package.json`, `web/vite.config.ts`, `web/index.html`, `web/src/main.ts`, `web/src/tokens.css`, `web/src/components/ledger-row.ts`, `web/src/components/stamp.ts`, `web/src/components/balance-rail.ts`, `web/src/pages/ledger.ts`, `web/src/pages/positions.ts`, `web/src/pages/settings.ts`, `web/src/pages/calls.ts`
- Test: `web/src/__tests__/render.test.ts`
- Modify: `src/main.rs` (ServeDir path)

**Interfaces:**
- Consumes: `/api/dashboard`, `/api/settings`, `/api/instruments/:symbol`, `/api/calls`
- Produces: static bundle in `web/dist`

**Component Checkpoint** (required by the design skill, recorded here so the implementer
inherits the reasoning rather than re-inventing it):

```
Intent:     one retail investor, one fixed pot, phone in the day / laptop at night;
            answer "is there something to do, can I afford it" in <10s; must feel calm,
            not like a trading terminal
Palette:    paper cream + blue-black ink + red-oxide and indigo stamps — from the
            passbook/ledger world; two stamp hues because buy/sell must be
            color-blind-safe
Depth:      borders-only, 0.5px hairlines — a ledger is a dense printed record, not
            floating cards
Surfaces:   paper (canvas) < paper-raised (white) < paper-inset (inputs) — inset signals
            "type here"; light only by default because the object being replaced is paper
Typography: IBM Plex Sans + IBM Plex Mono — institutional print heritage, tabular figures;
            every figure is mono or numbers misalign
Spacing:    4px base, 4/8/12/16/24/32/48; 32px rows desktop, 44px touch
```

- [ ] **Step 1: Write the failing tests**

```ts
test("ledger row renders the running balance after the action", () => {
  const html = renderLedgerRow(signalFixture);
  expect(html).toContain("1,405.20");     // the price
  expect(html).toContain("84,900");       // the balance column — the signature
  expect(html).toContain("data-stamp=\"sell\"");
});

test("estimate rows are labelled, never presented as firm", () => {
  const html = renderLedgerRow(mfSignalFixture);
  expect(html).toContain("ESTIMATE");
  expect(html).toContain("NAV at today's 15:00 cutoff");
  expect(html).not.toContain("data-stamp=\"sell\""); // MF never gets a firm sell stamp
});

test("blocked rows show the shortfall and are not red", () => {
  const html = renderLedgerRow(blockedFixture);
  expect(html).toContain("insufficient cash");
  expect(html).toContain("short by");
  expect(html).toContain("data-blocked");
});

test("settings disables save when budget is below committed", () => {
  const html = renderSettings({ committed: 142000, budget: 100000, currency: "INR", currency_locked: false });
  expect(html).toContain("Committed is");
  expect(html).toContain("disabled");
});

test("settings disables currency once locked, with the reason", () => {
  const html = renderSettings({ committed: 0, budget: 100000, currency: "INR", currency_locked: true });
  expect(html).toContain("Currency is fixed once the ledger has entries");
});

test("empty ledger renders ruled blanks, not a spinner", () => {
  expect(renderLedger([])).toContain("ledger-blank");
});
```

- [ ] **Step 2: Run, expect failure**

Run: `cd web && npm i && npx vitest run`
Expected: FAIL.

- [ ] **Step 3: Implement the tokens and components**

Copy the token block from `.interface-design/system.md` into `web/src/tokens.css` verbatim —
primitive and semantic names included (`--paper`, `--ink`, `--rule`, `--stamp-oxide`). Use
`@layer tokens, reset, base, components`.

`stamp.ts` renders `<span class="stamp" data-stamp="buy|sell|estimate|blocked">` — ink on
tint, 2px radius, 11px/700/uppercase/0.1em, rotated `-1.5deg` for buy and sell only. Text
is always inside the stamp.

`ledger-row.ts` is a real `<tr>` with a `<th scope="row">` symbol cell; the balance cell is
the last column and right-aligned mono. Expanding is a `<button aria-expanded>` that toggles
a details row containing reasons, rationale, and cited headlines.

`balance-rail.ts` renders Available at 34px mono, Committed/Budget at 16px, then three class
chips. Sticky at `top: 24px`, hairline on the left, no shadow.

Nothing outside `tokens.css` may contain a hex, `rgb()`, or `oklch()` literal — component
styles reference semantic tokens only.

- [ ] **Step 4: Run the component tests**

Run: `cd web && npx vitest run`
Expected: PASS.

- [ ] **Step 5: Build and check the served bundle parses**

```bash
cd web && npm run build
node --check dist/assets/*.js && echo "bundle parses"
```

Expected: `bundle parses`.

- [ ] **Step 6: Verify it renders, at both widths**

Run the server with seeded fixtures, then:

```bash
for w in 1440 390; do
  google-chrome --headless=new --disable-gpu --no-sandbox --window-size=$w,900 \
    --screenshot=/tmp/ui-$w.png --virtual-time-budget=9000 http://127.0.0.1:8080/
done
```

Read both PNGs and confirm: paper background, double rule under the header, a stamp badge,
the balance column right-aligned, and at 390px the rail collapsed into a top strip with no
horizontal scroll. Then load `/settings` and confirm the save button is disabled while the
typed budget is below committed.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "feat: ledger dashboard built on the paper/ink design system"
```

---

### Task 15: Backtest harness

**Files:**
- Create: `src/backtest.rs`, `tests/backtest.rs`
- Test: `tests/backtest.rs`

**Interfaces:**
- Consumes: `indicators::compute`, `evaluate::candidate`
- Produces: `backtest::run(bars: &[Bar], cfg: &RuleConfig, budget: f64) -> BacktestReport`; `BacktestReport { trades: usize, hit_rate: f64, max_drawdown: f64, blocked_by_budget: usize }`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn synthetic_uptrend_produces_trades_and_bounded_drawdown() {
    let r = backtest::run(&synthetic_uptrend(), &RuleConfig::default(), 100_000.0);
    assert!(r.trades > 0);
    assert!(r.max_drawdown <= 100_000.0);
    assert!(r.hit_rate >= 0.0 && r.hit_rate <= 1.0);
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test backtest`
Expected: FAIL.

- [ ] **Step 3: Implement**

Walk bars forward one day at a time, calling the same `evaluate` the live engine uses — no duplicated logic. Exit when stop or target is crossed. Count a candidate the budget refused in `blocked_by_budget` rather than dropping it. Expose as `POST /api/backtest` for ad-hoc runs.

- [ ] **Step 4: Run tests, then commit**

Run: `cargo test --test backtest`
Expected: PASS.

```bash
git add -A && git commit -m "feat: backtest harness reusing the live rule engine"
```

---

### Task 16: End-to-end smoke with fixtures

**Files:**
- Create: `tests/e2e.rs`, `tests/fixtures/` (reuse)
- Test: `tests/e2e.rs`

**Interfaces:**
- Consumes: everything
- Produces: one passing end-to-end test

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn full_cycle_signals_once_then_stays_quiet() {
    // fixture market + fixture chat (need_more then decide) + fake notifier
    // poll #1: 1 signal, 1 notification, 2 chat rows, 1 fetch row
    // poll #2: 0 signals, 0 notifications, 0 new chat rows
}

#[tokio::test]
async fn hallucinated_stop_falls_back_to_rules_numbers() {
    // fixture chat returns a stop above entry → signal uses candidate.stop
}
```

- [ ] **Step 2: Run, expect failure**

Run: `cargo test --test e2e`
Expected: FAIL.

- [ ] **Step 3: Implement**

Wire real `engine::poll_once` with fixture-backed `Transport` and `FakeNotifier`. Assert on `api_calls` counts and `notifications` rows, not on log output.

- [ ] **Step 4: Run everything**

Run: `cargo test`
Expected: PASS, zero network calls.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "test: end-to-end cycle with fixture llm and no network"
```
