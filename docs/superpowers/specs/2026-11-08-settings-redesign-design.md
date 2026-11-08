# Settings redesign — everything live, next to nothing in the environment

Status: approved in conversation, 2026-11-08. Binding for the implementation plan
`docs/superpowers/plans/2026-11-08-settings-redesign.md`.

Companion: `.interface-design/system.md` (paper/ink) stays binding; this does not change its
tokens, only the arrangement of one page and the shape of the data behind it.

---

## Intent and the why

The app runs 24×7 as a black box. Today, changing how often it polls, how big a price move earns
a model call, or how much tape goes into a prompt means editing `docker-compose.yml`, restarting the
container, and being physically at the machine. That is the wrong shape for a self-hosted service
whose owner is on a phone.

The owner also wants to run it behind an HTTP proxy, and there is no way to do that except an
environment variable guessed at — `HTTP_PROXY` — which reqwest silently honours and nothing in the
product admits exists.

So: the environment shrinks to the facts that exist *before* a database does, and everything else
becomes a settings row that takes effect without a restart. The settings page, which was one long
column and is about to grow by two sections, becomes a set of tabs.

## Goals

1. Environment carries only `BIND`, `DB_PATH`, `PUBLIC_URL`. Nothing else is read at all.
2. Every previously-environmental knob is a settings row, visible and editable in the UI, applied
   without a restart.
3. A proxy is settable in the UI, applies to all outbound traffic, and can be tested from the page.
4. The settings page shows one section at a time, each section stamped on its own.
5. No secret becomes readable through the API — including a proxy URL that embeds credentials.

## Non-goals

- No multi-user support, no permissions, no encryption at rest. The server has no auth by design;
  see the README's security note. Anything written here inherits that.
