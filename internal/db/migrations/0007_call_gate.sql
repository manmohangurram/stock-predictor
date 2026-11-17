-- When the model was last consulted about an instrument, and at what price. Used to skip
-- pointless calls when nothing moved, while guaranteeing a call at least hourly.
ALTER TABLE llm_memos ADD COLUMN last_price REAL;
ALTER TABLE llm_memos ADD COLUMN last_call_at TEXT;
