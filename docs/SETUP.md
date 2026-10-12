# Setup guide

Everything that needs a person (a router, your Mac, your phone, accounts) is here. Passwords and pairing codes are **never** stored in this repository.

## 0. Status on the Ubuntu machine (done on 2026-10-11)

Done: build libraries installed; sync server installed as a service and verified on ports 25 and 5025 (also on the LAN address); pairing codes created in `~/lexis-pairing/`; the desktop app built (`~/Applications/LadyBird.AppImage`, 310 MB, packs inside) with an app-menu launcher called **LadyBird**; the app is already paired with the server and has received the Barron's quizzes (data in `~/.local/share/dev.lexis.gre`).

Still needs a person: the router forward for 5025 (section 2), the Mac (section 5), the phone (section 6), and the optional keys (section 7).

## 1. What is already running on the Ubuntu machine

The sync server is installed as a systemd service (`gredict-syncd`), starts at boot, and listens on:

| Port | For | Notes |
|---|---|---|
| 25 | devices on the home network | |
| 5025 | devices away from home | must be forwarded on the router (section 2) |

```bash
systemctl status gredict-syncd            # is it running?
journalctl -u gredict-syncd -n 50         # recent log
sudo ./deploy/install-syncd.sh            # (re)install after rebuilding: cargo build --release
```

Files: program `/usr/local/bin/gredict-syncd`, data and certificate `/var/lib/lexis-syncd`, Barron's copy that paired devices receive `/var/lib/lexis-syncd/private/barrons.sqlite` (re-run the installer after re-importing the book).

### Pairing codes

One code per device, created with:

```bash
/usr/local/bin/gredict-syncd --data /var/lib/lexis-syncd pair <name>    # prints a code starting `lexis1:`
/usr/local/bin/gredict-syncd --data /var/lib/lexis-syncd devices        # list paired devices
/usr/local/bin/gredict-syncd --data /var/lib/lexis-syncd revoke <name>  # lock a lost device out
```

Codes for `mac`, `phone` and `ubuntu` were created in `~/lexis-pairing/` (private to your user). **A code is a secret**: anyone holding it can read and change your synced notes. Do not post it or commit it; re-pairing a name replaces its old code. Paste a code into **Settings → Sync server → Pairing code** on that device.

## 2. Router

1. Give the Ubuntu machine a fixed address (already done: `192.168.1.7`).
2. Forward **external TCP 5025 → 192.168.1.7 port 5025**.
3. Port 25 is for the home network only. At the time of setup it also answered from outside, which means a forward for 25 exists. Because the server demands a pinned certificate and a token this is not a hole, but if you do not want it, delete the forward for 25: the app uses 25 only on `Anan` / `Anan 5g` and 5025 everywhere else.
4. DuckDNS (`lhan.duckdns.org`) is updated by `~/duckdns/duck.sh`; the record followed your public address when checked.
5. Test from outside: on your phone with Wi-Fi **off**, pair the app and press **Sync now**. The header pill should say **Remote**; on the home Wi-Fi it should say **Home**.

If a VPN is on, the pill shows **VPN**. If the server can't be reached with a VPN active, allow local network access or split tunnelling for the app.

## 3. The app on Ubuntu (this machine)

Libraries (already installed): `libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev libasound2-dev`.

```bash
cargo install tauri-cli --version "^2" --locked          # once
cd app/src-tauri && cargo tauri icon icons/source.png    # once: makes the icon set
cargo tauri build                                        # AppImage under target/release/bundle/appimage/
```

The build was run from `app/src-tauri` (the pre-build step expects that). Install or update the launcher after a rebuild:

```bash
cp target/release/bundle/appimage/LadyBird_<version>_amd64.AppImage ~/Applications/LadyBird.AppImage
```

