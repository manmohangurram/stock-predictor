-- A plan is several moves that stand or fall together: every row carries the id of the first.
-- Rotation keeps using pair_id, so nothing already stored changes shape.
ALTER TABLE signals ADD COLUMN plan_id INTEGER;
CREATE INDEX IF NOT EXISTS signals_by_plan ON signals (plan_id);
