# Roadmap and known issues

Kept current by whoever changes the app (Claude Code included): when you fix something, delete its line here; when you find a bug or plan something, add it. Last reviewed 2026-10-12 (after v0.7.0).

## Known issues and unverified things

**Needs a real-device check (built and unit-tested, never seen working)**
- **Mac app, real UI:** the Take photo camera (needs the camera entitlement and the macOS Camera prompt), the "Pair now" button and auto-pair on the home Wi-Fi, Merriam-Webster with no dictionary downloaded, the search box edit fix and the plus icon were all verified only by tests/builds. Report anything odd.
- **NPU on the phone (S24 Ultra, Snapdragon 8 Gen 3):** NNAPI path is untested on the real device. Run Settings → On-device AI → Test it; if it says CPU, the reason is shown under it.
- **NPU on the Mac:** CoreML (Neural Engine) path never run.

**Fixed on 2026-10-11, waiting for the owner's check on the real apps**
- **Note reading:** the server now reads photos with a local vision model (Ollama `qwen2.5vl:3b`, tesseract only as fallback). Tested end to end from a clean client on the owner's page 1: the 18 lines came back right (see `docs/OCR-SAMPLES.md`), about 3 minutes cold and under a minute warm, on the box's CPU. Page 2 (Vietnamese words, brackets, arrows) has not been run through it yet.
- **Camera screen (Mac):** video fits the window, X button, Rotate button; falls back to a file picker.
- **Settings cards** keep a gap; **About** card shows the version; the search **Clear** button works on touch (pointer events). The earlier phone report may have been the old APK; confirm on the new one.
- **Quizzes:** answers are chosen first and submitted with "Check answer"; "Pause & exit" keeps the session and the Quizzes screen lists it with a Resume button (progress and misses restored).
- **History** merges consecutive rows of the same word and lists every dictionary used.
- **Examples** highlight the headword and common endings.
- **Selectable text:** the whole app is non-selectable except definitions, examples, passages, quiz questions, notes and inputs.
- **Self-update (built, never run on a device):** Settings → About and a banner show a newer GitHub release, download it, verify the SHA-256 and install it (Mac: swaps the app bundle and reopens; Linux: replaces the AppImage; Android: hands the APK to the system installer, which asks once to allow installs from LadyBird). Tested against a stand-in for GitHub only.

**Still open (reported by the owner)**
- **Gemini note reading (built, key on the box, server not yet restarted):** engine order is Gemini (`gemini-3.6-flash`, then `gemini-2.5-flash`), the local Ollama model, tesseract; a "Read again carefully" button on the review screen puts `gemini-3.1-pro-preview` first. Tested directly against Google on the owner's page 2: 3.6 Flash and 2.5 Flash read all 28 lines well (see the model comparison in SETUP); 3.8 Flash hung or answered 503, and 3.1 Pro needs a billed key (429 on the free one). **Needs `sudo` on the box once:** copy the new `gredict-syncd` to `/usr/local/bin` and restart the service. The photo-sync fix and the Read again button ship in the next release (v0.7.1).
- **Note reading, remaining work:** try the whole path from the phone once the server runs the new build; show per-line confidence; the notes grammar still builds a relation from anything with `=`, so odd lines can produce odd relations.

**Known defects and gaps**
- **macOS is not notarized** (no paid Apple account): the app is ad-hoc signed, so macOS shows a one-time "unidentified developer" prompt and a freshly downloaded copy needs `xattr -dr com.apple.quarantine /Applications/LadyBird.app`.
- **macOS hides the Wi-Fi name from apps**, so auto-pair falls back to "this device is on the server's /24 (192.168.1.x)". Another network that also uses 192.168.1.x and has a device at 192.168.1.7:25 could be mistaken for home (the device must also answer TLS and the enrollment request). Fix idea: ask for Location permission so the SSID is readable, or confirm the server certificate fingerprint once.
- **Auto-enroll trusts the home network:** anyone on the home Wi-Fi (a guest, say) can enroll a device while the server allows it. Mitigations: `--no-auto-enroll`, 20 per hour, log lines, `gredict-syncd devices` / `revoke`. Idea: an approval step or a time window.
- **Unpairing on purpose turns auto-pair off** until "Pair now" is pressed (by design); there is no per-device list or revoke button in the app yet.
- **APK signing key lives only on the Ubuntu box** (`~/.lexis-keys/`): test builds from the Mac use a separate application id and debug signing so they never replace the real app. Back the key up.
- **Many Rust tests skip on the Mac** (no `data/packs`, no `private/`); copy the packs from the box (see SETUP step 5) to run them all. The browser e2e test needs the same.
- **Mobile UI** was fixed for the Settings screen (tap highlight, selection, tab-bar fade, wrapping rows); other screens have not been audited at phone width.
- `quiz.rs:511` has a harmless "does not need to be mutable" compiler warning.

## Upcoming

**Next up**
- **Self-update, device check:** install v0.7.0 by hand once, then run the update on the Mac, a Linux AppImage and the phone when v0.7.1 is out.
1. Real-device test pass on the Mac and the S24 Ultra, recording what the NPU actually uses.
2. Keep SSH from the Mac to the box working (key login is set up; see SETUP "Which machine is which").

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
