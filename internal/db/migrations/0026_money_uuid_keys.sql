-- trades, positions and news take the uuid as their primary key. Nothing outside the app
-- references any of them (notifications point at signals, not trades), so the three rebuilds are
-- self-contained; the instrument reference stays integer until the hub moves.

CREATE TABLE trades_new (
    id            TEXT PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    side          TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
    qty           REAL NOT NULL,
    price         REAL NOT NULL,
    realised      REAL,
    traded_at     TEXT NOT NULL,
    note          TEXT,
    mode          TEXT NOT NULL DEFAULT 'long'
);

INSERT INTO trades_new (id, instrument_id, side, qty, price, realised, traded_at, note, mode)
SELECT COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       instrument_id, side, qty, price, realised, traded_at, note, mode
  FROM trades;

DROP TABLE trades;
ALTER TABLE trades_new RENAME TO trades;
CREATE INDEX IF NOT EXISTS trades_by_mode ON trades (mode, traded_at);
CREATE INDEX IF NOT EXISTS trades_by_instrument ON trades (instrument_id, traded_at);

CREATE TABLE positions_new (
    id            TEXT PRIMARY KEY,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    qty           REAL NOT NULL,
    avg_cost      REAL NOT NULL,
    opened_at     TEXT NOT NULL,
    stop          REAL,
    target        REAL,
    state         TEXT NOT NULL,
    mode          TEXT NOT NULL DEFAULT 'long'
);

INSERT INTO positions_new (id, instrument_id, qty, avg_cost, opened_at, stop, target, state, mode)
SELECT COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       instrument_id, qty, avg_cost, opened_at, stop, target, state, mode
  FROM positions;

DROP TABLE positions;
ALTER TABLE positions_new RENAME TO positions;
CREATE INDEX IF NOT EXISTS positions_by_instrument ON positions (instrument_id, mode, state);
CREATE INDEX IF NOT EXISTS positions_by_mode ON positions (mode, state);

CREATE TABLE news_new (
    id            TEXT PRIMARY KEY,
    instrument_id INTEGER REFERENCES instruments(id),
    published_at  TEXT NOT NULL,
    title         TEXT NOT NULL,
    publisher     TEXT,
    url           TEXT NOT NULL UNIQUE,
    body          TEXT,
    score         REAL
);

INSERT INTO news_new (id, instrument_id, published_at, title, publisher, url, body, score)
SELECT COALESCE(uuid, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
                     || substr(hex(randomblob(2)), 2) || '-'
                     || substr('89ab', abs(random()) % 4 + 1, 1)
                     || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6)))),
       instrument_id, published_at, title, publisher, url, body, score
  FROM news;

DROP TABLE news;
ALTER TABLE news_new RENAME TO news;
CREATE INDEX IF NOT EXISTS news_by_instrument ON news (instrument_id, published_at DESC);
