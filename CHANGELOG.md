## v0.28.0

Funds became first-class: a NAV chart, a fill that asks for rupees instead of units, and a plan that
can actually be approved. The audit and the headlines took on a three-month horizon instead of
growing for ever. And a long-term holding now defends itself — its stop follows the price up — while
the plan is asked when it runs past the pot's ceiling.


### A fund plan could not be approved

`approve_plan` checked the balance against `budget::available_in`, which reads the pot's **main**
purse. A fund plan spends the fund purse, so on a book whose stock purse was empty the approval
was refused with *"cash is 0.00"* however much the fund purse held. It now reads the purse that
belongs to the plan's own asset class.

Found while making the fill form honest: a fund is bought with rupees at a NAV, so the form now
asks for **Amount ₹** and **NAV** and divides, instead of asking for units the model never named.

### A fund plan was filed under Stocks

The decisions box tagged every plan as a stock plan, so a `plan_mf_v1` result sat under the
**Stocks** heading. Plans now carry their own scope, and the fund heading reads *Funds* when a
plan and a scan row share it.

### The commentary is gone

255 lines of explanatory comments across `src/` and `web/src/`, reduced to the one-line whys the
rule allows. `///` rustdoc and `//!` module docs stay: those are published documentation, not
commentary beside code.

### A fund decision asks for rupees too

The fund plan's fill was fixed to ask Amount and NAV; the fund *scan* row still asked for a
quantity the scan never proposed (`0 units`) and left you to work out the units yourself. It asks
Amount ₹ and NAV now, prefills the NAV it knows, and says *size on approval*. The units are
divided on submit.

### News is English and from today only

Every search that feeds a prompt now carries `lang:en` and an `after:` bound of yesterday, and what
comes back is filtered to articles dated **today** — an undated page is not an article, and a
week-old one is not news. Applied to the morning pass and to the ceiling trigger's own search.

### The ceiling trigger searches before it asks

It was passing four numbers and the morning's headlines. It now runs its own dated search for that
holding and hands the fresh hits to the plan — *"instead of the morning ones"*, which was the point.

The band decision moved into `rules::evaluate::band_reason`, a pure function with its own test, so
the edge logic is provable without driving a whole poll.

### News is a morning web search, not a per-tick feed

Every instrument's news is now one dated `/v1/tools/search` per IST day, stored as headlines
(`engine::refresh_news`, guarded by `news_search_last`), instead of a Yahoo news request on every
poll. A tick no longer fetches news at all — `gather` reads what the search stored — so a
15-minute poll costs a bar fetch, not a bar fetch plus a news fetch.

The e2e and engine fixtures had to change with it: a new headline now means a **new stored row**,
not a different reply from the fake market. Three tests encoded the old contract and were updated
rather than the code being bent to keep them.

### A winner's stop follows the price up

Every open long-term holding now trails: `stop = max(stop, cost, close − 2×ATR)`, raised on each
poll and never pulled back down. Once the price is above what you paid the stop moves to at least
break-even, so a profit cannot quietly become a loss. Deterministic — no model, no tokens — because
the plan format has no "raise the stop" move: a stop only reaches the book through a BUY.

### The plan is asked when a holding passes the ceiling

The rules already act on a breached stop or a reached target. The gap is a holding that runs **past
the pot's `max_profit_pct`** without touching its own recorded target: nothing asked. That now pulls
the plan, once per IST day per holding, with the ceiling and the stop written into its context. It
fires only when the rules found nothing this tick, so it never duplicates an exit the rules already
made.

The first draft of this fired on the stop too — the engine tests caught it doing exactly that on
positions the rules had already decided, so the trigger was narrowed to the case they miss.

### Headlines keep three months too

`news` had the same shape as the audit: `INSERT OR IGNORE` by url, never pruned. It is swept with
the same 90-day cutoff, in the same tick as `api_calls`.

Worth knowing what that sweep is *not* bounding: every active instrument's news is fetched on every
poll tick — held or merely watched, ~28 requests per name per session day at the default cadence. A
scheme-code fund is skipped entirely; a fund added by a Yahoo symbol goes through the stock path,
news included, because that is the "add the Yahoo code to get advice" case.

### Refresh reads; it does not rebuild an archived book

Refresh always fetched **and stored**, so pressing it on an archived instrument re-created the
bars archiving had deliberately cleared — the archive was undone by a button that promised to
price nothing. It now fetches and stores only when the instrument is in the book (`cached: true`);
on an archived one it reads the source and reports *archived, so nothing was stored*, leaving the
bar count at zero. Restoring puts it back in the book and caching resumes.

