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
cat > "$DATA/bin/agent" <<'EOF'
#!/bin/sh
echo '{"status":"ERROR","error":"test stand-in"}'
exit 3
EOF
chmod +x "$DATA/bin/agent"
export LEXIS_CLAUDE_BIN="$DATA/bin/agent" LEXIS_AGY_BIN="$DATA/bin/agent"
# Work chats make git worktrees and branches in the app's checkout: give them a throwaway one, never this repo.
mkdir -p "$DATA/repo/docs" && echo "# Roadmap" > "$DATA/repo/docs/ROADMAP.md"
git -C "$DATA/repo" init -q && git -C "$DATA/repo" add -A && git -C "$DATA/repo" -c user.name=e2e -c user.email=e2e@localhost commit -qm init
export LEXIS_REPO="$DATA/repo"
./target/release/gredict-syncd --data "$DATA/syncd" --san 127.0.0.1 --san localhost serve --lan "127.0.0.1:$LAN" --wan "127.0.0.1:$WAN" >"$DATA/syncd.log" 2>&1 &
PIDS+=($!)
# The Oxford recordings are large and optional: the test of them runs only where they have been built.
if [ -f private/oald-audio.pack ]; then mkdir -p "$DATA/a/packs"; ln -s "$PWD/private/oald-audio.pack" "$DATA/a/packs/oald-audio.pack"; export HAVE_OALD_AUDIO=1; fi
./target/release/gredict-server --port "$A" --data "$DATA/a" >"$DATA/a.log" 2>&1 &
PIDS+=($!)
./target/release/gredict-server --port "$B" --data "$DATA/b" >"$DATA/b.log" 2>&1 &
PIDS+=($!)
for p in $A $B; do for _ in $(seq 50); do curl -sf -X POST "localhost:$p/api/sources" >/dev/null && break; sleep 0.2; done; done
# Never let a test device auto-pair with the real sync server (the default address is the owner's own box).
for p in $A $B; do curl -sf -X POST "localhost:$p/api/sync_set_autopair_host" -d '{"host":"127.0.0.1:1"}' >/dev/null; done
pair() { ./target/release/gredict-syncd --data "$DATA/syncd" --san 127.0.0.1 --san localhost pair "$1" --lan "127.0.0.1:$LAN" --wan "127.0.0.1:$WAN"; }
export PAIR_A=$(pair device-a) PAIR_B=$(pair device-b)
BASE="http://127.0.0.1:$A/" BASE_B="http://127.0.0.1:$B/" node "${SCRIPT:-app/ui/e2e/smoke.mjs}"
