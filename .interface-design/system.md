# stock-predicter — interface design system

Companion to `docs/superpowers/specs/2026-10-27-stock-predicter-design.md`.
Binding for the front-end tasks. Not a mood board.

---

## Intent

**Who.** One person. Indian retail investor with a fixed, modest pot of money. Mostly on a
phone during the day (notifications), on a laptop at night (the dashboard). Not a trader —
they have a job. They are not watching charts.

**What they must accomplish.** Answer one question in under ten seconds: *is there
something I need to do, and can I afford it?* Then, less often: record what they actually
bought, and adjust the pot.

**What it must feel like.** A calm account book, not a trading terminal. The interface
should make over-trading feel slightly boring. Nothing blinks. Nothing celebrates. The
money is visible at all times and visibly finite.

**Explicit non-feel:** not exciting, not a game, not "markets are moving". If they open
this and feel a pull to check it every hour, the design failed.

---

## Domain Exploration

**Domain concepts (territory, not features):** the bahi-khata / passbook ledger; the
broker's rubber stamp; the order pad; railway timetable (platform, time, price); the
allotment letter; the "you are here" board; the ration card's fixed entitlement; ticker
tape; the settlement calendar; the examiner's red margin.

**Color world.** A physical version of this product is a ledger on a desk: cream ruled
paper, blue-black fountain-pen ink, a red-oxide rubber stamp, the pale green of a bank
passbook, the amber of a carbon-copy form, the graphite of a pencil margin note, the
magenta of stamp paper.

**Signature.** **The running balance column.** Every signal row ends with the cash that
would remain after that action, in mono, right-aligned, so the pot visibly runs down the
length of the page. The budget is never a progress bar — it is a column of numbers that
empties as you read.

**Defaults rejected:**
1. *Dark trading terminal with candlesticks and a giant P&L number.* Rejected: it exists to
   make trading feel like progress, and P&L is not the question this person has.
2. *Card grid of metric boxes with sparklines.* Rejected: it hides the "money running out"
   story and is the default shape of every generated dashboard.
3. *A chat window with the LLM.* Rejected: the model is the author of one paragraph inside
   a row, not the interface. Conversation would invite asking it for more trades.

**Swap test.** Replace the palette with slate+blue, the type with Inter, the layout with
sidebar+cards: you get a generic fintech dashboard, and the passbook reading disappears.
The choices below are doing work.

---

## Direction

**Personality:** Institutional calm — Warmth & Approachability foundation, executed with
Precision & Density discipline. Paper, not glass.

| Decision | Value | Why |
|---|---|---|
| Foundation | Warm, hue 85 (paper cream) | The object this replaces is a paper ledger; cream lowers arousal vs. white or slate |
| Depth | Borders-only, 0.5px hairlines | Dense accounting structure; shadows would imply cards floating, i.e. a product, not a record |
| Spacing base | 4px, scale 4/8/12/16/24/32/48 | Dense rows for scanning, generous section breaks for rhythm |
| Radius | 3px surfaces, 2px stamps | Stamped forms are not rounded; 0 would read brutal, 8 would read SaaS |
| Accent | Two stamps, not one: indigo (BUY) + red-oxide (SELL) | Buy and sell must be distinguishable without red/green, for color-blind safety |
| Type | IBM Plex Sans + IBM Plex Mono | Institutional print heritage, real tabular figures; Inter would erase the form-like feel |
| Theme | Light only by default | A ledger is a light object; dark mode offered via `prefers-color-scheme` at semantic level |

---

## Tokens

Names come from the world, not from a palette generator.

