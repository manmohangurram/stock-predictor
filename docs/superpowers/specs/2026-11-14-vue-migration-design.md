# Vue migration — design (2026-11-14)

Agreed with the owner. Approach **B** from the conversation: a Vue shell and a component kit, then
one page at a time, so the app works at every step.

## What changes, and what does not

**Changes:** the rendering layer. Plain TypeScript that builds HTML strings becomes Vue single-file
components; `main.ts`'s hand-rolled router becomes `vue-router`; state becomes reactive instead of
"re-render the whole page and re-wire every listener".

**Does not change:** the design system. `.interface-design/system.md` stays binding — same paper, same
ink, same stamps, same rules. Financer's `UI_STANDARDS.md` is *the same material world* (a ledger
page, a margin column, ticks) with a different job, so its **document shape** is copied and its
fiction is not.

**Deliberately not adopted:** Tailwind and daisyUI. They are financer's styling engine; this app
already has tokens and a written system, and swapping the engine would silently redesign every
screen — which is the second job, not this one.

## The kit

`web/src/components/ui/` — one primitive per file, following financer's set where we have the same
need: `AppButton`, `AppStamp`, `AppPill`, `AppTable`, `AppModal`, `AppEmpty`, `AppSkeleton`,
`AppTabs`, `AppField`. Each is a thin wrapper over a class we already have (`.btn`, `.stamp`, `.pill`,
`.blotter`), so the look cannot drift when the markup moves into components.

## Stages

Each stage ships on its own and is verified against `docs/testing/flows.md`.

| # | Stage | Ends when |
|---|---|---|
| 1 | Vue + router + kit + shell; every route mounted with its current renderer | the app is unchanged on screen, and the stack is in place |
| 2 | **Ledger** (long and short) as a real view | decisions, tables and figures are reactive; no `innerHTML` |
| 3 | Instrument page | chart, fills, edit/delete, archive |
| 4 | Audit | filters, drawer with the markdown prompt, copy, auto-refresh |
| 5 | Archived | rows, restore link |
| 6 | Settings | six routed tabs, save/discard |
| 7 | Delete `pages/*.ts` and `wiring/*.ts` | nothing imports them; gates and flows green |

## Rules for the port

- A page moves **whole**. Half a page in Vue and half in a string is how two renderers end up
  disagreeing about the same screen.
- **Wiring dies with the page.** No `addEventListener` in a view; a component owns its state, and a
  watcher owns the refresh timer.
- The **API layer stays** (`api.ts`, `format.ts`): it is the contract, and it is already tested.
- Text tests stay. `render.test.ts` asserts strings today; each stage adds component tests and the
  string assertion for that page is deleted in the same commit.
