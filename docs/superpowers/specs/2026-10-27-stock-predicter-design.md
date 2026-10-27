# stock-predicter — design

Date: 2026-10-27
Status: awaiting approval

## Intent

A self-hosted tool on a home server that watches a small set of instruments, tells
the owner what to buy or sell and at what price, keeps a ledger of what is actually
held, and pushes a phone notification only when there is something to do.

Explicitly **not** a price oracle. The engine is a deterministic strategy; the word
"predictor" is the product name, not a claim.

## Goals

- Poll market data every 10 minutes during market hours; evaluate; notify on change.
- Suggest buy and sell actions with entry price, stop, target, and validity window.
- Hard-capped cash: the system never proposes spending beyond the configured budget.
- Track holdings per asset class so sell suggestions are grounded in real positions.
- Different timing semantics for stocks, mutual funds, and IPOs.
- Push to phone via ntfy; one notification per actionable change, not per poll.

## Non-goals

- Placing orders. All execution is manual at the broker.
- Intraday minute-level price prediction.
- Portfolio optimisation across asset classes, tax handling, or multi-user support.
- Guaranteed returns, or any language implying them.

## Anti-goals (failure even if it runs)

- Spending more than the budget, including by counting cash locked in IPO applications.
- Suggesting a sell of something not held, or in a quantity not held.
- Notification spam: repeated alerts for an unchanged signal.
- An unexplainable signal. Every suggestion must carry the reason it fired.

## Constraints

- Rust server (axum + tokio + reqwest + serde + sqlx/sqlite). Home server, single user.
- Vite front-end, served as static files by the same axum process.
- ntfy for push. Behind a `Notifier` trait so Web Push can replace it later.
- **No authentication, by decision.** Consequence: the server binds to LAN/localhost only,
  and the ntfy topic name is the only secret in the system. A public exposure of this
  service would leak the whole portfolio, so it must not be port-forwarded as-is.
- Yahoo Finance has no official API and no SLA. Quote endpoints increasingly want a
  cookie+crumb pair. Endpoint shapes change without notice. This is a standing
  maintenance cost, mitigated by a single client module all access goes through.
- **Mutual funds:** Yahoo coverage of Indian MFs is unreliable. Use AMFI `NAVAll.txt`
  (daily NAV, one file) as the source of truth when the instrument is an Indian MF.
- **IPOs:** Yahoo has no structured IPO feed, but Yahoo news does carry IPO coverage.
  Discovery therefore comes from news, and "should I apply" requires a web search,
  because that is a judgement about business quality, not a price series.
- **DeepSeek API:** structured suggestion generation + explanation, plus Search and Fetch
  for web access. No price data of its own, and no ability to be trusted with numbers
  unvalidated. Search and Fetch are billed at $0.75 per 1,000 successful requests;
  failed requests and timeouts are not billed.
- Every LLM and search request/response is persisted, permanently, for later audit.

## Approaches considered

**A. Rules engine — indicators + news scoring.** Deterministic signals from SMA/RSI/
ATR/52-week range over daily bars, entry/stop/target derived from measured volatility.
Explainable, testable, free, no training.

**B. Trained ML model (Python sidecar).** Predict next-N-day return from historical
features. Matches the "predictor" name, but needs a training pipeline, feature store,
drift monitoring, and yields unexplainable calls. Rejected for v1.

**C. LLM decides from headlines and indicators (DeepSeek).** Chosen by the owner. Richer
reasoning, can weigh news against IPOs and produce a structured verdict. Failure mode:
fabricated numbers, non-determinism, per-call cost.

**Trade chosen: A as a cheap pre-filter, C as the decision and explanation, plus a
validator that clamps C's numbers.** A alone was rejected because the owner wants the
LLM's judgement, especially on IPOs where indicators do not apply. C alone was rejected
because an unvalidated LLM price is how money disappears quietly. B deferred.

## How it works

### Poll loop

