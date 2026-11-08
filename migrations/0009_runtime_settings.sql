-- Knobs that used to be environment variables. Written here so an upgraded database reads the
-- same values a fresh one does, and so the rows are inspectable. `Runtime::from_map` tolerates
-- their absence too: correctness does not depend on this migration.
INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES
    ('poll_secs',           '900',   datetime('now')),
    ('off_hours_secs',      '7200',  datetime('now')),
    ('prompt_window_hours', '4',     datetime('now')),
    ('min_price_move',      '2',     datetime('now')),
    ('max_silence_secs',    '3600',  datetime('now')),
    ('daily_cap',           '200',   datetime('now'));

-- The deployment's own address is an environment variable now.
DELETE FROM settings WHERE key = 'public_url';
