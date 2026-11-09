-- Which model conversation produced a decision, so Audit can show what happened to the call.
ALTER TABLE signals ADD COLUMN conversation_id TEXT;
CREATE INDEX IF NOT EXISTS idx_signals_conversation ON signals (conversation_id);
