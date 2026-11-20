-- One instrument per symbol. A symbol typed in a different case ("infy.ns") was a second row with
-- the same name and its own id: the oldest row wins, every child row moves onto it, the extra goes.

CREATE TEMP TABLE symbol_merge AS
SELECT i.id AS drop_id,
       (SELECT MIN(k.id) FROM instruments k
         WHERE upper(trim(k.symbol)) = upper(trim(i.symbol))) AS keep_id
  FROM instruments i;

DELETE FROM symbol_merge WHERE drop_id = keep_id;

UPDATE instruments SET active = 1
 WHERE id IN (SELECT keep_id FROM symbol_merge)
   AND EXISTS (SELECT 1 FROM instruments d
                WHERE d.active = 1
                  AND d.id IN (SELECT m.drop_id FROM symbol_merge m WHERE m.keep_id = instruments.id));

INSERT OR IGNORE INTO bars (instrument_id, ts, open, high, low, close, volume)
SELECT (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = b.instrument_id),
       b.ts, b.open, b.high, b.low, b.close, b.volume
  FROM bars b
 WHERE b.instrument_id IN (SELECT drop_id FROM symbol_merge);
DELETE FROM bars WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

INSERT OR IGNORE INTO llm_memos (instrument_id, headline_fp, decided_at, last_price, last_call_at)
SELECT (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = l.instrument_id),
       l.headline_fp, l.decided_at, l.last_price, l.last_call_at
  FROM llm_memos l
 WHERE l.instrument_id IN (SELECT drop_id FROM symbol_merge);
DELETE FROM llm_memos WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE news SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = news.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE signals SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = signals.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE positions SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = positions.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE trades SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = trades.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE ipo_applications SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = ipo_applications.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

UPDATE search_results SET instrument_id =
  (SELECT m.keep_id FROM symbol_merge m WHERE m.drop_id = search_results.instrument_id)
 WHERE instrument_id IN (SELECT drop_id FROM symbol_merge);

DELETE FROM instruments WHERE id IN (SELECT drop_id FROM symbol_merge);

UPDATE instruments SET symbol = upper(trim(symbol));

CREATE UNIQUE INDEX IF NOT EXISTS instruments_symbol_nocase ON instruments (symbol COLLATE NOCASE);

DROP TABLE symbol_merge;
