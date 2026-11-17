-- The model states how sure it is. It was being parsed and then dropped; store it so the UI can
-- show it. NULL means the signal came from the rules alone, with no model judgement behind it.
ALTER TABLE signals ADD COLUMN confidence TEXT;
