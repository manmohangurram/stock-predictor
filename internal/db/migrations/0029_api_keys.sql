-- Access for an assistant, not a login: a key with a name, a scope and an end date. Only the hash
-- is stored, so a key can be shown once and never recovered. user_id is null today and binds a key
-- to a signed-in user when login lands.

CREATE TABLE IF NOT EXISTS api_keys (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    scope        TEXT NOT NULL CHECK (scope IN ('read', 'write')),
    key_hash     TEXT NOT NULL UNIQUE,
    prefix       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    expires_at   TEXT,
    last_used_at TEXT,
    revoked_at   TEXT,
    user_id      TEXT
);

CREATE INDEX IF NOT EXISTS api_keys_live ON api_keys (revoked_at, expires_at);

-- Every MCP tool call, append-only like api_calls: what an assistant did to the book, and when.
CREATE TABLE IF NOT EXISTS mcp_calls (
    id        TEXT PRIMARY KEY,
    ts        TEXT NOT NULL,
    key_id    TEXT NOT NULL REFERENCES api_keys(id),
    tool      TEXT NOT NULL,
    args_json TEXT NOT NULL DEFAULT '{}',
    status    TEXT NOT NULL,
    error     TEXT,
    ms        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS mcp_calls_by_key ON mcp_calls (key_id, ts DESC);
