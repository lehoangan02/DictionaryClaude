#!/usr/bin/env bash
# Puts ONNX Runtime (with the NNAPI provider) into the Android project, so the
# app can run its AI model on the phone's NPU. Run once; release.sh runs it too.
set -euo pipefail
cd "$(dirname "$0")/.."
VER=1.22.0
DEST=app/src-tauri/gen/android/app/src/main/jniLibs
if [ -f "$DEST/arm64-v8a/libonnxruntime.so" ] && [ -f "$DEST/x86_64/libonnxruntime.so" ]; then exit 0; fi
T="$(mktemp -d)"
curl -fsSL -o "$T/ort.aar" "https://repo1.maven.org/maven2/com/microsoft/onnxruntime/onnxruntime-android/$VER/onnxruntime-android-$VER.aar"
unzip -q -o "$T/ort.aar" 'jni/arm64-v8a/*' 'jni/x86_64/*' -d "$T"
for abi in arm64-v8a x86_64; do mkdir -p "$DEST/$abi"; cp "$T/jni/$abi/libonnxruntime.so" "$DEST/$abi/"; done
rm -rf "$T"
echo "ONNX Runtime $VER installed for Android"
