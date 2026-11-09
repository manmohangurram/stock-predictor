-- Why the model was asked: a price move, a new headline, a rotation, a short-term scan.
ALTER TABLE api_calls ADD COLUMN trigger TEXT;

-- The audit list joins a call to the decision it produced by conversation.
CREATE INDEX IF NOT EXISTS idx_api_calls_conversation ON api_calls (conversation_id);
