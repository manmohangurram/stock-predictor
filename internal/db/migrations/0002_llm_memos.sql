-- One row per instrument: the headline fingerprint the model last answered for. Prices may
-- move freely in between; only new headlines justify another call.
CREATE TABLE IF NOT EXISTS llm_memos (
    instrument_id INTEGER PRIMARY KEY REFERENCES instruments(id),
    headline_fp   TEXT NOT NULL,
    decided_at    TEXT NOT NULL
);
