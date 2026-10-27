# Ledger — stock-predicter, 2026-10-27

Spec: docs/superpowers/specs/2026-10-27-stock-predicter-design.md
Plan: docs/superpowers/plans/2026-10-27-stock-predicter.md
Branch: feat/stock-predicter (git init'd here; no worktree needed — repo had no code)
Mode: inline (executing-plans), backend → notifications → UI

## Task log

- Task 1 ✅ crate scaffold, config, schema. 3 tests green.
  Ruling: inline UNIQUE/PK constraints instead of separate `CREATE UNIQUE INDEX`
  (equivalent outcome, fewer statements). Cost if wrong: none, same constraints.
  Ruling: added `src/lib.rs` so integration tests can import the crate; `main.rs` is a
  thin binary over it. Cost if wrong: none.
  Ruling: `SqliteConnectOptions::new().filename()` instead of parsing a URL string —
  `FromStr` was not resolving against sqlx 0.8's feature set.
- Task 2 ✅ yahoo client, 6 tests.
  Ruling: news comes from `/v1/finance/search` (title/publisher/link/providerPublishTime)
  rather than the undocumented `xhr/ncp` stream — it returns exactly the headline fields
  and no body, which is the point of the parser. Cost if wrong: swap one URL.
  Ruling: a missing `news[]`/`chart.result` key is an error (drift must be loud); an empty
  array is a valid empty result.
- Task 3 ✅ amfi, 3 tests.
- Task 4 ✅ indicators, 4 tests.
  Ruling (plan defect): `compute` requires 200 bars (SMA200), but the plan's own
  known-series test used 60 — contradictory. Kept the 200-bar floor (a NaN SMA200 would
  leak into rules) and made the test series 250 bars, whose mean is still exactly 3.0.
  Cost if wrong: a symbol with <200 daily bars gets no signal, which is the safe direction.
