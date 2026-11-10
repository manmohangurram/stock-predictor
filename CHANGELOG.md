# Changelog

Tags before `v0.19.0` were cut without notes; their subjects are in `git log`. This file starts
here and records what changed for a reader of the software, not for a reader of the diff.

## v0.21.0

### Audit: the schedule first, the record below

- **One card per scheduled prompt** — Stock plan, Fund plan, Short-term pass — each showing the
  **time it last ran**, what came of it, **when it is due next**, and its cadence in words
  (*"once a day, first tick after midnight IST"*). `GET /api/schedule` computes that from the same
  markers the scheduler uses, so the page never re-implements the rules.
- **`Run again` now runs.** A manual run used to be silently refused by the once-a-day gate and
  reported as *"nothing worth doing"*; it now skips that gate (the daily call cap still applies)
  and writes the day marker.
- **Scheduled polls stop writing run rows** — ninety-six "nothing happened" rows a day that no
  screen showed, while the calls they made are already the history table. Migration `0018` deletes
  the old ones. The strip shows the three prompts that decide something.
- **Filters**: prompt, call, model and review, server-side because the list is paged, with
  `x-total-count` following the filter and the values read from `/api/calls/facets`. The channel
  filter is `?channel=` so it cannot swallow `?call=<id>`, the deep link that opens one call.
- **The drawer is two columns**: what was sent, and what came back — the parsed verdict above the
  verbatim reply, instead of the same content shown twice.
- Model names read as labels (`decide_v1` → **Decide v1**); prompt type and call are capitalised;
  an empty review is an empty cell, not "no decision"; the Outcome column is gone.

### Market state

- **`GET /api/market`** reads the clock and the data: weekend, before the open, after the close,
  open — or, on a weekday inside session hours with nothing priced today, *probably a holiday*, and
  it says probably. There is no holiday calendar in this app, and the tooltip says so.
- Shown **at the foot of the rail**: a dot for colour, the status in words, and the one date that
  matters — *"Opens 11 Nov 09:15"*, or *"Closes 15:30"* when it is open.

### The frame

- **The rail never scrolls.** It is a full-height column that ends in the market block; only the
  page column moves. On a phone the rail is a bar, the market state moves under the page title.

## v0.20.0

### Fills are correctable, and removable

- **Delete a fill**, next to Save and Cancel in the edit row. The trade row goes, the position moves
  back by exactly what that fill moved it, and the money returns as its own `adjust` entry — the
  ledger stays append-only. A fill that satisfied a decision **reopens that decision**, so an
  approval never exists without a fill behind it. A delete that would leave the book short is
  refused. The button asks twice, in place.
- Every money write is now one transaction: position, trade row and cash ledger commit together,
  and a plan posts all its legs or none.

### Archived instruments

- **Realised P&L** from the fills themselves — there is no price left to value an archived
  instrument at. The list shows `+300.00 / 300.00 out`, and `—` rather than a confident `+0.00`
  when nothing has been sold.
- **Restore** on the instrument's own page (the Manage card swaps Archive for Restore); the
  archived list points there instead of duplicating the button. Prices are fetched again on the
  next poll.
- **Decisions and headlines fold**, opening themselves only when they have content.

### Audit

- **Opening a call works again.** The drawer's query named the holding from the call's purpose but
  only the list query got the join, so every View answered 500. A test now opens a call with a
  decision and one without.
- **The Outcome column is gone**: a failed call says so as a `failed` pill in Review, and the
  message is in the drawer. The duplicated caption line went with it, and "+N earlier" moved out of
  the last run card.

### Interface

- `--green` and `--green-bg` were used in five rules and **defined nowhere**, so every gain, PASS
  and DONE rendered as plain ink. Defined, light and dark: status now reads as colour *and* word.
- **One alignment rule**: tables default to text-left, numbers opt in with `.num`. Every table's
  headers share their cells' edges — measured, not eyeballed.
- A zero no longer wears a plus sign (`Unrealised +0` → `0`).
- Settings: the Money tab's duplicated paragraph is gone, the saved/unsaved pill sits beside the
  title, and a select is the same width as the input next to it.
- Mobile: one run card per row (no sideways scroll), pills and timestamps never wrap, the pager
  wraps its count, and touch targets are 40px.
