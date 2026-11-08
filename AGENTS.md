# stock-predictor — agent notes

## Reading code: never open a whole file to inspect one function

**Use the `code-read` skill if it is available** — it encodes this whole flow. The commands
below are the concrete spelling (CLI) and the fallback when the skill is not loaded.

One index, three queries, then one narrow read.

**1. Index the repo** (once; persists and re-syncs):

```sh
codebase-memory-mcp cli index_repository --repo-path /home/mohan/projects/stock-predictor
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
- The proxy is chosen only by the database: `src/http.rs` is the one client builder and calls
  `.no_proxy()` first, so `HTTP_PROXY` can never win.

## Quality gates

```sh
cargo test                                   # 139 tests, no network
cargo clippy --all-targets                   # must be warning-free
cargo bench --bench hot_paths                # indicators, prompt render, bar upsert
cd web && npx vitest run && npm run build
```

Budgets to keep: `indicators::compute` ~2.8 µs per 250 bars, prompt render ~17 µs,
a poll's bar write ~0.2 ms (only the tail is written; a cold 250-row load is ~18 ms).

Token knobs: `PROMPT_WINDOW_HOURS` (default 4) is the intraday tape length, the biggest single
lever; the static prompt block is a prefix so DeepSeek's context cache can hit it.
Request count: one per page (`/api/dashboard` carries recent trades, `/analysis` carries signals
and fills), and the dashboard answers 304 when its version tag is unchanged.
