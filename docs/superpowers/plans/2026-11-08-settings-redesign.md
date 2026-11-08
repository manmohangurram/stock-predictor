# Settings Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Shrink the environment to `BIND`, `DB_PATH`, `PUBLIC_URL`; move the seven tuning knobs and a proxy into settings that apply without a restart; rebuild the settings page as six strict tabs.

**Architecture:** A new `Runtime` struct read from the `settings` table at the top of every poll iteration becomes the single source for cadence, gate thresholds and proxy. All three outbound clients are built through one helper that calls `.no_proxy()` first, so only the database can choose a proxy. The UI gains `/settings/:tab` routes, one section each, stamped independently.

**Tech Stack:** Rust (axum, tokio, sqlx/SQLite, reqwest 0.13), Vite + TypeScript front end, vitest.

**Spec:** `docs/superpowers/specs/2026-11-08-settings-redesign-design.md`

## Global Constraints

- No test may touch the network. Fixtures and local listeners only.
- `cargo clippy --all-targets` must be warning-free; `cargo test` green; `cd web && npx vitest run && npm run build` green.
- The environment is read in exactly one place (`Config::from_env`) and yields exactly three fields after this work: `bind`, `db_path`, `public_url`.
- Every outbound `reqwest::Client` construction goes through one helper. A test must fail if a fourth construction site appears.
- Secrets are never serialised. Masked fields treat empty as "keep the stored value"; `null`/absent means "leave alone".
- Design system (`.interface-design/system.md`) is binding: paper/ink tokens, hairline rules, 3px surfaces, IBM Plex, stamps not primary buttons.
- Retired variables are ignored with one `warn!` per boot naming them; never silently.
- Update `README.md` and `docker-compose.yml` in the same task that changes behaviour, not at the end.

## File Structure

| File | Responsibility after this plan |
|---|---|
| `src/rules/runtime.rs` (new) | `Runtime` struct, its key names, ranges, `parse`, `clamp`. No I/O beyond one query. |
| `src/rules/settings.rs` | Owns validation on write (`set`) and the `runtime()` reader; drops `public_url`. |
| `src/config.rs` | Three deployment fields only; the retired-variable warning list. |
| `src/http.rs` (new) | One `client(&Runtime)` builder; `.no_proxy()` first, then the stored proxy. |
| `src/market/yahoo.rs`, `src/llm/deepseek.rs`, `src/notify.rs` | Take a `&reqwest::Client` (or `&Runtime`) instead of building their own. |
| `src/main.rs` | Poll loop re-reads `runtime()` each iteration; wires `PUBLIC_URL` into notify. |
| `src/engine.rs`, `src/llm/orchestrate.rs` | Read knobs from `Runtime` instead of `Config`. |
| `src/api.rs` | `runtime` object in `GET /api/settings`; patch keys; proxy test route; no `public_url`. |
| `migrations/0009_runtime_settings.sql` | Inserts default rows; deletes `public_url`. |
| `web/src/pages/settings.ts` | Six tab renderers, one per route; per-tab draft and stamp. |
| `web/src/main.ts` | `/settings/:tab` routing. |
| `web/src/api.ts` | `runtime` in the `Settings` type; `putRuntimeSettings`; `testProxy`. |

---

### Task 1: `Runtime` — the struct, its ranges, and its migration

**Files:**
- Create: `src/rules/runtime.rs`
- Modify: `src/rules/mod.rs` (module declaration)
- Create: `migrations/0009_runtime_settings.sql`
- Modify: `src/db.rs` (register the migration, following the `0008` entry)
- Create: `tests/runtime.rs`

**Interfaces:**
- Produces: `pub struct Runtime { poll_secs: u64, off_hours_secs: u64, prompt_window_hours: u32, min_price_move: f64, max_silence_secs: i64, daily_cap: u32, proxy: Option<String>, proxy_bypass: Option<String> }`, `Runtime::default()`, `Runtime::from_map(&BTreeMap<String,String>) -> Runtime` (never fails), `crate::rules::settings::runtime(&SqlitePool) -> anyhow::Result<Runtime>`.

