-- Every index here answers a query the poll loop or a page load actually runs. Measured with
-- EXPLAIN QUERY PLAN: signals, news, positions and api_calls were all full scans.
CREATE INDEX IF NOT EXISTS signals_by_instrument ON signals (instrument_id, id DESC);
CREATE INDEX IF NOT EXISTS signals_by_mode ON signals (mode, id DESC);
CREATE INDEX IF NOT EXISTS news_by_instrument ON news (instrument_id, published_at DESC);
CREATE INDEX IF NOT EXISTS positions_by_instrument ON positions (instrument_id, mode, state);
CREATE INDEX IF NOT EXISTS api_calls_by_endpoint_ts ON api_calls (endpoint, ts);
