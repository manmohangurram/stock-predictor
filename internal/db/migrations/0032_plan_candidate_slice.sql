-- Two offers, not one: the five-session movers the ranking is built from, and the names that are
-- barely moving. A list of only movers is a list the long-term mandate says to refuse.
ALTER TABLE plan_candidates ADD COLUMN slice TEXT NOT NULL DEFAULT 'momentum';
