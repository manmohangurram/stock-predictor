# syntax=docker/dockerfile:1.4
# One container: the Go server serves both the JSON API and the built Vite app.
# The database driver is pure Go, so CGO stays off and the cross-compile needs no toolchain:
# arm64 is one GOARCH value, not a whole sysroot.

FROM --platform=$BUILDPLATFORM node:22-alpine AS frontend
WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY web/ ./
# Same-origin: the browser calls the API on whatever host served the page.
RUN npm run build

FROM --platform=$BUILDPLATFORM golang:1.27-bookworm AS builder
ARG TARGETARCH
ENV CGO_ENABLED=0
WORKDIR /app
# Dependencies first: this layer only moves when go.mod/go.sum do.
COPY go.mod go.sum ./
RUN --mount=type=cache,target=/go/pkg/mod go mod download
COPY . .
RUN --mount=type=cache,target=/go/pkg/mod \
    --mount=type=cache,target=/root/.cache/go-build \
    GOOS=linux GOARCH=${TARGETARCH} go build -trimpath -ldflags="-s -w" \
    -o /app/stock-predictor ./cmd/server

FROM debian:trixie-slim AS runtime
# ca-certificates for HTTPS; tzdata so IST formatting is never at the mercy of the host.
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
