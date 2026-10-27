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