### The audit keeps three months

`api_calls` was append-only and never pruned — every outbound model call since the first day, with
its full request and response. It now keeps **90 days** and the poll loop sweeps the rest
(`audit::prune`); the rows newer than the cutoff are untouched.

### The fund scan stored NAV history for funds you never added

Storing a scheme's 3,400-point series was done for every curated fund that happened to be an
instrument — including the ones the scan itself had created as proposals — and again for each new
proposal. So a month of scanning left hundreds of thousands of bars for funds nobody bought.

It now stores only for a fund you **hold**. A fund you add by hand, or Refresh on its page, still
fetches its own series, so the chart arrives when there is something to chart.

### Two money writes were not transactions

`POST /api/positions` and `PATCH /api/positions/{id}` moved the units and the cash as separate
statements. A process killed between them left a holding with money that never left the purse —
against the app's own rule that every money write is one transaction. Both run in one now, and the
sale posts to the purse its own instrument class belongs to instead of always the stock purse.

### A fund added by hand gets its chart

Only the curated scan list ever fetched a NAV series, so a fund added by AMFI code had no chart
and the Refresh button asked the equity feed about a scheme code. Refresh now calls AMFI's mirror
and stores the series, and adding a fund by its AMFI code fetches it immediately. Verified live:
`120503` → 3,419 NAV points, 2013 to today.

### The market said "closed" while it was open

`priced_today` asked whether any bar was from today. Bars are **daily candles**, and a daily
candle is not published until the close — so between 09:15 and 15:30 the freshest price was
yesterday's, every weekday session read as a public holiday, and the chip said closed. It also
counted fund NAVs, which land after the close, as trading.

The clock decides now: in session is open, and "nothing has priced" only downgrades to holiday
after 09:45, when the absence is actually evidence.

### Funds get a NAV chart

The fund scan fetched a scheme's whole NAV history and then threw it away: it read the last NAV
and the trailing return and dropped the series. So a fund had no chart, ever. The series is stored
as daily bars now (every price is the NAV, volume 0), for any curated fund already in the book —
proposed today or held for a year.

The chart draws the **target NAV** — what the owner asked for on what they paid, `avg cost ×
(1 + expected %)` — as the dashed rule, and the caption carries the trailing year:

```
91 NAV points · 1y +17.9% · target NAV 59.69 · last 61.29
```

### A fund's page is a fund's page

The instrument page was built for a stock: the NAV chart was headed **Price**, the sidebar carried
*What the rules see* (RSI, SMA200, ATR — computed on a NAV series that no fund decision consults)
and *Indicators*, and the fill form asked for a **Price**. A fund now shows **NAV**, keeps the
stock rule-check and indicators off the page, and records a fill in **units at a NAV**, long-term
only.

### Every control is 40px on a phone

The audit toolbar, the settings fields and the limits table were 28–38px tall; the back link was
17px. All of them reach 40 below 900px. Every route now measures zero controls under 40 at 390.

## v0.27.11

### The add dialog put the wrong instrument in the book

Found by the flow gate. The dialog filtered foreign listings — a NYSE ADR would be priced in
dollars and read as rupees — but it did so **after** slicing to six and numbering the rows. So:

- clicking the second result added the **first**;
- if the first six hits were all foreign, the dialog said *Nothing matches that* while Indian
  listings sat just past the slice;
- the *"N more"* hint counted the foreign ones you cannot pick.

The filter now runs first, and the slice, the row numbers and the selection all index the same
list. Typing `wipro` offers WIPRO.NS and WIPRO.BO, and picking the second adds `.BO`.

## v0.27.10

### A fund shows its name, not its AMFI code

The plan card reads the reply, which names moves by symbol — so a fund appeared as `0P0000XV5C.BO`.
The call detail now carries the names of the symbols that call actually moved, and the card prints
*Axis ELSS Tax Saver*.

### The ledger's touch targets were under 40px on a phone

The browser-flow gate caught it: the mode tabs were 30px tall and the decision-filter icons 26px.
Both are 40px below 900px, and the icons are 40×40 below 720px.

### The flow gate is green

`docs/testing/flows.md` walked at 1440 and 390 against a throwaway database — nine sections,
including approve/cancel/reject/delete-fill and a settings save that survives a reload. Run log
updated. One false alarm worth recording: the instrument page's chart is an in-page scroller, not a
page overflow.

