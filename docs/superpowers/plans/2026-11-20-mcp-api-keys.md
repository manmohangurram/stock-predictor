# MCP access with scoped API keys — plan

Spec: `docs/superpowers/specs/2026-11-20-mcp-api-keys-design.md`. One PR per task, each green on
its own; the tasks are a stack, task 2 based on task 1, the bottom one on `main`.

## Task 1 — the key, and the page that issues it

- Migration `0029_api_keys`: `api_keys` and `mcp_calls` (columns in the spec), indexes on
  `key_hash` (unique), `(revoked_at, expires_at)`.
- `internal/apikeys`: `New(scope, name, days) (token string, key Row, err)`; `Verify(token) (*Row,
  error)` returning `ErrUnknown` / `ErrExpired` / `ErrRevoked`; `List()`, `Revoke(id)`,
  `Touch(id)` (at most once a minute).
- Token: 32 bytes, base64url, `spk_` prefix; store `sha256` hex and the 8-char display prefix.
- API: `GET /api/keys`, `POST /api/keys {name, scope, days|null}`, `DELETE /api/keys/:id`.
  The create response carries the plaintext once; list never does.
- UI: a seventh Settings tab, `/settings/access` — table of keys (name, scope, created, expires,
  last used, prefix), a New-key dialog with name, scope and lifetime (1/7/30/90/365 days, never),
  a one-time reveal panel, and a two-step Revoke.
- Tests: mint → verify; expired → `ErrExpired`; revoked → `ErrRevoked`; the plaintext is not in the
  database; the create response is the only place it appears; revoke keeps the row.

## Task 2 — `/mcp`, read tools, and the audit row

- `internal/mcpserver`: builds a `*mcp.Server` for one key's scope, registers the eight read tools,
  each a thin call into the same code the REST handlers use (not a second SQL path).
- `POST /mcp` on the router, behind `apikeys` middleware: bearer → row → context; `401` with
  `WWW-Authenticate` on failure; the SDK's `NewStreamableHTTPHandler` in stateless JSON mode.
- One `mcp_calls` row per `tools/call`, with the arguments (redacted) and the outcome.
- Tests with the SDK's in-memory transport: `tools/list` for a read key contains exactly the read
  tools; `get_dashboard` returns the book; a revoked key is refused; an unknown tool is a JSON-RPC
  error.

## Task 3 — write tools

- Eight write tools from the spec, each calling the existing handler logic; `write` scope required.
- A read key's catalogue must not list them, and a direct `tools/call` on one is `403`.
- `update_settings` reuses the settings patch builder and never returns a secret; the audit row
  redacts the three secret fields.
- Tests: approve a pending decision over MCP and see the position move; record a fill; a read key
  refused; `update_settings` writes `poll_secs` and refuses an invalid value; no secret appears in
  any response.

## Task 4 — the README and the release

- A README section: how to create a key, the endpoint URL, the two scopes, what expiry means, and
  the warning that a `write` key can move money.
- `CHANGELOG.md` entry; release as a minor (`v0.33.0`), because a new surface and a new table land.

## Verification for every task

`go build ./...`, `gofmt -l cmd internal`, `go vet ./...`, `go test ./...` (no network), and
`cd web && npx vitest run && npm run build`. Task 2 and 3 also get an MCP client test against the
real handler, and the browser walkthrough of the Access tab is added to `docs/testing/flows.md`.
