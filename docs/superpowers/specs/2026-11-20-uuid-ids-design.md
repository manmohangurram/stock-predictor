# UUID ids — design

Date: 2026-11-20. Status: proposed; the slices land one pull request at a time.

## Why

Every table keys on `INTEGER PRIMARY KEY`, which is SQLite's rowid: the id is the row's position in
the file, it is reused after a delete, and it leaks how many rows exist. Sixteen tables and thirty-odd
foreign keys carry it through the stores, the JSON and the frontend. A row that travels — an exported
fill, a decision copied between databases, a URL pasted into a bug report — says nothing about which
database it came from, and two databases will hand the same id to different rows.

## Decision

**UUIDv7 as the primary key, stored as lower-case canonical text (36 characters).**

- **Version 7, not 4.** A v7 carries a millisecond timestamp in its first 48 bits, so ids sort in
  creation order. That matters here: `signals`, `api_calls` and `runs` are read `ORDER BY id DESC`
  all over the code and a random v4 would turn every one of those into an index scan plus a sort.
  With v7 the existing queries keep working and keep meaning "newest first".
- **Text, not BLOB.** SQLite has no native UUID type. Text is debuggable in a test fixture, sorts
  correctly, and the JSON is a string in both directions. The 36-byte cost is irrelevant at this
  table size, where the largest table is measured in thousands of rows.
- **Generated in Go** (`internal/ids.New()`), not by SQL: one generator, one place to test the
  clock and the monotonic counter, and no dependency on SQLite's `randomblob` spelling.
- **FKs are TEXT** and keep their `REFERENCES` clauses.

`settings.key`, `plan_candidates(day, symbol)` and `trend_candidates(day, symbol)` stay as they are:
they are natural keys, not identities.

## Migration mechanics

SQLite cannot change a primary key in place, so each table is rebuilt: `CREATE TABLE _new`, copy with
generated ids through a mapping table, drop, rename, re-create indexes. For sixteen tables that is one
migration file, applied in order, with `PRAGMA foreign_keys = OFF` around it (the migration runner
executes outside a transaction, which is what makes the pragma effective).

The backfill generates one v7 per row, and the mapping table translates the old integer to it; child
tables join through the mapping rather than trying to encode the old id.

## Slices

Each slice is its own PR and leaves `main` green and the app working.

1. **Expand — add the identity, keep the key.** `internal/ids` with v7 + tests; `ALTER TABLE` every
   table to add `uuid TEXT`; backfill in one pass; `UNIQUE` index on it. The app still reads and
   writes integer ids. Nothing user-visible changes, and the column can be dropped to roll back.
2. **Switch the reads.** Stores select and return the uuid as the public id; JSON ids become strings;
   the frontend's `number` id types become `string` and every `/api/.../{id}` keeps working because
   the handler looks up by uuid. Integer ids are still written for the old rows to keep the FKs
   valid — writes set both.
3. **Switch the writes and the FKs.** Children reference the parent's uuid; the mapping is no longer
   needed. At the end of this slice the integer columns are unused but still present.
4. **Contract.** Rebuild each table with `uuid` as `PRIMARY KEY`, drop the integer column, drop the
   `AUTOINCREMENT` habits and the last `Scan(&id int64)` sites.
5. **Tidy the schema.** Delete the duplicated migration tree (`migrations/` is a copy of
   `internal/db/migrations/`, which is the one the embed reads), and record the convention in
   `AGENTS.md`: every identity is a UUIDv7, every foreign key is its text.

Slices 1–3 are reversible by dropping a column; slice 4 is the one that cannot be undone after the
fact, so it ships last and alone.

## What this touches

- `internal/models` (`ID int64` → `string`), `internal/store`, `internal/api` (every `Scan(&id)` and
  every path parameter), `internal/trades`, `internal/runs`, `internal/ledger`, `internal/benchmark`,
  `internal/llm` (audit rows), `internal/trend`, and the tests that seed fixed ids.
- `web/src/api.ts` types, the views that build `/i/:symbol` and `/calls?...` links, and the decision
  actions that post a signal id.
- `plan_id` grouping: a plan groups signals by an id that today equals the first leg's integer. It
  becomes the first leg's uuid, which is already what the code means.

## Risks

- **Money writes.** `trades::record`, `trades::edit` and `approve_plan` are single transactions; the
  type change must not alter their shape. Slice 3 must run the trade tests and the flows in
  `docs/testing/flows.md` before merge.
- **Ordering.** Anything that sorts by id must sort by a v7, never a v4; a test asserts that two ids
  created a millisecond apart compare in insertion order.
- **Old databases.** The expand migration must be idempotent and must not touch rows whose uuid is
  already set, so a retried boot is safe.
- **JSON compatibility.** No external consumer exists today; the only client is the bundled frontend,
  which ships in the same binary, so the id type can change in one step without a version negotiation.
