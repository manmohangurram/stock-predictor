# stock-predictor — agent notes

## Comments: never add noise

**Do not add explanatory comments.** No narration of what the code does, no paragraph above a
CSS rule or a branch saying why it is nicer this way, no "this fixes X" beside a line. If a change
needs a story, the story belongs in `CHANGELOG.md` or the commit message.

**Nothing on a declaration.** A field, a var, a constant, a struct member and a one-line helper are
named well enough by their name. A comment over them that says what they are — `// The owner's
profit expectation for this instrument, if any.`, `// staleDaily is set when …` — is noise, and it
is removed when found. Rename the field instead.

The only comment allowed is one line at a point of non-obvious logic — a boundary that looks
arbitrary (`// 15:30 IST == 10:00 UTC; NSE has no DST`), a trap (`// The WHERE 1 = 1 anchor is
load-bearing`), an ordering rule (why this write comes before that one), or a trade-off with a
number behind it. One line. If it needs two, it is not a comment, it is a changelog entry.

The declarations were swept once and that is the baseline: do not restore a field comment "for
clarity", and do not add one to a field you introduce.

## Reading code: always load `code-read`, never open a whole file for one function

**Every session, before touching code: load the `code-read` skill** (`skill` tool, name
`code-read`; it lives in `~/.agents/skills/code-read/SKILL.md`). It is the required route into this
repo — locate by name, file name or keyword, outline the file, read one function. The commands
below are the concrete spelling for when the skill is already loaded.

**1. Index** (once per checkout; re-index when `index_status` is older than the last commit):

```sh
codebase-memory-mcp cli index_status --project stock-predictor
codebase-memory-mcp cli index_repository --repo-path "C:/Users/Mohan/Documents/Projects/stock-predictor"
```

**2. Locate.** Never guess a path:

```sh
codebase-memory-mcp cli search_graph --project stock-predictor --name-pattern '<symbol>'
codebase-memory-mcp cli search_graph --project stock-predictor --file-pattern '<file fragment>'
codebase-memory-mcp cli search_graph --project stock-predictor --query '<what it does>'
codebase-memory-mcp cli search_code  --project stock-predictor --pattern '<exact string>'
```

**3. Resolve a symbol to its source range:**

```sh
codebase-memory-mcp cli get_file_outline --project stock-predictor --file-path internal/llm/orchestrate.go
codebase-memory-mcp cli trace_path --project stock-predictor --function-name <Name> --direction both --depth 2
codebase-memory-mcp cli get_code_snippet --project stock-predictor --qualified-name <qn>
```

`--qualified-name` is the flag; `--name` silently returns nothing. The `qn` column from step 2 is
the argument.

**4. Read only those ranges**, batched into one turn. Read a whole file only if it is under
~120 lines or the question genuinely spans it. `search_code` confirms an exact literal — then
read the range around the hit.

A stale graph is worse than no graph: it answers with code that no longer exists. If a lookup for a
file you know is missing, re-index before concluding anything.

Markdown (docs, specs, `.interface-design/`) is indexed by the graph too, but for prose read the
files directly when you need exact wording.

## Project facts worth not rediscovering

- Spec: `docs/superpowers/specs/2026-10-27-stock-predictor-design.md` (binding).
- Plan: `docs/superpowers/plans/2026-10-27-stock-predictor.md` (binding, task by task).
- Design system: `.interface-design/system.md` (binding for all front-end work).
- No test may hit the network. Fixtures only.
- **Every identity is a UUIDv7 in text** (`internal/ids`, a wrap over `google/uuid`), and every
  foreign key is that text — a plan's `plan_id`, a fill's `instrument_id`, a notification's
  `signal_id`. There is no integer key and no second `uuid` column: the id sorts in creation order,
  so `ORDER BY id DESC` still means "newest first". The migrations are only in
  `internal/db/migrations` (the tree the embed reads); do not recreate `migrations/` at the root.

- `api_calls` and `cash_ledger` are append-only. Nothing updates or deletes those rows.
- **Every money write is one transaction.** A fill moves the position, the trade row and the
  cash ledger together (`trades::record`), a correction does the same (`trades::edit`), and a whole
  plan posts inside one transaction (`approve_plan`). A leg that cannot post rolls the others back —
  half a swap is worse than none.
- The environment is four variables only: `BIND`, `DB_PATH`, `PUBLIC_URL`, `SECRET_KEY`.
- A developer's `.env` (gitignored; `.env.example` is the template) is read at start-up and may
  carry `DEEPSEEK_API_KEY` (`OPENAI_API_KEY`, `GEMINI_API_KEY`, `CUSTOM_API_KEY`) so prompts can
  be exercised without a key at rest. It never writes a setting: a stored key wins, and an
  already-exported variable is not overridden. Nothing from `.env` ships.
- Provider keys, the ntfy token and the proxy password are encrypted at rest (`internal/secrets`,
  `enc:v1:` prefix). Losing the key means re-entering them. Everything else —
  budget, limits, provider and keys, ntfy, polling cadence, the proxy — is a `settings` row,
  edited on the Settings page and applied without a restart.
