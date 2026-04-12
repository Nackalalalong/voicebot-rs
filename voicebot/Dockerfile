# ── Stage 1: Build (static musl — no glibc dependency) ───────────────────────
FROM rust:slim AS builder

WORKDIR /build

# musl-tools provides musl-gcc cross-linker; pkg-config for any sys crates
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

# Create stub lib/main files so `cargo build` can compile dependencies
RUN for crate in common vad asr agent tts core; do \
      mkdir -p crates/$crate/src && echo "// stub" > crates/$crate/src/lib.rs; \
    done && \
    mkdir -p crates/transport/websocket/src && echo "// stub" > crates/transport/websocket/src/lib.rs && \
    mkdir -p crates/transport/asterisk/src && echo "// stub" > crates/transport/asterisk/src/lib.rs && \
    mkdir -p crates/server/src && echo "fn main() {}" > crates/server/src/main.rs

RUN cargo build --release --target x86_64-unknown-linux-musl --bin voicebot 2>&1 | tail -5 || true

# Copy real source and rebuild
COPY crates/ crates/
RUN touch crates/*/src/*.rs crates/transport/*/src/*.rs 2>/dev/null || true && \
    cargo build --release --target x86_64-unknown-linux-musl --bin voicebot

# ── Stage 2: Runtime (tiny — static binary needs no libc) ────────────────────
FROM alpine:3

RUN apk add --no-cache ca-certificates

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/voicebot /usr/local/bin/voicebot

WORKDIR /etc/voicebot

EXPOSE 8080

ENTRYPOINT ["voicebot", "/etc/voicebot/config.toml"]