- [ ] Write `tests/runtime.rs`: `empty_table_yields_the_documented_defaults` — seed a database, call `settings::runtime`, assert 900 / 7200 / 4 / 2.0 / 3600 / 200 / `None` / `None`.
- [ ] Run it; it fails to compile. That is the failing test.
- [ ] Write `src/rules/runtime.rs` with the struct, `Default`, and `from_map`, clamping each value into its range and falling back to the default on a parse failure (`warn!` once per bad key).
- [ ] Add `pub mod runtime;` to `src/rules/mod.rs`; add the migration to `src/db.rs`; write `migrations/0009_runtime_settings.sql` with `INSERT OR IGNORE` for the eight keys and `DELETE FROM settings WHERE key = 'public_url'`.
- [ ] Add `settings::runtime` to `src/rules/settings.rs`: one `SELECT key, value FROM settings`, into `from_map`.
- [ ] Run `cargo test --test runtime`; expect pass.
- [ ] Add the remaining cases: `corrupt_row_falls_back_to_default_and_does_not_error`, `each_value_is_clamped_into_its_range`, `migration_rows_match_the_defaults`.
- [ ] Run the full suite; commit: `feat: runtime settings struct with documented ranges`.

---

### Task 2: Retire the environment

**Files:**
- Modify: `src/config.rs` (shrink to three fields; add the retired-name list and its warning)
- Modify: `src/main.rs` (call the warning once at boot; pass `PUBLIC_URL` into the notifier)
- Modify: `src/rules/settings.rs`, `src/api.rs` (drop `public_url`)
- Modify: `src/notify.rs` (take the public URL as a parameter instead of reading settings)
- Modify: `tests/config.rs`, `tests/api.rs`

**Interfaces:**
- Produces: `Config { bind: String, db_path: String, public_url: String }`; `Config::retired_warnings() -> Vec<&'static str>` returning the names found in the environment.
- Consumes: `Runtime` from Task 1.

- [ ] In `tests/config.rs`, add `retired_variables_are_not_read_even_when_set`: build a map with `POLL_SECS=1, LLM_DAILY_CAP=7, PROMPT_WINDOW_HOURS=99`, assert `Config::from_map` succeeds and the resulting config has no knob fields (compile-level: the struct has three fields) and that `retired_warnings` lists all three.
- [ ] Add `public_url_comes_from_the_environment`: `PUBLIC_URL=https://x` → `config.public_url == "https://x"`, absent → `""`.
- [ ] Run `cargo test --test config`; expect failures.
- [ ] Shrink `Config`; keep `from_map` for testability; implement `retired_warnings`.
- [ ] Delete `public_url` from `Settings`, `SettingsPatch`, `settings::get`/`set`, and the API patch handling; read it from `Config` where the notifier is constructed.
- [ ] In `tests/api.rs`, delete any assertions on `public_url` in the settings payload; add `settings_payload_has_no_public_url`.
- [ ] `cargo test`; expect green. Commit: `refactor: environment carries only bind, db path and public url`.

---

### Task 3: The poll loop and the engine consume `Runtime`

**Files:**
- Modify: `src/main.rs` (loop calls `runtime()` at the top of each iteration)
- Modify: `src/engine.rs` (`EngineDeps { runtime: &Runtime }` replacing `cfg: &Config` where knobs were read; `gather` uses `runtime.prompt_window_hours`, `min_price_move`, `max_silence_secs`)
- Modify: `src/llm/orchestrate.rs` (`daily_cap` from `Runtime`)
- Modify: `tests/engine.rs`, `tests/orchestrate.rs`, `tests/scenarios.rs`

**Interfaces:**
- Produces: `EngineDeps::runtime: Runtime` (by value; it is small and cheap to clone).
- Consumes: `Runtime` (Task 1).

- [ ] In `tests/engine.rs`, add `the_gate_uses_the_runtime_values_not_the_defaults`: two polls with the same bars, one with `min_price_move: 1e9` (no call) and one with `0.0` plus a stale memo (call expected). Assert on the recorded decision count.
- [ ] Run it; expect failure.
- [ ] Thread `Runtime` through `EngineDeps` and replace every `cfg.*` knob read; leave `cfg` out of `EngineDeps` entirely once unused.
- [ ] In `src/main.rs`, call `settings::runtime(&pool)` inside the loop before building `EngineDeps`, and use the value for the `sleep` choice.
- [ ] Add `cadence_changes_take_effect_on_the_next_tick` as a unit test on the helper that chooses the interval (extract it if it is inline).
- [ ] `cargo test`; commit: `feat: cadence and model gate read from settings each tick`.

