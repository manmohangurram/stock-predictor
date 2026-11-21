# UI standards — stock-predictor

Read as one document: intent, tokens, components, pages are three views of the same decision. Every
number here is a token that exists in `src/tokens.css` — if a value is written here and not there, the
document is wrong, not the code.

`docs/testing/flows.md` is the behavioural half of this document: it says what each screen must do.
This one says what it must look like while doing it.

---

## 1. Intent

**Who.** One person: an Indian retail investor with a fixed, modest pot. On a phone during the day
(a notification), at a laptop at night (the book). Not a trader — they have a job. They are not
watching charts.

**What they must do.** Answer one question in under ten seconds: *is there something I need to do, and
can I afford it?* Then, less often: record what they actually bought.

**How it must feel.** A calm account book, not a trading terminal. The interface should make
over-trading feel slightly boring. Nothing blinks. Nothing celebrates. The money is visible at all
times and visibly finite.

**The test for every choice:** *would this read the same if the numbers were someone else's, in
someone else's currency?* If a styling decision only works because it is dramatic, it is wrong here.

---

## 2. Tokens

### Paper

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `oklch(0.985 0.006 85)` | `oklch(0.16 0.008 85)` | the page |
| `--paper-raised` | `oklch(1 0 0)` | `oklch(0.2 0.008 85)` | a card, an input |
| `--paper-inset` | `oklch(0.965 0.008 85)` | `oklch(0.13 0.008 85)` | the rail, a table head |
| `--paper-settled` | `oklch(0.955 0.02 155)` | `oklch(0.22 0.02 155)` | a settled row |

### Ink

| Token | Light | Meaning |
|---|---|---|
| `--ink` | `oklch(0.22 0.012 60)` | the number you came for |
| `--ink-secondary` | `oklch(0.44 0.012 60)` | prose |
| `--ink-tertiary` | `oklch(0.58 0.01 60)` | labels, uppercase |
| `--ink-muted` | `oklch(0.7 0.008 60)` | an em dash, "no price yet" |

Four levels, and nothing else. A fifth grey is a design decision nobody recorded.

### Radius — chosen by role, not size

| Token | Value | On |
|---|---|---|
| `--radius-sm` | 6px | a tick (`tick-toggle`), an inline chip |
| `--radius` | 10px | a field: input, select, textarea |
| `--radius-card` | 12px | a surface: card, panel, decision box, error band, table wrapper, settings tab |
| `--radius-lg` | 14px | a large surface: a dialog, a drawer |
| `--radius-pill` | 999px | **every button, at every size** — `d-btn`, `icon-btn`, `row-caret`, a segment and the group that holds it — plus a filter or a status pill |
| `--radius-stamp` | 4px | a stamp (`BUY`, `SELL`, `DONE`) — it is a printed mark, not a button |

**A button is a pill; a field is not.** Radius follows the control's role, not its height: every
action control wears `--radius-pill` at every size, so a 28px `d-btn-xs` and a 40px `d-btn` share one
silhouette — `Refresh`, `Save`, `Approve`, `Archive`, the icon buttons, and a segmented control whose
group and segments are both pills. An input keeps `--radius`, a stamp keeps its printed edge, and a
text action (`Edit`, `Sign out`) has no shape at all. Never round a table's own corners — the wrapper
is what holds a radius.

### Rules

`--rule-subtle` (0.08) · `--rule` (0.14) · `--rule-strong` (0.24) — hairline, normal, emphatic.
`--rule-focus` is the focus ring, and it is the only thing allowed to be `oklch(0.42 0.12 262)` at
full strength.

### Stamps — colour always paired with a word

| Stamp | Ink | Tint | Means |
|---|---|---|---|
| ink | `--stamp-ink` | `--stamp-ink-bg` | BUY, a decision, an estimate |
| oxide | `--stamp-oxide` | `--stamp-oxide-bg` | SELL, a loss, a failure |
| amber | `--stamp-amber` | `--stamp-amber-bg` | blocked, refused, unsaved |
| graphite | `--stamp-graphite` | `--stamp-graphite-bg` | resolved, done, neutral |
| green | `--green` | `--green-bg` | a gain, an approval, a run that finished |

**A stamp is never colour alone.** `stamp.ts` renders the word; the tint is the second signal. The
same rule applies to the market dot: `.dot` plus the words *Market closed*.

### Type

| Token | Size | Use |
|---|---|---|
| `--text-xs` | 0.6875rem / 11px | uppercase labels, stamps, table heads, timestamps |
| `--text-sm` | 0.8125rem / 13px | controls, help lines, secondary rows |
| `--text-base` | 0.875rem / 14px | body |
| `--text-md` | 1rem / 16px | a figure in a card |
| `--text-lg` | 1.25rem / 20px | a page section heading |
| `--text-2xl` | 2.125rem / 34px | the page title, and the biggest number on it |

Two families, and they mean different things:
**`--font-form`** (IBM Plex Sans) is the interface — labels, buttons, prose.
**`--font-ledger`** (IBM Plex Mono) is the record — money, quantities, dates, prices, ids.

A number is monospace. A word is not. That is the whole rule, and it is why columns line up.

### Spacing