The launcher starts it with `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which avoids a blank window with some NVIDIA setups; remove that variable in `~/.local/share/applications/ladybird.desktop` if everything renders fine without it. The AppImage carries the dictionary packs. Optional background sync with no window, started at login:

```bash
~/Applications/LadyBird.AppImage --install-agent
```

Sound: the window's web engine cannot play audio inside an AppImage, so the desktop app plays recordings itself through the system audio output (the `rodio` library; ALSA/PipeWire on Linux, CoreAudio on a Mac). When no recording is saved and you are offline it speaks the word with `spd-say` (Linux) or `say` (Mac). `cargo run --example sound_check -- <file.mp3>` in `app/src-tauri` plays one recording to test the path.

During development you can instead run `./target/release/gredict-server` and open http://127.0.0.1:8787.

## 4. Data that is not in git

Dictionary packs and Barron's are built, not downloaded from this repository (sizes: about 340 MB of packs).

```bash
# download the open datasets into data/raw (kaikki.org English dump, english-wordnet-2024.xml.gz,
# gcide-0.53.tar.xz, mthesaur.txt, cmudict.dict), then:
cargo build --release
P=./target/release/packer
$P wiktionary data/raw/kaikki.org-dictionary-English.jsonl.gz data/packs/wiktionary.pack
$P wordnet data/raw/english-wordnet-2024.xml.gz data/packs/wordnet.pack
$P moby data/raw/mthesaur.txt data/packs/moby.pack
(cd data/raw && mkdir -p gcide && tar -xJf gcide-0.53.tar.xz -C gcide) && $P gcide data/raw/gcide/gcide-0.53 data/packs/gcide.pack
$P cmudict data/raw/cmudict.dict data/packs/cmudict.pack
$P fst data/packs/wiktionary.pack data/packs/wiktionary.fst
# your own copy of Barron's (needs `pdftotext`, from poppler-utils):
$P barrons private/barrons-essential-words-4e.pdf --out private/barrons.sqlite
```

The packer refuses to write the Barron's database if any unit, exercise or answer key fails its checks.

## 5. The Mac (M2 MacBook Air)

**Easiest: install the DMG from the GitHub release.** In Terminal:

```bash
pkill -x ladybird; rm -rf /Applications/LadyBird.app          # replace any old copy
cd ~/Downloads
gh release download v<version> -R lehoangan02/DictionaryClaude -p '*macos-arm64.dmg' --clobber
hdiutil attach -nobrowse LadyBird-<version>-macos-arm64.dmg
cp -R /Volumes/LadyBird/LadyBird.app /Applications/ && hdiutil detach /Volumes/LadyBird
xattr -dr com.apple.quarantine /Applications/LadyBird.app      # without this macOS says the app is "damaged"
open /Applications/LadyBird.app
```

The app is ad-hoc signed (no paid Apple account, so no notarization). On first use macOS asks for **Local Network** (needed to reach the server at 192.168.1.7; if you refuse, pairing fails with "No route to host", and you can change it in System Settings → Privacy & Security → Local Network) and **Camera** (Take photo). On the home Wi-Fi the app pairs with the server by itself, which needs the server running the current `gredict-syncd` (`git pull`, rebuild, `sudo systemctl restart gredict-syncd`); skip to step 6. Merriam-Webster works straight away, even before any dictionary is downloaded. `scripts/release-mac.sh v<version>` rebuilds and replaces the release's DMG.

To build from source instead:

1. Install Xcode command line tools (`xcode-select --install`), Rust (`rustup`), Node 20+.
2. Get the code: `git clone https://github.com/lehoangan02/DictionaryClaude && cd DictionaryClaude`.
3. Copy the packs from Ubuntu (they are not in git):
   ```bash
   mkdir -p data/packs
   scp -P 22 'lhan@lhan.duckdns.org:/home/lhan/Documents/AILab/DictionaryClaude/data/packs/*' data/packs/
   scp -P 22 lhan@lhan.duckdns.org:~/lexis-pairing/mac.txt ~/Desktop/lexis-mac-code.txt
   ```
