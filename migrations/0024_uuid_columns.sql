-- Expand: every row gets a uuid beside its integer key. Nothing reads it yet — the columns are
-- nullable and this slice rolls back by dropping one column per table — but from here on the
-- identity exists and the backfill in db.Migrate fills it oldest-row-first.

ALTER TABLE instruments ADD COLUMN uuid TEXT;
ALTER TABLE news ADD COLUMN uuid TEXT;
ALTER TABLE signals ADD COLUMN uuid TEXT;
ALTER TABLE positions ADD COLUMN uuid TEXT;
ALTER TABLE cash_ledger ADD COLUMN uuid TEXT;
ALTER TABLE ipo_applications ADD COLUMN uuid TEXT;
ALTER TABLE notifications ADD COLUMN uuid TEXT;
ALTER TABLE api_calls ADD COLUMN uuid TEXT;
ALTER TABLE search_results ADD COLUMN uuid TEXT;
ALTER TABLE trades ADD COLUMN uuid TEXT;
ALTER TABLE runs ADD COLUMN uuid TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS instruments_uuid ON instruments (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS news_uuid ON news (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS signals_uuid ON signals (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS positions_uuid ON positions (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS cash_ledger_uuid ON cash_ledger (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS ipo_applications_uuid ON ipo_applications (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS notifications_uuid ON notifications (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS api_calls_uuid ON api_calls (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS search_results_uuid ON search_results (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS trades_uuid ON trades (uuid);
CREATE UNIQUE INDEX IF NOT EXISTS runs_uuid ON runs (uuid);
