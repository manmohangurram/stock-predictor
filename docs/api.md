# API

Every endpoint, what it is for, and whether the UI uses it. `—` means API-only: kept because it is
part of the surface (a script or a curl can use it), not because a page calls it.

| Method | Path | Used by UI | Purpose |
|---|---|---|---|
| GET | `/api/health` | — | Liveness: `{"ok":true}` |
| GET | `/api/dashboard?mode=&decisions=` | yes | The ledger in one request: budget, positions, pending signals, recent fills, ETag |
| GET / PUT | `/api/settings` | yes | Every settings row; PUT writes only the keys present |
| POST | `/api/settings/proxy/test` | yes | One real request through the stored proxy |
| POST | `/api/instruments` | yes | Add by code (AMFI scheme or Yahoo symbol) |
| GET | `/api/instruments/{symbol}` | yes | Instrument row with headlines and signals |
| POST | `/api/instruments/{symbol}/refresh` | yes | Fetch bars and headlines now |
| PUT | `/api/instruments/{symbol}/target-profit` | yes | That stock's profit floor, inside the pot's band |
| DELETE | `/api/instruments/{symbol}` | yes | Delete (no history) or archive (has fills); refuses while held |
| GET | `/api/instruments/{symbol}/analysis` | yes | The overview page in one request: bars, indicators, checks, decisions, fills |
| GET | `/api/instruments/{symbol}/trades` | yes | Fills for one symbol |
| POST | `/api/instruments/{symbol}/trades` | yes | Record a fill (backdatable, pot-aware) |
| GET | `/api/trades` | yes | The last 200 fills, all instruments |
| GET | `/api/search?q=` | yes | Yahoo + AMFI search |
| GET | `/api/calls` | yes | Audit list: prompt type, call, holding name, review state, verdict, tokens |
| GET | `/api/calls/{id}` | yes | One call with its prompt and raw reply |
| POST | `/api/signals/{id}/approve` | yes | Record the fill you got; approves the whole group (both legs of a rotation, every move of a plan) |
| POST | `/api/signals/{id}/reject` | yes | Reject with an optional reason; a plan is rejected whole; no money moves |
| POST | `/api/notify/test` | yes | Send one push |
| POST | `/api/positions` | — | Add a position without a ledger entry (import path) |
| PATCH | `/api/positions/{id}` | — | Adjust a position (import path) |
| POST | `/api/ipo` | — | Reserve cash for an IPO application; not surfaced in the UI yet |

**Contract rules**

- Bodies are JSON; a request with a body must send `content-type: application/json` (the server
  answers 415 otherwise).
- Errors: `400` bad input or a refused fill, `404` no such row, `409` reserved for conflicts, `500`
  internal. The JSON shape is `{"error": "..."}`.
- `mode` accepts `long` or `trade` (and `trading` as an alias); a stray `?` in a hand-written query is
  ignored rather than 400-ing.
- The dashboard answers `304` when its ETag is unchanged, and the ETag covers pending decisions so
  approving or rejecting always refreshes the page.
