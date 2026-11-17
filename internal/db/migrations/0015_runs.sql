-- A run is one scheduled (or manual) piece of work: a poll, a book plan, a short-term pass.
-- It is opened before the work starts, so the audit page can show "running", and closed with a
-- summary when it ends. api_calls remain the per-call record; a run is the box around them.
CREATE TABLE IF NOT EXISTS runs (
    id              INTEGER PRIMARY KEY,
    prompt          TEXT NOT NULL,          -- poll | plan_stock | plan_fund | short
    trigger         TEXT NOT NULL,          -- schedule | change | manual
    status          TEXT NOT NULL,          -- running | done | failed
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    summary         TEXT,                   -- one line a person can read
    conversation_id TEXT,                   -- the plan's call, when there was one
    error           TEXT
);

CREATE INDEX IF NOT EXISTS runs_by_started ON runs (started_at DESC);
