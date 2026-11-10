-- The silence interval is gone: a call is for a change, never for the passage of time.
DELETE FROM settings WHERE key = 'max_silence_secs';
