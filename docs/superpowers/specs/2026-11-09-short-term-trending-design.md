# Short-term mode: NSE gainers, daily, and a decision per name

Status: agreed in conversation, 2026-11-09. Supersedes the "no trading strategy yet" limitation.

---

## Intent

The trading pot becomes **Short-term** and stops waiting for a candidate to walk in. Every day it
asks the market what actually moved — NSE's own list of the day's top gainers across all securities
— takes the **top ten**, pulls each one's price history and news, and asks the model for a short-term
call: buy, sell, or pass, with an entry, a stop, a target and a horizon between **one hour and one
week**.

The money still decides. A proposal that does not fit the pot's allowance is not silently shrunk — it
is rejected, or answered with a rotation when something in the book is worth selling.

## Decisions

- **Source: NSE only, no fallback.** `GET https://www.nseindia.com/api/live-analysis-variations?index=gainers`,
  group `allSec`, top 10 by `perChange`. Verified reachable without cookies: the payload carries
  `symbol`, `series`, `open/high/low`, `ltp`, `prev_price`, `net_price`, `trade_quantity`,
  `turnover`, `perChange` and a `timestamp`. If NSE refuses, the pass records nothing and says so —
  an empty day is honest, a stale list presented as today's is not.
- **Every tick, gated per symbol.** Gainers turn over within the session, so the pass runs on each
  poll tick *during the session* — a 10-minute cadence for the short-term side — and each name is
  asked about only if **its own** fingerprint changed: price moved since the last call for that
  symbol, or its headlines changed. This is the same `memo::should_call` gate the long-term path
  uses, keyed by instrument, so a flat name costs nothing however often the pass runs. The daily
  call cap still applies across both pots.
- **Discovery does not start polling.** A trending symbol is created as an **inactive** instrument
  (`active = 0`) with a pending decision. It costs no polling and no calls until you approve it;
  approval activates it and records the fill.
- **One decision per name.** Horizon and expiry are explicit: `valid_until` is at most seven days
  out, and the prompt must state the horizon in hours. A short-term position that reaches its expiry
  without hitting stop or target is proposed for exit rather than left to drift.
- **Short-term is a label change.** The database keeps `trade`; every screen says **Short-term**.
  The rules, limits and books are unchanged.

## The pass

1. Fetch gainers; take the top ten by `perChange`, skipping any symbol already held in either pot.
2. For each: create the instrument inactive if new, fetch daily bars (a year is plenty) and news.
3. For each name that passes the gate, ask the model — one call — with: the gainer line, its recent
   tape, its headlines, the pot's cash and limits, and the horizon rule. A name whose price has not
   moved and whose headlines are unchanged is skipped without a call.
4. Parse and validate like any other decision — cash, per-trade caps, one-instrument exposure, stop
   on the correct side, target above the minimum profit for the horizon.
5. Store a `pending` decision with `kind = 'trend'`, symbol, entry, stop, target, horizon and
   `valid_until`. Publish one notification summarising how many were found, not one per name.

## Prompt

`prompts/trend_v1.md`: momentum on the day, but the question is whether the move continues for hours
or days — not whether the company is good. Required output: `BUY` with entry, stop, target, horizon
hours and a one-line reason citing headlines, or `PASS` with a reason. No target beyond a week.

## Tests

- Parser: the NSE payload with a fixture; a malformed payload is an error, not an empty list.
- Pass: with a fixture of ten gainers, exactly ten pending decisions appear, each with an inactive
  instrument, `valid_until ≤ now + 7 days`, and `kind = 'trend'`.
- Gating: a symbol whose price and headlines are unchanged is not called on the next tick; one
  whose price moved is. A second tick within the same minute adds no duplicate decision.
- Approving one activates its instrument and records the fill; rejecting leaves it inactive.
- No network in any test.

## Left to the builder

- Whether the notification lists the names or just the count.
- Whether a symbol that appears three days running keeps being re-proposed (I would keep a
  one-decision-per-symbol-per-day rule, so it can be re-proposed tomorrow).
