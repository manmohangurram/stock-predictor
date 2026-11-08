# stock-predicter

A self-hosted portfolio ledger that watches your holdings, suggests buy and sell decisions with
prices and an exit plan, and pushes one notification when something actually needs doing.

It is **not** a price oracle. The engine is a deterministic strategy plus a language model's
judgement on the news, and every number it suggests is validated before you see it.

```
                ┌───────────── poll every 15 min (2 h after the close) ─────────────┐
Yahoo Finance ──┤ prices, daily bars, intraday tape, headlines                      │
AMFI (India) ───┤ mutual fund NAV                                                │
                └────────────────────────┬──────────────────────────────────────────┘
                                         ▼
                            rules layer (trend, RSI, ATR, budget)
                                         │ candidate or "nothing here"
                                         ▼
                            DeepSeek: decide / ask for an article / pass
                                         │  one escalation, ≤3 articles, ≤7 days
                                         ▼
                            validator: ATR band, stop side, cash, holdings,
                                       profit expectation, citations
                                         ▼
                          ledger row + one ntfy push (deduplicated)
```

## What it does

- **Tracks a real portfolio** across stocks, mutual funds and IPO applications, with two
  independent pots (long-term and trading) that never draw on each other.
- **Keeps a permanent record**: every buy and sell is a row in `trades`, with the market close on
  the day of the fill, and realised profit/loss computed at sale time. Older fills can be
  backdated.
- **Suggests decisions with an exit plan.** A buy must carry both a stop and a take-profit, or it
  is discarded. Every decision shows money at risk, risk per unit and reward:risk, computed from
  the validated levels rather than reported by the model.
- **Respects your budget.** Spend is capped by the ledger; a signal that would overspend is
  recorded as `BLOCKED` with the shortfall and never pushes a notification.
- **Knows its own limits.** Each pot has five numbers — minimum and maximum profit, minimum and
  maximum per trade, and a cap on exposure to any single instrument. They are passed into the
  prompt and enforced in the validator, so neither the rules nor the model can exceed them.
- **Explains silence.** Each instrument page shows which rule blocked a trade and with what
  numbers, so an empty ledger is never a mystery.
- **Pushes to your phone** through ntfy — configured in Settings, testable from the same page,
  one notification per actionable change and nothing else.
- **Costs little.** The model is only called when the price moved ₹2, when a new headline
  arrived, or once an hour; the static prompt block is a cacheable prefix.

## Screens

**Ledger** — the portfolio strip, holdings with their latest signal, search-and-add, recent fills.

![Ledger](docs/screenshots/ledger.png)

**Instrument** — price history with both moving averages, a fill recorder, the decision stream,
the full fills history, headlines, and the rule checklist. Each buy carries its whole exit plan
(`sell if it falls to` / `take profit at`), the money at risk and reward:risk, and how confident
the model said it was. The rules panel on the right is what the *mechanical* layer sees; the
decision above it is the model's, and the validator — not either of them — has the final say.

![Instrument](docs/screenshots/instrument.png)

**Trading pot** — a separate ceiling, a separate book, separate limits. Same instrument set,
different capital.

![Trading](docs/screenshots/trading.png)

Recommendations come from the long-term pot only, and cover **stocks and mutual funds** — an MF is a
plain BUY at NAV with no stop or target, because neither exists for it. The trading pot gets no
recommendations yet; it is for fills you record yourself.

**Settings is six tabs**, one section each, stamped on its own — every value is a database row, and
a change applies on the next tick without a restart. On a phone the rail becomes a scrollable strip
and the stamp bar pins to the bottom.

Money — two pots that never touch:

![Settings: money](docs/screenshots/settings-money.png)

Risk limits — both pots side by side, so they are compared rather than read in sequence. These bound
each stock's own profit target:

![Settings: risk limits](docs/screenshots/settings-limits.png)

Model — DeepSeek, OpenAI, Gemini or any OpenAI-compatible endpoint. Keys are write-only: the API
reports only whether one is configured, and it is stored encrypted at rest.

![Settings: model](docs/screenshots/settings-model.png)

Cadence — how often the app wakes and when it may spend a model call. This is the tab that used to
be six environment variables:

![Settings: cadence](docs/screenshots/settings-cadence.png)

Network — one proxy for all outbound traffic, with address, username and password as separate
fields, plus a button that proves it works before you rely on it:

![Settings: network](docs/screenshots/settings-network.png)

## Long-term and trading: what actually differs

Two books, one process. The difference is *capital and rules*, not a different engine.

| | Long-term | Trading |
|---|---|---|
| Capital | Long-term pot | Its own pot; never draws on the other, and the other never draws on it |
| Limits | Its own five numbers | Its own five numbers, usually tighter |
| Holdings | Separate positions, even in the same symbol | Separate positions, even in the same symbol |
| Mutual funds | Yes — bought at NAV and held | **No**: no intraday exit exists, so the API and the page refuse it |
| History | Own fills and realised P&L | Own fills and realised P&L |
| Polling | Same poll, same cadence | Same poll, same cadence |

**How the server behaves differently for them:** the poll tick walks the watchlist once and
evaluates each instrument against the pot it belongs to, using that pot's limits — so a switch to
the Trading tab changes which book you see, not how often anything is fetched. Cash, exposure and
the per-trade caps are computed per pot, which is why a swing trade cannot block a long-term buy.
Because both live in one process, one instrument can appear in both books with two independent
positions (the screenshot above shows TCS in the trading book while the long-term book holds it
separately).

**What is *not* different yet, plainly:** there is no separate trading strategy and no candidate
discovery. Both pots run the same rules (trend, RSI, ATR) and the same prompt, with the trading
pot distinguished by its capital, its limit set and its own books. A momentum scanner and a
swing-specific prompt are the missing half of the trading mode, and they are not built.

## Quick start

Requires Rust 1.85+ and Node 22+.

```bash
cd web && npm ci && npm run build && cd ..
cargo build --release

BUDGET=500000 CURRENCY=INR ./target/release/stock-predicter
```

Then open <http://127.0.0.1:8080>, and on **Settings**:

1. set the budget (it is seeded from `BUDGET` on first boot and owned by the database after
   that),
2. paste a DeepSeek API key if you want the model layer (without one the rules run alone and no
   call is ever made),
3. add instruments by searching — `INFY`, `Wipro`, `axis elss`. Stocks come from Yahoo, mutual
   funds from AMFI's daily file.

`cargo run -- --seed` fills a demo portfolio at live prices; `--poll-once` runs one cycle
outside market hours; `--ask SYMBOL` asks the model about one instrument and writes nothing.

## Configuration

**Three environment variables. Everything else is a settings row, edited in the UI.**

| Variable | Default | Meaning |
|---|---|---|
| `BIND` | `127.0.0.1:8080` | Listen address (`0.0.0.0:8080` in a container; the image already sets it). |
| `DB_PATH` | `data/stock-predicter.db` | SQLite file. The parent directory is created on boot. |
| `PUBLIC_URL` | *(empty)* | The address the app is reachable at, used only for the deep link on a notification. |
| `SECRET_KEY` | *(generated file)* | 32 bytes — 64 hex characters or base64 — that encrypts the stored secrets. `openssl rand -hex 32`. |

`RUST_LOG` is the usual tracing filter and is not part of the product surface.

There is **no `.env` loading** — nothing reads a file, so a `.env` sitting in the working directory
is ignored. Set variables in the process environment or the Compose `environment:` block.

### Where did the rest go?

Every knob that used to be an environment variable is now a tab on the Settings page, applied
without a restart — the scheduler reads them at the top of every tick:

| Tab | What it holds |
|---|---|
| **Money** | the long-term pot, the trading pot, the currency |
| **Risk limits** | both pots side by side: min and max profit %, smallest and largest trade, one-stock cap |
| **Model** | provider, model, base URL, API key |
| **Notifications** | ntfy server, topic, token, and a test button |
| **Cadence** | `POLL_SECS`, `OFF_HOURS_SECS`, `PROMPT_WINDOW_HOURS`, `LLM_DAILY_CAP`, `LLM_MIN_PRICE_MOVE`, `LLM_MAX_SILENCE_SECS` |
| **Network** | the proxy URL and a bypass list, with a test button |

