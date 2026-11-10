-- The short-term list used to be whatever NSE's gainers endpoint happened to return this minute.
-- It is now asked once a day from three places — the model's reading of the web, the Nifty 50's
-- own movers, and the exchange gainers — because a symbol discovered by a search must survive the
-- tick it was discovered in, and a tick must not pay for the search again.
CREATE TABLE IF NOT EXISTS trend_candidates (
  day        TEXT NOT NULL,             -- IST day the list belongs to
  symbol     TEXT NOT NULL,             -- Yahoo form, e.g. TRANSWORLD.NS
  source     TEXT NOT NULL,             -- search | nifty50 | gainers
  why        TEXT NOT NULL DEFAULT '',  -- one line: why it is on the list
  ltp        REAL,
  change_pct REAL,
  previous_close REAL,
  turnover   REAL,
  added_at   TEXT NOT NULL,
  PRIMARY KEY (day, symbol)
);