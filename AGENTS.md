# stock-predicter — agent notes

## Reading code: never open a whole file to inspect one function

Cheapest first. Two lookups, then one narrow read.

**1. Locate.** Never guess a path:

```sh
semble search "<what you're hunting>" --max-snippet-lines 0 --top-k 5
```

Returns `{file_path, start_line, end_line, score}`. Works with no setup.

**2. Resolve a symbol to its exact range** (once `graphify-out/graph.json` exists):

```sh
python3 ~/.pi/agent/skills/code-read/scripts/read-ranges.py <Symbol>          # path:start-end
python3 ~/.pi/agent/skills/code-read/scripts/read-ranges.py <Symbol> --show   # and print it
python3 ~/.pi/agent/skills/code-read/scripts/read-ranges.py --file src/llm/orchestrate.rs
```

**3. Build / refresh the graph** (needed after the first Rust files land):

```sh
graphify . && python3 ~/.pi/agent/skills/code-read/scripts/add-end-lines.py
# after any rebuild — `graphify --update` wipes end lines, so always re-enrich:
graphify . --update && python3 ~/.pi/agent/skills/code-read/scripts/add-end-lines.py
```

**4. Read only those ranges**, batched into one turn. Read a whole file only if it is under
~120 lines or the question genuinely spans it. `grep`/`rg` confirms an exact literal — then
read the range around the hit.

Semble indexes source files only; markdown (docs, specs, `.interface-design/`) is not
searchable there, so read those directly.

## Project facts worth not rediscovering

- Spec: `docs/superpowers/specs/2026-10-27-stock-predicter-design.md` (binding).
- Plan: `docs/superpowers/plans/2026-10-27-stock-predicter.md` (binding, task by task).
- Design system: `.interface-design/system.md` (binding for all front-end work).
- No test may hit the network. Fixtures only.
- `api_calls` and `cash_ledger` are append-only. Nothing updates or deletes those rows.
- The budget ceiling lives in the `settings` table and is edited from the UI; `BUDGET` /
  `CURRENCY` env values seed it on first boot and are then ignored.

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