| Retired variable | Now |
|---|---|
| `POLL_SECS`, `OFF_HOURS_SECS` | Cadence tab (default 900 s / 7200 s) |
| `LLM_MIN_PRICE_MOVE`, `LLM_MAX_SILENCE_SECS` | Cadence tab (default 2.0 / 3600 s) |
| `LLM_DAILY_CAP` | Cadence tab (default 200 calls) |
| `PROMPT_WINDOW_HOURS` | Cadence tab (default 4 hours of tape) |
| `DEEPSEEK_BASE_URL` | gone — the Model tab's Base URL, or the provider's default |

If one of those is still set, the app logs a warning at boot and ignores it. That is deliberate: a
silently ignored `POLL_SECS=600` costs an evening of wondering why nothing changed.

### The proxy

One proxy for all outbound traffic — market data, the model API and ntfy. It lives in the database,
so **the environment is never consulted**: the client is built with `no_proxy()` first, which is what
stops `HTTP_PROXY` from quietly outranking what the page says.

Credentials go in the URL and are sent as a proxy authorization header:

```
http://user:password@proxy.example.in:3128
```

The address, the username and the password are three separate fields — the password is encrypted at
rest like the model key, and none of them are ever returned by the API: you get the host, the
username you typed, and `proxy_pass_configured`. **Test the proxy** on the Network tab before
trusting it; a wrong proxy otherwise shows up as silence.

### Secrets at rest

The model API key, the ntfy token and the proxy password are stored encrypted with
XChaCha20-Poly1305, as `enc:v1:…`. The key comes from `SECRET_KEY`; without it, a `secret.key` file
is created beside the database (mode 0600).

What this buys: a copied `stock-predicter.db`, a backup, or `sqlite3` open in a terminal shows
ciphertext instead of your keys. What it does **not** buy: protection from anyone who has the data
directory or `SECRET_KEY` — and it is still not authentication, because this server has none.
Lose the key and the stored secrets are gone; they are re-entered on the Settings page.

## Phone notifications

