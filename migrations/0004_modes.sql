-- Two strategies, two pots. Every money row belongs to exactly one mode, and the two never
-- draw on each other: a swing trade cannot block a long-term buy, or vice versa.
ALTER TABLE cash_ledger ADD COLUMN mode TEXT NOT NULL DEFAULT 'long';
ALTER TABLE positions   ADD COLUMN mode TEXT NOT NULL DEFAULT 'long';
ALTER TABLE trades      ADD COLUMN mode TEXT NOT NULL DEFAULT 'long';
ALTER TABLE signals     ADD COLUMN mode TEXT NOT NULL DEFAULT 'long';

CREATE INDEX IF NOT EXISTS positions_by_mode ON positions (mode, state);
CREATE INDEX IF NOT EXISTS trades_by_mode ON trades (mode, traded_at);
