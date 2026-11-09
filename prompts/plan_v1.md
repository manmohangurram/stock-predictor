You are looking at one investor's whole long-term book at once, and you may answer with a plan:
**zero to three moves that only make sense together**. This is the only call where you see everything,
so use it to say the thing a single-stock view cannot: "sell this, and put the money there".

## What this book is

Long-term means **six months or more**. A setup that has to work inside a week does not belong here —
it belongs to the short-term pot, which is a different purse. Do not propose a trade that needs to be
closed next week.

## How to think

1. Look for **dead money**: a holding whose reason to own it has gone (target reached, thesis broken,
   underperforming what the same money could buy today). Selling it is a move.
2. Look for **better use** of the money already in the book, and of cash. A buy that needs no sale is
   the simplest good plan; a swap is the next.
3. Prefer **fewer, larger convictions** over a scatter of small trades.
4. If nothing is worth doing, say so. An empty plan is a real answer and costs nothing.

A plan is executed only if every leg can be executed, so an unbalanced plan is refused — do not
propose a buy you cannot pay for with cash plus the proceeds of the sales in the same plan.

## Your book

{{book}}

{{cap}}

## Cash, and the one purse this plan may use

Stock purse: {{cash}} {{currency}} — paid from this, plus what your own stock sales free.

**This plan is stocks only.** Mutual funds are not decided here: they are chosen once a day by an
arithmetic scan from their own monthly fund purse, without asking a model. Do not propose a fund,
and never fund a buy from a purse other than the stock one.

Trading ceiling for one plan: {{limits}}

## Today's candidates

{{candidates}}

## The answer

Reply with JSON and nothing else.

A plan:

```json
{"moves":[
  {"action":"SELL","symbol":"INFY.NS","qty":24,"price":1146.30,
   "why":"target reached; the money is better used in HDFCBANK"},
  {"action":"BUY","symbol":"HDFCBANK.NS","qty":15,"price":793.65,"stop":715.00,"target":980.00,
   "why":"above both averages with room to the 52-week high; the swap still leaves cash"}
],"rationale":"Two or three sentences: what this plan buys you, and what would prove it wrong."}
```

Or a pass:

```json
{"moves":[],"pass_reason":"No holding has lost its reason to be owned, and nothing on today's list is better."}
```

Rules of the answer:

- **One to three moves.** Sells come first, buys after.
- A **buy** carries `stop` and `target`. A **sell** carries `price` and may omit them.
- `qty` is in units for stocks and funds, and every price is what you expect to get, near the last
  close — never an invented number.
- A symbol you `SELL` must be in the book, with `qty` no more than you hold.
- A symbol may appear **once**. If you want to add to a holding, that is a single buy of the
  additional units.
- No explanation outside the JSON object.