Push goes through [ntfy](https://ntfy.sh): no account, no Apple or Google dependency, works from
a home server to a phone in another room or another country. Self-hosted ntfy instances work the
same way — point the *Server* field at yours.

**Setting it up, all of it in the UI.** Open **Settings → Phone notifications**:

| Field | What it is |
|---|---|
| **Topic** | The channel name. Make it long and random (`stock-predicter-7f3k9m2q`) — anyone who knows it can read your alerts. |
| **Server** | `https://ntfy.sh`, or your own instance. |
| **Access token** | Optional, only for a protected topic (`tk_…`). Write-only: it is never returned by the API and an empty field keeps the stored one. |
| **Link from the notification** | `http://192.168.1.10:8080` — tapping the push opens that instrument's page. Leave it blank for no link; a `127.0.0.1` link would be dead from a phone. |

Press **Send a test notification** and it arrives — that button uses the same send path as a real
signal, so a success there means a signal will reach you. From a terminal, the equivalent is:

```bash
./target/release/stock-predicter --test-notify
```

The same values can be seeded from `NTFY_TOPIC`, `NTFY_TOKEN`, `NTFY_BASE_URL` and `PUBLIC_URL`
on first boot; after that the database owns them and edits apply on the next poll.

**What gets sent:**

- `SELL · INFY 17 units` — high priority, red-light tag, with the stop that triggered it.
- `BUY · HDFCBANK 12 units · limit 1,650.00` — with `sell if ≤ 1,590 · take profit ≥ 1,735` and
  the cost, so the whole trade can be placed from the notification.

**What never gets sent:** `BLOCKED` signals (there is nothing to do about them), unchanged
signals, and `pass`. A signal already pushed is not pushed again, and a failed push leaves no
record so the next tick retries it rather than losing it.

## Risk limits

Settings carries a separate set for each pot, because a swing book and a long-term book should
not share a ceiling:

| Setting | Default | What it stops |
|---|---|---|
| Minimum profit % | 5 | the floor for any stock's profit target |
| Maximum profit % | 50 | a fantasy target that will never fill |
| Minimum per trade | 500 | fills eaten by fees |
| Maximum per trade | 50,000 | one order becoming the portfolio |
| Maximum per instrument % | 25 | concentration — total exposure to one symbol, all fills counted |

**These bound a stock's target, not the pot.** `min` and `max` are the allowed band for any
single instrument's profit expectation; a stock's own target defaults to `min` and is set on its
page. A stock may raise its floor inside the band, never above `max` — the ceiling is global, so
0.5%-per-trade scalping and 200%-fantasy targets are both impossible. The
validator checks, in this order: cash and holdings, then the per-trade amount, then the
per-instrument total, then the profit band, then price sanity. A refused trade is recorded as
`BLOCKED` with the reason, never silently dropped. The rules layer clamps its own sizing to the
same numbers, so a fallback cannot sneak past them either.

## Choosing a model provider

**Settings → Model provider.** Any of these, with one key field for whichever is selected:

| Provider | Default model | Notes |
|---|---|---|
| **DeepSeek** | `deepseek-chat` | Cheapest of the four; the only one that can also fetch an article's body for the escalation step. |
| **OpenAI** | `gpt-4o-mini` | OpenAI-compatible request, JSON response mode. |
| **Google Gemini** | `gemini-2.0-flash` | Different request and response shape, handled in `src/llm/provider.rs`. |
| **Custom** | — | Any OpenAI-compatible endpoint: OpenRouter, Groq, a local Ollama (`http://localhost:11434/v1`), or a proxy. |

Each provider's key is stored separately, so switching between them does not lose the others.
The base URL and model name are editable, which is what makes the custom option work. Without any
key the rules layer runs alone and no call is ever made.

## Running 24×7

The server is a single long-lived process with no session or login: start it and it keeps
polling. On a home server, either run the container with `restart: unless-stopped` (the Compose
file does) or a systemd unit:

```ini
[Unit]
Description=stock-predicter
After=network-online.target

[Service]
User=stock
WorkingDirectory=/srv/stock-predicter
ExecStart=/srv/stock-predicter/stock-predicter
Environment=BIND=0.0.0.0:8080
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

What survives a restart: everything in the database. What does not: nothing that matters — the
poll loop restarts on its own schedule and the next tick resumes where the last one stopped. A
Yahoo outage, a failed model call or an unreachable ntfy is logged and retried on the next tick
rather than stopping the loop.

## Self-hosting with Docker

One published image, one way to publish it: a `v*` tag builds `linux/amd64` and `linux/arm64` on
native runners and merges them into a single manifest, so the same tag runs on a laptop and a Pi.
The image is never built from `main`.

### Docker Compose (recommended)

```yaml
services:
  stock-predicter:
    image: ghcr.io/manmohangurram/stock-predictor:${IMAGE_TAG:-latest}
    container_name: stock-predicter
    restart: unless-stopped
    ports:
      - "${PORT:-8080}:8080"
    environment:
      PUBLIC_URL: ${PUBLIC_URL:-}
      SECRET_KEY: ${SECRET_KEY:?set SECRET_KEY in .env — openssl rand -hex 32}
      RUST_LOG: ${RUST_LOG:-info}
      TZ: ${TZ:-Asia/Kolkata}
    volumes:
      - ${DATA_DIR:-./data}:/data
```

`BIND` and `DB_PATH` need no lines: the image already sets `0.0.0.0:8080` and
`/data/stock-predicter.db`, and declares `/data` as its volume. Copy `.env.example` to `.env`
(mode 0600) and Compose does the rest.

The container runs as **root**, like the sibling financer image, because a bind mount of `./data`
must work whatever uid owns it on the host. A fixed unprivileged uid is the tidier choice until
that uid does not match your directory — which is exactly how *unable to open database file* starts.
Root inside a container only sees the mounted directory, but it is still worth knowing.

`128m` is comfortable rather than tight: this process sits at about 21 MB resident with a 250-bar
history loaded. Drop it to `64m` if you want the ceiling to actually bite.

### Time zone

Not required for correctness, and deliberately so:

- The NSE window is computed in **IST regardless of the container clock** (a fixed +05:30 offset in
  `src/schedule.rs`), so polling opens at 09:00 IST on a server in UTC, Berlin or anywhere else.
- Every timestamp is stored as **UTC RFC3339**; the browser renders it in its own zone.
- `TZ` therefore only affects what other tools in the container see (`date` reads
  `Sun Nov 8 17:13 IST 2026`). It is set to `Asia/Kolkata` in Compose and `tzdata` is installed, so it
  works if you add tooling later. The app's own log lines stay in UTC.

### Environment declarations

The **app** reads four variables. Compose substitutes `${…}` from a `.env` file next to
`docker-compose.yml` — that is Compose's feature, not the app's: the binary itself reads only the
process environment and never opens a file.

```sh
# .env — for docker compose, next to docker-compose.yml (mode 0600). Copy .env.example.
SECRET_KEY=    # openssl rand -hex 32 — encrypts the stored secrets
PUBLIC_URL=http://192.168.1.10:8080
RUST_LOG=info
```

| Variable | Default | Meaning |
|---|---|---|
| `BIND` | `127.0.0.1:8080` | Listen address. The image sets `0.0.0.0:8080`. |
| `DB_PATH` | `data/stock-predicter.db` | SQLite file. The image sets `/data/stock-predicter.db`. |
| `PUBLIC_URL` | *(empty)* | The address the app is reachable at, for the deep link on a notification. |
| `SECRET_KEY` | *(key file)* | 32 bytes, 64 hex characters or base64, encrypting stored secrets. |
| `TZ` | `UTC` | The container clock's zone. Cosmetic: the NSE window is IST and storage is UTC. |

### Plain Docker

```bash
docker run -d --name stock-predicter \
  --restart unless-stopped \
  -p 8080:8080 \
  -e PUBLIC_URL=http://192.168.1.10:8080 \
  -e SECRET_KEY="$(openssl rand -hex 32)" \
  -e RUST_LOG=info \
  -e TZ=Asia/Kolkata \
  -v "$PWD/data:/data" \
  --memory 128m \
  ghcr.io/manmohangurram/stock-predictor:latest
```

### Building from source instead

```bash
docker build -t stock-predicter .
docker run -d --name stock-predicter -p 127.0.0.1:8080:8080 -v "$PWD/data:/data" stock-predicter
```

Or with Compose: uncomment `build: .` in `docker-compose.yml` and drop the `image:` line.

> Budget, currency, the model provider and its key, the risk limits, the polling cadence and the
> proxy are **not** container environment variables: they live in the database and are edited on the
> Settings page.

**Back up the whole `data` directory**, not just the `.db` file — SQLite in WAL mode keeps recent
commits in `-wal` beside it.

> **Security.** There is no authentication, by design: this is a single-user home tool.
> It binds to `127.0.0.1` locally and to the container's network in Docker. Put it on a trusted
> LAN or behind a VPN, never port-forward it to the internet, and treat the ntfy topic name as a
> secret. The DeepSeek key is stored in plaintext in the database.

## Development

```bash
cargo test                          # no test touches the network
cargo clippy --all-targets          # must be warning-free
cargo bench --bench hot_paths       # indicators, prompt render, bar upsert
cd web && npx vitest run && npm run build
```

The live model checks are opt-in and cost money:

```bash
DEEPSEEK_API_KEY=sk-... cargo test --test live_deepseek -- --ignored --nocapture
```

Design and plan live in `docs/superpowers/specs/` and `docs/superpowers/plans/`; the interface
system is `.interface-design/system.md`. `AGENTS.md` records the working agreements and the
performance budgets.

## Limitations worth knowing

- Yahoo Finance has no official API. Endpoints and shapes change without notice; all access goes
  through `src/market/yahoo.rs` so there is one place to fix.
- Headline coverage is uneven: Yahoo returns nothing at all for some NSE names, and those
  instruments are decided on prices alone.
- Mutual funds have no intraday price. They trade at NAV; cash arrives T+1/T+2.
- Cost basis is weighted-average, so a backdated buy restates the average used for later
  realised profit. FIFO lot accounting is the upgrade path.
- The trading mode's discovery (scanned candidates) is not built yet; both pots currently work
  from instruments you add yourself.
- Nothing places orders. Every suggestion is yours to execute at your broker.

## Licence

Apache-2.0, see [LICENSE](LICENSE).