## v0.27.9

### A fund plan's rupee size was being thrown away

`engine.rs` stores a plan's move in the `qty` and `entry` columns. A fund move has neither — it is
sized in rupees and arrives at NAV — so `amount` was dropped on the way in and the card drew a line
with nothing on it. The amount now travels in the move's JSON and reaches the card:

```
FUND PLAN · 1 move
BUY  Axis ELSS Tax Saver                                    Amount 2,250.00
```

- A single fund move prints its amount once; the `Cost` row is suppressed when it would repeat the
  same figure. Two fund moves still tot up into a `Cost`.
- A fund leg with no stored size still says **size on approval** rather than printing nothing.

### Comments

Removed the commentary added across this run. What stays is a one-line why where the code cannot
show it — the session deadline's weekend rule, and why the Dockerfile carries no cache mounts.

## v0.27.8

### One plan, one design

The audit drawer had the docket; the **ledger's** decisions box — where you actually approve the
thing — still drew the old plan card. Same plan, two looks. The ledger now draws the same docket:
a line per move, then the purse, then one paragraph, then the approve and reject controls.

- A fund leg, which carries no stored quantity, says **size on approval** rather than printing a
  quantity it does not have, and no longer shows a price stop a fund cannot have.
- A sale that exactly covers the buy no longer prints **"Also from the purse 0.00"**: an exact
  cover is not a figure worth a row.

### The Archived page's empty state was 490px wide

`.blotter td` is `nowrap`, so the sentence *"Nothing archived yet…"* set that column's floor and
the table ran off a 390px page. An empty state is prose, not a figure; it wraps now.

Swept at a real 390px viewport — Holdings, Short-term, Archived, Audit, a call drawer, and both
Settings tabs: no page overflow, no element past the edge, no table scrolling sideways. Settings'
tab rail scrolls by design.

## v0.27.7

### The docket covers the two shapes it was missing

`v0.27.6` read a move as `qty × price`, which is how a **stock** move is sized. A **fund** move is
sized in rupees — `amount`, no units and no price, because units arrive at NAV — so the card drew a
blank line and a cost of zero.

- A fund move now prints `Amount 2,250.00`, and the purse totals by amount.
- A three-move stock plan was checked, not assumed: three lines, then
  `From the sale 30,700.00 · To buy 32,000.00 · Also from the purse 1,300.00`.

### Stacked, the two columns ran together

Below 900px `SENT TO THE MODEL` and `RECEIVED` stacked with nothing between them, so the heading
read as another line of the prompt above it. A hairline now separates them.

## v0.27.6

### The plan card is a docket, not a table

The moves table was six columns in a drawer: at 1,440 it was legible, at 390 it was a squeeze, and
its per-move justification — five citations and a paragraph under every leg — buried the four
numbers the reader actually has to check.

It is now one line per move plus the purse:

```
SELL  TATAMOTORS.NS      Qty 40  At 780.50        Gives 31,220.00
BUY   GABRIEL.NS         Qty 27  At 1,519.00  Stop 1,450.00  Target 1,700.00  Till 16 Nov
                         ─────────────────────────────────────────────────
                         From the sale     31,220.00
                         To buy            41,013.00
                         Also from the purse  9,793.00
```

- **The purse arithmetic is the point.** A sale that funds a buy now says what it frees, what the
  buy still needs and which way the difference runs — the same running-balance idea as the book.
  A buy-only plan prints `Cost`, a sell-only plan prints `Frees`.
- **The expiry is shown.** Every trade move carries `valid_until` and the card never printed it.
  A trade that goes stale at 15:30 has to say so.
- **The model's per-move `why` is gone from the card.** It is still in the raw exchange below it;
  the card keeps only the plan's own paragraph.

### Hotfix: the audit's Holding cell never wrapped

`v0.27.5` tightened the audit table for a phone and it still came up 16px short. The cause was not
padding: the Holding cell had no `.holding` class, so it inherited the blotter's `nowrap` and the
longest company name on the page — *Tata Consultancy Services* — set that column's floor. The ledger
has always wrapped it; the audit never got the same class.

With it, a 390px viewport measures table 358 in a 358 slot, no horizontal scroll, and the Show
control inside the page instead of 7px past it.

## v0.27.5

### The plan card's figures sit on a grid, and its reason on the Holding column

