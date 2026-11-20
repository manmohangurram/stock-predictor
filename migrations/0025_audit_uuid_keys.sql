-- runs and api_calls take the uuid as their primary key. Their ids are published (the audit page
-- links to a call), nothing references them, and the integer rowid was only ever a counter. This
-- is the first table pair of the contract step; the rest follow once their relations move.

CREATE TABLE runs_new (
    id              TEXT PRIMARY KEY,
    prompt          TEXT NOT NULL,
    trigger         TEXT NOT NULL,
    status          TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    summary         TEXT,
    conversation_id TEXT,
    error           TEXT
);

INSERT INTO runs_new (id, prompt, trigger, status, started_at, finished_at, summary,
                      conversation_id, error)
SELECT COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       prompt, trigger, status, started_at, finished_at, summary, conversation_id, error
  FROM runs;

DROP TABLE runs;
ALTER TABLE runs_new RENAME TO runs;
CREATE INDEX IF NOT EXISTS runs_running ON runs (id) WHERE status = 'running';

CREATE TABLE api_calls_new (
    id                TEXT PRIMARY KEY,
    ts                TEXT NOT NULL,
    conversation_id   TEXT,
    round             INTEGER,
    endpoint          TEXT NOT NULL,
    purpose           TEXT,
    prompt_template   TEXT,
    prompt_hash       TEXT,
    request_json      TEXT,
    response_json     TEXT,
    http_status       INTEGER,
    prompt_tokens     INTEGER,
    completion_tokens INTEGER,
    latency_ms        INTEGER,
    validator_verdict TEXT,
    error             TEXT,
    trigger           TEXT
);

INSERT INTO api_calls_new (id, ts, conversation_id, round, endpoint, purpose, prompt_template,
                           prompt_hash, request_json, response_json, http_status, prompt_tokens,
                           completion_tokens, latency_ms, validator_verdict, error, trigger)
SELECT COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       ts, conversation_id, round, endpoint, purpose, prompt_template, prompt_hash,
       request_json, response_json, http_status, prompt_tokens, completion_tokens, latency_ms,
       validator_verdict, error, trigger
  FROM api_calls;

DROP TABLE api_calls;
ALTER TABLE api_calls_new RENAME TO api_calls;
CREATE INDEX IF NOT EXISTS api_calls_by_endpoint_ts ON api_calls (endpoint, ts);