- Market hours (stocks): 09:10–15:35 IST on trading days, every 10 min.
  Fetch quotes + bars + news for watchlist symbols only, never the whole market.
- Outside market hours: one daily pass after ~19:00 to pick up AMFI NAV and any
  IPO status changes.
- Every poll writes a `signal` row only when the derived state differs from the last
  one for that instrument. Unchanged state is a no-op, which is what keeps the phone
  quiet.

### Signal

A signal is `(instrument, action, reason, entry, stop, target, valid_until,
confidence_bucket)`. `action` ∈ {BUY, SELL, HOLD, NONE}.

Rules (v1, tunable in config, not code):

- BUY candidate: trend up (close above 50d and 200d SMA), not overbought (RSI < 70),
  news score neutral-to-positive, and cash available covers one position unit.
- SELL candidate: held position where price crosses below its recorded stop, or RSI
  overbought with trend break, or target reached.
- Entry price: last close ± a fraction of ATR, rounded to tick. Stop: entry − k·ATR.
  Target: entry + k2·ATR. `valid_until`: end of next session.

Every signal stores the indicator values that produced it, so the UI can show why.

This layer exists for two reasons: it is free, and it is the gate that keeps LLM spend
proportional to actual candidates rather than to poll count.

### Prompt assembly

Prompts live as files under `prompts/`, injected at runtime from a typed context struct —
no string concatenation scattered through the code. Each file's SHA-256 is recorded on
every call, so a signal can always be traced to the exact prompt version that produced it.

A prompt is given, and is given nothing else:

- instrument: symbol, class, currency, last close
- bars: last N daily closes summarised (not the full series)
- indicators: SMA50, SMA200, RSI14, ATR14, 52w high/low
- headlines: `id`, `title`, `publisher`, `date` — **title only, never article body**
- position: qty, avg cost, existing stop/target if held; otherwise none
- cash: available, committed, budget remaining
- class rules: the timing semantics for this asset class (what can actually be executed)
- for IPOs: the DeepSeek Search summaries already fetched

