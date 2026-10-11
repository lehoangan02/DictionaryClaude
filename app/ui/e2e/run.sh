#!/usr/bin/env bash
# Hermetic UI test: a sync server, two app instances ("two devices"), built UI.
set -euo pipefail
cd "$(dirname "$0")/../../.."
A=${PORT:-8799}; B=$((A+1)); LAN=$((A+10)); WAN=$((A+11))
DATA=$(mktemp -d)
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$DATA"; }
trap cleanup EXIT
(cd app/ui && npm run build >/dev/null)
export LEXIS_WIFI=Anan   # pretend to be on the home network
export LEXIS_OCR=tesseract   # fast and hermetic; the vision model is checked by hand
# Tests must never spend agent quota: any agent request goes to harmless stand-ins.
mkdir -p "$DATA/bin"
printf '#!/bin/sh\necho {\"status\":\"ERROR\",\"error\":\"test stand-in\"}\nexit 3\n' > "$DATA/bin/agent"; chmod +x "$DATA/bin/agent"
export LEXIS_CLAUDE_BIN="$DATA/bin/agent" LEXIS_AGY_BIN="$DATA/bin/agent"
./target/release/gredict-syncd --data "$DATA/syncd" --san 127.0.0.1 --san localhost serve --lan "127.0.0.1:$LAN" --wan "127.0.0.1:$WAN" >"$DATA/syncd.log" 2>&1 &
PIDS+=($!)
./target/release/gredict-server --port "$A" --data "$DATA/a" >"$DATA/a.log" 2>&1 &
PIDS+=($!)
./target/release/gredict-server --port "$B" --data "$DATA/b" >"$DATA/b.log" 2>&1 &
PIDS+=($!)
for p in $A $B; do for _ in $(seq 50); do curl -sf -X POST "localhost:$p/api/sources" >/dev/null && break; sleep 0.2; done; done
pair() { ./target/release/gredict-syncd --data "$DATA/syncd" --san 127.0.0.1 --san localhost pair "$1" --lan "127.0.0.1:$LAN" --wan "127.0.0.1:$WAN"; }
export PAIR_A=$(pair device-a) PAIR_B=$(pair device-b)
BASE="http://127.0.0.1:$A/" BASE_B="http://127.0.0.1:$B/" node "${SCRIPT:-app/ui/e2e/smoke.mjs}"
