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

**Trading pot** — the same instrument set, a separate budget, a separate book.

![Trading](docs/screenshots/trading.png)

**Settings** — budget and currency, then the two pots' risk limits side by side so they can be
compared, then the model provider (DeepSeek, OpenAI, Gemini or any OpenAI-compatible endpoint)
with its key, and the ntfy push channel with a test button. Keys are write-only: the API reports
only whether one is configured.

![Settings](docs/screenshots/settings.png)

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

The environment carries deployment settings only. **Budget, currency, API keys and ntfy are set
in the UI and live in the database** — there is one source of truth for each, and it is not the
environment, so a stale `.env` cannot silently override what you configured.

| Variable | Default | Meaning |
|---|---|---|
| `DB_PATH` | `data/stock-predicter.db` | SQLite file; the directory is created |
| `BIND` | `127.0.0.1:8080` | Listen address (use `0.0.0.0:8080` in Docker) |
| `POLL_SECS` | `900` | In-session cadence |
| `OFF_HOURS_SECS` | `7200` | Cadence outside 09:00–16:00 IST |
| `PROMPT_WINDOW_HOURS` | `4` | Hours of intraday tape per prompt (biggest token lever) |
| `LLM_MIN_PRICE_MOVE` | `2.0` | Skip a call unless the price moved this much |
| `LLM_MAX_SILENCE_SECS` | `3600` | ...but never stay quiet longer than this |
| `LLM_DAILY_CAP` | `200` | Hard ceiling on model calls per day |

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
| Minimum profit % | 5 | a take-profit that is not worth the trade |
| Maximum profit % | 50 | a fantasy target that will never fill |
| Minimum per trade | 500 | fills eaten by fees |
| Maximum per trade | 50,000 | one order becoming the portfolio |
| Maximum per instrument % | 25 | concentration — total exposure to one symbol, all fills counted |

Percentages are of that pot, so the caps keep their meaning when the budget changes. The
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

Build and run:

```bash
docker build -t stock-predicter .
docker run -d --name stock-predicter \
  -p 8080:8080 \
  -v "$PWD/data:/data" \
  -e BUDGET=500000 -e CURRENCY=INR \
  -e NTFY_TOPIC=your-private-topic \
  stock-predicter
```

Or with Compose, which is the same thing in a file:

```bash
docker compose up -d --build
```

A published multi-arch image is built on every `v*` tag:

```bash
docker run -d --name stock-predicter -p 8080:8080 -v "$PWD/data:/data" \
  ghcr.io/manmohangurram/stock-predictor:latest
```

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