---

### Task 4: One HTTP client helper, and the proxy

**Files:**
- Create: `src/http.rs`
- Modify: `src/lib.rs` (module)
- Modify: `src/market/yahoo.rs`, `src/llm/deepseek.rs`, `src/notify.rs` (accept a client; stop calling `Client::builder`)
- Modify: `src/main.rs` (build clients once per tick from `Runtime`)
- Create: `tests/proxy.rs`

**Interfaces:**
- Produces: `pub fn client(rt: &Runtime) -> reqwest::Result<reqwest::Client>`.
- Consumes: `Runtime.proxy`, `Runtime.proxy_bypass`.

- [ ] Write `tests/proxy.rs::the_environment_never_chooses_the_proxy`: `std::env::set_var("HTTP_PROXY", "http://127.0.0.1:1")`, build a client from a default `Runtime`, request a local `TcpListener` that accepts one connection; assert the listener received it (the env proxy would have refused). Use a short timeout. Restore the environment afterwards.
- [ ] Add `a_configured_proxy_is_used`: a `TcpListener` acting as the proxy; `Runtime { proxy: Some("http://127.0.0.1:PORT") }`; request an arbitrary `http://example.invalid/`; assert the proxy received the request line.
- [ ] Run; expect failures (no `http` module).
- [ ] Implement `src/http.rs`: `.no_proxy()`, then optionally `.proxy(Proxy::all(url)?)`, plus `Proxy::no_proxy(host)` for each bypass entry, and a 10s overall timeout.
- [ ] Change the three construction sites to accept a `&Client`; delete their builders.
- [ ] Add `tests/proxy.rs::only_one_client_builder_exists`: read the source files under `src/` and assert `Client::builder` appears in `src/http.rs` only — a text assertion, cheap and effective.
- [ ] `cargo test`; commit: `feat: single http client helper with a database-configured proxy`.

---

### Task 5: API — `runtime` object, patch keys, proxy test route

**Files:**
- Modify: `src/api.rs`
- Modify: `tests/api.rs`

**Interfaces:**
- Produces: `GET /api/settings` → existing payload plus `"runtime": {...}` (see spec §4); `PUT /api/settings` accepts `poll_secs`, `off_hours_secs`, `prompt_window_hours`, `min_price_move`, `max_silence_secs`, `daily_cap`, `proxy`, `proxy_bypass`; `POST /api/settings/proxy/test` → `{ ok, status, millis, error }`.
- Consumes: `Runtime`, `settings::set`, `http::client`.

- [ ] In `tests/api.rs`, add `settings_expose_runtime_defaults` asserting the eight fields and the two proxy fields; `put_runtime_settings_persists_and_echoes`; `put_proxy_with_credentials_never_echoes_the_url` (assert the response body string does not contain the password); `put_runtime_settings_rejects_out_of_range` (400 with the message); `proxy_test_reports_failure_without_a_proxy` (no proxy configured and no network: expect `ok: false` and a non-empty `error`, 200 status — the endpoint reports, it does not fail).
- [ ] Run; expect failures.
- [ ] Implement: extend the GET payload; extend `SettingsPatch` handling to call `settings::set` for the runtime keys with validation; add the proxy test handler using `http::client` with a 5s timeout against `https://api.deepseek.com/`.
- [ ] Run; commit: `feat: settings api carries runtime knobs and a proxy test`.

---

### Task 6: UI shell — routes and the tab component

**Files:**
- Modify: `web/src/main.ts` (`/settings/:tab`)
- Create: `web/src/pages/settings-tabs.ts` (tab list, rail markup, keyboard handling)
- Modify: `web/src/pages/settings.ts` (split the renderer per tab; keep the field markup)
- Modify: `web/src/api.ts` (`Runtime` fields in `Settings`, `putRuntimeSettings`, `testProxy`)
- Modify: `web/src/render.test.ts`

