-- A holding cannot be invisible: discovery creates instruments with active = 0, so an instrument
-- bought afterwards kept its position out of the ledger while its fill stayed in the history.
UPDATE instruments SET active = 1, archived_at = NULL
WHERE active = 0
  AND id IN (SELECT instrument_id FROM positions WHERE state <> 'closed' AND qty > 0);
