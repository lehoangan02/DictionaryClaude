# Roadmap and known issues

Kept current by whoever changes the app (Claude Code included): when you fix something, delete its line here; when you find a bug or plan something, add it. Last reviewed 2026-10-11 (after v0.6.0).

## Known issues and unverified things

**Needs a real-device check (built and unit-tested, never seen working)**
- **Mac app, real UI:** the Take photo camera (needs the camera entitlement and the macOS Camera prompt), the "Pair now" button and auto-pair on the home Wi-Fi, Merriam-Webster with no dictionary downloaded, the search box edit fix and the plus icon were all verified only by tests/builds. Report anything odd.
- **NPU on the phone (S24 Ultra, Snapdragon 8 Gen 3):** NNAPI path is untested on the real device. Run Settings → On-device AI → Test it; if it says CPU, the reason is shown under it.
- **NPU on the Mac:** CoreML (Neural Engine) path never run.
- **Android "Take photo":** relies on the file input's `capture` attribute inside the Android web view; not confirmed on the phone.

**Known defects and gaps**
- **macOS is not notarized** (no paid Apple account): the app is ad-hoc signed, so macOS shows a one-time "unidentified developer" prompt and a freshly downloaded copy needs `xattr -dr com.apple.quarantine /Applications/LadyBird.app`.
- **macOS hides the Wi-Fi name from apps**, so auto-pair falls back to "this device is on the server's /24 (192.168.1.x)". Another network that also uses 192.168.1.x and has a device at 192.168.1.7:25 could be mistaken for home (the device must also answer TLS and the enrollment request). Fix idea: ask for Location permission so the SSID is readable, or confirm the server certificate fingerprint once.
- **Auto-enroll trusts the home network:** anyone on the home Wi-Fi (a guest, say) can enroll a device while the server allows it. Mitigations: `--no-auto-enroll`, 20 per hour, log lines, `gredict-syncd devices` / `revoke`. Idea: an approval step or a time window.
- **Unpairing on purpose turns auto-pair off** until "Pair now" is pressed (by design); there is no per-device list or revoke button in the app yet.
- **Release flow:** the v0.6.0 tag was cut before the Mac fixes (camera entitlement, search edit fix, plus icon). The Mac DMG on that release was built from a later commit by hand; ship v0.6.1 so the tag matches.
- **Ubuntu box runs an older `gredict-syncd`** until it is pulled, rebuilt and restarted, so `/v1/enroll` does not exist there yet and auto-pair cannot work.
- **APK signing key lives only on the Ubuntu box** (`~/.lexis-keys/`): test builds from the Mac use a separate application id and debug signing so they never replace the real app. Back the key up.
- **Many Rust tests skip on the Mac** (no `data/packs`, no `private/`); copy the packs from the box (see SETUP step 5) to run them all. The browser e2e test needs the same.
- **Mobile UI** was fixed for the Settings screen (tap highlight, selection, tab-bar fade, wrapping rows); other screens have not been audited at phone width.
- `quiz.rs:511` has a harmless "does not need to be mutable" compiler warning.
- Sound uses the web view's speech and downloaded recordings; there is no offline speech fallback.

## Upcoming

**Next up**
1. Release v0.6.1 from the Ubuntu box (AppImage + signed APK), letting the release-mac workflow attach the DMG; update and restart `gredict-syncd` there.
2. Real-device test pass on the Mac and the S24 Ultra, recording what the NPU actually uses.
3. Keep SSH from the Mac to the box working (key login is set up; see SETUP "Which machine is which").

**Planned (from `docs/PLAN.md`)**
- Bluetooth LE sync between Mac and phone (`btleplug` on the Mac, a Kotlin plugin on Android); LAN peer-to-peer discovery via mDNS (`_gredict._tcp`) as a fallback if the server's address changes.
- QR-code pairing (today: automatic on the home Wi-Fi, or paste a code).
- Offline photo queue with an on-device OCR draft when the server is unreachable.
- Optional import of old paper notes by photo, with a correction screen (low priority: offline handwriting OCR is weak).
- Qualcomm QNN provider for the Hexagon NPU (needs Qualcomm's SDK; NNAPI is the route used today).

**Ideas, not committed**
- Notarized macOS builds (needs an Apple Developer account).
- A device list in Settings with revoke and rename.
- Read the Wi-Fi name on macOS (Location permission) so auto-pair does not depend on the subnet check.
- Confirm the server certificate fingerprint on first auto-pair.
