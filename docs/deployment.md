# Running it 24×7

Docker, Compose, a systemd unit, backups, and the container's one sharp edge (uid and `/data`).

The server is a single long-lived process with no session or login: start it and it keeps
polling. On a home server, either run the container with `restart: unless-stopped` (the Compose
file does) or a systemd unit:

```ini
[Unit]
Description=stock-predictor
After=network-online.target

[Service]
User=stock
WorkingDirectory=/srv/stock-predictor
ExecStart=/srv/stock-predictor/stock-predictor
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
  stock-predictor:
    image: ghcr.io/manmohangurram/stock-predictor:${IMAGE_TAG:-latest}
    container_name: stock-predictor
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
`/data/stock-predictor.db`, and declares `/data` as its volume. Copy `.env.example` to `.env`
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
| `DB_PATH` | `data/stock-predictor.db` | SQLite file. The image sets `/data/stock-predictor.db`. |
| `PUBLIC_URL` | *(empty)* | The address the app is reachable at, for the deep link on a notification. |
| `SECRET_KEY` | *(key file)* | 32 bytes, 64 hex characters or base64, encrypting stored secrets. |
| `TZ` | `UTC` | The container clock's zone. Cosmetic: the NSE window is IST and storage is UTC. |

The app also serves an icon set (`favicon.svg`, `icon-192.png`, `icon-512.png`,
`apple-touch-icon.png`, `manifest.webmanifest`) so "Add to Home Screen" gives you the real icon and a
standalone window rather than a browser tab.

### Plain Docker

```bash
docker run -d --name stock-predictor \
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
docker build -t stock-predictor .
docker run -d --name stock-predictor -p 127.0.0.1:8080:8080 -v "$PWD/data:/data" stock-predictor
```

Or with Compose: uncomment `build: .` in `docker-compose.yml` and drop the `image:` line.

> Budget, currency, the model provider and its key, the risk limits, the polling cadence and the
> proxy are **not** container environment variables: they live in the database and are edited on the
> Settings page.

**Back up the whole `data` directory**, not just the `.db` file — SQLite in WAL mode keeps recent
commits in `-wal` beside it.

## Security

There is **no authentication**, by design: this is a single-user home tool. It binds to `127.0.0.1`
locally and to the container's network in Docker. Put it on a trusted LAN or behind a VPN, never
port-forward it to the internet, and treat the ntfy topic name as a secret — anyone who knows it can
read your notifications.

Provider keys, the ntfy token and the proxy password are encrypted at rest (XChaCha20-Poly1305, key
from `SECRET_KEY`). That protects a copied `stock-predictor.db`, a backup, or `sqlite3` open in a
terminal. It does **not** protect a machine someone already has, and it is not authentication.

If you run it behind the bundled Compose file, no port is published at all: the container is reachable
only through the `proxy` network. Add `ports: ["8080:8080"]` only for a LAN trial.

---

Back to the [README](../README.md) · every setting in [configuration.md](configuration.md)