The moves table inherited the audit blotter's `white-space: nowrap`, so the sentence under each
move could not wrap: it widened the Holding column until the figures were pushed off the card and
`880.00` was clipped mid-digit.

- The reason is its own row now, starting on the Holding column's text edge — prose that begins
  under the BUY stamp reads as part of the stamp.
- The table spans the card's content box exactly: the stamp starts on the body's left margin, the
  Target figure ends on its right one, and every header shares its figure's right edge. Measured,
  not eyeballed: 850/850, 1359/1359, 904/904.
- Figures are `tabular-nums`, and never wrap — half a price on the next line is not a price.

### The drawer had no phone layout

`SENT TO THE MODEL | RECEIVED` stayed two columns at every width, so on a phone the answer was
clipped at the drawer's right edge. It stacks below 900px. The audit table itself was 28px wider
than a 358px page; cell padding and the plan card's fixed stamp column give that back, and Qty and
Stop drop on a phone so the four remaining columns fit.

## v0.27.4

### Hotfix: the build cache was making the build slower

`v0.27.3` added `--mount=type=cache` to the two cargo steps. A cache mount keeps its contents
**out of the layer**, so when the `cargo chef cook` layer hit — which is every release, since a
version bump does not change `recipe.json` — it replayed with an empty `target/`, and the build
step then re-downloaded and recompiled every dependency. Measured on the v0.27.3 run: `cook`
0.1s, `build` **114.4s**. The same pair on v0.27.1, without the mounts, was 88s and 26.5s.

The layer *is* the cache here — `cache-to: type=registry,mode=max` preserves it across builds and
a mount does not. The mounts are gone; a dependency rebuild now happens only when `Cargo.toml` or
`Cargo.lock` actually changes.

## v0.27.3

### Hotfix: a Friday trade plan was refused for naming Tuesday

A live `plan_trade_v1` call was refused whole — `plan_refused` — on a name with a clean, dated
catalyst. The expiry it named, Tuesday 17 Nov's close, is the **second** session after that Friday.
The validator added three *calendar* days to the clock instead, and three days from Friday evening
is Monday, so Tuesday read as "more than 3 sessions out".

- The deadline is now the close of the third **trading** day, weekends skipped. Exchange holidays
  are not modelled, which only ever makes it stricter, never looser.
- `plan_trade_v1` carried two expiry rules that could not both hold — *"tomorrow's close at the
  latest"* and *"never more than three sessions"* — so the model was guessing which one the app
  meant. It is one rule now, spelled with the stamp format.

### Hotfix: the audit printed `plan_refused` instead of the reason

A refused plan keeps its sentence in `reasons`, not `detail`, and `blocked_reason` only looked for
`detail` — so the call viewer printed the bare key. It reads *"BONFIG.NS: expiry is more than 3
sessions out — that is not this book's trade"* now.

### Hotfix: a refused plan said "awaiting you"

The audit row forced every call that touched a plan to `PLAN`, so a plan refused whole wore the
badge **plan · awaiting you** — the one thing it was not. It says **plan · refused**, and the
reason is behind View.

### The call viewer says what the next click does

The audit's last column was **Open**, with a button that read "Open" whether the exchange was
showing or not. It reads **Show** until you open it and **Hide** once it is open, so the control
says what it will do rather than what it did.

## v0.27.2

### Hotfix: a refused proposal was offered as a decision

The ledger's Decisions box let a refused proposal through — a call the validator turned down,
stored `BLOCKED` with a `detail` — and stamped it REFUSED. There was nothing to approve on it:
no fill to post, no money to move. It is a record of what happened, not a thing to decide.

The box is a to-do list again: only `BUY` and `SELL` proposals reach it. Blocked and refused rows
stay where the record lives — the instrument's page and the audit.

## v0.27.1

### Hotfix: a sold-out holding lingered as a zero-quantity row

A position fully sold off is closed (`qty = 0, state = 'closed'`) in the database, but the ledger
query that builds the holdings table returned every position for the mode, closed or not. The
result was a phantom row — the same stock again, with `Qty 0`, `Invested ₹0.00` and `P&L ₹0.00` —
sitting under the live holding. The instrument page already filtered to `state = 'open'`; the
ledger now does too, so a closed position leaves the book the moment it is sold.

## v0.27.0

### The short-term list has floors

The scan was offering ₹0.73–₹3.85 micro-caps pinned at their circuit band, and every plan came back
*"nothing clears costs"* — correctly, because you cannot buy a locked band or trade a stock whose
whole move is three ticks. Three floors now decide what the model is even offered, and all three are
**settings** (Settings → Cadence → *What the short-term scan may offer*):

