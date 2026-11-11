# Two books, two appetites — design (2026-11-11)

Agreed with the owner in conversation. Supersedes the short-term parts of
`2026-11-09-short-term-trending-design.md`.

## The split

| | Long-term (stock purse) | Short-term (trade purse) |
|---|---|---|
| Question | does this holding still earn its place | what makes money in the next few hours |
| Cadence | one plan call per IST day | one plan call every 2 hours in session |
| Candidates | **Nifty 100** ranked by 5-session return | NSE gainers + web-search trending |
| Horizon | six months or more | hours, up to three sessions |
| Exits | stop/target/thesis | **stop, target, or the idea's own `valid_until`** |

## Long-term candidates: Nifty 100, top 10

- Constituents from the NSE archives CSV (`ind_nifty100list.csv`), fetched once per IST day.
  NSE's `equity-stockIndices` endpoint refuses non-browser clients, so the index cannot be asked
  for its own performance.
- Each constituent's **5-session return** from one Yahoo daily chart call per symbol: ~100 requests
  once a day, about two seconds.
- Keep only **positive** returns, rank, take the **top 10**. A top-10 drawn from a falling market
  would otherwise be the names that fell least.
- Stored per IST day in `plan_candidates(day, symbol, ret5, close)`, so the once-a-day plan reads a
  table instead of paying for 100 quotes again on a retry.

## Short-term: one book call every two hours

- `prompts/plan_trade_v1.md` — **replaces** `trend_v1`'s per-stock calls (which stay in the file and
  in the audit's history, unresolved for new runs).
- Asked at the session slots (09:15, 11:15, 13:15 IST; marker `trade_plan_last_slot`), never outside
  09:15–15:30.
- Input: what the trade purse holds (with entry, stop, target, expiry, age in sessions), the cash,
  and today's candidates — **gainers + trending search**, with tape and dated web evidence for the
  **top 4** only. Twenty candidates' worth of evidence is prompt spent on names nobody will buy.
- Answer: **0–2 moves that only make sense together** — sell A to fund B is the shape it exists for.
  Every buy carries entry, stop, target, `valid_until` (≤ 3 sessions).
- Validation reuses the plan machinery (leg by leg, then as a whole, atomic on approval), with
  `Mode::Trade` semantics: trade purse only, stocks only, high confidence.

## Exits: the stop, the target, or the idea's expiry

- **Loss trigger is the plan's stop**, not the entry: a 0.2% wobble must not close a good trade.
- Checked on every tick, and **at the first tick of each new session** — which is the earliest
  moment a delivery position can actually be sold.
- A trade that has touched neither is sold when its own `valid_until` passes, clamped to **3
  sessions**. The model states how long the reason lasts; the app does not invent a day.

## What does not change

- Two purses, never lending (structurally, via `cash_ledger.bucket`).
- No shorting; the trade book is long only.
- Delivery, not MIS: nothing can be sold the same session it was bought, and every prompt says so.
- Approval is the recorded fill; a plan posts every leg or nothing, in one transaction.

## Staging

1. **Long-term candidates** — `plan_candidates` + Nifty 100 fetch + the plan prompt's list.
2. **Trade plan** — `plan_trade_v1`, the 2-hour slots, replacement legs, `trend_v1` off the cadence.
3. **Expiry exit** — sell on `valid_until` (3-session ceiling) alongside stop and target.

Each stage is shippable alone; stage 2 is the one that changes behaviour the most.
