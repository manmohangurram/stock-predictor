-- signals take the uuid as their primary key, and the two references to them move with it:
-- signals.plan_id, signals.pair_id and notifications.signal_id. A temporary map holds the old
-- integer to the new text id so every reference is rewritten in one pass.
--
-- Foreign keys are off for the rewrite: notifications.signal_id references signals, and SQLite
-- refuses to drop a table something points at. The pragma must sit outside the transaction, so it
-- comes first, and the whole rewrite runs in one so a failure leaves no half-built table behind.

PRAGMA foreign_keys = OFF;
BEGIN;

DROP TABLE IF EXISTS signals_new;
DROP TABLE IF EXISTS notifications_new;

CREATE TEMP TABLE signal_map AS
SELECT id AS old_id,
       COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))) AS new_id
  FROM signals;

CREATE TABLE signals_new (
    id              TEXT PRIMARY KEY,
    instrument_id   INTEGER NOT NULL REFERENCES instruments(id),
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
SELECT m.new_id, s.instrument_id, s.action, s.qty, s.entry, s.stop, s.target, s.valid_until,
       s.reason_json, s.created_at, s.mode, s.confidence, s.status, s.status_at,
       s.status_reason, s.filled_qty, s.filled_price,
       (SELECT m2.new_id FROM signal_map m2 WHERE m2.old_id = s.pair_id),
       s.kind, s.conversation_id,
       (SELECT m3.new_id FROM signal_map m3 WHERE m3.old_id = s.plan_id)
  FROM signals s JOIN signal_map m ON m.old_id = s.id;

DROP TABLE signals;
ALTER TABLE signals_new RENAME TO signals;
CREATE INDEX IF NOT EXISTS signals_by_instrument ON signals (instrument_id, id DESC);
CREATE INDEX IF NOT EXISTS signals_by_mode ON signals (mode, id DESC);
CREATE INDEX IF NOT EXISTS signals_by_plan ON signals (plan_id);
CREATE INDEX IF NOT EXISTS idx_signals_conversation ON signals (conversation_id);

CREATE TABLE notifications_new (
    id         TEXT PRIMARY KEY,
    signal_id  TEXT REFERENCES signals(id),
    sent_at    TEXT NOT NULL,
    channel    TEXT NOT NULL,
    dedupe_key TEXT NOT NULL UNIQUE
);

INSERT INTO notifications_new (id, signal_id, sent_at, channel, dedupe_key)
SELECT COALESCE(n.uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       (SELECT m.new_id FROM signal_map m WHERE m.old_id = n.signal_id),
       n.sent_at, n.channel, n.dedupe_key
  FROM notifications n;

DROP TABLE notifications;
ALTER TABLE notifications_new RENAME TO notifications;

DROP TABLE signal_map;

COMMIT;
PRAGMA foreign_keys = ON;