- **Price floor ₹10** — below that a single tick is a large part of the move.
- **Turnover floor, one crore a day** — an order from this purse has to be a rounding error. Typed in
  **whole crore**; stored in rupees.
- **Locked-band cut-off 19.5%** — a stock at its ceiling has no fill at any price you name.

Same day, same exchange list: **21 candidates became 14**, and the plan that followed was the first
actionable one — sell RELIANCE (+11.6%, no stop, no catalyst), buy GABRIEL 27 @ 1619 with a stop at
1540, a target at 1790 and an expiry at tomorrow's close.

### Every prompt was tested against the live model

`plan_stock_v1`, `plan_mf_v1`, `plan_trade_v1`, `decide_v1` and `trending_v1`, on a throwaway
database, with real replies read back. No prompt carried an unfilled placeholder, nothing 4xx'd,
nothing failed to parse. Three bugs fell out:

- **A trade SELL was refused for having no expiry.** An exit has no shelf life; the expiry is a
  BUY's contract. That refusal threw away a perfectly good *"take the profit now"*.
- **A refused plan was filed in the long purse** whatever purse asked.
- **`plan_trade_v1` invited a placeholder second leg** — the model emitted `qty: 0, why:
  "placeholder"` to fill the slot and the whole plan was refused. The prompt now says one move is a
  plan, and that a leg you would not place refuses the plan whole.

Removed as unreachable: `prompts/trend_v1.md`, `prompts/short_v1.md`, `render_trend` and
`decide_template` — no `RunInput` is ever built with `Mode::Trade`.

### Web search asks about the company, not the ticker

`ANTGRAPHIC news today` was tokenised into `ant` and returned **Anthropic's IPO**. The query now
quotes the company's name (`"Antarctica" OR ANTGRAPHIC share news`), and a result reaches the prompt
only if the ticker or the first two words of the name appear in its title or snippet.

### Development: a `.env`, and never in the repository

A gitignored `.env` can carry `DEEPSEEK_API_KEY` (`OPENAI`/`GEMINI`/`CUSTOM` too) so prompts can be
exercised without a key at rest. It only fills variables that are not already exported and **never
writes a setting** — a stored key wins. `.env.example` is the template; `.env` never ships.

### Packages: arm64 only, cached in Actions, pruned weekly

Every release left nine package versions behind — one tagged index, the platform manifests, their
attestations and two registry cache images — and GHCR has no retention policy for a user-owned
package. Now: build cache in the Actions cache, **arm64 only** (the deployment is arm64; the second
architecture doubled every manifest and layer), and a **weekly prune** keeping the newest five
releases. Swept by hand: **423 versions → 27**, **94 artifacts → 0**.

## v0.26.0

The two books have different appetites now. Design: `docs/superpowers/specs/2026-11-11-two-books-design.md`.

### The long-term book is offered the market, not itself

Its "today's list" was every active instrument — the book showing itself its own holdings, which is
no choice at all. It is now the **Nifty 100, ranked by five-session return**: positive only, best
first, ten names.

- Constituents from the NSE archives CSV (that endpoint refuses a non-browser client), one Yahoo
  daily chart per symbol, once per IST day.
- Five sessions, and six closes are required to be ranked; a window containing a zero or a NaN is
  dropped rather than read as a crash.
- Positive only: a top ten from a falling market would be the names that fell least.
- Kept in `plan_candidates` for the day, so a retry costs nothing.

### The trade purse gets a book call every two hours

`plan_trade_v1` — **0–2 moves that only make sense together**, at **09:15, 11:15, 13:15 IST**. It
replaced the per-stock short-term pass, which could never say *"sell A to fund B"* because each call
knew only its own candidate.

- Delivery, T+1: nothing can be sold the session it was bought, and the first exit is the next open.
- **The stop is the loss trigger**, not the entry — a 0.2% wobble must not close a good trade.
- Every move carries an **expiry**, enforced at three sessions.
- Evidence for four candidates, not twenty: tape, stored headlines and dated web results.
- `trend::sweep_glances` removes the names a plan looked at and did not move on, so a twenty-ticker
  scan that buys one leaves one row. Migration `0021` clears the glances older builds left behind.

### An expired idea is a sell the app makes

