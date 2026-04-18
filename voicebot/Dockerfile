# ── Stage 1: Build (static musl — no glibc dependency) ───────────────────────
FROM rust:slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    musl-tools \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl

# Cache dependency compilation by copying manifests first
COPY Cargo.toml Cargo.lock ./
COPY crates/common/Cargo.toml       crates/common/Cargo.toml
COPY crates/vad/Cargo.toml          crates/vad/Cargo.toml
COPY crates/asr/Cargo.toml          crates/asr/Cargo.toml
COPY crates/agent/Cargo.toml        crates/agent/Cargo.toml
COPY crates/tts/Cargo.toml          crates/tts/Cargo.toml
COPY crates/core/Cargo.toml         crates/core/Cargo.toml
COPY crates/transport/websocket/Cargo.toml  crates/transport/websocket/Cargo.toml
COPY crates/transport/asterisk/Cargo.toml   crates/transport/asterisk/Cargo.toml
COPY crates/server/Cargo.toml       crates/server/Cargo.toml
COPY crates/db/Cargo.toml           crates/db/Cargo.toml
COPY crates/cache/Cargo.toml        crates/cache/Cargo.toml
COPY crates/auth/Cargo.toml         crates/auth/Cargo.toml
COPY crates/storage/Cargo.toml      crates/storage/Cargo.toml
COPY crates/api/Cargo.toml          crates/api/Cargo.toml
COPY crates/scheduler/Cargo.toml    crates/scheduler/Cargo.toml

# Stub sources for dependency caching
RUN for crate in common vad asr agent tts core db cache auth storage; do \
      mkdir -p crates/$crate/src && echo "// stub" > crates/$crate/src/lib.rs; \
    done && \
    mkdir -p crates/transport/websocket/src && echo "// stub" > crates/transport/websocket/src/lib.rs && \
    mkdir -p crates/transport/asterisk/src && echo "// stub" > crates/transport/asterisk/src/lib.rs && \
    mkdir -p crates/server/src && echo "fn main() {}" > crates/server/src/main.rs && \
    mkdir -p crates/api/src && echo "fn main() {}" > crates/api/src/main.rs && echo "// stub" > crates/api/src/lib.rs && \
    mkdir -p crates/scheduler/src && echo "fn main() {}" > crates/scheduler/src/main.rs && echo "// stub" > crates/scheduler/src/lib.rs

RUN cargo build --release --target x86_64-unknown-linux-musl 2>&1 | tail -5 || true

# Copy real source and rebuild all binaries
COPY crates/ crates/
# Touch migration files so they're embedded
COPY crates/db/migrations/ crates/db/migrations/
RUN touch crates/*/src/*.rs crates/transport/*/src/*.rs crates/api/src/routes/*.rs 2>/dev/null || true && \
    cargo build --release --target x86_64-unknown-linux-musl \
        --bin voicebot --bin voicebot-api --bin voicebot-scheduler

# ── Stage 2: Runtime for voicebot core (existing server binary) ───────────────
FROM alpine:3 AS server

RUN apk add --no-cache ca-certificates
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/voicebot /usr/local/bin/voicebot
WORKDIR /etc/voicebot
EXPOSE 8090
ENTRYPOINT ["voicebot", "/etc/voicebot/config.toml"]

# ── Stage 3: Runtime for Management API ───────────────────────────────────────
FROM alpine:3 AS api

RUN apk add --no-cache ca-certificates
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/voicebot-api /usr/local/bin/voicebot-api
EXPOSE 8080
ENTRYPOINT ["voicebot-api"]

# ── Stage 4: Runtime for Campaign Scheduler ───────────────────────────────────
FROM alpine:3 AS scheduler

RUN apk add --no-cache ca-certificates
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/voicebot-scheduler /usr/local/bin/voicebot-scheduler
ENTRYPOINT ["voicebot-scheduler"]

