# UI and API refactor — plan

Status: proposed. Supersedes nothing; the binding UI rules stay in `.interface-design/system.md`
and `web/UI_STANDARDS.md`.

## Why

One shared stylesheet holds two generations of layout. `.rail`, `.main` and `.page` were used by the
deleted renderer and by the Vue shell, so a later rule silently moved the other layout: the rail
carried `top: 24px` and a 17px inset from a dead block, and the page header carried the browser's
`h1` margin. Small fixes cost hours because the ownership of a selector is unknown.

Measured before this plan: `app.css` 1,476 lines, 263 class selectors, 70 of them never referenced
from `.vue`/`.ts`, 113 defined more than once (`.rail` 15×, `.row` 17×, `.card` 14×, `.btn` 14×).

## Phase 1 — a design contract per page

One screen each in `.interface-design/pages/`:

- `ledger.md` — the holdings book: header figures, mode row, decisions, stocks, funds
- `instrument.md` — one stock: back, title, figures, chart, fills, decisions, headlines, side cards
- `archived.md` — what left the book: header figures, list, empty state
- `audit.md` — runs, filters, call list, pager
- `settings.md` — six routed tabs, save bar
- `access.md` — keys and MCP history
- `login.md` — the gate: setup and sign-in

Each contract names the page's job, hierarchy, every state (empty, loading, error, disabled,
permission), and its rules at 1440 / 768 / 390. A contract is the review baseline: a change that
contradicts it is a spec change, not a CSS fix.

## Phase 2 — CSS ownership

Target: `app.css` keeps tokens, base, the shell and shared primitives only (about 500 lines).
Page-specific rules move into each view's scoped `<style>` block, so no selector can reach another
page. Generic names (`.row`, `.name`, `.sub`, `.num`, `.on`, `.main`, `.card`) are renamed where
they cross pages; `.rail` keeps its own namespace.

Done so far: deleted the dead renderer layout (`.page`, `.header`, `.shell`, `.body`, `.side`,
`.nav`, legacy `.rail`, `.detail-head`, `.head-market`), 1,476 → 1,409 lines.

## Phase 3 — Vue structure

- Split `InstrumentView.vue` (≈950 lines) into `InstrumentHeader`, `FillsCard`, `DecisionsCard`,
  `ManageCard`; the view keeps state and routing.
- Move the repeated card head/empty/help markup into the existing `components/ui/` kit.
- Keep `api.ts` and `format.ts` as the only contracts between views and data.

## Phase 4 — API conformance

Review against `api-and-interface-design`. Instruments are addressed by `{symbol}` everywhere while
trades, positions and keys use `{id}`; that split is deliberate (one row per symbol, migration 0023) and stays. The failure is in meaning, not in parameters:

| Today                                                          | Problem                                                                                             |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `DELETE /api/instruments/{symbol}`                             | Archives. `DELETE` promises destruction; the response admits `{"deleted": false}`.                  |
| `DELETE /api/instruments/{symbol}/records`                     | Deletes. The destructive act hides under a vague sub-resource while the harmless one owns `DELETE`. |
| `POST /api/instruments/{symbol}/restore`                       | An action verb, but archive has none — inconsistent pair.                                           |
| `400` when archiving an archived stock, or deleting a held one | Both are conflicts with current state; `409` is the mapped code.                                    |
| Delete answers `{symbol, deleted, cash}`                       | Resource deletion should answer `204`; the cash figure is a side effect, not the resource.          |

Target contract:

```
POST   /api/instruments/{symbol}/archive   -> 200 {symbol, archived: true}     409 if already archived
POST   /api/instruments/{symbol}/restore   -> 200 {symbol, archived: false}    409 if not archived
DELETE /api/instruments/{symbol}           -> 204                              409 if held
GET    /api/instruments?archived=true      -> the archived list (replaces /api/archived, optional)
```

This is a breaking rename of a pre-1.0 surface: the UI and the docs change in the same stack, and the
old `/records` path is removed rather than aliased, because it was introduced in the same release.

Also in scope, as one separate step: error bodies are `{"error": "text"}`. The standard shape is
`{"error": {"code", "message", "details"}}`; every view's error rendering changes with it, so it is
its own PR, not folded into the rename.

## Phase 5 — delivery

Stacked PRs, merged bottom-up. Backend and UI stay in different PRs so either can be reviewed and
merged alone.

1. `feat/api-delete-instrument` — the delete endpoint, tests, `docs/api.md`
2. `feat/ui-shell-and-delete` — `DeleteInstrument.vue`, the Manage card, `api.ts`, rail icons, phone
   bottom nav, rail spacing, CSS dead-code deletion, flows doc
3. `docs/ui-api-refactor-plan` — this plan and the page contracts

Nav and delete share one CSS file today, so they ship in one PR. They split into separate PRs once
Phase 2 has moved page rules into scoped `<style>` blocks. Phase 4 is a later PR stacked on the
backend branch.

## Verification for every step

`go build ./...`, `go vet ./...`, `go test ./...`, `gofmt -l`, `npx vitest run`, `npm run build`,
then real-browser screenshots at 1440 and 390 in light and dark, with the alignment measured by
`getBoundingClientRect()` rather than eyeballed.