An open trade position whose approved entry has passed its own `valid_until` gets a SELL —
recorded, pending, in the trade purse, with the reason on the record, and a push. One exit at a
time; none while one waits to be approved. The plan says how long the reason lasts; the app does
not hope it changes its mind.

### A refused proposal is no longer invisible

`Outcome::Rejected` fell back to the rules' candidate, which is usually a HOLD, and a HOLD records
nothing — so a `decide_v1` verdict could be a real buy with entry, stop and target, refused for a
concrete reason, and leave no trace outside Audit. It is now recorded as BLOCKED with the validator's
reason, shown in the ledger's Decisions box as **REFUSED**, and never pushed (there is nothing to
approve). Short-term proposals also announce themselves again: `report.ideas` was read to build the
push and never written.

### Adding an instrument makes it active

`INSERT OR IGNORE` honoured the row the scan had already created for a discovered name — which is
`active = 0` — so the record appeared to be created and then vanished from the ledger (whose
holdings query filters `i.active = 1`) while the instrument page offered **Restore** for something
never archived. Adding promotes the row; a genuinely archived name is refused with *restore it from
its own page*.

### A browser flow suite, and the bugs it found

`docs/testing/flows.md` — every page, every control, what it must do — walked with the Chrome
DevTools MCP against a throwaway database. **A release is not cut until it is green.** CI does not
run it; it needs a browser.

Walking it found: a NYSE ADR could be added and its dollars read as rupees (**+11,674%** "vs close");
the refresh button printed *undefined headlines* (client read `news`, server sends `headlines`);
Archive used a native `confirm()` unlike every other two-step control; the reject form's submit
duplicated the opener's label and could reject a decision by accident; the currency select looked
editable while locked; and a filtered audit page with no matches said *No calls yet*.

### Removed

`src/backtest.rs` (a 120-line harness with no caller), `balance-rail.ts`, `ledger-row.ts`,
`budget::available_in_on`, 43 CSS classes with no markup behind them, two tokens nothing read, and
the per-stock short-term pass with its run kind and manual path (old audit rows still read).

## v0.25.1

### Hotfix: the Archived page showed everything the scan had ever noticed

Discovery and archiving both wrote `instruments.active = 0`, so the page that lists what *you*
archived was listing **every ticker the short-term scan mentioned** — in a demo database, twenty
rows of names that were never in the book, one of which was the owner's.

- Migration `0020` adds `instruments.archived_at`: the record of a decision, null for a name the
  scan merely noticed. The page lists on that.
- The backfill keeps every instrument with **fills or decisions** — history is a person's trail — and
  drops the ones with neither.
- **Restore** clears the date, so a row is neither archived nor mistaken for discovered.
- The trailing **"open to restore"** column is gone: the whole row is the link to the instrument,
  where Restore lives, so the column repeated what the row already does.

### Hotfix: the Last fill column did not share the page's edge

Its header was left-aligned over a right-aligned value. Header and date are both right now, so
Fills, Decisions, Last fill and Realised end on the same line as every other figure on the page
(measured: header right edge 1159/1159, Realised 1393/1393). The instrument page's `vs close` column
had the same inversion and is fixed with it.

## v0.25.0

### The window goes into the search query

I had said DeepSeek's search had no date filter. It has one: the query operators are `site:`,
`-site:`, `-term`, `-"phrase"`, `"phrase"`, `lang:`, **`after:`** and **`before:`**. So the window is
now asked for, not only enforced afterwards:

```
CHOLAFIN news today after:2026-11-08 lang:en
Indian stock market biggest movers news after:2026-11-08 lang:en
```

Asking matters because `count` is five: filtering afterwards means asking for five and discarding
three, and getting two. `lang:en` also drops the non-English quote pages that were turning up in the
prompt. The `date` filter stays — the server **ranks** by recency, only our filter **guarantees** it.

### The audit page ticks

It is a live log while a run is happening and a static record otherwise, so the reader decides.

- A **tick in the filter toolbar**, on by default, reloading the list every **five seconds** — with a
  select beside it for **5 s · 15 s · 30 s · 1 min**, both remembered in `localStorage`.
- It sits with the filters it updates, not in the page head: same height, same border, aligned to the
  same baseline as the selects (measured, identical 377/404 box, flush right edge).
- The dot is the state and the words are the meaning; the checkbox behind it is a real, focusable
  input with an ink focus ring, not a decorated `<div>`.
- One timer, re-armed on every render; it never fires while a form or a select has focus, or while
  the tab is hidden. Verified: one request per 5 s, none at 15 s, none when off.

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
