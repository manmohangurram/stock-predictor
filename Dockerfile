# syntax=docker/dockerfile:1.4
# One container: the Rust server serves both the JSON API and the built Vite app.
# Build stages run on BUILDPLATFORM (native speed, no QEMU): they build natively
# when that equals TARGETPLATFORM and cross-compile otherwise. cargo-chef plus
# cargo cache mounts keep the dependency build cached so only this crate
# recompiles when source changes.

FROM --platform=$BUILDPLATFORM node:22-alpine AS frontend
WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
# Same-origin: the browser calls the API on whatever host served the page.
RUN npm run build

FROM --platform=$BUILDPLATFORM rust:1.98 AS chef
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install --locked cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG TARGETARCH
ARG TARGETPLATFORM
ARG BUILDPLATFORM

# Cross-compile only when the build host differs from the target. A native arm64
# runner (build == target) uses its own toolchain, so the dependency layers are
# stable and cache from one arm64 build to the next. The cross toolchain and
# `--target` are paid for only when building arm64 from an amd64 host.
RUN if [ "${BUILDPLATFORM}" != "${TARGETPLATFORM}" ]; then \
        case "${TARGETARCH}" in \
            arm64) rustup target add aarch64-unknown-linux-gnu && \
                   apt-get update && apt-get install -y --no-install-recommends \
                     gcc-aarch64-linux-gnu libc6-dev-arm64-cross && \
                   rm -rf /var/lib/apt/lists/*;; \
        esac; \
    fi
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    if [ "${BUILDPLATFORM}" != "${TARGETPLATFORM}" ] && [ "${TARGETARCH}" = "arm64" ]; then \
        CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
        cargo chef cook --release --locked --target aarch64-unknown-linux-gnu --recipe-path recipe.json; \
    else \
        cargo chef cook --release --locked --recipe-path recipe.json; \
    fi
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    if [ "${BUILDPLATFORM}" != "${TARGETPLATFORM}" ] && [ "${TARGETARCH}" = "arm64" ]; then \
        CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
        cargo build --release --locked --target aarch64-unknown-linux-gnu && \
        cp target/aarch64-unknown-linux-gnu/release/stock-predictor /app/stock-predictor; \
    else \
        cargo build --release --locked && \
        cp target/release/stock-predictor /app/stock-predictor; \
    fi

# Must match the builder's glibc: rust:1.98 is Debian trixie (2.41). A bookworm runtime here
# produced binaries asking for GLIBC_2.38 and failing at startup on arm64.
FROM debian:trixie-slim AS runtime
# Runs as root inside the container, like the sibling financer image, so a bind mount of ./data
# works whatever uid owns it on the host. No USER, no useradd: a mismatched uid was making
# SQLite fail with "unable to open database file".
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/stock-predictor /usr/local/bin/stock-predictor
COPY --from=frontend /app/web/dist ./web/dist

# The server has no authentication on purpose: it is meant for a home LAN.
# Binding 0.0.0.0 is what makes it usable in a container, so publish the port
# only on a trusted network and never port-forward it to the internet.
ENV BIND=0.0.0.0:8080 \
    DB_PATH=/data/stock-predictor.db
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["stock-predictor"]
