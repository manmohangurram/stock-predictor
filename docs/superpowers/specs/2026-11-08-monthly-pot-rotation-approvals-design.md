# Monthly pot, portfolio rotation, and decision approvals

Status: proposed, 2026-11-08. Supersedes the "budget" section of
`2026-10-27-stock-predicter-design.md` where it disagrees.

Companions: `.interface-design/system.md` (paper/ink, binding for the UI),
`docs/configuration.md`, `docs/how-it-works.md`.

---

## Intent and the why

The app currently has one number — a total pot — and a decision that is either shown or not. Two
things about that are wrong for how the money actually works.

**The pot is monthly.** Money arrives once a month and does not roll over. A total ceiling says
"you may hold ₹5,00,000 of stock", which is not the question; the question is "how much new money do
I have to deploy this month, and what is left of it". Unspent allowance is *not* saved: it is gone at
month end, and that has to be recorded honestly rather than looking like a trading loss.

**A decision is a proposal, not an event.** Right now a signal appears in the ledger and sits there.
In reality you look at it, maybe place the order at your broker at a slightly different price, and
record what actually happened. So a decision needs a life: `pending` (under review), `approved`
(with the fill you actually got, which may differ in quantity and price), `rejected` (with a reason).
The ledger should show what needs your attention; everything else should get out of the way.

And one more thing the model should be able to say: *"you are out of cash, but HDFC Bank is up 42% —
sell it and buy this instead."* That is a rotation: a sell that funds a buy, as one decision.

## Goals

1. A monthly allowance per pot, reset on the 1st, unspent expiring as its own ledger kind.
2. `pending` / `approved` / `rejected` on every decision, with the ledger showing only `pending`.
3. Approving opens a fill form pre-filled from the decision — **quantity and price editable** — and
   posts the ledger entry and the status together.
4. Rotation: one decision with two legs (sell to fund, then buy), approved or rejected as a unit.
5. Either pot can be deleted cleanly: an instrument with no fills and no position is removed; one
   with history is archived instead, and the app says which it did.
6. No model call when neither the price nor the headline set has changed.

## Non-goals

- No broker integration, no order placement, no live execution. "Approve" records what you say you did.
- No multi-currency allowance, no per-category budgets, no forecasts of next month's money.
- No automated selling. A rotation is a suggestion to you, never an action by the app.
- No change to how the model is prompted beyond the gate and the rotation input.

## Anti-goals

- A monthly reset that deletes or rewrites ledger rows. `cash_ledger` stays append-only.
- Expiring cash recorded as a trading loss, or as a "withdrawal" that pollutes realised P&L.
- An approval that can half-apply: a rotation never posts its sell without its buy.
- A "delete" that silently removes a symbol you hold, or leaves orphaned bars.

## Constraints

- `cash_ledger`, `api_calls` are append-only: inserts only, never `UPDATE`/`DELETE`.
- Two independent pots (`long`, `trade`) with separate capital, limits and books.
- Mutual funds are long-term only and have no intraday exit (no stop/target).
- Every number on screen comes from the server; the UI never computes a balance.
- Migrations continue from `0009`; SQLite, sqlx, no new dependency expected.

---

## Decisions

### 1. The monthly pot

`cash_ledger.kind` gains `expiry`. The month is the IST calendar month, the same clock the market
window uses.

