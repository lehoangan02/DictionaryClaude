#!/usr/bin/env bash
# Builds, tags and (optionally) publishes a LadyBird release.
#
#   scripts/release.sh <version> [--publish] [--skip-tests] [--notes "text"]
#
# <version> is 0.MINOR.PATCH: bump MINOR for features, PATCH for fixes.
# Without --publish nothing leaves this machine: the AppImage lands in
# $LEXIS_RELEASES (default ~/lexis-releases) and a local tag is made.
# With --publish the branch and tag are pushed and a GitHub release is created
# with the AppImage attached (needs `gh` signed in).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-}"; shift || true
PUBLISH=0; TESTS=1; ANDROID=1; NOTES=""
while [ $# -gt 0 ]; do
  case "$1" in
    --publish) PUBLISH=1 ;;
    --skip-tests) TESTS=0 ;;
    --no-android) ANDROID=0 ;;
    --notes) NOTES="$2"; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
  shift
done
[[ "$VERSION" =~ ^0\.[0-9]+\.[0-9]+$ ]] || { echo "usage: $0 0.MINOR.PATCH [--publish] [--skip-tests] [--notes TEXT]" >&2; exit 2; }
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
step() { echo "STEP: $*"; }

OUT="${LEXIS_RELEASES:-$HOME/lexis-releases}"
TAG="v$VERSION"
BRANCH="$(git rev-parse --abbrev-ref HEAD)"

step "checking the working tree"
if [ -n "$(git status --porcelain)" ]; then
  echo "ERROR: uncommitted changes; commit or discard them first:" >&2; git status --short >&2; exit 1
fi
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then echo "ERROR: $TAG already exists" >&2; exit 1; fi
PREV="$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"

if [ "$TESTS" = 1 ]; then
  step "running the Rust tests"
  cargo test --workspace 2>&1 | tail -n 5
  step "type-checking and building the interface"
  (cd app/ui && npm run check 2>&1 | tail -n 2 && npm run build 2>&1 | tail -n 1)
  step "running the browser test"
  (cd app/ui && npm run e2e 2>&1 | grep -E 'FAIL|passed' | tail -n 3)
fi

step "setting the version to $VERSION"
sed -i -E "s/^(  \"version\": \")[0-9]+\.[0-9]+\.[0-9]+(\",)/\1$VERSION\2/" app/src-tauri/tauri.conf.json
sed -i -E "0,/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/s//version = \"$VERSION\"/" app/src-tauri/Cargo.toml
grep -q "\"version\": \"$VERSION\"" app/src-tauri/tauri.conf.json && grep -q "^version = \"$VERSION\"" app/src-tauri/Cargo.toml

step "building the AppImage (this takes a few minutes)"
(cd app/src-tauri && APPIMAGE_EXTRACT_AND_RUN=1 cargo tauri build --bundles appimage 2>&1 | tail -n 3)
BUILT="app/src-tauri/target/release/bundle/appimage/LadyBird_${VERSION}_amd64.AppImage"
[ -f "$BUILT" ] || { echo "ERROR: the build did not produce $BUILT" >&2; exit 1; }

