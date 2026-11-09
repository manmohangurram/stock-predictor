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
- The environment is four variables only: `BIND`, `DB_PATH`, `PUBLIC_URL`, `SECRET_KEY`.
- Provider keys, the ntfy token and the proxy password are encrypted at rest (`src/secrets.rs`,
  `enc:v1:` prefix). Losing the key means re-entering them. Everything else —
  budget, limits, provider and keys, ntfy, polling cadence, the proxy — is a `settings` row,
  edited on the Settings page and applied without a restart.
- The settings page is six routed tabs (`/settings/:tab`), each stamped on its own.
- Prompt files: `prompts/{decide_v1,escalate_v1,rotate_v1,trend_v1,short_v1,plan_stock_v1,plan_mf_v1}.md`.
  Two book calls, two books, two purses: `plan_stock_v1` (whole stock book, 0–3 moves, every buy
  carries a stop and target, stored as one `plan_id` group) and `plan_mf_v1` (fund book only, 0–2
  moves sized in rupees, no stops — a fund has no price to stop at). The fund plan runs after the
  daily arithmetic scan; `plan_v1` still resolves to the stock prompt for old audit rows.
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

## Quality gates

```sh
cargo test                                   # 261 tests, no network
cargo clippy --all-targets                   # must be warning-free
cargo bench --bench hot_paths                # indicators, prompt render, bar upsert
cd web && npx vitest run && npm run build
```

Budgets to keep: `indicators::compute` ~2.8 µs per 250 bars, prompt render ~58 µs (the book block and cap line cost ~2 µs),
a poll's bar write ~0.2 ms (only the tail is written; a cold 250-row load is ~18 ms).

Token knobs: `PROMPT_WINDOW_HOURS` (default 4) is the intraday tape length, the biggest single
lever; the static prompt block is a prefix so DeepSeek's context cache can hit it.
Request count: one per page (`/api/dashboard` carries recent trades, `/analysis` carries signals
and fills), and the dashboard answers 304 when its version tag is unchanged.

The tag fingerprints the **data**, not row counts: an edited fill changes no id, an archive changes
no id, and a count-based tag answered 304 through both — the screen then looked frozen until a
hard refresh. Any new in-place write needs to be in that hash, or it will be invisible.
