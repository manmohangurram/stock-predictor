# Changelog

Tags before `v0.19.0` were cut without notes; their subjects are in `git log`. This file starts
here and records what changed for a reader of the software, not for a reader of the diff.

## v0.19.0

### The book

- **The model sees the whole book.** Every prompt carries the holdings and one plain sentence —
  *you may spend at most ₹12,45,570 in this purse; a buy costing more is refused, not shrunk* — so a
  proposal can no longer be sized against money that does not exist.
- **Two plans, two prompts, two purses.** `plan_stock_v1` sees the stock book, sizes in units, and
  every buy must carry a stop and a target. `plan_mf_v1` sees the fund book only, sizes in rupees,
  and has no stops — a fund has no price to stop at. Neither can name the other's instrument.
- **Both plans are once-a-day book decisions** (`stock_plan_last`, `mf_plan_last`), asked on the
  first tick of the IST day — including the first tick after a restart. Per-stock calls cover the
  intraday.
- **A buy needs no sale.** A plan is judged on whether it fits the purse, not on whether it
  contains a sell. Sells still execute first.
- **Approval posts every leg, or none.** A fill moves the position, the trade row and the cash
  ledger in one transaction; a plan is one transaction end to end.

### Exit plans that survive approval

- Approving a decision now writes its stop and target onto the **position**, which is what the
  rules layer compares the close against. Before this, an approved buy had no exit at all.
- A fund shows a NAV exit derived from its profit expectation (`76.75 → +25%`), because a fund has
  no price to stop at.

### Audit

- The page shows **runs** — poll, stock plan, fund plan, short-term pass — opened before the work
  and closed after it, so *running* is visible while it happens. Orphans are closed at boot.
- **Run now** executes the same code the scheduler does, through the poll loop that owns the
  clients. A manual run is one run, not a run plus a tick.
- The calls table pages with `offset`/`limit` and a page-size selector; the total travels in a
  header.
- A quiet tick says why it was quiet: *0 decisions · 2 not asked: INFY.NS, HDFCBANK.NS*.
- Decisions lead with the holding's **name**; a fund shows its scheme name, not `0P0000XV5C.BO`.
- Blocked proposals no longer sit in the decisions box — that box is for decisions. The refusal
  stays on the instrument's record and in the audit.
- The drawer is one sheet: a verdict card, then the exchange. It no longer repeats the row.

### Behaviour removed

- **The silence interval is gone.** A call happens when the price moved or the headlines changed,
  and for no other reason. `max_silence_secs` no longer exists.

### Data

- Three indexes that no query could use are dropped; `runs_running` is a partial index (running
  rows only) so the audit page's count stays at 0.011 ms with 36,000 runs behind it.
- NSE's gainers payload mixes `22.86` with `"22.86 Cr"`. One formatted value used to reject the
  whole list; every numeric field now reads numbers, numeric strings, percentages and Indian
  magnitudes.
- Append-only `cash_ledger` and `api_calls` are unchanged. A corrected fill still posts an
  `adjust` entry rather than rewriting history.

### Known

- `cargo audit` reports RUSTSEC-2026-0459 against the SQLite bundled by `libsqlite3-sys` 0.30.1,
  which sqlx 0.9 pins. It is acknowledged in `.cargo/audit.toml` with the reason — the exploit
  needs a database or SQL the app did not write, and this app opens its own file and binds every
  parameter — and the ignore is to be deleted the day sqlx allows `libsqlite3-sys` 0.39.

### Front end

- Three layers: `pages/` renders, `components/` holds the atoms (one stamp, one pill, one date
  helper), `wiring/` binds events per feature. `main.ts` is a router and a timer.
- The app shell revalidates (`no-cache`), so a rebuilt front end no longer needs a hard refresh.
- Free to deploy sits with the figures; the decisions list scrolls inside its card.
