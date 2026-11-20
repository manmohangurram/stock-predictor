# Admin sign-in with JWT — plan

Spec: `docs/superpowers/specs/2026-11-20-admin-auth-design.md`. One PR per task, each green alone.

## Task 1 — the admin, the tokens, and the middleware

- Migration `0030_admin_auth`: `admin`, `refresh_tokens`; `api_keys.user_id` is filled when an admin
  exists (additive only — no existing row is rewritten).
- `internal/auth`: bcrypt hash/verify; `IssueAccess` / `VerifyAccess` (HS256, 15 min, key derived
  from `SECRET_KEY`); refresh mint/rotate/revoke with `sha256` storage and the reuse rule.
- `/api/auth/status|setup|login|refresh|logout|me|password`.
- Middleware on `/api/*` except `/api/health` and `/api/auth/*`; expiry refusals carry a distinct
  reason so the SPA can tell "refresh me" from "you are not signed in".
- Tests: setup once then `409`; login wrong password; access expiry; refresh rotation and replay
  refusal; middleware `401`; an existing database keeps its rows and gains the admin.

## Task 2 — the screens

- `/login`: one page with two modes. No admin → *Set up the admin* (username, password, confirm);
  admin exists → *Sign in*.
- SPA plumbing: the access token in memory, `Authorization` on every call, one refresh-and-retry on
  `401`, redirect to `/login` when refresh fails, and a **Sign out** control in the settings rail.
- The router refuses pages without a session; the shell renders before the guard resolves so there
  is no flash of the book.
- Screenshots at 1440 and 390, light and dark, plus the measured contrast of the form.

## Task 3 — docs and release

- `docs/api.md` rows, `flows.md` sign-in step, README note that the app now asks for a password,
  CHANGELOG, `v0.34.0` (a new table, a new gate, a changed first-run experience).
- The upgrade note: an existing database boots, shows the setup screen once, and every row it had
  is still there.

## Verification for every task

`go build ./...`, `gofmt -l cmd internal`, `go vet ./...`, `go test ./...`, `cd web && npx vitest run
&& npm run build`. Task 2 also gets the headless-Chrome screenshots and the flow walk.
