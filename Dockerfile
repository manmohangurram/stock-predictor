# syntax=docker/dockerfile:1.4
# One container: the Rust server serves both the JSON API and the built Vite app.
# Build stages run on BUILDPLATFORM (native speed, no QEMU) and cross-compile to
# TARGETARCH. cargo-chef caches the dependency build so only this crate recompiles
# when source changes.

FROM --platform=$BUILDPLATFORM node:22-alpine AS frontend
WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
# Same-origin: the browser calls the API on whatever host served the page.
RUN npm run build

FROM --platform=$BUILDPLATFORM rust:1.98 AS chef
RUN cargo install --locked cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH
RUN case ${TARGETARCH} in \
        arm64) rustup target add aarch64-unknown-linux-gnu && \
               apt-get update && apt-get install -y --no-install-recommends \
                 gcc-aarch64-linux-gnu libc6-dev-arm64-cross && \
               rm -rf /var/lib/apt/lists/*;; \
    esac
COPY --from=planner /app/recipe.json recipe.json
RUN case ${TARGETARCH} in \
        arm64) cargo chef cook --release --locked --target aarch64-unknown-linux-gnu --recipe-path recipe.json;; \
        *)     cargo chef cook --release --locked --recipe-path recipe.json;; \
    esac
COPY . .
RUN case ${TARGETARCH} in \
        arm64) cargo build --release --locked --target aarch64-unknown-linux-gnu && \
               cp target/aarch64-unknown-linux-gnu/release/stock-predicter /app/stock-predicter;; \
        *)     cargo build --release --locked && \
               cp target/release/stock-predicter /app/stock-predicter;; \
    esac

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /data app
WORKDIR /app
COPY --from=builder /app/stock-predicter /usr/local/bin/stock-predicter
COPY --from=frontend /app/web/dist ./web/dist

# The server has no authentication on purpose: it is meant for a home LAN.
# Binding 0.0.0.0 is what makes it usable in a container, so publish the port
# only on a trusted network and never port-forward it to the internet.
ENV BIND=0.0.0.0:8080 \
    DB_PATH=/data/stock-predicter.db
VOLUME ["/data"]
USER app
EXPOSE 8080
ENTRYPOINT ["stock-predicter"]
