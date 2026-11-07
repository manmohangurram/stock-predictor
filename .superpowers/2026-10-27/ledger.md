# Ledger — stock-predicter, 2026-10-27

Spec: docs/superpowers/specs/2026-10-27-stock-predicter-design.md
Plan: docs/superpowers/plans/2026-10-27-stock-predicter.md
Branch: feat/stock-predicter (git init'd here; no worktree needed — repo had no code)
Mode: inline (executing-plans), backend → notifications → UI

## Task log

- Task 1 ✅ crate scaffold, config, schema. 3 tests green.
  Ruling: inline UNIQUE/PK constraints instead of separate `CREATE UNIQUE INDEX`
  (equivalent outcome, fewer statements). Cost if wrong: none, same constraints.
  Ruling: added `src/lib.rs` so integration tests can import the crate; `main.rs` is a
  thin binary over it. Cost if wrong: none.
  Ruling: `SqliteConnectOptions::new().filename()` instead of parsing a URL string —
  `FromStr` was not resolving against sqlx 0.8's feature set.
- Task 2 ✅ yahoo client, 6 tests.
  Ruling: news comes from `/v1/finance/search` (title/publisher/link/providerPublishTime)
  rather than the undocumented `xhr/ncp` stream — it returns exactly the headline fields
  and no body, which is the point of the parser. Cost if wrong: swap one URL.
  Ruling: a missing `news[]`/`chart.result` key is an error (drift must be loud); an empty
  array is a valid empty result.
- Task 3 ✅ amfi, 3 tests.
- Task 4 ✅ indicators, 4 tests.
  Ruling (plan defect): `compute` requires 200 bars (SMA200), but the plan's own
  known-series test used 60 — contradictory. Kept the 200-bar floor (a NaN SMA200 would
  leak into rules) and made the test series 250 bars, whose mean is still exactly 3.0.
  Cost if wrong: a symbol with <200 daily bars gets no signal, which is the safe direction.
- Task 5 ✅ budget + settings, 9 tests.
  Ruling: the ceiling is derived as SUM(cash_ledger WHERE kind='budget'), not stored in
  `settings.budget`. Spec said settings holds it; two sources for one number drift, and the
  ledger already gives the history. `settings` stores only `currency`. Cost if wrong: one
  query instead of one read; no migration needed to change back.
  Ruling: currency changes are refused when any non-budget ledger row exists.
- Task 6 ✅ evaluate, 7 tests.
  Ruling (plan deviation): `Candidate.stop`/`target` are `Option<f64>`, not `f64`. The plan
  also demanded MF/IPO never carry an intraday stop, which a bare f64 cannot express without
  a 0.0 sentinel that downstream code would read as a real price. Cost if wrong: a few match
  arms, no behaviour change.
  Ruling: added `urgent` to Candidate so a stop-hit can take ntfy priority without the
  notifier re-deriving it.
- Task 7 ✅ prompts, 4 tests. Prompt text typed into a template file, hash = sha256(template).
- Task 8 ✅ deepseek client, 5 tests. One `Transport` seam; the fixture impl is in the
  crate (cfg-free) so every later test can drive multi-round conversations.
  Ruling: FixtureTransport serves replies FIFO, not LIFO — a two-round escalation test in
  LIFO order would silently test the wrong round.
- Task 9 ✅ parse + validate, 10 tests.
  Ruling (plan defect): the plan said entry/stop/target must all sit inside the ATR band,
  but the band is ±1 ATR and the rules' own stop is 2 ATR — every legitimate stop would be
  rejected, including the fallback the e2e test expects. The band now applies to **entry
  only**; stop/target get a distance sanity check (5 and 10 ATR). Cost if wrong: a wild but
  correctly-signed stop could pass; the max-distance check bounds it.
  Ruling: added MissingCitation as its own error instead of overloading UnknownHeadlineId.
- Task 10 ✅ orchestrate, 7 tests.
  Ruling: `run` takes a `RunInput` struct instead of the plan's `CandidateInput` — it needs
  held_qty and search summaries, which CandidateInput does not carry.
  Ruling: any phase-1 transport error, parse error, or validation failure falls back to the
  rules candidate. Every path is covered by a test, including an unparseable response.
  Note: the char-cap test counts runs >= 100 because the JSON envelope contributes a few
  stray single letters; measuring the article run is the actual invariant.
- Task 11 ✅ notify, 6 tests. Dedupe row is inserted only after a 2xx, so an outage retries.
  Ruling: ntfy header values are folded to ASCII (the copy uses '·'); reqwest would reject a
  non-latin-1 header at send time and the user would lose the alert entirely.
- Task 12 ✅ engine + schedule, 7 tests.
  BUG FOUND by the blocked-path test: the validator rejected an over-budget proposal as
  InsufficientCash and orchestrate then fell back to the rules numbers — which fit — so the
  trade was silently down-sized, the exact anti-goal in the spec. Fixed by adding
  Outcome::Rejected{proposal, error}: the engine maps InsufficientCash to a BLOCKED record and
  every other rejection to the rules fallback. Cost if wrong: none, both paths are tested.
  Note: BLOCKED rows are re-inserted on each poll (no dedupe). They are never pushed, so the
  phone stays quiet; the table grows by a few rows a day in the worst case.
- Task 13 ✅ api, 12 tests. Added \`available\` as a serialised field on Settings (computed
  once in \`get\`) — the UI needs it and deriving it in two places invites drift.
  Ruling: a malformed JSON body is answered by axum's own rejection (client error) rather
  than a hand-rolled 400; the test asserts client-error class, not the exact code.
  Note: two test helper bugs were mine, not the code's — the patched position was closed
  (so committed correctly dropped to 0) and the shape of Settings was assumed wrong.
- Task 15 ✅ backtest, 3 tests.
- Task 16 ✅ e2e, 2 tests.
  Ruling: added a **pre-LLM** dedupe on the rules fingerprint (action/entry/stop, 2dp)
  stored in reason_json. Without it the plan's own e2e expectation ("0 new chat rows on the
  second poll") is impossible and every quiet tick would re-buy the same answer. Cost if
  wrong: a genuinely changed LLM view on unchanged rules data is not re-asked until the rules
  fingerprint moves — that is the intended trade (cost control over churn).
- Backend done: 88 tests passing, zero network in tests.
- Task 14 ✅ UI, 10 vitest tests, verified by screenshot at 1440 and 390.
  Three defects only rendering could find, all fixed:
  1. `signals` had no `qty` column — the ledger literally could not say how many units. Added
     the column, bound it in the engine, exposed it in the API. Caught by looking at the page.
  2. `/settings` returned 404 on direct load: ServeDir had no SPA fallback. Now unknown paths
     fall through to `index.html`; `/api/*` still returns JSON.
  3. The save-guard compared against `form.dataset.committed`, which was never set, so the
     button never disabled. Extracted `shouldDisableSave()` and tested it.
  Also: detail rows now start collapsed, BLOCKED rows have their own "Not actionable" table,
  and mobile rows are stacked label/value pairs (the first attempt squeezed the table and
  overlapped rows — the fixed 32px row height had to become auto).
  Ruling: dropped the Target column from the main table (kept in the expanded row) so the
  identity column has room; two 96px figure columns plus an auto column does not fit at 1080.

## Final state
- 88 Rust tests, 10 UI tests, all green. No test touches the network.
- Backend complete (tasks 1-13, 15, 16). UI complete (task 14).
- Not wired yet, by design and stated in the spec: AMFI NAV ingestion is written but the
  daily pass does not call it yet; IPO data entry is manual via POST /api/ipo.

## Hygiene
- `target/` (6,911 files) and `web/node_modules` were committed before `.gitignore` existed.
  Rewrote the 17-commit history with `filter-branch` to drop them: .git went 2.6G → 312K,
  74 tracked files. Nothing had been pushed, so this cost only local history. Tests re-run
  green after the rewrite.

## Live-data session (real Yahoo + AMFI)

Ran the server against real endpoints for the first time. Five bugs, none of which any test
or screenshot could have caught:

1. `fetch_bars` never put the symbol in the URL: every request was
   `/v8/finance/chart?range=1y` → 404 for every symbol. Added `chart_url()` + regression test.
2. Yahoo's search returns 0 news for `INFY.NS` and 5+ for `INFY`. Added `news_query()` to
   strip the exchange suffix (`.NS`/`.BO` only — `BRK.B` must survive) + test.
3. AMFI moved: `www.amfiindia.com` now redirects to `portal.amfiindia.com`.
4. AMFI changed its column layout: 8 columns (`Net Asset Value` at index 6, `Date` at 7),
   CRLF line endings, new `Plan`/`Option` columns. The parser read index 4/5 and would have
   parsed the *plan* as the NAV. Now header-driven with a legacy fallback, plus a CRLF test.
   Fixture replaced with a real capture from 2026-10-27.
5. The engine never wrote to `bars` — the data model had the table, nothing filled it. Bars
   are now upserted, which is what makes the instrument page and a future backtest possible.

Added while making the UI legible:
- `evaluate::diagnose()` returns the same predicates as `candidate()` plus the numbers, so the
  UI can answer "why is the ledger empty?" instead of just showing nothing. Guarded by a test
  that asserts diagnose and candidate agree.
- `GET /api/instruments/{symbol}/analysis` + `/i/{symbol}` page: 120-session ink chart, rule
  checklist, real headlines.
- `--seed` (real closes from Yahoo, real NAV from AMFI, never an invented price) and
  `--poll-once` (run a poll outside market hours).
- Empty `NTFY_TOPIC` now skips the push instead of logging a failed notification as a poll
  error. Test added.

Observation from the real data (2026-10-27): INFY 1163 vs SMA200 1222, TCS 2497 vs 2443 with
RSI 81, HDFCBANK 680 vs 794, RELIANCE 1196 vs 1335. Nothing passes `close > SMA50 > SMA200`,
so the ledger is correctly empty. The SELL path was exercised by setting a stop above market.

## DeepSeek key in settings

The key moved from env-only to the `settings` table, editable from `/settings`, with three
rules that matter more than the field itself:

1. **Never returned.** `Settings` carries `deepseek_configured: bool` only. This server has no
   auth, so anything serialised is readable by anyone on the LAN. Tests assert the key string
   appears in neither the PUT response nor the GET body.
2. **Blank does not wipe.** The field is a masked password; saving the form with it empty
   keeps the stored key. A separate test pins that, because the alternative loses a working
   key to an accidental save.
3. **No key means no call, not a failed call.** The engine checks `settings::deepseek_key`
   before orchestrating and goes straight to rules. Without that gate every tick would write
   an `api_calls` error row and burn the daily cap on requests that cannot succeed.

Env `DEEPSEEK_API_KEY` is now a first-boot seed like `BUDGET`/`CURRENCY`; a re-seed never
overwrites a key set from the UI. `main` reads the key per tick, so changing it in the UI takes
effect on the next poll without a restart.

Risk to state plainly: the key is stored in plaintext in `stock-predicter.db`. Anyone with
read access to that file, or to the LAN this server listens on, has your key.

## Headline-triggered model calls, intraday context, rewritten prompts

Trigger moved from "the price-derived candidate changed" to **"the headline set changed"**.
`llm_memos` (migration 0002, first real migration runner in `db::migrate`) stores a
title+url fingerprint per instrument. Same news → no call; the model's action/stop/target are
kept and only the entry is refreshed, and only while the price stays within one ATR of the
entry it decided at and `valid_until` has not passed. Outside that band the view has expired
and the rules take over.

Ruling: a model *failure* (401/429) does not write the memo, so the next tick retries. That
means a bad key costs one call per tick until the daily cap stops it — deliberate, because a
transient failure should not blind the system until new news arrives. Cost is bounded by
`llm_daily_cap`.

Ruling: signals are refreshed **in place** when (action, stop, target) are unchanged and only
the entry moved. Without this, "prices refresh freely" would have written a new signal and
pushed a notification every 15 minutes.

Prompts rewritten from scratch around a decision procedure (position → materiality →
technicals → prices → their own recent losses), an explicit "what you do not have" fence, a
legitimate `pass`, per-field hard rules tied to the validator, and worked JSON examples.
`{{headlines}}` ids are the only way to cite; article bodies stay off the wire until asked.

Context added to every prompt: 15-minute tape for 6 hours, last 10 daily closes, the open
position, trades recorded in the last 7 days, and the previous view on the instrument. The
7-day ceiling on any history the model can trigger is a constant (`MAX_HISTORY_DAYS`), not a
suggestion.

Cadence 10 → 15 minutes (`POLL_SECS` default 900).

## Hybrid UI, search, instrument pages, trade history

Chosen from the six-direction lab (`web/design/lab.html`, served at `/lab.html`). Built:
portfolio strip, blotter, click-through instrument page, search-and-add, full trade history.

Backend:
- `0003_trades` + first real use of the migration runner. Trades are the record; positions are
  a projection; `realised` is computed at sale time and stored, so P&L never depends on
  today's average cost.
- Existing positions are backfilled as buys so history is not blank on first upgrade.
- `trades::record` is all-or-nothing: history row, position change and cash movement.
- `GET /api/search` merges Yahoo equities with AMFI scheme names (AMFI file cached 6h in
  process — 3 MB, not per keystroke). Yahoo's opaque Indian fund codes (`0P0000N0B6.BO`) are
  filtered out; AMFI is the fund source.
- `POST /api/instruments` adds a code idempotently; `GET/POST /api/instruments/:symbol/trades`.
- Dashboard gained `realised` and per-position `ltp`/`value` from the latest stored bar.

Ruling: `exchDisp` is the Yahoo field name — my earlier `exchdisp` rename silently produced
empty venues. Caught by looking at the search response, not by a test.
Ruling: the "stop not hit" rule reads as "stop not breached" — with `passed=false` the UI
showed BLOCK, which reads like a blocked trade when it is actually the sell trigger.

## Dated fills (backfilling old records)

Trades can now carry the date they actually happened, so the portfolio can be filled in
retroactively rather than starting from today.

- `side`, `qty`, `price`, `note`, and **`at`** on `POST /api/instruments/:symbol/trades`.
  `YYYY-MM-DD` or full RFC3339; omitted means now.
- A bare date means that NSE session's close (15:30 IST = 10:00 UTC), so a backfilled fill
  sorts after the morning and before the next day.
- The date is propagated, not just stored on the trade: position `opened_at`, and the
  `cash_ledger` row (new `budget::record_at`). Otherwise a July fill would appear at the top
  of a ledger of October entries.
- The trade form has a date field defaulting to today, capped at today.

Two gaps the backfill exposed, both fixed and tested:
1. Nothing stopped a sale dated before the position existed — a history running backwards.
2. A backfilled *buy* did not move `opened_at`, so any older sale was then refused. A buy now
   moves the position's start date backwards (min of existing and the fill), which is what
   makes entering history in any order work.

Ruling: cost basis stays weighted-average, so a backdated buy restates the average cost used
for later realised P&L. FIFO lot accounting would be more accurate and is the upgrade path if
per-lot P&L is ever wanted. Recorded here because it is a real approximation, not a bug.

## Bug: cannot buy more than you already hold

Reported as "clicking an added stock and trying quantity 10 shows an error" when holding fewer.

Cause: the trade form rendered `max="<held>"` on the quantity input unconditionally. The browser
then refused any value above the holding for a **buy** as well, with a native
"must be less than or equal to N" message — before any request was made. Server-side was never
wrong.

Fix: the cap is applied only while `side = sell`, toggled by JS on the select
(`qtyLimit(side, held)`), and the held quantity moved into the placeholder instead. Regression
test asserts a buy form for a 2-unit holding contains no `max="2"`.

Also made the two remaining refusals say the numbers: "cost 1000000.00 exceeds available cash
91000.00" and "cannot sell 99, only 9 held".

Verified end to end on a throwaway database: buy 2, buy 10 (now succeeds), sell 3, oversell and
over-budget refused with the figures.

## Prices and headlines on add; close-on-the-day for fills

- `store::{bars, headlines, refresh}` extracted from the poll loop so both paths persist
  identically. `POST /api/instruments` now fetches the 1-year daily series and the headlines
  before returning (20s timeout), so a newly added symbol has a chart on first open instead of
  waiting for the next 15-minute tick. A failed fetch still returns the instrument — the add
  succeeded, the data did not.
- Mutual funds are skipped deliberately: Yahoo carries no Indian fund series, and their price
  comes from AMFI at add time.
- Fills now carry `market_close`: the close on the day the fill happened, from the stored
  daily bar, plus a "vs close" percentage. A backfilled record can therefore be judged against
  what the market actually did that day. No bar for that date (older than a year) renders an
  em dash, never a zero.

Observed while verifying: `TCS.NS` returns **zero** headlines from Yahoo for every query shape
("TCS", "TCS.NS", "Tata Consultancy Services"), while "Infosys" returns five. So headline
coverage is a property of Yahoo's feed, not of our query. Recorded because it is the weakest
link in the chain and the next thing to fix.

## Exit plan is mandatory on every buy

- Prompt (both templates): "A BUY must carry both exits — stop (sell if it falls through) and
  target (take profit). A buy without both is discarded, so the person can place the trade in
  one go and walk away."
- Validator: new `MissingExit` for exchange-traded instruments. Funds and IPOs are exempt
  (`requires_exit = false`) because they cannot be exited at a price.
- Notification copy now carries the plan: "sell if ≤ 1412.00 · take profit ≥ 1510.00", so the
  alert alone is enough to place both legs.
- Signal card in the UI shows the two exits side by side under the decision.

## Two pots: long-term and trading never share capital

Mode is a column, not a UI filter: `cash_ledger`, `positions`, `trades`, `signals` all carry
`mode` ('long' | 'trade'), migration 0004. Every budget query is mode-scoped
(`budget_total_in`, `committed_in`, `available_in`, `fits_in`, `realised_in`), and
`trades::record` refuses a buy that does not fit *its own* pot.

- The same symbol can be held separately in both pots (tested): 2 units long and 5 units trade
  are two positions, two histories, two cash movements.
- IPO reservations count only in the long pot: a trading book never reserves for allotment.
- `/api/dashboard?mode=long|trade` and `GET /api/settings` carry both pots; `PUT
  /api/settings` accepts `trading_budget` with the same never-below-committed guard.
- UI: Long-term / Trading tabs, each with its own strip, book and history. The fill form has a
  pot selector so a trade can be booked into either.

Bug found while wiring it: the trades history SELECT had `mode` before the `market_close`
subquery while the row tuple expected `Option<f64>` in that slot, so sqlx failed to decode TEXT
into REAL and every `/trades` call 500'd. Positional column order is load-bearing here.

## Prompt contract tested, and a real API call made

`tests/scenarios.rs` — 19 dummy responses driven through the real orchestrator, no network:
valid BUY, BUY without a target, BUY without a stop, stop on the wrong side, entry outside the
band, a fund legitimately omitting exits, SELL of an unheld instrument, pass, prose around the
JSON, truncated JSON, a string where a number belongs, unknown action, unknown citation, no
citation, expired `valid_until`, and extra fields the prompt never asked for. Plus four
escalation shapes: articles only, history only, both, and asking twice.

**Bug the suite caught immediately:** the prompt example told the model to send
`{"headlines":[{"id":3,…}]}` while `HeadlineRequest` required `headline_id`. A model following
the instructions exactly would have had its escalation discarded and fallen back to rules.
The parser now accepts `id` / `headline_id` / `headlineId`, and the prompt example uses
`headline_id`.

`tests/live_deepseek.rs` — two ignored tests that make a real billed call
(`DEEPSEEK_API_KEY=… cargo test --test live_deepseek -- --ignored --nocapture`):

1. Held position + one guidance headline → the model returned `pass`, and its reasoning named
   the position, the stop, the target, both moving averages and RSI. Correct.
2. Clean uptrend, material news, no position → the model returned
   `BUY entry 1200 stop 1160 target 1250 cites [1]`, rationale 54 words. Both exits present,
   stop below entry, target above, entry within one ATR.

**Second finding:** the first attempt at case 2 passed, and rightly so — my fixture set
`last_close` to 1200 while the daily summary and tape still described 1163. Given a
self-contradictory prompt the model refused to trade. The context builder feeds `last_close`
from the rules candidate and every window from the same fetch, so live prompts are coherent,
but this is worth remembering when hand-writing a prompt context.

Also capped prose: rationale under 60 words, `pass.why` one sentence.

## Confidence is stored and shown

The model's `confidence` was parsed and then dropped on the floor: `insert_signal` had a
10-value bind list against a 9-column INSERT, and sqlx silently ignored the extra value, so
every row landed with NULL. Migration 0005 adds the column; the engine now writes it, and
`confidence` is NULL for rules-only signals — deliberately, because "no model judgement" is not
the same as "low confidence" and inventing a level would be a lie.

UI: three bars, filled one/two/three for low/medium/high, hollow-and-dashed for rules-only.
Shape carries the level and colour only reinforces it, so it survives colour blindness, and
every instance has an `aria-label` (`Confidence: high`, `Confidence: rules only, no model
judgement`). Shown in the blotter row and, with words, on the decision card.

Two layout defects fixed while looking at it: the chart repeated SMA50/SMA200 as labels that
collided with the same numbers in the rail (labels moved inside the plot, one copy only), and
the trade form's six controls were clipped under the rail at 1280px (now wraps).

## Confidence appears only where a signal is, and only when it exists

Checked placement: `renderConfidence` is used in exactly two places, both signal displays — the
Signal column of the blotter and the decision card. Not on the strip, positions, settings or
headlines.

Change made after the question: **no score now draws nothing.** The hollow "rules only" bars were
removed, because an empty gauge reads as "scored zero", which is a different and false claim from
"the model never scored this". Rules-derived signals say so in words in the card footer
("rules-derived, no model score") and show no gauge at all.

## Risk factor and a per-instrument profit expectation

- `instruments.target_profit_pct` (migration 0006), editable from the instrument page
  (`PUT /api/instruments/:symbol/target-profit`, blank clears it).
- The prompt receives it: "Profit expected on this position: at least 15.0% (a take-profit
  below 441.66 is rejected)", or "no minimum set" when unset.
- The validator enforces it for BUYs: a take-profit that does not reach the expectation is
  rejected as `TargetBelowRequirement`, so the rules/orchestrator fall back rather than
  recommending a 4% trade on a holding meant to make 15%. Sales are not judged on it.
- **Risk is computed, never reported by the model.** From the validated numbers: money at risk
  (qty × |entry − stop|), risk per unit, reward:risk, profit at target, and whether the target
  clears the owner's expectation. A model that can state its own risk can state a comfortable
  one; arithmetic cannot.
- UI: the risk block sits under the exit plan on each decision card, with "meets 15%" /
  "below 15%" beside it. The expectation is editable in the rail.

Verified live: with 15% set, DeepSeek's answer read "the 15% target (441.66) is far above a
falling price; exit near cost rather than hold a losing knife" — it used the number we injected
rather than inventing one.

## Call gate and market-hours cadence

- **Per-instrument gate** (`llm_memos.last_price`, `last_call_at`): the model is skipped when the
  price moved less than `LLM_MIN_PRICE_MOVE` (default ₹2) AND the headline set is unchanged AND
  a call has happened within `LLM_MAX_SILENCE_SECS` (default 1 hour). Any one of the three
  overrides: a ₹2 move, a new headline, or an hour of silence.
- The gate refreshes the standing signal's entry price without a model call, so the decision's
  price does not age while its reasoning is left alone.
- Attempts are recorded whether they succeed or fail, so a broken key cannot loop every tick.
- Cadence: 15 minutes inside 09:00–16:00 IST, **2 hours outside it** (`OFF_HOURS_SECS`), so
  evening headlines still produce a decision for the next open. The old 19:00 daily pass is
  subsumed by the off-hours cadence.
- Default `LLM_DAILY_CAP` raised 50 → 200, since the gate now runs per instrument per tick.

Ruling found by an existing test: the price gate initially also suppressed calls when headlines
had changed, which is exactly the wrong time to stay quiet. Headline change is now an override,
not something the gate has to consider separately.

## Cleanup pass: duplication, dead code, typed rows, benchmarks

- `engine::gather` — `poll_instrument` and `advise` had copies of the same ten-step gathering
  and had already drifted (`budget::available` vs `available_in`). One helper now, both callers.
- One `tests/common/FakeMarket` replaces four copies that had diverged; ~150 lines of test
  scaffolding gone.
- `#[derive(sqlx::FromRow)]` structs replace positional tuples in `api::{signals,positions,calls}`
  and `trades::history` and the engine's three row reads. Two shipped bugs came from positional
  column order (TEXT decoded as REAL); a named field cannot.
- Dead code removed: `schedule::off_hours_interval` (main computes it inline).
- Clippy: 11 warnings → 0. The important one was `!(stop < entry)`, which is **false for NaN**,
  so an unparseable stop used to pass validation. Now a positive finite test, plus a regression
  test.

## Benchmarks (`cargo bench --bench hot_paths`)

| Path | Before | After |
|---|---|---|
| indicators/250 bars | 2.75 µs | 2.75 µs (unchanged, already fine) |
| prompt render, 20 headlines | 21 µs | 21 µs |
| bar write, 250 rows (cold) | **68 ms** | 18.5 ms |
| bar write, one new bar (warm) | 68 ms worth | **0.18 ms** |

The 68 ms was real: `store::bars` inserted row-by-row in autocommit. Now one transaction per
batch, WAL + synchronous=NORMAL, and — the bigger win — only bars at or after the newest stored
timestamp are written, because a poll re-fetches a year but only the tail is new.

## Database, request and token optimisations

### Indexes (migration 0008)
EXPLAIN QUERY PLAN showed four full scans on live paths. After:

| query | before | after |
|---|---|---|
| latest signal for an instrument | SCAN signals | SEARCH signals_by_instrument |
| signals by mode | SCAN s | SEARCH signals_by_mode |
| headlines for an instrument | SCAN news + temp b-tree | SEARCH news_by_instrument |
| open position | SCAN positions | COVERING INDEX positions_by_instrument |
| chat calls today | SCAN api_calls | COVERING INDEX api_calls_by_endpoint_ts |

`daily_chat_calls` used `substr(ts,1,10) = ?`, which no index can serve; it is now a half-open
range `ts >= 'YYYY-MM-DD'`.

### API calls per page
- Ledger: 2 requests -> 1 (`/api/dashboard` now carries `recent_trades`).
- Instrument: 3 requests -> 1 (`/api/instruments/:symbol/analysis` carries `signals` and
  `trades`).
- `/api/trades` and `/api/instruments/:symbol/trades` stay for API consumers.

### Prompt tokens
- Templates reordered so the ~700-token static block is a **prefix** and the variable data comes
  last. DeepSeek caches matching prefixes, so the repeated part is billed at the cache rate.
- Boilerplate tightened: 6887 -> 5921 characters on the same context, about 1721 -> 1480 tokens
  (−14%) before cache effects.
- Render cost itself fell 21 µs -> 17.4 µs.
- Remaining big item: the 24-bar intraday tape is ~260 tokens. `window_hours` is the knob if a
  shorter tape ever looks sufficient.

## Follow-up: the suggestions that were left on the table

- `PROMPT_WINDOW_HOURS` (default 4, was a hardcoded 6): the intraday tape is the largest
  remaining prompt item, ~11 tokens per bar.
- `ETag`/`If-None-Match` on `/api/dashboard`, keyed on max signal id, max trade id, position
  count and settings count for that mode. Measured: 304 with 0 bytes versus 200 with 3092.
  The UI now keeps its DOM on a 304, which also stops the 60-second poll collapsing any row the
  user had expanded.
- `PRAGMA optimize` after migrations, so the planner's statistics match the new indexes.
- Not done, deliberately: multi-row `INSERT ... VALUES` batching (0.18 ms per tick) and
  ANALYZE on a 74-row database.

## Dependency audit

- `tokio` was on `features = ["full"]` — every subsystem compiled in. Now only what is used:
  `rt-multi-thread` (main + bench runtime), `macros` (`#[tokio::main]`, `#[tokio::test]`),
  `net` (TcpListener), `time` (sleep/timeout), `sync` (Mutex).
- Dropped unused features: `reqwest/json` (no `.json()` call), `sqlx/chrono` (times are stored as
  strings), `chrono/serde` (no chrono field is serialised), `tower-http/trace` (unused),
  `criterion/html_reports`.
- Upgrades: reqwest 0.12 -> 0.13, sqlx 0.8 -> 0.9, tower-http 0.6 -> 0.7, sha2 0.10 -> 0.11,
  criterion 0.5 -> 0.8. `cargo update` for the rest.
- **`rsa` and RUSTSEC-2023-0071 are gone entirely.** The crate came from `sqlx-mysql`, which the
  old sqlx pulled in as a build-time dependency of its derive macros. sqlx 0.9 no longer does,
  so `cargo audit` is clean with no ignore file. The temporary `.cargo/audit.toml` ignore was
  removed rather than left in place.
- Migration work for the majors: `sha2` 0.11 dropped `LowerHex` on `finalize()` (one `hex`
  helper now), `sqlx` 0.9 requires dynamic SQL to be wrapped in `AssertSqlSafe` (four sites, all
  interned by us), `reqwest` 0.13 renamed the TLS features and moved `.query()` behind a
  feature.

## ntfy, verified against the real service

`--test-notify` posts one message and reports the result. Run against ntfy.sh it returned
`sent to https://ntfy.sh/sp-test-bi23`, and polling the topic's JSON API returned the message
with its title intact — so the integration is proven end to end, not just unit-tested.

Gaps found and closed:
- **No auth.** A protected ntfy topic rejects anonymous posts. `NTFY_TOKEN` now adds a bearer
  token.
- **No deep link.** A notification you cannot act on is half a notification. `PUBLIC_URL` adds a
  `Click` header pointing at `/i/<symbol>`. Without it, no link is sent at all: a `127.0.0.1`
  link is dead from a phone, and a dead link is worse than none.
- The headers and body are now built by a pure `payload()` function, so a header typo is a test
  failure rather than a silently missing alert. Tests assert the exact names ntfy reads
  (`Title`, `Priority`, `Tags`, `Click`) and that the non-ASCII separator is folded.

## ntfy configured from the UI

Topic, server, access token and tap-through link are now settings rows, edited on `/settings`
and read per tick, exactly like the DeepSeek key. Env values seed them on first boot only.

- `Settings` returns topic/server/link and a `ntfy_token_configured` boolean; the token itself
  is never serialised, and an empty token field keeps the stored one.
- `POST /api/notify/test` sends through the same code path a real signal uses, so the "Send a
  test notification" button proves the channel rather than describing it.
- Verified against ntfy.sh: saved `sp-ui-d66` through the UI's API, pressed the endpoint, and
  the message came back from the topic's JSON API with its title intact.

Ruling: the topic is returned in full (a name the user must be able to read back and copy into
their phone) while the token is not. The topic is still a secret — anyone who knows it can
subscribe — so the UI says so next to the field.

## Auth dropped, 24x7 restored, providers selectable, env no longer owns secrets

- **Auth removed.** The envelope-key design would have required unlocking after every restart,
  which defeats 24x7 operation. `src/auth.rs` and its four crates (aes-gcm, argon2, base64,
  rand, zeroize) are gone. Consequence, stated in the README: the API keys sit in plaintext in
  the database and the server has no login.
- **One source of truth.** Budget, currency, model provider, keys and ntfy are settings rows
  edited in the UI. `Config` no longer carries them, so a stale `.env` cannot override what you
  configured. Only deployment knobs remain: BIND, DB_PATH, poll cadence, token caps.
- **Provider selection**: DeepSeek, OpenAI, Google Gemini, or any OpenAI-compatible endpoint
  (OpenRouter, Groq, Ollama) with editable base URL and model. Each provider keeps its own key,
  so switching back does not lose the other. Gemini's request/response shape is handled in
  `src/llm/provider.rs`; article fetch is DeepSeek-only and the escalation degrades to price
  history for the rest.
- 24x7: nothing in the loop exits on error. A failed fetch, model call or push is logged and
  retried next tick; the README documents a systemd unit and the Compose restart policy.

## Risk limits, per pot

Five numbers per pot, stored as `limits_<mode>_<name>`: min/max profit %, min/max trade amount,
and a per-instrument exposure cap as a percentage of the pot (chosen so the cap keeps its meaning
when the budget changes; the per-trade amounts stay absolute).

Enforced twice, deliberately:
- the **validator** after the model answers, in a fixed order (cash → amount → instrument cap →
  profit band → price sanity), so the error names the first thing that failed;
- the **engine**, clamping the rules candidate too. Without this the rules fallback could write a
  200k signal through a 50k ceiling — the model was constrained and the fallback was not.

The prompt receives the same numbers as one sentence, including the room actually left on that
instrument, so the model is asked for something that can be accepted.

`Settings::set` refuses an incoherent set (max below min, negative, cap outside 0–100) rather
than storing it and confusing the validator later.

## Per-stock profit vs the global band, and a WAL data-loss trap

Profit expectation is two different things, and the code now says so:
- **Global `min_profit_pct` / `max_profit_pct`** (per pot) bound what *any single stock* may ask
  for. They are a band on the stock's target, not a rule about the pot's overall return.
- **`instruments.target_profit_pct`** is that stock's own floor. Blank means the band minimum
  (`required = limits.min`); a stored value above the ceiling is clamped, and `PUT
  /api/instruments/:symbol/target-profit` refuses a value outside the band outright.

Before this, the per-stock field was an orphan: stored and editable in the UI, but the prompt
only ever received the *global* minimum, so setting 15% on a stock changed nothing. That is now
wired end to end (engine → prompt → validator → UI copy).

**WAL trap, twice now.** Killing the server with data still in `stock-predicter.db-wal`, then
deleting or recreating the `.db`, produces an empty database beside an orphaned 400 KB WAL — which
is exactly what happened to the demo files. Fixed properly: SIGTERM/Ctrl-C now runs
`PRAGMA wal_checkpoint(TRUNCATE)` and closes the pool, so the database is self-contained on exit.
Verified: 168957 bytes of WAL before SIGTERM, no `-wal` after, all rows intact.
