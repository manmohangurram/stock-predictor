# Configuration

Everything the app reads, and where to change it. The short version: three environment variables,
and everything else is a settings row edited on the Settings page.

**Three environment variables. Everything else is a settings row, edited in the UI.**

| Variable | Default | Meaning |
|---|---|---|
| `BIND` | `127.0.0.1:8080` | Listen address (`0.0.0.0:8080` in a container; the image already sets it). |
| `DB_PATH` | `data/stock-predictor.db` | SQLite file. The parent directory is created on boot. |
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

What this buys: a copied `stock-predictor.db`, a backup, or `sqlite3` open in a terminal shows
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
| **Topic** | The channel name. Make it long and random (`stock-predictor-7f3k9m2q`) — anyone who knows it can read your alerts. |
| **Server** | `https://ntfy.sh`, or your own instance. |
| **Access token** | Optional, only for a protected topic (`tk_…`). Write-only: it is never returned by the API and an empty field keeps the stored one. |
| **Link from the notification** | `http://192.168.1.10:8080` — tapping the push opens that instrument's page. Leave it blank for no link; a `127.0.0.1` link would be dead from a phone. |

Press **Send a test notification** and it arrives — that button uses the same send path as a real
signal, so a success there means a signal will reach you. From a terminal, the equivalent is:

```bash
./target/release/stock-predictor --test-notify
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

---

Back to the [README](../README.md) · how it decides in [how-it-works.md](how-it-works.md) · deployment in [deployment.md](deployment.md)
