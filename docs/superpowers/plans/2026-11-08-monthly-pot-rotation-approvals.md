# Monthly pot, rotations and approvals — implementation plan

> **For agentic workers:** use superpowers:executing-plans. Steps are checkboxes.

**Goal:** a monthly allowance per pot that expires unspent, decision statuses with an approve-as-fill
flow, portfolio rotation as one two-legged decision, instrument delete/archive, and a call gate that
only fires on a real change.

**Architecture:** cash becomes a true balance (every `cash_ledger` row, not just allowances); a new
`pot` module owns the month boundary; `signals` gains a status and a rotation pair; the gate loses its
forced call.

**Spec:** `docs/superpowers/specs/2026-11-08-monthly-pot-rotation-approvals-design.md`

## Global constraints

- `cash_ledger` and `api_calls` stay append-only: inserts only.
- No test touches the network.
- `cargo clippy --all-targets` warning-free; `cargo test` green; `cd web && npx vitest run && npm run build` green.
- Every new user-facing string goes through the existing `ApiError` variants.
- Expiry is never counted in realised P&L.

---

### Task 1 — cash is a balance, and the month has a boundary

- [x] `budget::cash_in(mode)` = `SUM(amount)` over every kind; `available_in` returns it.
      (`budget_total_in` keeps meaning "allowance granted", used only for display.)
- [x] `CashKind::Expiry` → `"expiry"`.
- [x] `rules/pot.rs`: `ensure_month(pool, mode, allowance, now) -> MonthReport`.
      If `settings.pot_month_<mode>` ≠ `YYYY-MM` (IST): expire the positive balance
      (`expiry −cash`, reference "unspent allowance expired"), then `budget +allowance`
      (reference "allowance 2026-11"), then write the month row — one transaction per pot.
- [x] Called from the poll loop for both pots, using the configured allowance.
- [x] Tests: reset once per month; idempotent on a second tick; zero cash writes no expiry;
      expiry does not touch realised P&L; `cash_in` follows buys and sells.

### Task 2 — the gate stops inventing calls

- [x] `Runtime.max_silence_secs` default `0`; `should_call` only applies the interval when `> 0`.
- [x] Cadence copy: "0 means never call just because time passed".
- [x] Tests: unchanged price + unchanged headlines = no call at any elapsed time; new headline = call.

### Task 3 — delete or archive an instrument

- [x] `DELETE /api/instruments/{symbol}`: refused while a position is open; hard delete (and its
      bars, news, memos, pending signals, fills) when there is no history; `archived` (`active = 0`)
      when there is history. Response names which happened.
- [x] Tests: clean delete removes the row and its bars; held symbol is refused; symbol with a
      closed fill is archived and unreadable from polling but keeps its fills.

### Task 4 — decision status and approve-as-fill

- [x] Migration `0010_decision_status`: `status`, `status_at`, `status_reason`, `filled_qty`,
      `filled_price`, `pair_id`, `kind` on `signals`, default `pending`.
- [x] `POST /api/signals/{id}/approve` with `{qty?, price?, at?}`: validates and records the fill
      through `trades::record` in one transaction with the status update; a failure leaves `pending`.
- [x] `POST /api/signals/{id}/reject` with `{reason?}`: status only, no ledger write.
- [x] `/api/dashboard` and `/api/calls` carry status; `?decisions=pending|approved|rejected|all`.
- [x] Tests: approve with edited qty/price posts those numbers; a fill that fails leaves pending;
      reject writes nothing; the ledger payload returns pending only by default.

### Task 5 — rotation (two legs, one decision)

- [x] Prompt `rotate_v1.md`: holdings table in, `{"action":"rotate","sell":{…},"buy":{…}}` or `PASS` out.
- [x] Trigger: a BUY candidate exists, cash does not cover it, and some held position is above the
      pot's minimum profit.
- [x] Validator: `proceeds + cash ≥ cost`, sell quantity held, buy within limits, never a fund in the
      trading pot.
- [x] Approval posts both legs or neither.

### Task 6 — UI

- [x] Ledger decisions: pending only, with a `Pending / Approved / Rejected / All` filter as a URL
      parameter; each pending row has `Approve` (inline qty/price/date form, pre-filled, editable) and
      `Reject` (reason).
- [x] Instrument page: status per decision; a rotation renders both legs in one row.
- [x] Audit: status column.
- [x] Instrument page: `Delete` / `Archive`, worded by what the server did.
- [x] Settings → Money: "Monthly allowance" wording, reset day, unspent expires.

### Task 7 — docs and release

- [x] `docs/configuration.md`, `docs/how-it-works.md`, README features table.
- [x] Fresh `docs/screenshot.png`; tag `v0.4.0`.
