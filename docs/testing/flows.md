# Browser flows — the release gate

Every page, every control, what it must do. Walked with the **Chrome DevTools MCP** against a
throwaway database, not with curl and not in CI: the point of these flows is what a person sees.

**A release is not cut until this list is green.** Record the run date at the bottom.

## How to run it

```sh
# 1. a throwaway database — never your real one
rm -f /tmp/flows.db*
DB_PATH=/tmp/flows.db BIND=127.0.0.1:8100 SECRET_KEY=$(printf 'a%.0s' {1..64}) \
  ./target/debug/stock-predictor --seed        # instruments only; the seed stops at "does not fit the budget"

DB_PATH=/tmp/flows.db BIND=127.0.0.1:8100 SECRET_KEY=$(printf 'a%.0s' {1..64}) \
  ./target/debug/stock-predictor &             # the server

# 2. fund the purses and give the books something to show
curl -X PUT localhost:8100/api/settings -H 'content-type: application/json' \
  -d '{"budget":200000,"trading_budget":50000}'
for s in INFY.NS TCS.NS HDFCBANK.NS; do
  curl -X POST "localhost:8100/api/instruments/$s/trades" -H 'content-type: application/json' \
    -d '{"side":"buy","qty":10,"price":100.0,"at":"2026-11-05T05:00:00Z"}'; done
curl -X POST "localhost:8100/api/instruments/RELIANCE.NS/trades" -H 'content-type: application/json' \
  -d '{"side":"buy","qty":5,"price":1200.0,"at":"2026-11-06T05:00:00Z","mode":"trade"}'
```

Then, with the MCP attached to that page:

1. `navigate_page` to the URL in the first column.
2. Wait ~900 ms (`evaluate_script` with a `setTimeout` sleep) — the page renders client-side.
3. Act on the control in the second column, assert the third.
4. **A click that is a plain `<a href>` destroys the execution context** — assert its `href`
   instead, and check the destination by navigating to it. Only JS-driven controls can be clicked
   and asserted in one call.
5. Native dialogs need a **trusted** key: `press_key` with `Escape` closes the add dialog,
   `evaluate_script` synthesising the event does not.

## 1. Rail (every page)

| Control | Must do | How to check |
|---|---|---|
| Ledger | Go to `/` | `nav.rail a[href="/"]` exists |
| Audit | Go to `/calls` | `href="/calls"` |
| Settings | Go to `/settings` | `href="/settings"` |
| Archived | Go to `/archived` | `href="/archived"` |
| Market state | Dot + words + next open | `.rail-market .lbl` non-empty; `[data-market][data-state]` is `open`/`closed`/`holiday` |

The rail is a frame: it never scrolls, and only `.main` moves.

## 2. Ledger — long-term (`/`)

| Control | Must do |
|---|---|
| Head figures | Invested, Now, Unrealised, Free to deploy — the last with *of 2,00,000* |
| Add instrument… | Opens the dialog (`#add-dialog[open]`) |
| Long-term / Short-term tabs | `href="/?mode=long"` / `/?mode=trade`; the active one has `aria-current="page"` |
| Decision filter icons | `href` carries `?mode=long&decisions=pending|approved|rejected|all` |
| Approve (✓) | Opens `.approve-form` **unhidden**, pre-filled with qty/price/date; no money moves yet |
| Approve → Post fill | Records the fill, decision leaves the pending list, position size changes |
| Cancel | The open form hides, nothing written |
| Reject (✕) | Opens `.reject-form` with a reason field |
| Reject → **Reject decision** | Decision leaves pending; no money moves |
| Stocks table | Holding, Qty, Bought, Invested, Now, Value, P&L, Signal, Exit plan — numbers right-aligned to their own header |
| Empty decisions box | *Nothing under review.* |

## 3. Ledger — short-term (`/?mode=trade`)

| Control | Must do |
|---|---|
| Title | **Short-term book**, subtitle *separate capital, separate limits* |
| Free to deploy | The **trade** purse, not the long one |
| Section | **Short-term positions** only — long holdings absent |
| Decisions | Trade-purse decisions only; an expiry SELL shows its reason |

## 4. Add instrument dialog

| Control | Must do |
|---|---|
| ⌘K / Ctrl-K, or **Add instrument…** | Dialog opens, search field focused |
| Type `wipro` | Results appear; grouped **Stocks** / **Mutual funds** |
| Result list | **Only NSE/BSE and funds** — a NYSE ADR must not appear (it would be priced in dollars and read as rupees) |
| Pick the second of two results | It adds the code you clicked, not the first — the list is filtered before it is sliced and numbered |
| Click a result | Navigates to `/i/CODE`; chart, *Record fill* form, archive button |
| Escape (`press_key`) | Closes the dialog |
| Server: `POST /api/instruments {"code":"WIT"}` | **400** *not an Indian listing — add an NSE (.NS) or BSE (.BO) symbol…* |

## 5. Fund decisions (`kind=fundscan`, `plan_mf_v1`)

| Control | Must do |
|---|---|
| Ledger decisions box | A fund plan sits under **Funds**, never **Stocks** |
| Plan docket | `Amount 2,250.00` per move, never `Qty`/`Stop`/`Target` — a fund exits at a NAV |
| Approve a fund scan row | Fields are **Amount ₹** and **NAV** (NAV prefilled), *Proposed* reads *size on approval · NAV …* |
| Post the fill | Units are `amount ÷ NAV`, floored to 3 decimals, and the money leaves the **fund** purse |
| Approve a fund plan with an empty stock purse | It posts — the check reads the fund purse, not the main one |