- No proxy authentication beyond what fits in the URL (``http://user:pass@host:port``). No NTLM, no
  PAC files, no per-host proxy routing beyond reqwest's own bypass list.
- No config file, no `.env` loader, no live reload of `BIND`/`DB_PATH`/`PUBLIC_URL` — those three
  are deployment facts and need a restart, which is the whole point of them being in the
  environment.
- No theme or layout change outside the settings page.

## Anti-goals

- A settings page that needs scrolling to find a value, or a save button that applies 30 fields at
  once and hides which one was rejected.
- An environment variable that still works quietly. Two sources of truth for one value is how a
  stale `POLL_SECS` in compose overrides the number on screen and nobody notices for a week.
- A proxy field that cannot be verified, so a typo shows up as fifteen minutes of silence.

## Constraints

- SQLite, `sqlx`, one `settings` table keyed by `key TEXT PRIMARY KEY`, `value TEXT NOT NULL`,
  `updated_at TEXT NOT NULL` (`migrations/0001_init.sql`). New knobs are new rows; no schema change.
- Migration numbering continues from `0008_indexes`; the new one is `0009_runtime_settings`.
- Every outbound request goes through one of exactly three `reqwest::Client` constructions:
  `src/market/yahoo.rs:14-20`, `src/llm/deepseek.rs:37-45`, `src/notify.rs:132-147`.
- reqwest 0.13.4 has no per-request proxy; only `ClientBuilder::proxy` and `ClientBuilder::no_proxy`.
  Therefore a proxy change means rebuilding clients.
- `cargo clippy --all-targets` warning-free, `cargo test` with no network, vitest for the UI.

## Approaches considered

### Backend: where the knobs live

**A — extend `Settings`, re-read where used.** One struct, one query. Rejected: `Settings` is the
serialised API payload for the page; putting `daily_cap` in it grows a public contract nobody edits
in a form, and every new knob would touch the UI response type.

**B — a second small struct, `Runtime`, read per poll.** *(chosen)* `settings::runtime(pool) ->
Runtime` with the seven knobs and the proxy. `EngineDeps` carries it instead of `Config`. Keeps one
audience per type, gives one place to validate, and the per-poll read is the mechanism that makes
edits live rather than restart-only.

**C — raw key/value passthrough.** Cheapest to write, no validation, and a bad value surfaces as a
mid-poll error instead of a rejected save. Rejected.

### Front-end: how the page is arranged

**A — index rail with strict tabs.** *(chosen)* Six tabs down the left; one section rendered at a
time; each tab its own route, its own save, its own saved/unsaved state. On a 390px screen the rail
becomes a scrollable tab strip and the save bar pins to the bottom.

**B — two-page book spread.** Everything on one screen with a sticky receipt strip. Comfortable on
a laptop, and the worst phone page — which is the exact problem being fixed. Rejected as the
primary, kept as the mockup it was.

**C — numbered schedule.** One numbered form with a sticky summary panel. Longest scroll of the
three and the summary duplicates the page. Rejected.

Mockups for all three: `docs/design/settings/{a,b,c}.html` and `.png`.

---

## Decisions

### 1. The environment surface

| Variable | Default | Why it is environment |
|---|---|---|
| `BIND` | `127.0.0.1:8080` | Needed before any request can be served or any page opened. |
| `DB_PATH` | `data/stock-predicter.db` | Needed before a settings row can be read. Chicken and egg. |
| `PUBLIC_URL` | *(empty)* | The deployment's own address; used only to build notification deep links. Belongs to the deployment, like the port. |

`RUST_LOG` remains a standard tracing knob and is not part of the product surface.

Every other variable stops being read. On boot, if one of the retired names is present in the
environment, log one `warn!` naming it and saying it is ignored — a silent ignore is how someone
spends an evening wondering why `POLL_SECS=600` does nothing.

Retired: `POLL_SECS`, `OFF_HOURS_SECS`, `LLM_DAILY_CAP`, `LLM_MIN_PRICE_MOVE`,
`LLM_MAX_SILENCE_SECS`, `PROMPT_WINDOW_HOURS`, `DEEPSEEK_BASE_URL`.

`DEEPSEEK_BASE_URL` has no successor row: `llm_base_url` already exists and the provider table in
`src/llm/provider.rs` supplies per-provider defaults when it is blank.

### 2. `Runtime`, and what "live" means

```rust
pub struct Runtime {
    pub poll_secs: u64,           // 60..=86_400, default 900
    pub off_hours_secs: u64,      // 60..=86_400, default 7_200
    pub prompt_window_hours: u32, // 1..=24,      default 4
    pub min_price_move: f64,      // 0..=1e6,     default 2.0
    pub max_silence_secs: i64,    // 60..=86_400, default 3_600
    pub daily_cap: u32,           // 0..=10_000,  default 200
    pub proxy: Option<String>,    // empty = direct
    pub proxy_bypass: Option<String>,
}
```

`settings::runtime(pool)` reads the eight keys, applies defaults for missing rows, parses, and
clamps to the ranges. Parsing failure falls back to the default and logs a warning; it never fails a
poll, because a corrupt row must not stop the app from waking up.

Live means: the poll loop calls `runtime()` at the **top of every iteration**, so the next tick uses
the new cadence. `engine::gather` receives the `Runtime` it is given and uses it for the gate and the
prompt window. `orchestrate` uses `daily_cap` from the same value. No restart, no cache to
invalidate.

One deliberate limit: changing `poll_secs` does not cancel the `sleep` already in progress. The new
value applies from the following tick, at most one old interval later. Documented in the Cadence
tab's help text as "applies on the next tick".

### 3. The proxy

- Stored as `proxy` (a URL, may contain credentials) and `proxy_bypass` (comma-separated hosts).
- Every client is built through **one** helper in `src/market/mod.rs` (or a new
  `src/http.rs`): `fn client(rt: &Runtime) -> reqwest::Result<Client>`.
- The helper always calls `.no_proxy()` **first**. This is the load-bearing line: without it,
  reqwest would still honour `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, and the environment would win
  over the UI — the two-sources-of-truth failure this design exists to remove.
- When `proxy` is set: `.proxy(Proxy::all(url)?)`, and `.no_proxy()` is not used for the bypass list
  (reqwest's `Proxy::no_proxy` adds to it).
- An unparseable proxy URL is rejected at **save** time by `settings::set`, not at request time.
- The API never returns the URL. `GET /api/settings` returns `proxy_configured: bool` and
  `proxy_host: string` (host:port only, no userinfo). A patch with an empty string clears it;
  a patch with `null` leaves it alone, consistent with the other masked fields.
- `POST /api/settings/proxy/test` builds a client from the **stored** settings, requests
  `https://api.deepseek.com/` with a 5-second timeout, and returns
  `{ ok, status, millis, error }`. It is a diagnostic, not a health check; it does not fail the
  server when the proxy is down.

### 4. The API

`GET /api/settings` gains, under a `runtime` object so the page can render a tab without guessing:

```json
"runtime": {
  "poll_secs": 900, "off_hours_secs": 7200, "prompt_window_hours": 4,
  "min_price_move": 2.0, "max_silence_secs": 3600, "daily_cap": 200,
  "proxy_configured": true, "proxy_host": "10.0.0.2:3128",
  "proxy_bypass": "localhost,127.0.0.1"
}
```

`PUT /api/settings` accepts the same keys at the top level (not nested), each optional, and only
writes what is present — the existing patch semantics. `public_url` is **removed** from both the
response and the patch; the column-free settings row may remain, unread, or be deleted by the
migration. Prefer: the migration deletes the `public_url` row, so no dead value lingers.

Validation happens in `settings::set`, returns `SettingsError::BadRequest` with a specific message,
and is exercised by tests. Examples of messages: `"poll interval must be between 60 and 86400
seconds"`, `"proxy must be a http:// or https:// URL"`, `"prompt window must be between 1 and 24
hours"`.

### 5. The page

Route: `/settings/:tab`, `tab ∈ {money, risk, model, notifications, cadence, network}`. Unknown tab
redirects to `money`. Back/forward work; each tab is linkable.

Each tab renders: heading, one-line lede, fields, and a bottom bar with `Revert` / `Stamp` plus a
saved/unsaved pill in the header. `Stamp` sends **only that tab's fields**. Switching tabs keeps the
unsaved draft in memory and the pill continues to say unsaved; nothing is discarded without
`Revert`.

Accessibility: `role="tablist"` with `role="tab"` and `aria-selected`, panels as `role="tabpanel"`
with `aria-labelledby`; left/right arrow keys move between tabs; focus lands on the first field when
a tab is opened by keyboard. Touch targets 44px on the tab strip.

Tab content:

| Tab | Fields |
|---|---|
| Money | long-term pot, trading pot, currency |
| Risk limits | the two pots side by side in one comparison table (five rows) |
| Model | provider, model, base URL, API key (masked) |
| Notifications | ntfy server, topic, token (masked), and a `Send test` button (exists today) |
| Cadence | the six numeric knobs, each with its unit and a one-line consequence |
| Network | proxy URL, bypass list, `Test proxy` button with the last result |

The masked-secret rule is unchanged: an empty field means "keep what is stored".

### 6. Migration

`migrations/0009_runtime_settings.sql`:

- `INSERT OR IGNORE` the eight new keys with their defaults, so a fresh and an upgraded database
  read the same values (`runtime()` also tolerates absent rows — the migration is for inspectability,
  not correctness).
- `DELETE FROM settings WHERE key = 'public_url'`.

### 7. Documentation

README's Configuration section keeps only the three variables, plus a short "where did the rest go"
table pointing at the tabs. The Docker section already says budget/keys live in the database; it
gains the cadence and proxy. Compose stops carrying `-e` lines for retired names.

---

## Testing

- `tests/config.rs`: retired variables are not read, even when set (`POLL_SECS=1` leaves
  `Runtime` at its default); `BIND`/`DB_PATH`/`PUBLIC_URL` still are.
- `tests/runtime.rs` (new): defaults with an empty table; round-trip through `settings::set`; every
  out-of-range value rejected with its message; a corrupt row falls back to the default and does not
  error.
- `tests/proxy.rs` (new): client built with no proxy in settings and `HTTP_PROXY` set in the process
  environment does **not** use it (assert on the builder's effect via a local listener that must
  receive the request, or on reqwest's error for an unreachable proxy); a set proxy is used;
  credentials never appear in `GET /api/settings`.
- `tests/api.rs`: `runtime` object present and shaped as above; per-tab patch writes only its keys.
- `web/src/render.test.ts`: six tabs render; each shows only its own fields; the unsaved pill
  appears after an edit and clears after a stamp; `/settings/cadence` deep link selects Cadence.
- Manual: 1440px and 390px screenshots of every tab, compared against the approved mockups.

## Risks

1. **Proxy + no_proxy ordering.** If `.no_proxy()` is forgotten for one client, the environment
   silently wins for that traffic only, and it looks like a provider bug. Mitigation: one helper,
   one construction site, a test that fails if a fourth `Client::builder` appears outside it.
2. **Losing a credential on save.** The masked-field rule is already in place; the Network tab adds a
   second secret surface. Same rule applies: empty means keep.
3. **The tab strip hides a broken setting.** If a value is rejected, the error must appear on that
   tab's bar, not as a toast that outlives the context.
4. **Retired variables still set in someone's compose file.** They are ignored, and one warning line
   per boot names them. The README says so plainly.

## Left to the builder

- The exact split of `Runtime` parsing helpers and where the clamp table lives.
- Whether the proxy helper lives in `src/http.rs` or `src/market/mod.rs`; either is fine, one place.
- Markup details of the tab strip, as long as the tokens, roles and keyboard behaviour hold.
- Whether `Send test` and `Test proxy` share one backend helper.