**Interfaces:**
- Produces: `renderSettings(s: Settings, tab: Tab, drafts: Drafts) -> string`; `TABS: {id: Tab, label: string, count?: string}[]`; `wireTabs()` for arrow keys.
- Consumes: `GET /api/settings.runtime`.

- [ ] In `render.test.ts`, add: `six_tabs_render_and_only_the_active_one_shows_fields` (render `cadence`, assert `data-tab="cadence"` is selected and that a Money field is absent); `unknown_tab_falls_back_to_money`; `external link to a tab selects it` (pass the parsed path).
- [ ] Run `npx vitest run`; expect failures.
- [ ] Implement the tab shell with `role="tablist"`/`role="tab"`/`role="tabpanel"`, `aria-selected`, arrow-key movement, and a rail that becomes a scrollable strip under 640px.
- [ ] Route `/settings/:tab` in `main.ts`, redirecting unknown tabs to `money` via `history.replaceState`.
- [ ] Run vitest; commit: `feat(ui): settings becomes six routed tabs`.

---

### Task 7: UI tabs — Money, Risk limits, Model, Notifications

**Files:**
- Modify: `web/src/pages/settings.ts`
- Modify: `web/src/render.test.ts`

**Interfaces:**
- Consumes: existing `putSettings` for money/model/notifications; per-tab stamps.

- [ ] Add render tests: each of the four tabs renders its own fields and its own `Stamp`; the risk table has five rows and two columns; a masked key field renders empty with a `set` badge.
- [ ] Run; expect failures.
- [ ] Move the existing fields into their tabs; the Risk limits tab uses the comparison table from mockup `docs/design/settings/a-risk.html`.
- [ ] Implement per-tab draft state: edits put values in a `Map<tab, patch>`; `Stamp` sends only that tab's patch through `putSettings`; `Revert` clears it; the header pill reads `saved` / `unsaved`.
- [ ] Run vitest; commit: `feat(ui): four settings tabs with independent stamps`.

---

### Task 8: UI tabs — Cadence and Network

**Files:**
- Modify: `web/src/pages/settings.ts`
- Modify: `web/src/api.ts`
- Modify: `web/src/render.test.ts`

- [ ] Add tests: Cadence renders six numeric fields with units; Network renders a proxy field, bypass and `Test proxy`; after a successful stamp the pill reads `saved`.
- [ ] Run; expect failures.
- [ ] Implement both tabs from mockups `a-cadence.html` (desktop and 390px behaviour) and the Network pane in `b.html`; `Test proxy` calls `testProxy()` and prints `ok · 214 ms` or the error in place.
- [ ] Run vitest and `npm run build`; commit: `feat(ui): cadence and network tabs`.

---

### Task 9: Documentation, compose, and the gates

**Files:**
- Modify: `README.md`, `docker-compose.yml`, `AGENTS.md` (project facts: three env vars)
- Modify: `docs/superpowers/specs/2026-10-27-stock-predicter-design.md` (one line pointing to the new spec)

- [ ] Rewrite README Configuration: three variables, a "where did the rest go" table, and the retired-names warning explained.
- [ ] Remove retired `-e` lines from `docker-compose.yml`; keep `PUBLIC_URL` with a comment.
- [ ] Run the full gate: `cargo test`, `cargo clippy --all-targets`, `cd web && npx vitest run && npm run build`.
- [ ] Start the server against a copy of the demo database; screenshot all six tabs at 1440 and 390; compare against the mockups; fix anything that drifted.
- [ ] Commit: `docs: environment is three variables, everything else is a tab`.

---

## Review Focus

- **The `.no_proxy()` line.** If it is missing anywhere, the environment wins silently for part of the traffic. The text test in Task 4 exists to catch a fourth construction site, not to prove the ordering — read the helper.
- **Per-tab patching.** A stamp must send only its own keys; a shared form model that sends everything is the regression to watch for.
- **Clamping vs rejecting.** `Runtime::from_map` clamps (a corrupt row must not stop the app waking); `settings::set` rejects (a user typo must be visible). Both behaviours are required and they are deliberately different.
- **`public_url` removal is complete.** No dead row read, no dead field in the payload, no dead input in the UI.