```css
@layer tokens {
  :root {
    /* paper */
    --paper:          oklch(0.985 0.006 85);   /* canvas */
    --paper-raised:   oklch(1 0 0);            /* cards */
    --paper-inset:    oklch(0.965 0.008 85);   /* inputs, "type here" */
    --paper-settled:  oklch(0.955 0.02 155);   /* passbook green band, settled rows */

    /* ink */
    --ink:            oklch(0.22 0.012 60);
    --ink-secondary:  oklch(0.44 0.012 60);
    --ink-tertiary:   oklch(0.58 0.01 60);
    --ink-muted:      oklch(0.70 0.008 60);

    /* rules */
    --rule-subtle:    oklch(0.22 0.012 60 / 0.08);
    --rule:           oklch(0.22 0.012 60 / 0.14);
    --rule-strong:    oklch(0.22 0.012 60 / 0.24);
    --rule-focus:     oklch(0.42 0.12 262);

    /* stamps — always ink-on-tint, never white-on-saturated */
    --stamp-ink:      oklch(0.42 0.12 262);    /* BUY, primary action */
    --stamp-ink-bg:   oklch(0.93 0.03 262);
    --stamp-oxide:    oklch(0.42 0.15 35);     /* SELL */
    --stamp-oxide-bg: oklch(0.93 0.035 35);
    --stamp-amber:    oklch(0.45 0.10 70);     /* ESTIMATE / provisional */
    --stamp-amber-bg: oklch(0.95 0.05 85);
    --stamp-graphite: oklch(0.40 0.006 60);    /* BLOCKED */
    --stamp-graphite-bg: oklch(0.94 0.004 60);

    /* type */
    --font-form:   "IBM Plex Sans", system-ui, sans-serif;
    --font-ledger: "IBM Plex Mono", ui-monospace, monospace;

    --text-xs: 0.6875rem;  /* 11 — stamp labels */
    --text-sm: 0.8125rem;  /* 13 — row labels, meta */
    --text-base: 0.875rem; /* 14 — body */
    --text-md: 1rem;
    --text-lg: 1.25rem;    /* 20 — section titles */
    --text-xl: 1.625rem;   /* 26 — today's symbol */
    --text-2xl: 2.125rem;  /* 34 — available cash */

    /* space */
    --space-1: 4px;  --space-2: 8px;  --space-3: 12px; --space-4: 16px;
    --space-6: 24px; --space-8: 32px; --space-12: 48px;

    --radius: 3px;
    --radius-stamp: 2px;
    --row-h: 32px;         /* desktop ledger row */
    --row-h-touch: 44px;   /* mobile */
  }
}
```

**Dark mode** inverts at the semantic layer only: `--paper: oklch(0.16 0.008 85)`,
`--paper-raised: oklch(0.20 0.008 85)`, `--ink: oklch(0.92 0.008 85)`, rules switch to
white-based alpha. Stamps keep the same hues with chroma reduced ~15% and lightness raised.

---

## Typography

- **Every figure is mono with `font-variant-numeric: tabular-nums`** — prices, quantities,
  cash, dates, symbols. No exceptions; misaligned digits in a ledger are unreadable.
- **Labels** are 11px / 600 / uppercase / `letter-spacing: 0.08em` / `--ink-tertiary`.
- **Headings** are Plex Sans 600, tracking `-0.015em` at 20px+.
- **The available-cash number** is the largest thing on the page: 34px mono 500, tracking
  `-0.02em`. It is the focal point.

---

## Layout

One page, no sidebar. `max-width: 1080px`, grid `1fr 320px`, gap 24px.

- **Ledger** (left): the record. Runs the full height.
- **Balance rail** (right, sticky, `top: 24px`): available / committed / budget, split by
  asset class, above a `Settings` link. Separated by a single hairline, no shadow.
- **Page header:** title in Plex Sans 600, then the **double rule** — 1px `--rule-strong`
  with a second 1px `--rule-subtle` 3px below. One per screen, never repeated.
- **Mobile < 900px:** single column. The rail collapses into a sticky top strip showing
  `Available · Committed · Budget` inline, mono, 13px. Ledger rows become stacked blocks at
  `--row-h-touch`, with label/value pairs.

