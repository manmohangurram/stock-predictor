-- One admin, signed in with a password, holding a short access token and a long refresh token. The
-- table is additive: an existing database keeps every row it had and waits at the setup screen.

CREATE TABLE IF NOT EXISTS admin (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    id         TEXT PRIMARY KEY,
    admin_id   TEXT NOT NULL REFERENCES admin(id),
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    replaced_by TEXT,
    user_agent TEXT
);

CREATE INDEX IF NOT EXISTS refresh_tokens_live ON refresh_tokens (admin_id, revoked_at, expires_at);
