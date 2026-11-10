## v0.24.0

### A price in prose is not a price

A real prompt came back with a `pass` on CHOLAFIN, and it was right: the candidate line said
*"Up 3.82% to ₹1604.2"* while our own tape said **₹1790.80** on 09 Nov. That ₹1604.20 was our own
close from **29 October**, carried in the `why` line written by the *previous* scan's model from a
web snippet — and we pasted it into the next call as a fact. Two prices from two worlds; a
contradiction, and a contradiction is a `pass`.

- The scan's line is now labelled for what it is: **"scan's note, unverified — do not take a price
  from it"**, and the prompt says the tape is our own data and the only record of what the stock
  traded at. An idea that only works at a feed's number is a `pass`.
- **Feed versus tape is compared here, not by the model**: more than 5% apart and the line says so —
  *"the feed says 1604.20 while our last close was 1790.80 (−10.4%) — one of the two is stale; trust
  the tape"*.
- A candidate with no exchange price says **"no exchange price for it, so the tape below is the
  price"**, instead of "price read from the tape", which sounded like a measurement.

### Three days is news, older is background

The Search API has **no date filter** — `query`, `count`, `ori_question`, `localization`,
`need_content`, and nothing else. Each result does carry a `date`, so the window is ours:

- A hit older than **three days** is dropped, and so is an **undated** one — a quote page or an
  `/api/xbrl` filings page has no date because it is not an article, and a blank date reads as
  "current". In the CHOLAFIN prompt, four of eight results died to that rule.
- The same window applies to the daily trending scan, so a March earnings transcript cannot land on
  today's list.
- A result with **no title** is dropped for the same reason: it renders as a bare URL.
- When the search finds things but none inside the window, the prompt says *"nothing dated in the
  last three days — treat the move as having no known catalyst"*, and the run notes record how many
  hits were dropped.

## v0.23.0

### The audit page reads straight

- **A long answer is no longer cut off.** The drawer lives inside a blotter table cell, and the
  cell's `white-space: nowrap` was inherited by everything inside it — so a paragraph of reasoning
  laid itself out as one endless line, ran off the row, and looked truncated. The drawer is prose
  again; measured, not eyeballed: no horizontal overflow, prose `scrollWidth === clientWidth`, and
  the verdict card fills its column.
- **Copy, on both sides.** Every raw block carries a copy button in its top-right corner that copies
  the stored JSON **byte for byte** (the full text sits beside the truncated `<pre>`), with the
  count confirmed on the button and a 40px target on a phone.
- **A search call is not a failed chat.** It has its own view: the query, how many results were
  asked for, how many came back, and the ranked list with title, source, date and snippet — in a
  **fixed-height, scrollable panel**, so the sheet does not jump between one hit and five.
- **Prompt type says what the call was.** It was the first word of `purpose`, which for a search is
  the query — so `"MARUTI news today"` printed **MARUTI** as a prompt type and pushed "news today"
  into the holding column. Tool calls now read **Web search** / **Web fetch**; the holding column
  names the instrument, or the query when the name is not in the book.
- Search rows **name their holding**: a search is recorded as `search SYMBOL`, which the audit's
  purpose-join already reads.

### Less prompt, more signal

- What the model reads from a web search is now **title · day · source · snippet** — no URL it
  cannot open, no 700-word article dump, and **5** results instead of 8. A dated reason is the
  point; the rest was tokens.

## v0.22.0

### Notifications: off is off, and a push is provable

- **No ntfy topic used to mean a broken push, not a silent one.** The notifier was built with an
  empty topic and pointed at the service root, so every decision that produced a signal failed its
  push — and, worse, reported the failure against the decision. A notifier with no topic now reports
  `configured() == false`; nothing is sent, no `notifications` row is written, and the run says
  *"recorded without a push"* instead of silently doing nothing.
- A push that **fails** is still an error in the run report, and still retried on the next tick,
  because the dedupe row is only written after a successful send.
- A **live test** now exists for the question "is a notification actually arriving":
  `cargo test --test notify live -- --ignored`. It pushes to ntfy.sh, then reads the topic back —
  an accepted request is not proof the service kept it. Ignored by default, so no test hits the
  network.

### The short-term list is asked once a day, from three places

- **The exchange's gainers** and **the Nifty 50's own top movers** — both from one NSE request
  (`live-analysis-variations` carries an `allSec` group and a `NIFTY` group; the obvious
  `equity-stockIndices` endpoint refuses non-browser clients).
- **What the web says is moving** — one `/v1/tools/search` query, read by a new `trending_v1`
  prompt that answers with bare NSE tickers and one line of why. The search and fetch APIs were
  already written and audited in `src/llm/deepseek.rs`; nothing called them until now.
- **Deduplicated by symbol**, the exchange's row winning because it carries a price, and stored in
  `trend_candidates` (migration `0019`) for the IST day. A tick reads the list instead of buying it
  again; *Run again* clears it (`trend::forget_today`).
- **Each name is analysed with today's web**: one search per candidate, numbered results passed as
  `{{web}}`, cites still indices so the validator and the audit drawer keep working.
- The short-term prompt is now explicitly Indian: NSE/BSE delivery, **T+1 so the earliest exit is
  tomorrow**, 2/5/10/20% circuit bands and an upper circuit you cannot buy, T2T/ASM/GSM and
  F&O-ban names, no shorting, round-trip charges, IST session, `valid_until` in `+05:30`.
- **The short-term gate finally bites.** It was documented as "only when the price moved or the
  headlines changed" but never wrote its memo, so every tick asked about every name. It records
  now, success or failure.

### The audit drawer reads like a document

- The prompt is **rendered markdown** — headings, numbered rules, emphasis, code — instead of a
  `<pre>` dump, and it **scrolls in its own column** so the drawer is the height of the answer, not
  the question.
- The **duplicate JSON under the verdict is gone**: the card is the answer, and the raw bodies stay
  one click away under *Raw request and response*.

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