| When | Entry |
|---|---|
| 1st, first tick after midnight IST | `budget +B` — reference `allowance 2026-11` |
| 1st, before that entry, if cash > 0 (previous month's tail) | `expiry −cash` — reference `unspent Nov allowance expired` |
| during the month | `buy −cost`, `sell +proceeds` (unchanged) |

The order matters: expire the old balance, then add the new allowance, in one transaction per pot.

**Idempotency.** A `settings` row per pot, `pot_month_long` / `pot_month_trade`, holds `YYYY-MM`. The
reset runs when it differs from the current month, and writes the new value in the same transaction.
No cron, no scheduler: the poll tick already runs every 15 minutes and outside hours too.

**Allowance, not ceiling.** `budget` keeps meaning "the monthly allowance"; the value is B. Existing
databases keep their history — the derived cash at the start of the current month is whatever the
ledger says, and the first reset after upgrade writes an expiry for the remainder.

**Realised P&L is untouched.** Expiry is its own kind, excluded from realised P&L, and shown
separately as "expired allowance" so a month with unspent cash does not read as a bad month.

### 2. Decision status

`signals` gains:

| Column | Type | Meaning |
|---|---|---|
| `status` | `TEXT NOT NULL DEFAULT 'pending'` | `pending` \| `approved` \| `rejected` |
| `status_at` | `TEXT` | when it left `pending` |
| `status_reason` | `TEXT` | optional note, required for `rejected`? (see below) |
| `filled_qty`, `filled_price` | `REAL` | what you actually got, when approved |

Existing rows become `pending`, which is the honest default: they were never acted on.

**Ledger shows `pending` only.** Approved and rejected decisions are reachable from a filter on the
ledger (`?decisions=approved|rejected|all`) and from the instrument page, which lists all of them with
their status. Audit shows the status against each call.

**Approval is a fill.** Approving opens a form pre-filled with the decision's `qty` and `entry`, both
editable, plus the date. Submitting:
1. validates the fill exactly as `trades::record` does (cash, holdings, limits, pot rules),
2. writes the trade and the ledger entry,
3. sets `status = 'approved'`, `status_at`, `filled_qty`, `filled_price`.

All in one transaction. If the fill fails validation the decision stays `pending` and the error is
shown on the form — never a half-approval.

**Rejection** takes an optional reason, sets `status = 'rejected'`, writes nothing to the ledger.

### 3. Rotation — one decision, two legs

When the model proposes a rotation it returns the sell *and* the buy. Stored as:

- the primary row in `signals` (the SELL, with its qty, price, stop/target as usual),
- a second row with `pair_id` pointing at the primary and `action = 'BUY'`, the replacement,
- `kind = 'rotation'` on both.

Approval posts both legs in one transaction, in order sell → buy, and fails as a unit. Rejection
rejects both.

**When the model is asked for one.** Need-driven, not on a cadence: when a BUY candidate exists, cash
does not cover it, and at least one held position is in profit above the pot's minimum. The prompt
receives a compact table — symbol, qty, avg cost, LTP, P&L % , stop, target, last action — plus cash,
and is asked for a swap or `PASS`. If nothing is sellable it must `PASS`; a proposal naming a loss-maker
is validated away (`rotation_must_sell_a_profit`).

The validator additionally enforces: `sell_proceeds + cash ≥ buy_cost`, the buy respects the pot's
limits and the one-instrument cap, the sell quantity is held, and a fund is never proposed for the
trading pot.

### 4. Deleting an instrument

Two outcomes, one endpoint, decided by history:

- **No fills and no open position** → hard delete: the instrument row and its bars, news, memos and
  pending signals go. (FKs: bars, news, memos, signals, trades, positions.)
- **Any fill or position exists** → **archive**: `active = 0`, removed from polling and from the
  ledger, history intact, and the response says `archived` rather than `deleted` so the UI can say
  "hidden, not deleted — it has history".

A symbol with an open position is refused outright until it is sold: `cannot delete INFY.NS while 24
units are held`.

### 5. The call gate

`memo::should_call` today fires on a price move **or** `max_interval_secs` (default 3600). The forced
call is removed: **both** a price move ≥ `min_price_move` **and** an unchanged headline set mean no
call, however long it has been.

Kept:
- a new headline (any change in the set) always earns a call;
- a stop breached or a target reached is a rules decision and needs no model call at all;
- `LLM_DAILY_CAP` remains the ceiling.

The `max_silence_secs` setting becomes "reconsider a held position after this long **even without a
price move**" and defaults to `0` (off). The Cadence tab explains the trade-off in one line.

### 6. UI shape

- **Ledger**: the decisions area shows pending only, with a filter (`Pending`, `Approved`, `Rejected`,
  `All`) that is a URL parameter so it survives a refresh. Each pending row has `Approve` and `Reject`.
- **Approve** opens an inline form under the row: quantity, price, date, `Post the fill`, `Cancel`.
  Pre-filled, editable, validated server-side.
- **Reject** asks for a reason in the same inline area.
- **Instrument page**: every decision with its status; a rotation shows both legs in one expanded row
  (`sell 24 INFY → buy 8 HDFCBANK`), approved together or not at all.
- **Audit**: a status column and a filter, so "which of these did I act on" is answerable.
- **Settings → Money**: "Monthly allowance" replaces "Long-term pot", with the reset day and what
  happens to the remainder stated in one line under each field.

## Migration

`0010_monthly_pot_and_status.sql`:
- `ALTER TABLE signals ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'` (+ `status_at`,
  `status_reason`, `filled_qty`, `filled_price`, `pair_id`, `kind`).
- `ALTER TABLE cash_ledger` — the `kind` column already exists as TEXT; no schema change, only a new
  value. A test asserts `expiry` is accepted.
- No data rewrite. Existing budgets stay as history; the first reset after upgrade records the tail.

## Testing

- Monthly reset: expiry then allowance, once per month, idempotent across two ticks; a month with no
  cash writes no expiry; realised P&L unaffected by expiry.
- Approve: pre-filled qty/price edited → fill posted with the edited numbers; a rejected fill leaves
  status `pending`; approval of a rotation posts both legs or neither.
- Status: ledger lists pending only; `?decisions=approved` lists approved; audit carries status.
- Delete: clean instrument deleted with its bars; held instrument refused; instrument with history
  archived and still readable.
- Gate: unchanged price and unchanged headlines = no call, at any interval; a new headline = a call;
  a target reached = a SELL decision with no call.
- UI: the filter is a URL parameter; the approve form renders pre-filled; both legs appear in one row.

## Left to the builder

- Whether `kind` and `pair_id` live on `signals` or a separate `signal_legs` table. One table with a
  nullable `pair_id` is fewer moving parts; a legs table is cleaner if rotations ever get more than two
  legs.
- The exact wording of the rotation prompt and the size of the holdings table it receives.
- Whether the ledger's decision list paginates once approved/rejected accumulate.
- The visual treatment of an approved row (a stamp, a strike-through, or simply hidden by the filter).