- The settings page is six routed tabs (`/settings/:tab`), each stamped on its own.
- Prompt files: `prompts/{decide_v1,escalate_v1,rotate_v1,trend_v1,short_v1,plan_stock_v1,plan_mf_v1}.md`.
  Two book calls, two books, two purses: `plan_stock_v1` (whole stock book, 0–3 moves, every buy
  carries a stop and target, stored as one `plan_id` group) and `plan_mf_v1` (fund book only, 0–2
  moves sized in rupees, no stops — a fund has no price to stop at). Both daily plans are asked on
  the first tick at or after 09:05 IST (`stock_plan_last` / `mf_plan_last`), whether or not the scan
  ran that tick — so a restart at noon still asks. **Both** plans are once-a-day book decisions (`stock_plan_last` /
  `mf_plan_last`); per-stock calls cover the intraday. `plan_v1` still resolves to the stock prompt
  for old audit rows.
- The short-term list is asked once per IST day (`trend_list_last`) and stored in
  `trend_candidates`: NSE's `allSec` gainers, NSE's `NIFTY` group (the Nifty 50's own movers) and
  one DeepSeek `/v1/tools/search` read by `trending_v1`. Deduped by symbol, the exchange winning.
  Each candidate is then asked with its own search results as `{{web}}`. The search query carries
  the window as operators — `after:YYYY-MM-DD lang:en` — and a result older than three days, undated
  or untitled is dropped anyway (the API *does* support `after:`/`before:`/`lang:`/`site:`).
- The proxy is chosen only by the database: `internal/httpx` is the one client builder and calls
  `.no_proxy()` first, so `HTTP_PROXY` can never win.

## Front-end work: always use the `ui-ux-design-pro` skill

**A UI change is not done until screenshots exist — and they are shown.** Before you say a screen is
fixed, capture it at **1440 and 390**, in **light and dark**, and put the images in the conversation
(or the PR). Measure what matters with `getBoundingClientRect()` — shared left edges, right edges of
numbers, input widths, overflow — rather than describing it. A passing build, a green CI and a
type-check say nothing about how it looks; a screenshot that has not been taken is a claim you cannot
make. If a screenshot cannot be produced, say so plainly instead of implying the visual was checked.


Before any UI change, load `ui-ux-design-pro` (`/skill:ui-ux-design-pro`, or read its `SKILL.md`
and the reference it points at). It carries the token architecture, the type scale and the
checklist that keep this app looking like one hand drew it. Then obey `.interface-design/system.md`.

Non-negotiables it enforces here: one alignment rule per table (text left, numbers right), no
column whose content can force the table wider than the page (long text wraps in its own cell),
and no new rail item appended to a growing list — a page joins a group.

A UI change is done when a headless screenshot shows it, at 1440 and 390, not when the file is
written — and when the alignment is measured, not eyeballed: `getBoundingClientRect()` on the
left edge of every line that is supposed to share one, and on the right edge of every number.

## Front end: Vue, one page at a time

The stack is **Vue 3 + vue-router** (design: `docs/superpowers/specs/2026-11-14-vue-migration-design.md`).
The migration is **finished**: every page is a view (`views/`), the renderers (`pages/`, `wiring/`,
`renderer.ts`, `LegacyView.vue`) are deleted, and `main.ts` is a router and nothing else.

**daisyUI is installed with the `d-` prefix** (`web/src/styles/tailwind.css`): a ported page writes
`d-btn`, `d-input`, `d-modal`; an unported page keeps its own classes, so neither can restyle the
other. The theme is this app's paper and ink, not a daisyUI preset. Preflight is off on purpose —
it would reset every page still drawn by a renderer.

**`web/UI_STANDARDS.md` is binding** — tokens, type scale, alignment rules, component rules, states,
copy, and the review checklist. It is the written form of this file's UI rules; when they disagree,
the standards document wins on presentation and this one wins on behaviour.

The kit lives in `web/src/components/ui/` — `AppButton`, `AppStamp`, `AppEmpty` today, one per
primitive we actually reuse. A page never hand-writes a `.btn` or a `.stamp`.

The design system did **not** change: `.interface-design/system.md` is still binding, paper and ink
are still the material, and Tailwind/daisyUI were deliberately not adopted.

The layers that mattered are still separate — a view holds state and behaviour, `components/ui/` and
the named components hold markup, `api.ts`/`format.ts` are the contract, and `call-detail.ts` is the
one module that still produces HTML strings (its bodies are pure and escaped). The old layering notes
follow for reference:

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

## Delivery: every change is a PR

**Never cut a release without consent.** A tag, a GitHub release and a version bump in the CHANGELOG
are one action, and it happens only when the user asks for it in that turn ("release", "ship it",
"give me a release now"). Merging a PR is not consent. A green CI is not consent. "It is ready" is not
consent. If the work is finished, say it is finished and stop; the next release is the user's call.

**A feature that touches more than one layer is a stack, built with the `gh stack` extension.** A
schema change plus an API plus a page is three PRs, not one: install `gh extension install
github/gh-stack` once, then `gh stack init`, `gh stack add`, `gh stack push`, `gh stack submit`. The
bottom PR targets `main` and each one above it targets the branch below. One large PR for a
multi-layer feature is a policy violation, not a shortcut. A stack merges bottom-up; a red step
blocks everything above it.


**Never commit straight to `main`.** Every piece of work — a fix, a page, a migration — lands as a
pull request that passes the gates below. Open the PR as soon as the first commit exists, so CI runs
early rather than after the whole thing is written.

**A big feature is a stack of small PRs, not one large one.** Each branch is based on the one before
it, each PR targets that previous branch, the bottom one targets `main`, and the stack merges
bottom-up. Every step stays independently green and reviewable; if a step cannot be explained in one
sentence, split it. Use GitHub's stack extension — reference:
[`docs/reference/github-stacked-prs.md`](docs/reference/github-stacked-prs.md).

```sh
gh extension install github/gh-stack    # once
gh stack init                           # first branch of the stack
# work, commit
gh stack add BRANCH-NAME                # next layer
gh stack add -Am "message"              # stage + commit + next branch
gh stack push                           # all branches
gh stack submit                         # opens the PRs with chained bases
gh stack view                           # order, PR links, statuses
```

Without the extension, the same chain by hand: branch from the previous branch and
`gh pr create --base <previous-branch>`; the bottom PR targets `main`.

- Branches: `<type>/<short-topic>` — `feat/`, `fix/`, `chore/`, `docs/`.
- A PR says what changed, why, which spec/task it serves, and how it was verified (commands, flows
  walked); a stacked PR also says its position ("1/3").
- Refactors and behaviour changes are separate PRs; so are dependency bumps.
- A stack is never merged top-down, and a red step blocks the ones above it.

## Quality gates

Before a release, **the browser flows must pass**: `docs/testing/flows.md` is the list — every
page, every control, what it must do — and it is walked with the Chrome DevTools MCP against a
throwaway database. CI does not run it (it needs a browser); a release is not cut until it is green.

**Every push runs build, fmt and audit** — not only a release. They are cheap, and a red one is
always cheaper to fix now than after it lands.

```sh
go build ./...                               # must compile
gofmt -l cmd internal                        # must print nothing
go vet ./...                                 # must be warning-free
go test ./...                                # no network
go test ./internal/rules/ -bench . -run XXX  # indicators, prompt render, bar upsert
cd web && npx vitest run && npm run build
```

**Audit findings are fixed by updating dependencies**, not by ignoring them: bump the module
(`go get <module>@latest && go mod tidy`), and if a pin blocks the fix, say which pin and when it
lifts. `govulncheck ./...` when it is available; a suppression needs a reason and an **end
condition**, the same rule the old `.cargo/audit.toml` entry followed.

Indexes answer a query that exists. Every one was checked with `EXPLAIN QUERY PLAN` against the
real statements; three were dead (`runs_by_started`, `idx_api_calls_conversation`,
`idx_signals_status`) and are dropped. `runs_running` is a *partial* index — only running rows —
because `runs` only grows and the audit page asks "anything running?" on every load.

Budgets to keep (Go, measured on the dev machine): `rules.ComputeIndicators` ~2.7 µs per 250 bars,
prompt render ~41 µs (the book block and cap line are the cheap part), a poll's bar write ~94 µs for
the tail (a cold 250-row load is ~4.7 ms).

Token knobs: `PROMPT_WINDOW_HOURS` (default 4) is the intraday tape length, the biggest single
lever; the static prompt block is a prefix so DeepSeek's context cache can hit it.
Request count: one per page (`/api/dashboard` carries recent trades, `/analysis` carries signals
and fills), and the dashboard answers 304 when its version tag is unchanged.

A **run** (`internal/runs`, table `runs`) is one unit of scheduler work that *decides* something —
stock plan, fund plan, short-term pass, or a run you started by hand. A scheduled poll writes no
row: at 15-minute ticks that was ninety-six "nothing happened" rows a day, and the calls it made are
already in `api_calls`. `GET /api/schedule` computes "last ran" and "next due" from the same markers
the scheduler uses (`stock_plan_last`, `mf_plan_last`, the NSE window, `poll_secs`) — the page must
not re-implement those rules.

A **run** (`internal/runs`, table `runs`) is one unit of scheduler work — poll, stock plan, fund
plan, short-term pass — opened *before* the work and closed after it, because `api_calls` rows only
exist once a reply lands and therefore cannot show "running". Orphans left running by a killed
process are closed at boot. `AppState.manual` is the channel the API uses to ask the **loop** to run
something: the loop owns the market clients, so a manual run is the same code as a scheduled one.

The tag fingerprints the **data**, not row counts: an edited fill changes no id, an archive changes
no id, and a count-based tag answered 304 through both — the screen then looked frozen until a
hard refresh. Any new in-place write needs to be in that hash, or it will be invisible.
