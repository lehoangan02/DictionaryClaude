#!/usr/bin/env bash
# Installs the sync server as a systemd service on this machine. Run with sudo:
#   sudo deploy/install-syncd.sh
# It opens: 25 (home network only) and 5025 (anywhere, for use away from home).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=target/release/gredict-syncd
[ -x "$BIN" ] || { echo "build first: cargo build --release -p gredict-syncd"; exit 1; }
install -m 0755 "$BIN" /usr/local/bin/gredict-syncd
install -d -o lhan -g lhan /var/lib/lexis-syncd
install -m 0644 deploy/gredict-syncd.service /etc/systemd/system/gredict-syncd.service
systemctl daemon-reload
systemctl enable --now gredict-syncd
if command -v ufw >/dev/null && ufw status | grep -q "Status: active"; then
  ufw allow from 192.168.1.0/24 to any port 25 proto tcp comment 'lexis sync (home network)'
  ufw allow 5025/tcp comment 'lexis sync (remote)'
fi
echo
echo "Running. Pair a device with:"
echo "  sudo -u lhan /usr/local/bin/gredict-syncd --data /var/lib/lexis-syncd pair <device-name>"
echo "Your router must forward external TCP 5025 to this machine (192.168.1.7)."
