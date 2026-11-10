# stock-predictor — agent notes

## Reading code: never open a whole file to inspect one function

**Always load the `code-read` skill before touching code** (`/skill:code-read`, or `read`
its `SKILL.md`). It encodes this whole flow. The commands below are the concrete spelling (CLI)
for when the skill is already known.

One index, three queries, then one narrow read.

**1. Index the repo** (once; persists and re-syncs):

```sh
codebase-memory-mcp cli index_repository --repo-path /home/mohan/projects/stock-predicter
```

**2. Locate.** Never guess a path:

```sh
codebase-memory-mcp cli search_graph --project stock-predictor --name-pattern '<symbol>'
codebase-memory-mcp cli search_graph --project stock-predictor --query '<what it does>'
codebase-memory-mcp cli search_code  --project stock-predictor --pattern '<exact string>'
```

**3. Resolve a symbol to its source range:**

```sh
codebase-memory-mcp cli get_file_outline --project stock-predictor --file-path src/llm/orchestrate.rs
codebase-memory-mcp cli get_code_snippet --project stock-predictor --name <qualified.name>
```

**4. Read only those ranges**, batched into one turn. Read a whole file only if it is under
~120 lines or the question genuinely spans it. `search_code` confirms an exact literal — then
read the range around the hit.

Markdown (docs, specs, `.interface-design/`) is indexed by the graph too, but for prose read the
files directly when you need exact wording.

## Project facts worth not rediscovering

- Spec: `docs/superpowers/specs/2026-10-27-stock-predictor-design.md` (binding).
- Plan: `docs/superpowers/plans/2026-10-27-stock-predictor.md` (binding, task by task).
- Design system: `.interface-design/system.md` (binding for all front-end work).
- No test may hit the network. Fixtures only.
- `api_calls` and `cash_ledger` are append-only. Nothing updates or deletes those rows.
- **Every money write is one transaction.** A fill moves the position, the trade row and the
  cash ledger together (`trades::record`), a correction does the same (`trades::edit`), and a whole
  plan posts inside one transaction (`approve_plan`). A leg that cannot post rolls the others back —
  half a swap is worse than none.
- The environment is four variables only: `BIND`, `DB_PATH`, `PUBLIC_URL`, `SECRET_KEY`.
- Provider keys, the ntfy token and the proxy password are encrypted at rest (`src/secrets.rs`,
  `enc:v1:` prefix). Losing the key means re-entering them. Everything else —
  budget, limits, provider and keys, ntfy, polling cadence, the proxy — is a `settings` row,
  edited on the Settings page and applied without a restart.
- The settings page is six routed tabs (`/settings/:tab`), each stamped on its own.
- Prompt files: `prompts/{decide_v1,escalate_v1,rotate_v1,trend_v1,short_v1,plan_stock_v1,plan_mf_v1}.md`.
  Two book calls, two books, two purses: `plan_stock_v1` (whole stock book, 0–3 moves, every buy
  carries a stop and target, stored as one `plan_id` group) and `plan_mf_v1` (fund book only, 0–2
  moves sized in rupees, no stops — a fund has no price to stop at). The fund plan is asked on the
  first tick of each IST day (`mf_plan_last`), whether or not the scan ran that tick — so a restart
  at noon still asks. **Both** plans are once-a-day book decisions (`stock_plan_last` /
  `mf_plan_last`); per-stock calls cover the intraday. `plan_v1` still resolves to the stock prompt
  for old audit rows.
