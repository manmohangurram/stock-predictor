-- One table, four resolutions. Existing rows keep the default and every current query is unchanged;
-- the conversion to buckets ships with the reader that understands them, not before it.
ALTER TABLE bars ADD COLUMN resolution TEXT NOT NULL DEFAULT 'daily';
CREATE INDEX IF NOT EXISTS bars_by_resolution ON bars (instrument_id, resolution, ts);
