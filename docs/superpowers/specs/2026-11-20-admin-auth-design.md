# Admin sign-in with JWT — design

Date: 2026-11-20. Status: proposed. Intent: `docs/intent/admin-auth.md` (confirmed 2026-11-20).

## Why

The app is reachable at `PUBLIC_URL` with nothing in front of it. It holds the book, it can move
money, and it now issues MCP keys — but anyone who finds the URL can read and trade it. One door,
one key: an admin account.

## Decision

**One admin account, created on first run, signing in with a password; a short access JWT and a
rotating refresh token.**

- **Single admin, not users.** There is one row in `admin`. Signup exists exactly once: the first
  person to open the app sets the password. After that `/setup` is a sign-in and the setup call
  refuses. No per-user books: every signed-in session is the owner.
- **Passwords** are bcrypt (cost 12), never recoverable. The admin can change the password from
  Settings → Access, which revokes every refresh token.
- **Access token:** JWT HS256, 15 minutes, claims `sub` (admin id), `iat`, `exp`, `jti`. Signed
  with a key derived from `SECRET_KEY` (`sha256("jwt:" + SECRET_KEY)`), so it survives restarts and
  needs no new environment variable.
- **Refresh token:** 32 random bytes, base64url, 30 days, stored only as `sha256`. **Rotated on
  every use** — the presented token is revoked and its replacement recorded in `replaced_by`, so a
  replayed token is refused and the reuse revokes the whole chain. Sent to the browser as an
  HttpOnly, SameSite=Lax cookie scoped to `/api/auth`, `Secure` when `PUBLIC_URL` is https.
- **Transport:** the SPA keeps the access token in memory and sends `Authorization: Bearer`. On a
  `401` it calls `/api/auth/refresh` once and retries; a failed refresh sends it to `/login`.

## Routes

| Method | Path | Behaviour |
|---|---|---|
| GET | `/api/auth/status` | `{needs_setup, signed_in, username}` — the only unauthenticated read |
| POST | `/api/auth/setup` | `{username?, password}`; only while no admin exists; returns tokens |
| POST | `/api/auth/login` | `{username, password}` → access token in the body, refresh in the cookie |
| POST | `/api/auth/refresh` | cookie → new access token, rotated refresh cookie |
| POST | `/api/auth/logout` | revokes the presented refresh token and clears the cookie |
| GET | `/api/auth/me` | `{username}` for the signed-in admin |
| POST | `/api/auth/password` | `{current, next}`; revokes every refresh token |

Everything under `/api/` other than `/api/health` and `/api/auth/*` requires a valid access token.
Static files and the SPA shell stay public — the login page is part of the shell.

## Data

`admin`

| Column | Notes |
|---|---|
| `id` | UUIDv7 text |
| `username` | the name to type; default `admin` |
| `password_hash` | bcrypt |
| `created_at` | |

`refresh_tokens`

| Column | Notes |
|---|---|
| `id` | UUIDv7 text |
| `admin_id` | the single admin |
| `token_hash` | `sha256` hex, unique |
| `created_at` / `expires_at` | 30 days |
| `revoked_at` | set on rotation, logout, password change |
| `replaced_by` | the token that replaced this one, when rotated |
| `user_agent` | shown in the list, so an unknown session is recognisable |

`api_keys.user_id` is filled with the admin id once an admin exists, so the MCP keys are owned.

## Failures

| Case | Answer |
|---|---|
| No admin yet, any `/api/...` other than auth/health | `401`, and `/api/auth/status` says `needs_setup` |
| Wrong password | `401` `invalid credentials`; the check runs even when the username is wrong, so timing does not reveal it |
| Expired access token | `401` `token expired` — the SPA refreshes |
| Refreshed token reused after rotation | `401`, and the chain is revoked |
| Setup called twice | `409` `an admin already exists` |
| Password change with a wrong current password | `403` |
| Too many failed logins | a 1-second delay per failure, capped at 30 attempts per IP per 15 minutes, in memory |

## Consequences

- `SECRET_KEY` becomes load-bearing for auth as well as for stored secrets: rotating it logs every
  session out and makes existing refresh tokens unusable — which is the correct emergency lever.
- The browser flows change: every page needs a session, so `docs/testing/flows.md` gains a sign-in
  step at the front.
- MCP is unchanged: it keeps its own keys, now owned by the admin.
