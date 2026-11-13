# Ask the plan when a holding leaves its band, and trail the stop

Status: agreed in conversation, 2026-11-13. Replaces the first draft in this file.

---

## Intent

The per-stock call is asked on a **₹2** move. That number is blind twice over: absolute, so it is
0.26% on a ₹780 stock and 15% on a ₹13 one; and ignorant of the position, so a holding one tick
from its stop is asked about only if it also wobbled ₹2.

What the owner actually wants to know is not "did it move". It is *"am I still going to make the
profit I asked for"* — and when the answer is no, the **plan** is the prompt that can act, because
only the plan sees the whole book and can say "sell this, hold the cash".

Two things fall out, and they are different mechanisms:

1. a **trigger** — when to spend a call;
2. a **trailing stop** — how not to give a profit back. Not a trigger at all: a rule.

## 1. The trailing stop (no model, no tokens)

For every open long-term holding with a stop, on every poll:

```
next = max(current_stop, avg_cost, last_close - 2 × ATR14)
```

Raise only, never lower. Once the price is above what was paid, the stop moves to at least
break-even; from there it follows the price down at two ATRs' distance. A stop the owner set
higher is never pulled back down.

This exists because the plan format has **no "raise the stop" move** — a stop only reaches the book
through a BUY leg, so a trailing stop cannot be something the model proposes without inventing a
new move type. A deterministic rule cannot disagree with itself, costs nothing, and is testable.

## 2. The trigger

For every open long-term holding, on every poll:

| Condition | Fire? |
|---|---|
| `last_close >= avg_cost × (1 + max_profit_pct/100)` | **yes** — the band's ceiling is reached |
| `last_close <= recorded_stop` | **yes** — the thesis is wrong |
| otherwise | no call at all |

`max_profit_pct` is the pot's global limit. The per-instrument `target_profit_pct` is a **floor** to
reach, not a reason to buy more: at that line the app already says *take the profit*, and buying
more there would be the opposite instruction from the same observation. **Adding to a winner stays
in the short-term pot**, where the prompt already allows it.

When the trigger fires:

1. **Search the news again**, for that symbol, now — not the morning's copy.
2. Ask the **stock plan** with those results, replacing the stored headlines.
3. Record `stock_plan_trigger = "<ist-day>:<reason>"` so the same reason on the same day fires
   once, not ninety-six times.

## What does not change

- The morning news pass stays: search and store once a day, per holding.
- The per-stock `decide` call keeps its gate for **names not held** — that is how a candidate is
  considered at all.
- `stock_plan_last`, the trade book's slot logic and the fund plan are untouched.

## Anti-goals

- No new move type, no new validator rule, no change to the band.
- Not "buy more" in the long book.
- Not a percentage-fall-from-high trigger: it fires after the profit is already gone, and a flat
  percentage means nothing across stocks with different volatility. The stop already answers it.
