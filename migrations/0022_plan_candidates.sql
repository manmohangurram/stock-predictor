-- The long-term plan used to compare a holding against whatever happened to be in the book — which
-- is not a choice, it is the same names again. Today's list is now the Nifty 100's ten best
-- five-session performers, kept for the IST day so a retry does not pay for a hundred quotes twice.
CREATE TABLE IF NOT EXISTS plan_candidates (
  day      TEXT NOT NULL,
  symbol   TEXT NOT NULL,
  name     TEXT NOT NULL,
  close    REAL NOT NULL,
  ret5     REAL NOT NULL,              -- five-session return, as a fraction (0.0612 = +6.12%)
  added_at TEXT NOT NULL,
  PRIMARY KEY (day, symbol)
);
