-- A decision is a proposal until you act on it. `pending` is what the ledger shows.
ALTER TABLE signals ADD COLUMN status TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE signals ADD COLUMN status_at TEXT;
ALTER TABLE signals ADD COLUMN status_reason TEXT;
-- What you actually got, which may differ from what was proposed.
ALTER TABLE signals ADD COLUMN filled_qty REAL;
ALTER TABLE signals ADD COLUMN filled_price REAL;

-- A rotation is two legs of one decision: the second row points at the first.
ALTER TABLE signals ADD COLUMN pair_id INTEGER REFERENCES signals(id);
ALTER TABLE signals ADD COLUMN kind TEXT;

CREATE INDEX IF NOT EXISTS idx_signals_status ON signals (status, created_at);
