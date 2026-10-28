-- How much profit the owner expects from this instrument, in percent. A decision whose
-- take-profit does not reach it is rejected, so the app cannot quietly recommend a 4% trade
-- on a holding that is meant to make 20%.
ALTER TABLE instruments ADD COLUMN target_profit_pct REAL;
