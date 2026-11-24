-- The long-term prompt asks for "a confirmed trend: price against both averages", and the list
-- never carried one — so the model answered "no confirmed trend evidence" and passed every day.
-- The sweep already fetches a year of closes per name, so both averages cost no extra request.
ALTER TABLE plan_candidates ADD COLUMN dma50 REAL;
ALTER TABLE plan_candidates ADD COLUMN dma200 REAL;
