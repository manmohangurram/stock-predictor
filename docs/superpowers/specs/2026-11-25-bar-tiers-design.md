# Bars in tiers: one table, four resolutions, nothing kept twice

Status: agreed in conversation, 2026-11-25. Companion plan: `docs/superpowers/plans/2026-11-25-bar-tiers.md`.

---

## Intent

`bars` is the only table that grows without a horizon, and the poll writes far more of it than
anything reads.

Measured today:

- **Stored per instrument:** up to 400 daily closes, plus 5 days of 15-minute bars — and those 5
  days are re-fetched and re-written **on every poll** (`Intraday(symbol, "15m", "5d")`,
  `engine.go:860`). About **525 rows**, of which the useful ones are the newest 25.
- **Read by the model:** the per-stock decision reads **16 tape lines** (4 hours of 15-minute
  bars), **10 daily closes**, and **six indicator numbers**. The plan prompts read no series at
  all.
- **Needed to compute those six numbers:** 200 daily closes (SMA200), 252 (52-week high/low),
  50 (SMA50), 14 (RSI14, ATR14).

So the dense daily history exists to *calculate* six numbers, and the dense intraday history mostly
never gets read. The owner wants fewer rows in the database **and** fewer points on the chart, with
the indicators unchanged.

## What is stored

One table, `bars`, with a `resolution` column: `15m`, `60m`, `daily`, `bucket`. There is one shape
per period — never a dense copy beside a bucketed one.

| tier | rows | what |
|---|---|---|
| today | 25 | 15-minute bars, 09:15–15:30 IST |
| last week | 31 | 60-minute bars, five sessions (Yahoo has no 2-hour interval) |
| last ~3 months | 70 | daily closes, exact |
| months 3–12 | 20 | **buckets of 13 sessions**: mean close, highest high, lowest low, session count |

**146 rows per instrument** instead of ~525. A bucket row stores the session count so a weighted
mean is exact, not an average of averages.

**Why 70 and 13.** Both indicator windows must land exactly on a bucket boundary, or the 200-day
mean includes part of a bucket it cannot see inside: 200 − 70 = 130 = 10 × 13, and 252 − 70 = 182 =
14 × 13. Thirteen is the largest bucket that divides the 52 sessions between the two windows, and
thirteen sessions is about eighteen calendar days — near enough to the fifteen-day chart interval
this was asked for.

## Indicators from tiers

- **SMA50, RSI14, ATR14** — computed from the newest daily rows only (they need 50 and 14 individual
  closes; buckets would destroy them).
- **SMA200** — walk backwards from the newest session through daily rows, then buckets, until 200
  sessions are covered; the mean is weighted by each bucket's session count, so it equals the mean
  of the dense series.
- **High52 / Low52** — the same walk over 252 sessions; the bucket's high and low are the extremes
  of its sessions, so the maximum of bucket highs is the true high.

The acceptance test is equality: for a series that has both shapes, the tiered computation must
produce the same six numbers as `ComputeIndicators` on the dense series (float tolerance).

## What is fetched

| window | query | cadence |
|---|---|---|
| the session | `interval=15m`, `period1` = today's 09:15 IST, `period2` = now | every poll |
| the week | `interval=60m`, last 5 days | once per IST day |
| the year | `interval=1d`, last 400 days | once per IST day (already cached per symbol per IST day) |

Yahoo accepts `period1`/`period2` and the client already sends them (`chartQuery`); only the span
is a shorthand today.

## What is kept

| rows | keep | why |
|---|---|---|
| 15m, 60m | 10 days | the tape window is 4 hours; ten days covers a long weekend and a holiday week |
| daily | 70 | SMA50, RSI14, ATR14 need the individual closes |
| bucket | 20 | 260 sessions, so SMA200 and the 52-week range survive |

`store.PruneBars` runs once a day beside `PruneCalls`.

## Migration: one shape, in place

The conversion ships **with the reader that understands buckets**, never before it — with only 60
daily rows and no buckets every long-term decision would lose its 200-day average.

For each instrument, once:

1. keep the newest 70 daily rows as they are;
2. collapse the rest of the daily series into 13-session buckets (`mean`, `max(high)`, `min(low)`,
   `count`), up to 20 buckets;
3. delete the collapsed daily rows;
4. keep only the newest 10 days of intraday rows.

After the migration there is exactly one representation of each period. A second run finds nothing
to do.

## Chart

The existing chart reads one series; it now reads the newest rows of the resolution that fits the
range — 15m for a day, 60m for a week, daily for a month and three months, buckets for six months
and a year — plus a custom range that picks a resolution of its own. No new storage.

## Failure behaviour

- **Yahoo refuses (429):** the stored series is used, exactly as today (`"daily series refused;
  using the stored one"`). Buckets make that fallback cover a year in 20 rows.
- **Fewer than 200 sessions after a migration:** the long-term decision reports insufficient
  history and does not ask. It resumes on the next successful daily fetch.
- **No bucket rows** (a fresh database): the reader behaves exactly as it does today, over daily
  rows.

## Non-goals

- No second data provider, no re-fetch on every render, no per-second data.
- No change to which prompt receives which series: the decision still sees 16 tape lines, 10 closes
  and 6 numbers; the plans still see no series.
- Not in this spec: a notification per decision, the decision quick-view page, and its chart
  ranges. Those are separate work items.

## Invariants this must not break

- Every id stays a UUIDv7 in text; the schema change adds a column, nothing else.
- No test may touch the network: fixtures and the fake transport only.
- `api_calls` and `cash_ledger` stay append-only.
- The money paths are untouched by this spec.
