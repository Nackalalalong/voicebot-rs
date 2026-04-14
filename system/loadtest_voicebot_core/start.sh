#!/usr/bin/env bash
# start.sh — Start the voicebot-core WebSocket load test
#
# Usage (from project root):
#   system/loadtest_voicebot_core/start.sh [--no-build] [--calls N] [--concurrency N]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
THIS_DIR="$REPO_ROOT/system/loadtest_voicebot_core"
COMPOSE_FILE="$THIS_DIR/docker-compose.yaml"
VOICEBOT_HOST_PORT="${VOICEBOT_PORT:-18080}"

NO_BUILD=false
OVERRIDE_CALLS=""
OVERRIDE_CONCURRENCY=""

for arg in "$@"; do
  case $arg in
    --no-build)         NO_BUILD=true ;;
    --calls)            shift; OVERRIDE_CALLS="$1" ;;
    --concurrency)      shift; OVERRIDE_CONCURRENCY="$1" ;;
    *) ;;
  esac
done

COMPOSE=(docker compose -f "$COMPOSE_FILE")

# ── 1. Docker stack ──────────────────────────────────────────────────────────
echo "▸ Starting Docker stack (voicebot + mock-provider)..."
if [[ "$NO_BUILD" == "true" ]]; then
  "${COMPOSE[@]}" up -d mock-provider voicebot
else
  "${COMPOSE[@]}" up -d --build mock-provider voicebot
fi

PUBLISHED_WS_PORT="$("${COMPOSE[@]}" port voicebot 8080 | sed -n 's/.*://p' | tail -1)"
if [[ -z "$PUBLISHED_WS_PORT" ]]; then
  echo "ERROR: docker did not publish voicebot port 8080 to the host."
  echo "       Inspect with: docker compose -f $COMPOSE_FILE ps"
  exit 1
fi

if [[ "$PUBLISHED_WS_PORT" != "$VOICEBOT_HOST_PORT" ]]; then
  echo "ERROR: expected voicebot host port $VOICEBOT_HOST_PORT but docker published $PUBLISHED_WS_PORT"
  echo "       Inspect with: docker compose -f $COMPOSE_FILE ps"
  exit 1
fi

VOICEBOT_CONTAINER_ID="$("${COMPOSE[@]}" ps -q voicebot)"
if [[ -z "$VOICEBOT_CONTAINER_ID" ]]; then
  echo "ERROR: could not find the voicebot container id"
  exit 1
fi

echo "  Waiting for dockerized voicebot server to become healthy ..."
for i in $(seq 1 30); do
  HEALTH_STATUS="$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "$VOICEBOT_CONTAINER_ID")"
  if [[ "$HEALTH_STATUS" == "healthy" ]]; then
    echo "  Voicebot server is healthy and published on port $VOICEBOT_HOST_PORT."
    break
  fi
  if [[ $i -eq 30 ]]; then
    echo "ERROR: voicebot server did not become healthy after 30s"
    echo "       Check logs:  docker logs ws-loadtest-voicebot"
    exit 1
  fi
  sleep 1
done

# ── 2. Run loadtest ───────────────────────────────────────────────────────────
# Allow one-shot overrides via env or flags without editing the TOML.
EXTRA_ENV=""
[[ -n "$OVERRIDE_CALLS" ]]       && echo "  override total_calls=$OVERRIDE_CALLS (env TOTAL_CALLS)"
[[ -n "$OVERRIDE_CONCURRENCY" ]] && echo "  override concurrency=$OVERRIDE_CONCURRENCY (env CONCURRENCY)"

if [[ "$NO_BUILD" == "false" ]]; then
  echo "▸ Building loadtest service image..."
  "${COMPOSE[@]}" --profile loadtest build loadtest
fi

echo "▸ Running load test inside Docker Compose (service: loadtest)..."
"${COMPOSE[@]}" --profile loadtest run --rm --no-deps loadtest

echo ""
echo "▸ Done.  Artifacts: $REPO_ROOT/voicebot/artifacts/loadtest/"
echo "   Logs:  docker logs ws-loadtest-voicebot"
echo "   Stop:  docker compose -f $COMPOSE_FILE down"
