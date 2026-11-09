# The model sees the book and the money, and may propose a batch of moves

Status: agreed in conversation, 2026-11-09. Extends rotation into a general plan.

---

## Intent

Two failures caused this. The model proposed **1,000 units of Infosys at ₹1,148.70 — ₹11.5 lakh against
a ₹2 lakh purse**, because the prompt never said the one sentence that would have stopped it: *you may
spend at most this much*. And every call judged **one stock in a vacuum**: the model saw the open
position for the stock under review and nothing else, so "sell Infosys, buy HDFC Bank" could only ever
arrive through the rotation prompt, which asks that narrow question after the rules layer has already
picked a candidate.

The model should decide with the whole book in front of it, and it should be allowed to say a plan out
loud: up to three moves, executed together or not at all.

## Decisions

### 1. Every prompt carries a hard spending cap

A line the model cannot misread, ahead of the soft limits:

```
You may spend at most 1245570.00 INR in this purse.
A buy costing more than that is refused, not shrunk.
```

`{{cash}}` stays (it is the number), but `decide_v1.md` and `short_v1.md` state the consequence
explicitly. The validator already refuses over-budget buys; this stops the refusal being the first the
model hears of it. Percentage limits remain as a second fence.

### 2. Every prompt carries the book, not just this holding

`{{positions}}` becomes the whole long-term book — one line per holding, the instrument under review
marked `← this one`:

```
HDFCBANK.NS HDFC Bank: 15 units at 690.10, now 793.65, +15.0%, stop 700.00, target 980.00
INFY.NS Infosys: 24 units at 1042.30, now 1146.30, +10.0%, stop 1080.00, target 1420.00  ← this one
```

Capped at the 12 largest holdings by value, then a `+N more` line, so the prompt cannot grow without
bound. Mutual funds keep their `AMFI:` lines. The short-term prompt gets the short-term book; the two
pots never see each other's money.

### 3. One plan call per poll, and it may return up to three moves

A new prompt, `prompts/plan_v1.md`, asked once per poll after the per-stock calls, when the same
`memo::should_call` gate that governs everything else has fired for at least one instrument (plus the
daily call cap). It receives: the book, cash per purse, today's candidates with their prices, the
limits, the horizon rules.

It answers with a plan or a pass:

```json
{"moves": [
  {"action": "SELL", "symbol": "INFY.NS", "qty": 24, "price": 1146.30,
   "why": "target reached, and the money is better used elsewhere"},
  {"action": "BUY", "symbol": "HDFCBANK.NS", "qty": 15, "price": 793.65,
   "stop": 715.00, "target": 980.00, "why": "above both averages, and the swap still fits"}
], "rationale": "..."}
```

or `{"moves": [], "pass_reason": "..."}`. Moves are ordered: sells first, then buys.

### 4. Validated move by move, then as a plan

Each move goes through the existing validator (`stop` on the right side of entry, price inside the ATR
band, sold quantity actually held, not the same instrument twice). Then the plan is checked as a whole:
**buys may not cost more than cash plus the proceeds of the plan's own sells**. Any failure refuses the
*whole plan* — no partial plans, no half a swap — and the refusal is recorded with its reasons, the way
a blocked single decision already is.

### 5. Stored as one group, approved as one group

Migration `0014_plan`: `signals.plan_id INTEGER`, set to the first row's id for every row in a plan;
`kind = 'plan'`. Approval and rejection act on the whole group, exactly as rotation's `pair_id` legs do
today; `pair_id` keeps its current meaning for rotation so nothing existing changes shape.

### 6. Readable in the ledger and the audit

One card per plan, not three unrelated ones:

```
PLAN · 2 moves · 09 Nov
SELL  Infosys   24 @ 1,171.60  → target reached
BUY   HDFC Bank 15 @   793.65  → above both averages
1,171.60 × 24 = 28,118.40 covers 793.65 × 15 = 11,904.75, and 16,763.65 is left in the purse
[approve the plan] [reject]
```

A refused plan shows the reason it was refused. The audit row's prompt type reads `plan_v1`, and the
call's purpose is `plan book`.

### 7. What this does not change

Per-stock calls stay, so every decision remains individually explainable — that is what the audit page
is for. The rules layer still works on prices; the plan call is an addition, not a replacement. The
rotation call stays for now: it is the narrow, cheap question, and the plan call costs a full prompt.

## Limits we accept

- One extra call per active poll, and the book adds roughly 300–700 tokens to every prompt. The static
  block stays a prefix, so DeepSeek's context cache still hits.
- A plan is atomic: one implausible leg costs the whole plan. That is the right trade — a half-executed
  swap is worse than none.
- The book is capped at 12 lines, so a 30-holding portfolio is summarised, not enumerated. If that
  matters later, raise the cap behind a setting.

## Tests

- The spending cap sentence is in the rendered prompt, with the purse's own number.
- The book block lists every holding, marks the one under review, and truncates with `+N more`.
- A plan whose buys exceed cash plus its own proceeds is refused as a whole, with a reason.
- A plan's rows share `plan_id`; approving one approves all of them; rejecting one rejects all.
- The ledger renders a plan as one card reading sell-then-buy.
- No test hits the network: fixtures only.
