#!/usr/bin/env bash
# start.sh — Start everything needed for the voicebot-core demo
# Usage: ./start.sh [--no-build] [--web-port PORT]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEACHES_COMPOSE="$REPO_ROOT/system/speaches/compose.cpu.yaml"
VOICEBOT_DIR="$REPO_ROOT/voicebot"
DEMO_DIR="$REPO_ROOT/system/voicebot-core-demo"

WEB_PORT="${WEB_PORT:-3000}"
VOICEBOT_PORT="${VOICEBOT_PORT:-8080}"
NO_BUILD=false

for arg in "$@"; do
  case $arg in
    --no-build) NO_BUILD=true ;;
    --web-port) shift; WEB_PORT="$1" ;;
    *) ;;
  esac
done

# ── cleanup on exit ─────────────────────────────────────────────────────────
PIDS=()
cleanup() {
  echo ""
  echo "→ Shutting down..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
  echo "→ Done."
}
trap cleanup EXIT INT TERM

# ── 1. Speaches ──────────────────────────────────────────────────────────────
echo "▸ Starting Speaches (Docker)..."
if ! docker compose -f "$SPEACHES_COMPOSE" up -d 2>&1; then
  echo "ERROR: Failed to start Speaches. Is Docker running?"
  exit 1
fi

# Wait for Speaches to be healthy
echo "  Waiting for Speaches on http://localhost:8000/health ..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:8000/health >/dev/null 2>&1; then
    echo "  Speaches is ready."
    break
  fi
  if [[ $i -eq 30 ]]; then
    echo "ERROR: Speaches did not become healthy after 30s"
    exit 1
  fi
  sleep 1
done

# ── 2. Voicebot server ───────────────────────────────────────────────────────
if [[ "$NO_BUILD" == "false" ]]; then
  echo "▸ Building voicebot server..."
  (cd "$VOICEBOT_DIR" && cargo build --bin voicebot --release 2>&1)
fi

echo "▸ Starting voicebot server on port $VOICEBOT_PORT..."
VOICEBOT_BIN="$VOICEBOT_DIR/target/release/voicebot"
if [[ ! -x "$VOICEBOT_BIN" ]]; then
  echo "ERROR: Binary not found at $VOICEBOT_BIN — remove --no-build to compile first"
  exit 1
fi

(cd "$VOICEBOT_DIR" && "$VOICEBOT_BIN" config.toml) &
VOICEBOT_PID=$!
PIDS+=("$VOICEBOT_PID")

# Wait for voicebot server to be ready
echo "  Waiting for voicebot server on http://localhost:$VOICEBOT_PORT ..."
for i in $(seq 1 20); do
  if curl -sf http://localhost:$VOICEBOT_PORT/ >/dev/null 2>&1 ||
     nc -z localhost $VOICEBOT_PORT 2>/dev/null; then
    echo "  Voicebot server is ready."
    break
  fi
  if [[ $i -eq 20 ]]; then
    echo "  (voicebot server may still be starting — continuing anyway)"
    break
  fi
  sleep 1
done

# ── 3. Web demo ───────────────────────────────────────────────────────────────
echo "▸ Serving web demo on http://localhost:$WEB_PORT ..."
if command -v python3 &>/dev/null; then
  (cd "$DEMO_DIR" && python3 -m http.server "$WEB_PORT") &
elif command -v npx &>/dev/null; then
  (cd "$DEMO_DIR" && npx serve -l "$WEB_PORT" .) &
else
  echo "ERROR: No static file server found. Install python3 or Node.js."
  exit 1
fi
PIDS+=($!)

# ── Ready ─────────────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════════════╗"
echo "║  Voicebot Core Demo is running               ║"
echo "║                                              ║"
echo "║  Web Demo:      http://localhost:$WEB_PORT          ║"
echo "║  Voicebot WS:   ws://localhost:$VOICEBOT_PORT/ws     ║"
echo "║  Speaches API:  http://localhost:8000        ║"
echo "║                                              ║"
echo "║  Press Ctrl+C to stop everything            ║"
echo "╚══════════════════════════════════════════════╝"
echo ""

# Keep running until Ctrl+C
wait "${PIDS[0]}"
