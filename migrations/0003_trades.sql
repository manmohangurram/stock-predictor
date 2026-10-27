-- The record of what was actually bought and sold. Positions are a projection of this,
-- ledger rows move the cash, and this table keeps the history forever.
CREATE TABLE IF NOT EXISTS trades (
    id            INTEGER PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    side          TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty           REAL NOT NULL,
    price         REAL NOT NULL,
    realised      REAL,
    traded_at     TEXT NOT NULL,
    note          TEXT
);

CREATE INDEX IF NOT EXISTS trades_by_instrument ON trades (instrument_id, traded_at);

-- Anything already held was bought at some point; record that so history is not blank.
INSERT INTO trades (instrument_id, side, qty, price, traded_at, note)
SELECT instrument_id, 'buy', qty, avg_cost, opened_at, 'backfilled from position'
FROM positions
WHERE NOT EXISTS (
    SELECT 1 FROM trades t WHERE t.instrument_id = positions.instrument_id
);