The parser is what makes this cheap: Yahoo's news JSON is reduced to `title, publisher,
date, url`, the body is stored as NULL, and roughly fifteen tokens per headline reach the
model instead of a page of text.

### LLM layer (DeepSeek) — two-phase, escalating

Runs **only** when the rules layer produced a BUY/SELL candidate, or when the daily pass
found an IPO in the news. A poll with no candidate makes zero outbound LLM calls.

**Phase 1 — headlines.** One call containing the headline list above. The response is a
strict JSON union:

- `{"action":"decide", "signal":{action, entry, stop, target, valid_until, confidence,
  rationale, cites:[headline_id,...]}}`
- `{"action":"need_more", "requests":[{headline_id, why}, ...]}` — the model names which
  headlines look like they carry profit-relevant information, and says why.
- `{"action":"pass", "why":"..."}` — nothing worth doing.

**Phase 2 — fetch on demand, once.** For each requested headline the server resolves
`headline_id → url`, retrieves the article through DeepSeek Fetch
(`POST /v1/tools/fetch`, returns clean Markdown), truncates it, and re-calls with the
original context plus those extracts. The model then answers with `decide` or `pass`; a
second `need_more` is treated as `pass`.

Bounded so cost cannot run away — all config, all enforced server-side:

| Limit | Default |
|---|---|
| Escalation rounds | 1 (2 LLM calls max per candidate) |
| Headlines per escalation | 3 |
| Characters per fetched article | 4,000 |
| Total fetched characters per candidate | 10,000 |
| Daily LLM call cap | config, degrades to rules-only |

Article bodies are never passed as bare URLs for the model to read on its own: the server
fetches, strips to Markdown, truncates, and controls the size. The model never holds a
URL it can choose to fetch.

**Validate before anything else sees it.** Entry/stop/target must sit within ATR-derived
bands around the last close; stop must be below entry for a BUY and above for a SELL;
qty × price must fit available cash; the instrument must be held for a SELL;
`valid_until` must be a real future session; every `headline_id` cited or requested must
exist in the context that was sent. Any failure → the signal is rejected, logged, and the
rules-engine numbers are used instead.

Every round is one append-only `api_calls` row, tied together by `conversation_id`, so the
escalation is reconstructable end to end: what was asked, what was requested, what came
back, what the validator said. This is the audit trail for correctness and for cost, and
the first place to look when a suggestion looks wrong.

### Web and news access

Two sources, used for different jobs:

- **Yahoo news** — free, per-symbol headlines for the watchlist. Primary news input.
- **DeepSeek Search API** (`POST /v1/tools/search`) — used for IPO quality and for any
  symbol where Yahoo news came back thin. Query built from the symbol plus the specific
  question ("<company> IPO apply review fundamentals"), `count` 8, `need_content` false
  so only the 600-800 char `summary` is paid for and fed to the model. `ori_question`
  set when there is one. Results cached per instrument per day, so one IPO analysis
  costs one search, not one per poll.
- **DeepSeek Fetch API** (`POST /v1/tools/fetch`) — only when a specific claim needs
  checking against a full page. Off by default; it multiplies context size.

Search is **orchestrated by the server, not delegated to model tool-calling.** The server
performs the search and hands the model a fixed context. This keeps the LLM call
single-shot, makes cost predictable, keeps the request/response pair auditable as one
unit, and means the model cannot decide to search in a loop at 10-minute intervals.

### Asset class timing (this is the part that differs per class)

| Class | Price source | Buy timing | Sell timing |
|---|---|---|---|
| Stock | Yahoo chart/quote | Limit order, intraday or next open | Immediate at market/limit |
| Mutual fund | AMFI NAV (Indian) or Yahoo (fallback) | Units allotted at today's NAV if cut off before 15:00 | Redemption at that day's NAV; cash arrives T+1/T+2 |
| IPO | Deferred feed | Application locks cash until allotment | Not sellable until listing |

For MF and IPO the UI must label prices as *estimates* and sells as *"cash arrives
in N days"*. Showing an MF "sell price" as if it were firm would be a lie.

### Budget and risk layer

- `BUDGET` and `CURRENCY` live in the `settings` table, **edited from the UI**, not in env.
  Env values are a first-boot seed only; once the table has a value, the table wins.
- Cash ledger is the single source of truth: `available = budget − committed`.
- A budget cannot be lowered below `committed`. The UI refuses and shows the committed
  figure — otherwise `available` goes negative and every spend rule inverts.
- Currency is locked once any ledger entry exists. Changing it retroactively would
  reinterpret every stored amount without touching it, which is silent corruption.
  Before any entry, it is freely editable.
- `committed` = open stock positions at cost + MF units at cost + **cash reserved by
  pending IPO applications**. IPO reservations are the easy thing to forget and the
  most likely way the cap gets breached.
- A signal that would exceed `available` is emitted as `BLOCKED` with the shortfall,
  not silently dropped and not rounded down into a smaller trade.
- Optional per-class allocation caps (e.g. max 60% stocks) — config, default off.

### Data model (sqlite)

- `instruments(id, symbol, class, name, currency, active)`
- `settings(key, value, updated_at)` — keys: `budget`, `currency`. Seeded from env on
  first boot, thereafter authoritative.
- `bars(instrument_id, ts, open, high, low, close, volume)` — daily, plus last price
- `news(id, instrument_id, published_at, title, publisher, url, score)` — deduped by url
- `signals(id, instrument_id, action, entry, stop, target, valid_until, reason_json, created_at)`
- `positions(id, instrument_id, qty, avg_cost, opened_at, stop, target, state)`
- `cash_ledger(id, ts, kind, amount, ref)` — kinds: BUDGET, BUY, SELL, RESERVE, RELEASE
- `ipo_applications(id, instrument_id, qty, amount, status, applied_at)`
- `notifications(id, signal_id, sent_at, channel, dedupe_key)` — `dedupe_key` unique
- `api_calls(id, ts, conversation_id, round, endpoint, purpose, prompt_template,
  prompt_hash, request_json, response_json, http_status, prompt_tokens,
  completion_tokens, latency_ms, validator_verdict, error)` — append-only, never pruned.
  Covers chat, search, and fetch alike.
- `search_results(id, ts, instrument_id, query, provider, raw_json)` — daily cache

### HTTP surface

- `GET /api/dashboard` — cash, positions, latest signals, blocked list
- `GET /api/settings` — budget, currency, committed, currency_locked
- `PUT /api/settings` — set budget and/or currency; 409 with `committed` when the
  budget is below it; 409 when currency is locked
- `GET /api/instruments/:symbol` — bars, news, signal history, rationale
- `POST /api/positions` / `PATCH /api/positions/:id` — record a manual buy/sell
- `POST /api/ipo` — record an application (reserves cash)
- `GET /api/health`
- UI is plain Vite + TypeScript, one page, no framework unless a real need appears.

### Notification

On a new actionable signal: format a short message, `POST` to the ntfy topic, record
`dedupe_key = hash(symbol + action + rounded entry)`. Same key again → suppressed.
Priority high for SELL (stop hit), default for BUY.

## Verification

- Unit tests for indicator maths, ATR-based price derivation, and the budget rule
  (including the IPO-reservation case: a reservation must block a buy).
- Settings tests: a budget below `committed` is refused with the committed figure; a
  currency change with ledger entries present is refused; a legal budget change is
  reflected in `available` on the next read.
- A backtest harness over stored daily bars so the rules are measured, not assumed.
  Report hit rate, max drawdown, and how often the cap blocked a signal.
- A recorded-fixture test for the Yahoo client so endpoint drift is caught by a test,
  not by silent empty results.
- One end-to-end check with a fake notifier: poll → signal → single notification, and
  a second poll producing no second notification.
- Validator tests: an LLM response with a stop above entry, a price outside the ATR
  band, a quantity exceeding available cash, and a SELL of an unheld instrument must all
  be rejected and fall back to rules output.
- LLM calls are replayed from stored fixtures in tests; no test may hit the network.
- A search result set is replayed from fixture and asserted to produce a stable
  structured suggestion, so prompt or schema drift shows up as a failing test.
- Escalation tests: a `need_more` response triggers exactly one fetch round; a second
  `need_more` is converted to `pass`; exceeding 3 requested headlines or the character
  budget is truncated, not honoured; a cited `headline_id` that was never sent is
  rejected.
- Parser test: Yahoo news JSON in, headline rows out with no body field populated —
  proving the cheap path is the default path.

## Left to the builder

- Exact indicator thresholds and ATR multiples (config defaults, easy to tune).
- Whether AMFI or Yahoo is authoritative when both cover an instrument.
- Prompt wording, and the exact JSON schema for both the `need_more` and `decide` shapes.
- Which symbols route to DeepSeek Search rather than Yahoo news (threshold on headline
  count/age) — tune from `api_calls` data after a week of running.
- Whether the headline→article escalation earns its cost. `api_calls` records enough to
  compare tokens spent against signal quality later.
- ATR band widths used by the validator.
- Watchlist seed contents.
- Whether to keep plain TS or introduce React once the dashboard grows.

## Open, needed before first real trade

- The budget number and currency — now entered in the UI on first run, so nothing blocks
  the build.
- Confirmation that holdings live at an Indian broker (drives AMFI vs Yahoo for MFs).
- A DeepSeek API key and the daily call cap you are comfortable paying for.

## UI

The interface design — direction, tokens, layout, component states, and the pages the
build must produce — is specified in `.interface-design/system.md`. That document is
binding for the front-end tasks; it is not a mood board.
