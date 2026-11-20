# Intent — MCP access with scoped, expiring API keys

Confirmed 2026-11-20.

- **Outcome:** the app serves an MCP endpoint at `POST /mcp`, authenticated with
  `Authorization: Bearer <key>`, so a remote AI client can read the book — and, with the right key,
  act on it.
- **User:** the owner connecting an assistant to their own self-hosted instance. A signed-in user
  owns keys later; for now a key is app-level.
- **Why now:** the assistant should answer "what's in the book" and do the work, not have numbers
  copied in and out by hand.
- **Success:** a key minted on a new Settings → Access tab (name, scope, lifetime in days or
  "never"), used as a bearer token, connects a client that lists and calls tools; a revoked or
  expired key gets `401`; the plaintext is shown exactly once; the web UI and `/api/...` behave
  exactly as today.
- **Scope of a key:** `read` sees positions, decisions, trades, instruments, calls, runs, market and
  schedule; `write` (which implies read) also approves/rejects, records and corrects fills, adds and
  restores instruments, and writes settings — including the provider key, the ntfy token and the
  proxy password, written but never read back.
- **Constraint:** `/mcp` only — no REST auth and no login yet. Keys are stored hashed and cannot be
  recovered; MCP cannot mint, list or read keys; every tool call is recorded. `api_keys.user_id` is
  nullable so login can bind existing keys later.
- **Out of scope:** user login, sessions, multi-user data separation, REST authentication, key
  rotation policy, scheduler and prompt changes.