`4 · 8 · 12 · 16 · 24 · 32 · 48` (`--space-1` … `--space-12`). Nothing between them.

- Inside a control: `--space-2` / `--space-3`.
- Between label and field: `--space-2`.
- Between fields: `--space-4`.
- Between cards in a column: `--space-4`; between sections: `--space-6`.
- A number and its note: `2px` (`row-gap: 2px` in the figure block), because they are one reading.

### Shape

Radius comes from the table above: pills on buttons, `--radius` on fields, `--radius-card` on
surfaces, `--radius-stamp` on a stamp. `--row-h-touch: 44px` is the smallest a tappable row may be on
a phone.

---

## 3. Alignment

**One rule per table: prose left, numbers right.** Every numeric column header carries `.num`, and so
does every cell under it — a right-aligned number under a left-aligned header is how a page starts to
look careless.

- The left edge of every line that is supposed to share one: measure it, do not eyeball it. Cards in a
  column, table text, page title and back-link all sit on the same `x`.
- Numbers align on their **right** edge, so the decimal places stack.
- Dates are numbers here: a date column is right-aligned, and so is its header.
- A column whose content can force the table wider than the page is a bug: long text wraps inside its
  own cell (`white-space: normal; overflow-wrap: anywhere`), it does not push the page.
- `getBoundingClientRect()` is the arbiter. Before calling alignment done, read `left` for text
  columns and `right` for numbers, and confirm the page's `scrollWidth === clientWidth`.

---

## 4. Components

The kit lives in `src/components/`; `ui/` holds the primitives.

| Component | Rule |
|---|---|
| `ui/AppButton` | the one button shape, a pill at every size; `primary` for the action a page exists for, `ghost` for everything else. A page never hand-writes a `.btn` or a `d-btn` shape. |
| `ui/AppStamp` | a word in a tinted box; colour never carries meaning alone |
| `ui/AppEmpty` | one sentence about what is absent, never a shrug |
| `SettingField` | label, control, unit, help — in that order, always |
| `AppShell` | the rail and the page column; the rail never scrolls, only the page does |
| `HoldingRow` | name first, ticker second; numbers right |
| `DecisionItem` | one decision and one plan are the same component; a plan is approved whole or not at all |
| `CallDrawer` | the stored exchange: sent (markdown) \| received (a verdict, then the raw bodies one click away) |
| `ScheduleStrip` | one card per prompt that decides something; cadence and next-due come from the server |
| `PriceChart` | the tape as a reading aid, not the page: 196px tall, axes labelled, reference lines haloed |
| `AddInstrument` | ⌘K, grouped results, Indian listings only |

**daisyUI is installed with the `d-` prefix** (`src/styles/tailwind.css`). Controls are `d-btn`,
`d-input`, `d-select`, `d-modal`; the ledger's tables and cards keep their own classes because they are
tuned for density and daisyUI has no blotter. The theme is this file's paper and ink, not a preset, and
preflight is off so nothing resets the app's own base.

---

## 5. States — every one is designed

| State | Where it shows |
|---|---|
| hover | every link, button, row, and control: a paper shift, never a shadow |
| focus-visible | a 2px `--stamp-ink` ring with 1–2px offset. Every focusable element has one; the daisyUI buttons inherit it |
| disabled | 40% opacity and `cursor: default`, but the label stays readable — "cannot archive while held" is text, not a greyed mystery |
| loading | a sentence in the same space the answer will occupy ("Reading the stored exchange…"), never a spinner over the whole page |
| empty | one sentence that says what is absent and, where there is one, the next step |
| error | a `.error-band` at the top of the page with what failed and a Retry — never a toast, never silence |
| unsaved | the tab's stamp reads *unsaved* (amber), and *saved* (green) once the row is written |

---

## 6. Copy

- **A verb on a button, a fact in a label.** "Add instrument…", "Record fill", "Run again" — not
  "Submit", not "OK".
- **Say the number.** "opens at 09:15 IST", not "opens soon".
- **No celebration, no apology.** Never "Success!", never "Oops". The record changed; show it changed.
- **The unit belongs to the value.** `₹2,00,000`, `3.6%`, `9:15`, `40 units` — and Indian digit
  grouping (`2,00,000`, not `200,000`) everywhere.
- **Two decimals for money, none for totals**, and a minus is `−` (U+2212), not a hyphen.

---

## 7. Review checklist — before calling a screen done

1. **Squint.** Blur your eyes: can you still find the title, the money, and the one action?
2. **Spacing from the scale.** Every gap is 4/8/12/16/24/32/48. A `13px` is a decision nobody made.
3. **One accent.** The stamp colour is the only colour on the page that means something.
4. **Measure alignment** (§3): left edges shared, right edges shared, no horizontal overflow.
5. **Both widths.** 1440 and 390. On the phone: nothing clipped, touch targets ≥ 40px, the market state
   under the title.
6. **Every state** in §5 exists on the screen you touched.
7. **The money is visible.** Whatever else is on the page, the figure that decides affordability is on
   screen without scrolling.
8. **`docs/testing/flows.md` is green** for the pages this change touched.
9. **Buttons are pills.** Every action control — `d-btn` at any size, `icon-btn`, `row-caret`, a
   segmented group and its segments — measures `999px`; an input does not.
