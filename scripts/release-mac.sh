#!/usr/bin/env bash
# Builds the macOS (Apple silicon) DMG on a Mac and attaches it to an existing
# release, replacing any Mac files already there.
#
#   scripts/release-mac.sh v0.5.0
#
# The same build runs in CI (.github/workflows/release-mac.yml) whenever
# scripts/release.sh publishes a release; use this script to redo or replace it
# by hand. The app is ad-hoc signed (tauri.conf.json: signingIdentity "-"), which
# stops macOS calling a downloaded app "damaged". Needs `gh` signed in.
set -euo pipefail
cd "$(dirname "$0")/.."
TAG="${1:-}"
[[ "$TAG" =~ ^v0\.[0-9]+\.[0-9]+$ ]] || { echo "usage: $0 v0.MINOR.PATCH" >&2; exit 2; }
[ "$(uname -s)" = Darwin ] || { echo "ERROR: this builds the macOS app and must run on a Mac" >&2; exit 1; }
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
VERSION="${TAG#v}"
grep -q "\"version\": \"$VERSION\"" app/src-tauri/tauri.conf.json || echo "NOTE: tauri.conf.json is not at $VERSION; the DMG is built from this checkout but named for $TAG"

echo "STEP: building the interface"
(cd app/ui && npm ci >/dev/null && npm run build 2>&1 | tail -n 1)

echo "STEP: building the app (dictionaries are downloaded inside the app, so none are bundled)"
(cd app/src-tauri && cargo tauri build --bundles dmg --config '{"bundle":{"resources":null},"build":{"beforeBuildCommand":""}}' 2>&1 | tail -n 3)

APP="app/src-tauri/target/release/bundle/macos/LadyBird.app"
codesign --verify --deep --strict "$APP" || { echo "ERROR: the app is not signed" >&2; exit 1; }
DMG="$(ls app/src-tauri/target/release/bundle/dmg/*.dmg | head -n 1)"

OUT="${LEXIS_RELEASES:-$HOME/lexis-releases}"; mkdir -p "$OUT"
FILE="LadyBird-${VERSION}-macos-arm64.dmg"
cp "$DMG" "$OUT/$FILE"
(cd "$OUT" && shasum -a 256 "$FILE" > "$FILE.sha256")

echo "STEP: attaching to $TAG"
gh release upload "$TAG" "$OUT/$FILE" "$OUT/$FILE.sha256" --clobber
echo "STEP: done $TAG ($(du -h "$OUT/$FILE" | cut -f1))"