**Rhythm:** `--space-2` between rows, `--space-6` between sections, one `--space-12` break
before the audit link. Density is deliberate and uneven — a page with uniform gaps reads as
a template.

---

## Components

### 1. Signal row — the core unit

```
09:41  INFY  ·  STOCK          [SELL]   qty 5   stop 1,412.00   1,405.20   ₹84,900
                                                       target 1,510   valid till 15:30
```

- Grid: `72px time | 1fr identity | auto stamp | repeat(3, 96px) figures | 112px balance`.
- Figures right-aligned, mono. `--rule-subtle` bottom border only.
- The **right-hand balance** is the signature: cash remaining if this executes.
- Expanding a row reveals: rules reasons as a bulleted list (13px), the LLM rationale
  paragraph (14px, max 60ch), and cited headlines as `[3] Infosys guidance raised — Reuters, 24 Oct`.
- **Estimate rows** (MF, IPO) carry a `[ESTIMATE]` amber stamp and the timing note
  (`NAV at today's 15:00 cutoff`, `Cash arrives T+2`, `Allotment pending`). An MF price is
  never rendered in the same weight as a stock's firm price — estimates get
  `--ink-secondary` instead of `--ink`.

### 2. Stamp badge

Ink on tint, `--radius-stamp`, 11px/700/uppercase/`0.1em`, padding `2px 6px`.
**Rotated `-1.5deg` on BUY and SELL only.** That rotation is the single non-grid gesture in
the system, and it is what makes the page read as *stamped* rather than *labelled*.
Text is always present — color never carries meaning alone.

### 3. BLOCKED band

Not an error. A row with a 45° hatch (`repeating-linear-gradient`, 6% `--ink`, 6px pitch),
graphite text, and the shortfall printed: `insufficient cash — short by ₹12,400`. It never
appears in a push notification, because there is nothing to do about it.

### 4. Balance rail

Three stacked blocks: **Available** (34px mono), then Committed and Budget (16px mono) with
a hairline between. Below, three class chips: Stocks / Mutual funds / IPOs reserved — each
with its own committed figure. If `available < 0`, the number goes `--stamp-oxide` and a
one-line explanation appears; this is a bug state, and it should be impossible.

### 5. Settings form

Budget (number, mono, with the currency as a static prefix) and currency (select). The
**committed figure is shown inline next to the input while typing**, and the submit button
disables the moment the typed budget drops below it, with the reason in plain words:
`Committed is ₹1,42,000 — the budget cannot go below it.`
Currency is disabled with `Currency is fixed once the ledger has entries.` when locked.
Fields are `--paper-inset` with a 0.5px `--rule` border and a 2px `--rule-focus` ring on
focus-visible, offset 2px.

### 6. States — every one of these exists

| State | Treatment |
|---|---|
| Loading | Ruled blank lines — the empty ledger *is* the skeleton. No spinner. |
| Empty | `No entries today. Market closes 15:30.` in `--ink-tertiary`, no illustration |
| Error | Inline hairline band with graphite text and a `Retry` button; never a modal |
| Disabled | `--ink-muted` text, `--paper-inset` background, retains 4.5:1 for its label |
| Focus-visible | 2px `--rule-focus` ring, 2px offset, on every focusable element |
| New signal arrived | Row height animates open once, stamp presses (see Motion) |

### 7. Chart (instrument detail)

One ink line, no fills, no candles, no volume bars, no green/red. Two dotted hairlines at
stop and target, labelled in mono at the right edge. Axes are 11px `--ink-tertiary`, no
gridlines except a single baseline. The chart's job is to show where the price sits relative
to the two rules — nothing more.

---

## Pages

