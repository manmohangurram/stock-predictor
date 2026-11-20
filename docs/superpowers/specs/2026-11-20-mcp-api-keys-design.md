# MCP access with scoped, expiring API keys — design

Date: 2026-11-20. Status: proposed. Intent: `docs/intent/mcp-api-keys.md` (confirmed 2026-11-20).

## Why

The app is a self-hosted book: a person reads it through the browser, and an assistant can only
see it if the numbers are copied out by hand. MCP is the standard way to give an assistant tools
instead of a wall of text. The book is money, so access has to be a key with a name, a scope and an
end date — not a password shared with a chat window.

## Decision

**A stateless Streamable HTTP MCP endpoint at `POST /mcp`, authenticated by a bearer API key.**

- **Transport:** `github.com/modelcontextprotocol/go-sdk` v1.8.0, `NewStreamableHTTPHandler` with
  `Stateless: true, JSONResponse: true`. No session table, no server→client requests: every POST is
  self-contained, which is the only shape that survives a restart of a one-binary app.
- **Auth:** `Authorization: Bearer <token>`. Missing, malformed, expired or revoked → `401` with
  `WWW-Authenticate: Bearer`. Keys are app-level today; `api_keys.user_id` is nullable so a later
  login can bind them without a migration.
- **Scope:** `read` and `write`. `write` implies `read`. The scope picks which tools the server
  exposes at all — a read key's `tools/list` does not mention the write tools, so the assistant
  cannot ask for one.
- **Lifetime:** days, from 1 to any number, or never (`expires_at IS NULL`). Chosen at creation.
- **Storage:** only `sha256(token)` is stored, plus a display prefix. The plaintext is returned
  once, by the create call, and never again.
- **No key minting over MCP.** A key cannot create, list, read or revoke keys. Only the browser's
  Settings → Access tab can, so no token can extend itself.

## Tools

Read (scope `read`):

| Tool | Input | Returns |
|---|---|---|
| `get_dashboard` | `mode`: long \| trade | the page's payload: figures, positions, decisions, recent fills |
| `list_positions` | `mode` | holdings with qty, cost, value, P&L, exit plan |
| `list_decisions` | `mode`, `status` | proposals and their state |
| `list_trades` | `symbol?`, `limit` | fills, newest first |
| `get_instrument` | `symbol` | analysis: price series, indicators, exit plan, headlines |
| `list_archived` | — | archived instruments with their totals |
| `get_market` | — | market state, next open, schedule, running runs |
| `list_runs` | `limit` | recent runs and their summaries |

Write (scope `write`), each mapping onto an existing handler's transaction, not a second path:

| Tool | Input | Effect |
|---|---|---|
| `approve_decision` | `id`, `qty?`, `price?`, `at?`, `fills?` | same as `POST /api/signals/:id/approve` |
| `reject_decision` | `id`, `reason?` | same as `POST /api/signals/:id/reject` |
| `record_trade` | `symbol`, `side`, `qty`, `price`, `at?`, `note?`, `mode?` | `trades.Record`, one transaction |
| `edit_trade` | `id`, `qty?`, `price?`, `at?` | `trades.Edit` |
| `add_instrument` | `code`, `name?`, `class?` | the add path, including its Indian-listing refusal |
| `restore_instrument` | `symbol` | the restore path |
| `update_settings` | any subset of the settings patch | the same `PUT /api/settings` validation |
| `set_target_profit` | `symbol`, `pct` | the same `PUT /api/instruments/:symbol/target-profit` |

**Secrets are write-only.** `update_settings` may set the provider key, the ntfy token and the proxy
password; no tool and no endpoint returns them. The settings page already behaves this way — the
MCP path reuses its validation and its patch builder rather than inventing a second write.

Out of scope for v1: starting a manual run or a plan call (they spend model budget and write plan
rows), deleting a trade, deleting or archiving an instrument, minting keys.

## Data

`api_keys`

| Column | Notes |
|---|---|
| `id` | UUIDv7 text, as everywhere |
| `name` | what it is for, shown in the list |
| `scope` | `read` \| `write` |
| `key_hash` | `sha256` of the token, hex; unique |
| `prefix` | first 8 characters, so a key can be recognised without storing it |
| `created_at` | |
| `expires_at` | NULL = never |
| `last_used_at` | NULL until the first accepted call |
| `revoked_at` | NULL while live; revocation is a timestamp, never a delete |
| `user_id` | NULL today; owner once login lands |

`mcp_calls` (append-only, like `api_calls`; the newest 500 rows are kept and older ones are pruned after every insert)

| Column | Notes |
|---|---|
| `id` | UUIDv7 text |
| `ts` | |
| `key_id` | which key made the call |
| `tool` | tool name |
| `args_json` | the arguments, with secret fields redacted before storage |
| `status` | `ok` \| `error` |
| `error` | the message, when `error` |
| `ms` | how long the tool took |

## Failures

| Case | Answer |
|---|---|
| No `Authorization` header | `401`, `WWW-Authenticate: Bearer` |
| Unknown, revoked or expired key | `401`, same header, one line in the log with the prefix only |
| Read key calls a write tool | the tool is not in its catalogue; a direct `tools/call` is `403` |
| `approve_decision` on a non-pending decision | the handler's existing `400`, wrapped as a tool error (`isError: true`) |
| `update_settings` with an invalid value | the existing `400`, as a tool error |
| Tool panics | recovered into `isError: true`, recorded in `mcp_calls`, never a 500 that leaks a stack |
| Database locked / unavailable | `isError: true` with the message; the server stays up |

A tool error is content, not a transport failure: the client sees the reason and can retry or stop.
Only auth failures are HTTP-level, because a client without a valid key must not learn the tools.

## Security notes

- **Token:** 32 random bytes, base64url, prefixed `spk_`; compared after hashing, never by equality
  on the plaintext.
- **Timing:** lookup by hash is the comparison; no secret is compared in a loop.
- **Last used:** updated at most once a minute per key, so a busy assistant cannot turn a read into
  a write storm on SQLite.
- **Scope escalation:** impossible by construction — the tool set is built from the key's scope, and
  the write check is in the middleware, not in the tool body.
- **Secrets in audit:** `args_json` redacts `llm_key`, `ntfy_token`, `proxy_pass` before insert.
- **Logs:** the key appears only as its 8-character prefix.

## Consequences

- The REST API is untouched: no cookie, no change to the UI's fetches. Only `/mcp` requires a key.
- The web UI gains one Settings tab (`/settings/access`); every other page is unchanged.
- A later login binds `api_keys.user_id` and puts sessions in front of REST; this design does not
  block it and does not pretend to do it.
- New dependency: `github.com/modelcontextprotocol/go-sdk` (already downloaded at v1.8.0); it pulls
  `golang.org/x/tools` and `golang.org/x/text` forward, which `go mod tidy` records.