## 6. Instrument page (`/i/INFY.NS`)

| Control | Must do |
|---|---|
| Chart | Drawn, with the price line and the stop/target rules |
| Record fill (buy 25 @ 250) | Row appears in Fills; the position and the ledger update |
| Correct fill (pencil) | Editor opens; Save rewrites the price; *vs close* recomputes |
| Delete fill | Arms on the first click (*Delete? Click again*), deletes on the second |
| Expected profit % | Saves; the value survives a reload |
| Refresh (in the book) | Status becomes *Stored N bars and M headlines*, the button re-enables |
| Refresh (archived) | Fetches from the source but says *archived, so nothing was stored*; the bar count stays 0 — archiving cleared it and a read must not rebuild it |
| Refresh (a fund) | Goes to AMFI by scheme code, not the equity feed; a fund with only a Yahoo symbol falls back to Yahoo |
| Archive (not held) | Arms on the first click with the warning, archives on the second, returns to the ledger |
| Archive (held) | Disabled, with *cannot be archived — sell it first* |
| Decision tabs | `href` carries `?decisions=pending|approved|rejected|all` |
| Headline links | Open the publisher in a new tab |

## 7. Archived (`/archived`)

| Control | Must do |
|---|---|
| Row list | Only instruments **a person archived** — never one the scan noticed |
| Columns | Holding, Fills, Decisions, Last fill, Realised — the date right-aligned with the figures |
| Open an archived instrument | *It is archived: no prices are fetched and no decisions are made* + **Restore** |
| Restore | Buttons become **Archive** again; the instrument is back in the book |
| Empty state | *Nothing archived yet. Archive an instrument from its page…* |

## 8. Audit (`/calls`)

| Control | Must do |
|---|---|
| Schedule cards | Stock plan, Fund plan, **Trade plan** — last run, next due, cadence in words, *Run again* |
| Run again | Starts a run: that card shows *manual · running*, then the result |
| Run now → select + Run | Starts the chosen prompt; the card shows it running, then its result |
| Filters (prompt, call, model, review) | Server-side; the page navigates with the filter in the URL |
| Filter that matches nothing | *No calls match these filters — clear them.* (never "No calls yet") |
| Auto-refresh tick | Off by default only if the reader turned it off; reloads every N seconds; the choice and the interval persist |
| Interval select | 5 s / 15 s / 30 s / 1 min, remembered |
| Show / Hide | The control says what the next click does. Drawer: **Sent to the model** (markdown: headings, numbered rules, code) and **Received** (verdict card, one answer, no duplicate JSON) |
| Copy button | One per raw block, top-right; copies the stored JSON byte for byte |
| Page size | 25 / 50 / 100 per page, reflected in the URL |

## 9. Settings (`/settings/{money,risk,model,notifications,cadence,network}`)

| Control | Must do |
|---|---|
| Six tabs | Each loads its own fields; the tab label carries its state (*Model — no key*, *Notifications — off*, *Network — direct*) |
| Money | Three allowances (long, funds, short-term) with what each means and what is free this month |
| Currency | **Disabled** once the ledger has entries, with the reason underneath |
| Risk limits | Per-purse min/max profit and trade size, long and trade separately |
| Model | Provider, model, base URL, key — a stored key is never shown again |
| Notifications | ntfy URL, topic, token, and a **Test** button that reports what happened |
| Cadence | Poll seconds, off-hours seconds, prompt window, daily cap, minimum price move |
| Network | Proxy with credentials, and a **Test** button |
| Save | Writes, says *Saved*, and the value survives a reload |
| Discard | Reverts the field to the stored value without writing |

## 10. Mobile (390 × 844)

| Page | Must do |
|---|---|
| Ledger | Cards stack, no horizontal scroll, touch targets ≥ 40 px |
| Audit | Schedule cards stack; the toolbar wraps; the table scrolls inside itself |
| Archived / Settings | Readable, save reachable |

---

## Known limits (accepted, not bugs)

- **No live model in a flow run.** Without a key every model call ends in `http 401`, which is the
  expected shape: the flow proves the wiring, not the model's opinion.
- **Trade stops are advisory between plans.** A trade position whose stop is breached is closed by
  the next trade plan or the expiry sweep, not by the per-tick rules (those manage the long purse).
- **A discovered name keeps no row** unless it produced a decision, a fill or a position.
- **A web result older than three days, undated, or untitled never reaches a prompt.**
- **`window.confirm` / `alert` are not used** — two-step controls and inline status instead. If one
  appears, that is a regression.

## Run log

| Date | Build | Result | Notes |
|---|---|---|---|
| 2026-11-13 | `92aa174` + this commit | **green** | 9 sections walked at 1440 and 390. Found and fixed: the ledger's mode tabs (30px) and decision-filter icons (26px) were under the 40px touch target; the add dialog filtered foreign listings *after* slicing and indexing, so clicking the second result added the first. Confirmed: approve opens pre-filled and posts (TCS 10 → 15 units), cancel writes nothing, reject writes a reason and moves no money, delete fill arms then deletes, Save survives a reload (`poll_secs` 600). The instrument page's chart is an in-page scroller (324px view over a 560px drawing), not a page overflow. |
| 2026-11-11 | `ebd2413` + this commit | **green** | 9 pages, 60+ controls walked. Found and fixed: a US ADR could be added (nonsense `vs close`), the refresh status said *undefined headlines*, Archive used a native `confirm`, the currency select looked editable while locked, the reject form's submit duplicated the opener's label, and a filtered audit page said "No calls yet". |