4. Build: `cargo install tauri-cli --version "^2" --locked`, then `cd app/src-tauri && cargo tauri icon icons/source.png && cargo tauri build`. Move `LadyBird.app` to Applications.
5. Open it on the home Wi-Fi: it pairs itself (Settings → Sync server → **Pair now** if you want it immediately; the code in `lexis-mac-code.txt` still works under "Pair with a code instead"). Barron's quizzes arrive with the first sync.
6. Optional background sync at login: `/Applications/LadyBird.app/Contents/MacOS/ladybird --install-agent`.
7. Location permission lets the app read the Wi-Fi name; if you decline it the app still works by testing both routes.

Not yet built or tested on a Mac: the on-device NPU runtime (CoreML).

## 6. The phone (Galaxy S24 Ultra)

An Android build exists and has been run on the Android emulator (pairing, syncing, downloading dictionaries, looking words up). It has not yet been run on the real phone.

1. On the phone open the latest release at https://github.com/lehoangan02/DictionaryClaude/releases and download `LadyBird-<version>-android-arm64.apk`. Allow "install unknown apps" for your browser when Android asks.
2. Open LadyBird. It starts without dictionaries (the app is only about 10 MB).
3. On the Ubuntu machine create a code: `/usr/local/bin/gredict-syncd --data /var/lib/lexis-syncd pair phone`. Send it to yourself privately and paste it in **Settings → Sync server → Pairing code**, then **Pair**.
4. **Settings → Downloads**: press **Download** on Wiktionary, WordNet, Webster 1913 and the rest (about 340 MB in all, resumable; on the home Wi-Fi it takes under a minute). Barron's arrives with the first sync.
5. Updates: new APKs install over the old one (they are signed with the same key).

Building the APK yourself (done on the Ubuntu machine, `scripts/release.sh` does it for every release): Android SDK command-line tools plus `platforms;android-34` and an NDK (`sdkmanager "ndk;27.2.12479018"`), `rustup target add aarch64-linux-android`, then `cd app/src-tauri && cargo tauri android build --apk --target aarch64`. The APK is signed with a key kept in `~/.lexis-keys/` (never in git): **back that folder up**, because a phone only accepts updates signed by the same key.

On-device AI: see section 6b. Not yet built: Bluetooth sync. Sound uses the phone's web view (recordings) with no offline speech fallback yet.

## 6b. On-device AI (NPU → GPU → CPU)

One runtime (`core::ai`, ONNX Runtime) runs the small embedding model (bge-small, 35 MB) that makes quiz-session search work by meaning. Download it in **Settings → Downloads → Meaning search (AI model)**; **Settings → On-device AI** then shows the hardware actually in use and a **Test it** button (it reports the backend and the speed).

