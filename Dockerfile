# syntax=docker/dockerfile:1.6

# ─── Build stage ──────────────────────────────────────────────
FROM rust:1.78-slim AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency builds before copying source
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src target/release/deps/ferrobank*

COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY .sqlx ./.sqlx

ENV SQLX_OFFLINE=true
RUN cargo build --release

# ─── Runtime stage ────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/ferrobank /usr/local/bin/ferrobank
COPY --from=builder /app/migrations ./migrations
COPY static ./static

EXPOSE 8080
ENV APP_HOST=0.0.0.0
ENV APP_PORT=8080

CMD ["ferrobank"]