| Route | Job |
|---|---|
| `/` | Today's signals, newest first. Focal point: the top row, rendered at 26px symbol. |
| `/positions` | Held stocks (stop/target live), MF units (`cash in T+1/T+2`), IPO reservations (pending/allotted). Each row has `Record sell` / `Record buy`. |
| `/settings` | Budget + currency + rule thresholds. Carries the committed-floor guard. |
| `/calls` | `api_calls`, newest first: purpose, model, tokens, validator verdict. Expanding shows raw request/response in mono at 12px, scroll-capped. |
| `/i/:symbol` | Bars chart, headlines (title/publisher/date only), signal history. |

---

## Push notification copy

The notification is part of the interface and gets the same restraint.

- `SELL · INFY 5 units · stop 1,412 hit, now 1,405`
- `BUY · HDFCBANK 4 units · limit 1,650 · valid till 15:30`
- `MF · REDEEM · NAV estimated at today's cutoff · cash T+2`
- `IPO · XYZ closes today 17:00 · verdict: apply 1 lot`
- Never sent: BLOCKED signals, unchanged signals, HOLD.

---

## Motion

| Interaction | Duration | Easing |
|---|---|---|
| Row expand | 150ms | ease-out |
| Stamp press (new row, once) | 200ms | scale 1.06 → 1.00, ease-out |
| Toast (recorded a trade) | 200ms in / 150ms out | ease-out |
| Focus ring | 100ms | linear |

Nothing else moves. No pulsing, no live counters, no tickers. `prefers-reduced-motion`
disables the stamp press and row height animation entirely (content still appears).

---

## Accessibility

- WCAG 2.2 AA: 4.5:1 body, 3:1 large and UI. Stamps are ink-on-tint specifically so this
  holds without relying on saturated fills.
- Buy/sell distinguished by text + shape + hue; never hue alone. BLOCKED by hatch + text.
- Keyboard: every action reachable; row expand is a real `<button>` with `aria-expanded`;
  the audit JSON viewer is a `<details>`.
- Live region announces a new signal once: `New signal: sell 5 INFY`.
- Touch targets 44px minimum on mobile; `--row-h-touch` enforces it.
- The ledger is a real `<table>` with a `<caption>`, so the balance column has a header.

---

## Decisions log

- 2026-10-27 — Two stamp colors instead of one accent. Reason: buy/sell must be
  color-blind-safe, and one accent cannot carry both directions.
- 2026-10-27 — No dark mode as default, despite the "data & analysis" direction suggesting
  it. Reason: the object being replaced is paper; dark terminal aesthetics are the exact
  default this product is trying not to be.
- 2026-10-27 — Balance column instead of a budget progress bar. Reason: a bar answers "how
  much is left"; a column answers "what does this trade leave me", which is the actual
  decision.
- 2026-10-27 — BLOCKED rows are hatched, not red. Reason: red reads as an error the user
  caused; a blocked signal is the system working correctly.

---

## Chosen direction — Blotter spine (agreed 2026-10-27)

The passbook ledger was too thin to be a daily driver. After the six-direction lab
(`web/design/lab.html`), the hybrid was chosen:

- **Portfolio strip** pinned above everything: total value, invested, cash, realised, and a
  two-segment exposure bar. This answers "can I afford it" without a navigation step.
- **Blotter table** as the spine: 34px rows, mono figures, right-aligned numbers, one signal
  stamp per row. The whole row is a link — clicking any instrument opens its record.
- **Instrument page** carries what the old flat table could not: chart with SMA references,
  a fill recorder, the decision stream with rationale, the full fills history, headlines, and
  the rule checklist explaining why there is or is not a signal.
- **Side rail** holds search-and-add (stocks from Yahoo, funds from AMFI), recent trades
  across the portfolio, and this-week counts.

Kept from the original system: paper/ink palette, IBM Plex Sans + Mono, 0.5px hairlines,
stamps with the −1.5° rotation, `BLOCKED` as a hatch rather than red.

Deliberately not built: candlesticks, day-change percentages sourced from nowhere, a chat
window, and any pulsing "live" indicator.
