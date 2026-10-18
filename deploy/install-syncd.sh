#!/usr/bin/env bash
# Installs the sync server as a systemd service on this machine. Run with sudo:
#   sudo deploy/install-syncd.sh
# Ports: 25 (home network) and 5025 (remote; forward it on the router).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=target/release/gredict-syncd
[ -x "$BIN" ] || { echo "build first: cargo build --release -p gredict-syncd"; exit 1; }
STATE=/var/lib/lexis-syncd
install -d -o lhan -g lhan "$STATE" "$STATE/private" "$STATE/packs" "$STATE/bin"
# Owned by the service, which replaces it with its own rebuild after a chat merge (LEXIS_SELF_UPDATE).
install -o lhan -g lhan -m 0755 "$BIN" "$STATE/bin/gredict-syncd"
git -c safe.directory="$PWD" rev-parse HEAD > "$STATE/bin/gredict-syncd.commit" && chown lhan:lhan "$STATE/bin/gredict-syncd.commit"
ln -sf "$STATE/bin/gredict-syncd" /usr/local/bin/gredict-syncd
# The parsed Barron's book (never the PDF) is what paired devices receive.
if [ -f private/barrons.sqlite ]; then install -o lhan -g lhan -m 0644 private/barrons.sqlite "$STATE/private/barrons.sqlite"; fi
# The dictionary packs paired devices can download (Settings -> Dictionaries).
for f in data/packs/*.pack data/packs/*.fst data/packs/*.onnx data/packs/*.json; do [ -f "$f" ] && install -o lhan -g lhan -m 0644 "$f" "$STATE/packs/$(basename "$f")"; done
# The OALD recordings (Oxford's copyrighted audio) live in private/ so no build bundles them; devices download them on request.
if [ -f private/oald-audio.pack ]; then install -o lhan -g lhan -m 0644 private/oald-audio.pack "$STATE/packs/oald-audio.pack"; fi
install -m 0644 deploy/gredict-syncd.service /etc/systemd/system/gredict-syncd.service
systemctl daemon-reload
systemctl enable gredict-syncd
systemctl restart gredict-syncd
if command -v ufw >/dev/null && ufw status | grep -q "Status: active"; then
  ufw allow from 192.168.1.0/24 to any port 25 proto tcp comment 'lexis sync (home network)'
  ufw allow 5025/tcp comment 'lexis sync (remote)'
fi
sleep 1
systemctl --no-pager --lines=5 status gredict-syncd || true
echo
echo "Pair a device:  sudo -u lhan /usr/local/bin/gredict-syncd --data $STATE pair <device-name>"