| Device | Order tried | Status |
|---|---|---|
| Mac (Apple silicon) | CoreML (Neural Engine, then GPU) → CPU | built by CI; **not yet run on a real Mac** |
| Android | NNAPI (the phone's NPU/DSP/GPU as its driver decides) → CPU | runs on the Android emulator (NNAPI path, CPU-class speed there); **not yet run on the S24 Ultra** |
| Linux | CPU | tested (about 7 ms per batch) |

Please run **Test it** on the Mac and the phone and tell me what it reports: if it says CPU there, the NPU was not used and the reason is shown under it. The Snapdragon-specific QNN provider is not included (it needs Qualcomm's SDK); NNAPI is the route to the Hexagon NPU. `LEXIS_AI_BACKEND=cpu` forces the CPU.

## 7. Optional accounts and keys

- **Merriam-Webster**: done. The free keys (Collegiate Dictionary and Thesaurus, registered to the owner) ship inside the app as defaults (`crates/app/defaults/mw.json`), so every device starts with the Merriam-Webster tab ready; removing them in Settings sticks. The official logos are in `app/ui/public/` and are shown, unmodified, under every Merriam-Webster entry as their brand rules require. Terms to keep in mind: non-commercial use only, 1,000 lookups a day per key, and no bulk or automated queries (the app only looks a word up when you open it). The keys are public in this repository by the owner's choice; if the quota is ever abused, regenerate them at dictionaryapi.com and update that file.
- **Pronunciations**: Settings → Pronunciation → *Download Barron's pronunciations* caches the recordings (about 780 files). Wikimedia limits request speed, so this takes a while and pauses by itself.
- **Agents for hard handwriting**: the server uses the `claude` and `agy` programs already signed in under your user. Each press of **Ask agents** costs quota; `LEXIS_AGENT_CAP` in the service file (default 8 calls a day) is a safety limit. agy's quota was used up when tested; its error is shown in the app.
- **Better handwriting OCR** (not set up): Tesseract reads print well and handwriting poorly. A local vision model (for example through `ollama`) could replace it by setting `LEXIS_OCR=cmd:<program> {image}` in the service file. This needs a multi-gigabyte download, so it is your call.

## 8. If something is wrong

| Symptom | Check |
|---|---|
| Pill says **Offline** at home | `systemctl status gredict-syncd`; is the Mac on `Anan` / `Anan 5g`; Settings → home Wi-Fi names |
| Pill says **Offline** away from home | router forward for 5025; `curl -k https://lhan.duckdns.org:5025/v1/health` from outside |
| "unauthorized" | the code was replaced or revoked: create a new one and paste it |
| "not the paired certificate" | the server's certificate changed (it lives in `/var/lib/lexis-syncd/tls`): re-pair every device |
| Photos stay "Waiting to be read" | the server reads them (Tesseract); check `journalctl -u gredict-syncd` |
| Pronunciation download pauses | Wikimedia rate limit: it resumes by itself |

## 9. Releases

`scripts/release.sh <version> --publish` tests, builds the Linux AppImage and the Android APK, tags `v<version>` and creates the GitHub release (needs `gh auth login` once). The version is chosen by Claude as `0.MINOR.PATCH`. Files are also kept in `~/lexis-releases`.

Dictionary packs are not part of a release: the server offers them. After rebuilding packs run `sudo deploy/install-syncd.sh` to copy them to `/var/lib/lexis-syncd/packs`; devices see updates under **Settings → Downloads → Check the server**.

## 10. Publishing

The repository is public (`github.com/lehoangan02/DictionaryClaude`). Never commit `private/`, `data/`, pairing codes, API keys or anything quoted from Barron's. The local branch `checkpoints-archive` must never be pushed (see `CLAUDE.md`).

## Which machine is which

Day-to-day work happens on the Mac (this repo checkout, `gh` signed in, `scripts/release-mac.sh`). The Ubuntu box is reached over SSH when needed: `ssh -p 22 lhan@lhan.duckdns.org` at home, `ssh -p 2222 lhan@lhan.duckdns.org` by the public name on the local network, `ssh -p 1111 lhan@lhan.duckdns.org` when away or on WARP. Authorize the Mac's key once with `ssh-copy-id -p 22 lhan@lhan.duckdns.org`. After pushing from the Mac, update the server with `ssh -p 22 lhan@lhan.duckdns.org 'cd ~/Documents/AILab/DictionaryClaude && git pull && . ~/.cargo/env && cargo build --release -p gredict-syncd && sudo systemctl restart gredict-syncd'` (the `sudo` step needs a terminal; run it with `ssh -t`).

SSH key login from the Mac works because sshd also reads a root-owned key file: `/etc/ssh/authorized_keys/%u` (drop-in `/etc/ssh/sshd_config.d/50-extra-authorized-keys.conf`). `~/.ssh/authorized_keys` alone is ignored on the box, because `/home/lhan` is group-writable (shared with `lhan02`) and sshd's `StrictModes` refuses it. To authorize another machine, add its public key to `/etc/ssh/authorized_keys/lhan` with `sudo`.
