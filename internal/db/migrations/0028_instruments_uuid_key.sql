-- The hub. instruments takes the uuid as its primary key and every instrument_id in the database
-- becomes text in the same pass, because a foreign key to a text primary key must be text. A
-- temporary map carries the old integer to the new id; children join through it.
--
-- Foreign keys are off for the rewrite: notifications.signal_id still points at signals, which is
-- rebuilt here because it is a child of instruments. The pragma must sit outside the transaction,
-- so it comes first, and the whole rewrite runs in one so a failure leaves no half-built table.

PRAGMA foreign_keys = OFF;
BEGIN;

DROP TABLE IF EXISTS bars_new;
DROP TABLE IF EXISTS news_new;
DROP TABLE IF EXISTS signals_new;
DROP TABLE IF EXISTS positions_new;
DROP TABLE IF EXISTS trades_new;
DROP TABLE IF EXISTS ipo_applications_new;
DROP TABLE IF EXISTS search_results_new;
DROP TABLE IF EXISTS llm_memos_new;
DROP TABLE IF EXISTS cash_ledger_new;
DROP TABLE IF EXISTS instruments_new;

CREATE TEMP TABLE instrument_map AS
SELECT id AS old_id,
       COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))) AS new_id
  FROM instruments;

CREATE TABLE instruments_new (
    id                TEXT PRIMARY KEY,
    symbol            TEXT NOT NULL UNIQUE,
    class             TEXT NOT NULL,
    name              TEXT NOT NULL,
    currency          TEXT NOT NULL,
    active            INTEGER NOT NULL DEFAULT 1,
    target_profit_pct REAL,
    archived_at       TEXT
);

INSERT INTO instruments_new (id, symbol, class, name, currency, active, target_profit_pct, archived_at)
SELECT m.new_id, i.symbol, i.class, i.name, i.currency, i.active, i.target_profit_pct, i.archived_at
  FROM instruments i JOIN instrument_map m ON m.old_id = i.id;

CREATE TABLE bars_new (
    instrument_id TEXT NOT NULL REFERENCES instruments_new(id),
    ts            TEXT NOT NULL,
    open          REAL,
    high          REAL,
    low           REAL,
    close         REAL NOT NULL,
    volume        INTEGER,
    PRIMARY KEY (instrument_id, ts)
);
INSERT INTO bars_new (instrument_id, ts, open, high, low, close, volume)
SELECT m.new_id, b.ts, b.open, b.high, b.low, b.close, b.volume
  FROM bars b JOIN instrument_map m ON m.old_id = b.instrument_id;

CREATE TABLE news_new (
    id            TEXT PRIMARY KEY,
    instrument_id TEXT REFERENCES instruments_new(id),
    published_at  TEXT NOT NULL,
    title         TEXT NOT NULL,
    publisher     TEXT,
    url           TEXT NOT NULL UNIQUE,
    body          TEXT,
    score         REAL
);
INSERT INTO news_new (id, instrument_id, published_at, title, publisher, url, body, score)
SELECT n.id, (SELECT m.new_id FROM instrument_map m WHERE m.old_id = n.instrument_id),
       n.published_at, n.title, n.publisher, n.url, n.body, n.score
  FROM news n;

CREATE TABLE signals_new (
    id              TEXT PRIMARY KEY,
    instrument_id   TEXT NOT NULL REFERENCES instruments_new(id),
    action          TEXT NOT NULL,
    qty             REAL,
    entry           REAL,
    stop            REAL,
    target          REAL,
    valid_until     TEXT,
    reason_json     TEXT,
    created_at      TEXT NOT NULL,
    mode            TEXT NOT NULL DEFAULT 'long',
    confidence      TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    status_at       TEXT,
    status_reason   TEXT,
    filled_qty      REAL,
    filled_price    REAL,
    pair_id         TEXT REFERENCES signals_new(id),
    kind            TEXT,
    conversation_id TEXT,
    plan_id         TEXT
);
INSERT INTO signals_new (id, instrument_id, action, qty, entry, stop, target, valid_until,
                         reason_json, created_at, mode, confidence, status, status_at,
                         status_reason, filled_qty, filled_price, pair_id, kind,
                         conversation_id, plan_id)
SELECT s.id, (SELECT m.new_id FROM instrument_map m WHERE m.old_id = s.instrument_id),
       s.action, s.qty, s.entry, s.stop, s.target, s.valid_until, s.reason_json, s.created_at,
       s.mode, s.confidence, s.status, s.status_at, s.status_reason, s.filled_qty,
       s.filled_price, s.pair_id, s.kind, s.conversation_id, s.plan_id
  FROM signals s;

CREATE TABLE positions_new (
    id            TEXT PRIMARY KEY,
    instrument_id TEXT NOT NULL REFERENCES instruments_new(id),
    qty           REAL NOT NULL,
    avg_cost      REAL NOT NULL,
    opened_at     TEXT NOT NULL,
    stop          REAL,
    target        REAL,
    state         TEXT NOT NULL,
    mode          TEXT NOT NULL DEFAULT 'long'
);
INSERT INTO positions_new (id, instrument_id, qty, avg_cost, opened_at, stop, target, state, mode)
SELECT p.id, (SELECT m.new_id FROM instrument_map m WHERE m.old_id = p.instrument_id),
       p.qty, p.avg_cost, p.opened_at, p.stop, p.target, p.state, p.mode
  FROM positions p;