- **Button copy**: `Add instrument…`, `Post fill` / `Post fills`, `Record fill`, `Save`,
  `Delete fill`, `Refresh`, `Archive`, `Restore`, `Open`, `Send test`, `Test proxy`, `Discard`.
  *"Sell it before archiving"* was a sentence wearing a button — the control is a disabled
  `Archive` and the sentence is help text.

### Dependencies

- `chacha20poly1305` 0.11, `base64` 0.23, `vite` 8, `vitest` 5, `typescript` 7, plus the usual
  lockfile refresh. The new toolchain caught orphaned CSS and thirteen stale test fixtures.
- `Cargo.toml` reports `0.20.0`, matching the tag.

## v0.19.0

### The book

- **The model sees the whole book.** Every prompt carries the holdings and one plain sentence —
  *you may spend at most ₹12,45,570 in this purse; a buy costing more is refused, not shrunk* — so a
  proposal can no longer be sized against money that does not exist.
- **Two plans, two prompts, two purses.** `plan_stock_v1` sees the stock book, sizes in units, and
  every buy must carry a stop and a target. `plan_mf_v1` sees the fund book only, sizes in rupees,
  and has no stops — a fund has no price to stop at. Neither can name the other's instrument.
- **Both plans are once-a-day book decisions** (`stock_plan_last`, `mf_plan_last`), asked on the
  first tick of the IST day — including the first tick after a restart. Per-stock calls cover the
  intraday.
- **A buy needs no sale.** A plan is judged on whether it fits the purse, not on whether it
  contains a sell. Sells still execute first.
- **Approval posts every leg, or none.** A fill moves the position, the trade row and the cash
  ledger in one transaction; a plan is one transaction end to end.

### Exit plans that survive approval

- Approving a decision now writes its stop and target onto the **position**, which is what the
  rules layer compares the close against. Before this, an approved buy had no exit at all.
- A fund shows a NAV exit derived from its profit expectation (`76.75 → +25%`), because a fund has
  no price to stop at.

### Audit

- The page shows **runs** — poll, stock plan, fund plan, short-term pass — opened before the work
  and closed after it, so *running* is visible while it happens. Orphans are closed at boot.
- **Run now** executes the same code the scheduler does, through the poll loop that owns the
  clients. A manual run is one run, not a run plus a tick.
- The calls table pages with `offset`/`limit` and a page-size selector; the total travels in a
  header.
- A quiet tick says why it was quiet: *0 decisions · 2 not asked: INFY.NS, HDFCBANK.NS*.
- Decisions lead with the holding's **name**; a fund shows its scheme name, not `0P0000XV5C.BO`.
- Blocked proposals no longer sit in the decisions box — that box is for decisions. The refusal
  stays on the instrument's record and in the audit.
- The drawer is one sheet: a verdict card, then the exchange. It no longer repeats the row.

### Behaviour removed

- **The silence interval is gone.** A call happens when the price moved or the headlines changed,
  and for no other reason. `max_silence_secs` no longer exists.

### Data

- Three indexes that no query could use are dropped; `runs_running` is a partial index (running
  rows only) so the audit page's count stays at 0.011 ms with 36,000 runs behind it.
- NSE's gainers payload mixes `22.86` with `"22.86 Cr"`. One formatted value used to reject the
  whole list; every numeric field now reads numbers, numeric strings, percentages and Indian
  magnitudes.
- Append-only `cash_ledger` and `api_calls` are unchanged. A corrected fill still posts an
  `adjust` entry rather than rewriting history.

### Known

- `cargo audit` reports RUSTSEC-2026-0459 against the SQLite bundled by `libsqlite3-sys` 0.37.0,
  which sqlx 0.9 caps below the patched 0.39. It is acknowledged in `.cargo/audit.toml` with the reason — the exploit
  needs a database or SQL the app did not write, and this app opens its own file and binds every
  parameter — and the ignore is to be deleted the day sqlx allows `libsqlite3-sys` 0.39.

### Front end

- Three layers: `pages/` renders, `components/` holds the atoms (one stamp, one pill, one date
  helper), `wiring/` binds events per feature. `main.ts` is a router and a timer.
- The app shell revalidates (`no-cache`), so a rebuilt front end no longer needs a hard refresh.
- Free to deploy sits with the figures; the decisions list scrolls inside its card.
