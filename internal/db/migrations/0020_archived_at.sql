-- Discovery and archiving both wrote `active = 0`, so the Archived page could not tell a name the
-- short-term scan noticed from a holding the owner put away — and every discovered ticker appeared
-- there as if it had been archived. The date says which is which; it is null for never-archived.
ALTER TABLE instruments ADD COLUMN archived_at TEXT;

-- Backfill: a discovered name has no fills and no decisions. Anything with history was put away by
-- a person, so it keeps its place on the page.
UPDATE instruments
   SET archived_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
 WHERE active = 0
   AND (EXISTS (SELECT 1 FROM trades t WHERE t.instrument_id = instruments.id)
        OR EXISTS (SELECT 1 FROM signals s WHERE s.instrument_id = instruments.id));