CREATE TABLE trades_new (
    id            TEXT PRIMARY KEY,
    instrument_id TEXT NOT NULL REFERENCES instruments_new(id),
    side          TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty           REAL NOT NULL,
    price         REAL NOT NULL,
    realised      REAL,
    traded_at     TEXT NOT NULL,
    note          TEXT,
    mode          TEXT NOT NULL DEFAULT 'long'
);
INSERT INTO trades_new (id, instrument_id, side, qty, price, realised, traded_at, note, mode)
SELECT t.id, (SELECT m.new_id FROM instrument_map m WHERE m.old_id = t.instrument_id),
       t.side, t.qty, t.price, t.realised, t.traded_at, t.note, t.mode
  FROM trades t;

CREATE TABLE ipo_applications_new (
    id            TEXT PRIMARY KEY,
    instrument_id TEXT NOT NULL REFERENCES instruments_new(id),
    qty           REAL NOT NULL,
    amount        REAL NOT NULL,
    status        TEXT NOT NULL,
    applied_at    TEXT NOT NULL
);
INSERT INTO ipo_applications_new (id, instrument_id, qty, amount, status, applied_at)
SELECT COALESCE(a.uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       (SELECT m.new_id FROM instrument_map m WHERE m.old_id = a.instrument_id),
       a.qty, a.amount, a.status, a.applied_at
  FROM ipo_applications a;

CREATE TABLE search_results_new (
    id            TEXT PRIMARY KEY,
    ts            TEXT NOT NULL,
    instrument_id TEXT REFERENCES instruments_new(id),
    query         TEXT NOT NULL,
    provider      TEXT NOT NULL,
    raw_json      TEXT NOT NULL
);
INSERT INTO search_results_new (id, ts, instrument_id, query, provider, raw_json)
SELECT COALESCE(r.uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       r.ts, (SELECT m.new_id FROM instrument_map m WHERE m.old_id = r.instrument_id),
       r.query, r.provider, r.raw_json
  FROM search_results r;

CREATE TABLE llm_memos_new (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments_new(id),
    headline_fp   TEXT NOT NULL,
    decided_at    TEXT NOT NULL,
    last_price    REAL,
    last_call_at  TEXT
);
INSERT INTO llm_memos_new (instrument_id, headline_fp, decided_at, last_price, last_call_at)
SELECT (SELECT m.new_id FROM instrument_map m WHERE m.old_id = l.instrument_id),
       l.headline_fp, l.decided_at, l.last_price, l.last_call_at
  FROM llm_memos l;

CREATE TABLE cash_ledger_new (
    id     TEXT PRIMARY KEY,
    ts     TEXT NOT NULL,
    kind   TEXT NOT NULL,
    amount REAL NOT NULL,
    ref    TEXT,
    mode   TEXT NOT NULL DEFAULT 'long',
    bucket TEXT
);
INSERT INTO cash_ledger_new (id, ts, kind, amount, ref, mode, bucket)
SELECT COALESCE(c.uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       c.ts, c.kind, c.amount, c.ref, c.mode, c.bucket
  FROM cash_ledger c;

DROP TABLE bars;
DROP TABLE news;
DROP TABLE signals;
DROP TABLE positions;
DROP TABLE trades;
DROP TABLE ipo_applications;
DROP TABLE search_results;
DROP TABLE llm_memos;
DROP TABLE cash_ledger;

ALTER TABLE bars_new RENAME TO bars;
ALTER TABLE news_new RENAME TO news;
ALTER TABLE signals_new RENAME TO signals;
ALTER TABLE positions_new RENAME TO positions;
ALTER TABLE trades_new RENAME TO trades;
ALTER TABLE ipo_applications_new RENAME TO ipo_applications;
ALTER TABLE search_results_new RENAME TO search_results;
ALTER TABLE llm_memos_new RENAME TO llm_memos;
ALTER TABLE cash_ledger_new RENAME TO cash_ledger;

DROP TABLE instruments;
ALTER TABLE instruments_new RENAME TO instruments;

CREATE UNIQUE INDEX IF NOT EXISTS instruments_symbol_nocase ON instruments (symbol COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS news_by_instrument ON news (instrument_id, published_at DESC);
CREATE INDEX IF NOT EXISTS signals_by_instrument ON signals (instrument_id, id DESC);
CREATE INDEX IF NOT EXISTS signals_by_mode ON signals (mode, id DESC);
CREATE INDEX IF NOT EXISTS signals_by_plan ON signals (plan_id);
CREATE INDEX IF NOT EXISTS idx_signals_conversation ON signals (conversation_id);
CREATE INDEX IF NOT EXISTS positions_by_instrument ON positions (instrument_id, mode, state);
CREATE INDEX IF NOT EXISTS positions_by_mode ON positions (mode, state);
CREATE INDEX IF NOT EXISTS trades_by_mode ON trades (mode, traded_at);
CREATE INDEX IF NOT EXISTS trades_by_instrument ON trades (instrument_id, traded_at);
CREATE INDEX IF NOT EXISTS idx_cash_ledger_bucket ON cash_ledger (mode, bucket);

DROP TABLE instrument_map;

COMMIT;
PRAGMA foreign_keys = ON;