- The short-term list is asked once per IST day (`trend_list_last`) and stored in
  `trend_candidates`: NSE's `allSec` gainers, NSE's `NIFTY` group (the Nifty 50's own movers) and
  one DeepSeek `/v1/tools/search` read by `trending_v1`. Deduped by symbol, the exchange winning.
  Each candidate is then asked with its own search results as `{{web}}`.
- The proxy is chosen only by the database: `src/http.rs` is the one client builder and calls
  `.no_proxy()` first, so `HTTP_PROXY` can never win.

## Front-end work: always use the `ui-ux-design-pro` skill

Before any UI change, load `ui-ux-design-pro` (`/skill:ui-ux-design-pro`, or read its `SKILL.md`
and the reference it points at). It carries the token architecture, the type scale and the
checklist that keep this app looking like one hand drew it. Then obey `.interface-design/system.md`.

Non-negotiables it enforces here: one alignment rule per table (text left, numbers right), no
column whose content can force the table wider than the page (long text wraps in its own cell),
and no new rail item appended to a growing list — a page joins a group.

A UI change is done when a headless screenshot shows it, at 1440 and 390, not when the file is
written — and when the alignment is measured, not eyeballed: `getBoundingClientRect()` on the
left edge of every line that is supposed to share one, and on the right edge of every number.

## Front end: three layers, no framework

`web/src` is plain TypeScript that renders HTML strings. Keep the layers apart:

- `pages/*.ts` — render only: a payload in, markup out, no DOM access, no event listeners. This is
  why they can be unit-tested with `vitest` and why `renderLedger(d)` keeps its signature.
- `components/*.ts` — shared atoms: `stamp.ts` (`stamp`, `actionStamp`, `toneOf`, `pill`,
  `statusPill`), `decision-actions.ts`, `confidence.ts`, `risk.ts`, `shell.ts`, `call-viewer.ts`.
  A stamp or pill is never hand-written in a page; it goes through the atom, so a BUY looks the
  same on every screen.
- `wiring/*.ts` — behaviour: one module per feature (`settings`, `add-instrument`, `decisions`,
  `runs`, `fills`, `calls`). Each takes a `reload` callback instead of importing the router, which
  is what keeps `main.ts` a router and a timer (about 280 lines).
- `main.ts` — routing, the 60-second refresh, nothing else.

A list that is paged is filtered on the server: `/api/calls` takes `prompt`, `call`, `model` and
`review`, and `x-total-count` is the *filtered* total, because a pager that counts the whole record
is worse than no filter. The `WHERE 1 = 1` anchor in that handler is load-bearing — without it the
first clause lands in the JOIN's `ON`, where it silently filters nothing.

## Quality gates

```sh
cargo test                                   # 261 tests, no network
cargo clippy --all-targets                   # must be warning-free
cargo bench --bench hot_paths                # indicators, prompt render, bar upsert
cd web && npx vitest run && npm run build
```

Indexes answer a query that exists. Every one was checked with `EXPLAIN QUERY PLAN` against the
real statements; three were dead (`runs_by_started`, `idx_api_calls_conversation`,
`idx_signals_status`) and are dropped. `runs_running` is a *partial* index — only running rows —
because `runs` only grows and the audit page asks "anything running?" on every load.

Budgets to keep: `indicators::compute` ~2.8 µs per 250 bars, prompt render ~58 µs (the book block and cap line cost ~2 µs),
a poll's bar write ~0.2 ms (only the tail is written; a cold 250-row load is ~18 ms).

Token knobs: `PROMPT_WINDOW_HOURS` (default 4) is the intraday tape length, the biggest single
lever; the static prompt block is a prefix so DeepSeek's context cache can hit it.
Request count: one per page (`/api/dashboard` carries recent trades, `/analysis` carries signals
and fills), and the dashboard answers 304 when its version tag is unchanged.

A **run** (`src/runs.rs`, table `runs`) is one unit of scheduler work that *decides* something —
stock plan, fund plan, short-term pass, or a run you started by hand. A scheduled poll writes no
row: at 15-minute ticks that was ninety-six "nothing happened" rows a day, and the calls it made are
already in `api_calls`. `GET /api/schedule` computes "last ran" and "next due" from the same markers
the scheduler uses (`stock_plan_last`, `mf_plan_last`, the NSE window, `poll_secs`) — the page must
not re-implement those rules.

A **run** (`src/runs.rs`, table `runs`) is one unit of scheduler work — poll, stock plan, fund
plan, short-term pass — opened *before* the work and closed after it, because `api_calls` rows only
exist once a reply lands and therefore cannot show "running". Orphans left running by a killed
process are closed at boot. `AppState.manual` is the channel the API uses to ask the **loop** to run
something: the loop owns the market clients, so a manual run is the same code as a scheduled one.

The tag fingerprints the **data**, not row counts: an edited fill changes no id, an archive changes
no id, and a count-based tag answered 304 through both — the screen then looked frozen until a
hard refresh. Any new in-place write needs to be in that hash, or it will be invisible.
