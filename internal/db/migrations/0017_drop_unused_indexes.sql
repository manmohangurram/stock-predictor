-- Three indexes that no query can use. Checked with EXPLAIN QUERY PLAN against every statement
-- the app runs: the audit list, the ledger, the poll and the plan approval.
--
--   runs_by_started          recent() orders by id DESC; running() counts by status.
--   idx_api_calls_conversation  only `signals.conversation_id` is ever filtered; calls are read
--                               by id, or walked backwards on the primary key.
--   idx_signals_status       every status predicate is paired with a mode (signals_by_mode) or
--                            with an id (the primary key).
DROP INDEX IF EXISTS runs_by_started;
DROP INDEX IF EXISTS idx_api_calls_conversation;
DROP INDEX IF EXISTS idx_signals_status;

-- In its place, a partial index that answers the one question `runs` is asked on every audit
-- page: is anything running? Only running rows are in it, so it stays tiny as the table grows.
-- Measured at 36,000 runs: 1.21 ms without it, 0.011 ms with.
CREATE INDEX IF NOT EXISTS runs_running ON runs (id) WHERE status = 'running';
