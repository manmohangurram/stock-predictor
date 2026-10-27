CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS instruments (
    id       INTEGER PRIMARY KEY,
    symbol   TEXT NOT NULL UNIQUE,
    class    TEXT NOT NULL,
    name     TEXT NOT NULL,
    currency TEXT NOT NULL,
    active   INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS bars (
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    ts            TEXT NOT NULL,
    open          REAL,
    high          REAL,
    low           REAL,
    close         REAL NOT NULL,
    volume        INTEGER,
    PRIMARY KEY (instrument_id, ts)
);

CREATE TABLE IF NOT EXISTS news (
    id            INTEGER PRIMARY KEY,
    instrument_id INTEGER REFERENCES instruments(id),
    published_at  TEXT NOT NULL,
    title         TEXT NOT NULL,
    publisher     TEXT,
    url           TEXT NOT NULL UNIQUE,
    body          TEXT,
    score         REAL
);

CREATE TABLE IF NOT EXISTS signals (
    id            INTEGER PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    action        TEXT NOT NULL,
    entry         REAL,
    stop          REAL,
    target        REAL,
    valid_until   TEXT,
    reason_json   TEXT,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS positions (
    id            INTEGER PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    qty           REAL NOT NULL,
    avg_cost      REAL NOT NULL,
    opened_at     TEXT NOT NULL,
    stop          REAL,
    target        REAL,
    state         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cash_ledger (
    id     INTEGER PRIMARY KEY,
    ts     TEXT NOT NULL,
    kind   TEXT NOT NULL,
    amount REAL NOT NULL,
    ref    TEXT
);

CREATE TABLE IF NOT EXISTS ipo_applications (
    id            INTEGER PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    qty           REAL NOT NULL,
    amount        REAL NOT NULL,
    status        TEXT NOT NULL,
    applied_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS notifications (
    id         INTEGER PRIMARY KEY,
    signal_id  INTEGER REFERENCES signals(id),
    sent_at    TEXT NOT NULL,
    channel    TEXT NOT NULL,
    dedupe_key TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS api_calls (
    id                INTEGER PRIMARY KEY,
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
    error             TEXT
);

CREATE TABLE IF NOT EXISTS search_results (
    id            INTEGER PRIMARY KEY,
    ts            TEXT NOT NULL,
    instrument_id INTEGER REFERENCES instruments(id),
    query         TEXT NOT NULL,
    provider      TEXT NOT NULL,
    raw_json      TEXT NOT NULL
);