APK=""
if [ "$ANDROID" = 1 ] && [ -d "$HOME/Android/Sdk/ndk" ] && [ -f "$HOME/.lexis-keys/ladybird.jks" ]; then
  step "building the Android APK (arm64, signed with the LadyBird key)"
  export ANDROID_HOME="$HOME/Android/Sdk" NDK_HOME="$(ls -d "$HOME"/Android/Sdk/ndk/* | tail -n 1)"
  export JAVA_HOME="${JAVA_HOME:-$(dirname "$(dirname "$(readlink -f "$(command -v java)")")")}"
  BT="$(ls -d "$ANDROID_HOME"/build-tools/* | tail -n 1)"
  scripts/fetch-ort-android.sh
  (cd app/src-tauri && cargo tauri android build --apk --target aarch64 2>&1 | tail -n 2)
  RAW="app/src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk"
  [ -f "$RAW" ] || { echo "ERROR: the Android build did not produce an APK" >&2; exit 1; }
  "$BT/zipalign" -f -p 4 "$RAW" /tmp/lb-aligned.apk
  mkdir -p "$OUT"
  APK="LadyBird-${VERSION}-android-arm64.apk"
  "$BT/apksigner" sign --ks "$HOME/.lexis-keys/ladybird.jks" --ks-pass "file:$HOME/.lexis-keys/ladybird.pass" --out "$OUT/$APK" /tmp/lb-aligned.apk
  rm -f /tmp/lb-aligned.apk "$OUT/$APK.idsig"
  (cd "$OUT" && sha256sum "$APK" > "$APK.sha256")
else
  echo "NOTE: skipping the Android APK (no NDK or signing key here)"
fi

step "saving the release files"
mkdir -p "$OUT"
FILE="LadyBird-${VERSION}-x86_64.AppImage"
cp "$BUILT" "$OUT/$FILE"
SUM="$(sha256sum "$OUT/$FILE" | cut -d' ' -f1)"
echo "$SUM  $FILE" > "$OUT/$FILE.sha256"
if [ -n "$PREV" ]; then CHANGES="$(git log "$PREV..HEAD" --pretty='- %s' | grep -v '^- Release v' | head -n 40)"; else CHANGES="$(git log --pretty='- %s' | head -n 30)"; fi
BODY="${NOTES:+$NOTES

}## Changes
${CHANGES:-- (no changes recorded)}

## Install (Linux x86_64)
\`\`\`
chmod +x $FILE && ./$FILE
\`\`\`
Check the download with \`sha256sum -c $FILE.sha256\`. The app carries its dictionaries (Wiktionary, CC BY-SA 4.0; WordNet, CC BY 4.0; Webster 1913, Moby and CMUdict, public domain). It does not contain Barron's or OALD (Oxford's dictionary and recordings are downloaded from your own server, on request).
Android (arm64, for example the Galaxy S24 Ultra): install the .apk and download dictionaries in Settings → Downloads after pairing with your server. macOS (Apple silicon): open the .dmg and drag LadyBird to Applications; it is built and signed for you by the release-mac workflow. If macOS says the app is damaged, run \`xattr -dr com.apple.quarantine /Applications/LadyBird.app\` once."
printf '%s\n' "$BODY" > "$OUT/$TAG.md"
python3 - "$OUT" "$VERSION" "$FILE" "$SUM" <<'PY'
import json, sys, os, time
out, ver, file, sha = sys.argv[1:5]
notes = open(os.path.join(out, f"v{ver}.md")).read()
json.dump({"version": ver, "file": file, "sha256": sha, "size": os.path.getsize(os.path.join(out, file)),
           "built_at": int(time.time()), "platform": "linux-x86_64", "notes": notes},
          open(os.path.join(out, "latest.json"), "w"), indent=2)
PY

step "committing and tagging $TAG"
git add app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock 2>/dev/null || true
git commit -q -m "Release $TAG" || true
git tag -a "$TAG" -m "LadyBird $VERSION"

if [ "$PUBLISH" = 1 ]; then
  step "publishing to GitHub"
  git push -q origin "$BRANCH" "$TAG"
  gh release create "$TAG" "$OUT/$FILE" "$OUT/$FILE.sha256" ${APK:+"$OUT/$APK" "$OUT/$APK.sha256"} --title "LadyBird $VERSION" --notes-file "$OUT/$TAG.md"
fi
if [ "$PUBLISH" = 1 ]; then
  echo "NOTE: the macOS DMG is attached a few minutes later by the release-mac workflow (to redo it: scripts/release-mac.sh $TAG on a Mac)"
fi
step "done $TAG ($(du -h "$OUT/$FILE" | cut -f1))"
