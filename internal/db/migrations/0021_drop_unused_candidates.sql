-- The short-term scan used to create an instrument row for every ticker it looked at, and kept it
-- forever. A name that never produced a decision, a fill or a position is not part of the book: it
-- was a glance. This removes the glances left behind by older builds.
CREATE TEMP TABLE IF NOT EXISTS glance AS
SELECT i.id AS id, i.symbol AS symbol
  FROM instruments i
 WHERE i.active = 0
   AND i.archived_at IS NULL
   AND NOT EXISTS (SELECT 1 FROM signals   s WHERE s.instrument_id = i.id)
   AND NOT EXISTS (SELECT 1 FROM trades    t WHERE t.instrument_id = i.id)
   AND NOT EXISTS (SELECT 1 FROM positions p WHERE p.instrument_id = i.id);

DELETE FROM bars            WHERE instrument_id IN (SELECT id FROM glance);
DELETE FROM news            WHERE instrument_id IN (SELECT id FROM glance);
DELETE FROM llm_memos       WHERE instrument_id IN (SELECT id FROM glance);
DELETE FROM trend_candidates WHERE symbol IN (SELECT symbol FROM glance);
DELETE FROM instruments     WHERE id IN (SELECT id FROM glance);
DROP TABLE glance;
