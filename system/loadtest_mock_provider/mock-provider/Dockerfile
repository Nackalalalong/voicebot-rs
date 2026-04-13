# ── Stage 1: Build ───────────────────────────────────────────────────────────
# Build context must be the voicebot/ workspace root so Cargo.lock is available.
# docker compose sets `context: ../../../voicebot` — see docker-compose.yaml.
FROM rust:slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency compilation
COPY Cargo.toml Cargo.lock ./
# Minimal stubs for every workspace member so cargo can resolve the dep graph
RUN for crate in common vad asr agent tts core loadtest; do \
      mkdir -p crates/$crate/src && echo "// stub" > crates/$crate/src/lib.rs; \
    done && \
    mkdir -p crates/mock-provider/src && echo "fn main(){}" > crates/mock-provider/src/main.rs && \
    mkdir -p crates/transport/websocket/src && echo "// stub" > crates/transport/websocket/src/lib.rs && \
    mkdir -p crates/transport/asterisk/src  && echo "// stub" > crates/transport/asterisk/src/lib.rs && \
    mkdir -p crates/server/src              && echo "fn main(){}" > crates/server/src/main.rs && \
    for crate in common vad asr agent tts core loadtest mock-provider; do \
      cp /dev/null crates/$crate/Cargo.toml 2>/dev/null || true; \
    done
# Copy only the Cargo.toml files for proper dep caching
COPY crates/common/Cargo.toml        crates/common/Cargo.toml
COPY crates/vad/Cargo.toml           crates/vad/Cargo.toml
COPY crates/asr/Cargo.toml           crates/asr/Cargo.toml
COPY crates/agent/Cargo.toml         crates/agent/Cargo.toml
COPY crates/tts/Cargo.toml           crates/tts/Cargo.toml
COPY crates/core/Cargo.toml          crates/core/Cargo.toml
COPY crates/loadtest/Cargo.toml      crates/loadtest/Cargo.toml
COPY crates/mock-provider/Cargo.toml crates/mock-provider/Cargo.toml
COPY crates/transport/websocket/Cargo.toml crates/transport/websocket/Cargo.toml
COPY crates/transport/asterisk/Cargo.toml  crates/transport/asterisk/Cargo.toml
COPY crates/server/Cargo.toml        crates/server/Cargo.toml

RUN cargo build --release -p mock-provider 2>&1 | tail -5 || true

# Copy real source and rebuild
COPY crates/mock-provider/src/ crates/mock-provider/src/
RUN touch crates/mock-provider/src/main.rs && \
    cargo build --release -p mock-provider

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/mock-provider /usr/local/bin/mock-provider

EXPOSE 8000

ENV RUST_LOG=info

ENTRYPOINT ["mock-provider"]
