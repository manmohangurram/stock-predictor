# How it decides

The pipeline, the two pots, and why the two books are separate even for the same symbol.

- **Tracks a real portfolio** across stocks, mutual funds and IPO applications, with two
  independent pots (long-term and trading) that never draw on each other.
- **Keeps a permanent record**: every buy and sell is a row in `trades`, with the market close on
  the day of the fill, and realised profit/loss computed at sale time. Older fills can be
  backdated.
- **Suggests decisions with an exit plan.** A buy must carry both a stop and a take-profit, or it
  is discarded. Every decision shows money at risk, risk per unit and reward:risk, computed from
  the validated levels rather than reported by the model.
- **Respects your budget.** Spend is capped by the ledger; a signal that would overspend is
  recorded as `BLOCKED` with the shortfall and never pushes a notification.
- **Knows its own limits.** Each pot has five numbers — minimum and maximum profit, minimum and
  maximum per trade, and a cap on exposure to any single instrument. They are passed into the
  prompt and enforced in the validator, so neither the rules nor the model can exceed them.
- **Explains silence.** Each instrument page shows which rule blocked a trade and with what
  numbers, so an empty ledger is never a mystery.
- **Pushes to your phone** through ntfy — configured in Settings, testable from the same page,
  one notification per actionable change and nothing else.
- **Costs little.** The model is only called when the price moved ₹2, when a new headline
  arrived, or once an hour; the static prompt block is a cacheable prefix.

Two books, one process. The difference is *capital and rules*, not a different engine.

| | Long-term | Short-term |
|---|---|---|
| Capital | Long-term pot | Its own pot; never draws on the other, and the other never draws on it |
| Limits | Its own five numbers | Its own five numbers, usually tighter |
| Holdings | Separate positions, even in the same symbol | Separate positions, even in the same symbol |
| Mutual funds | Yes — bought at NAV and held | **No**: no intraday exit exists, so the API and the page refuse it |
| History | Own fills and realised P&L | Own fills and realised P&L |
| Polling | Same poll, same cadence | Same poll, same cadence |

**How the server behaves differently for them:** the poll tick walks the watchlist once and
evaluates each instrument against the pot it belongs to, using that pot's limits — so a switch to
the Short-term tab changes which book you see, not how often anything is fetched. Cash, exposure and
the per-trade caps are computed per pot, which is why a swing trade cannot block a long-term buy.
Because both live in one process, one instrument can appear in both books with two independent
positions (the screenshot above shows TCS in the trading book while the long-term book holds it
separately).

**What is *not* different yet, plainly:** there is no separate trading strategy and no candidate
discovery. Both pots run the same rules (trend, RSI, ATR) and the same prompt, with the trading
pot distinguished by its capital, its limit set and its own books. A momentum scanner and a
swing-specific prompt are the missing half of the short-term mode, and they are not built.

---

Back to the [README](../README.md) · configuration in [configuration.md](configuration.md)
