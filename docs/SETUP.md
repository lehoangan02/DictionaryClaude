# Setup guide

Everything that needs a person (a router, your Mac, your phone, accounts) is here. Passwords and pairing codes are **never** stored in this repository.

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

Libraries (already installed): `libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev`.

```bash
cargo install tauri-cli --version "^2" --locked          # once
cd app/src-tauri && cargo tauri icon icons/source.png    # once: makes the icon set
cargo tauri build                                        # AppImage under target/release/bundle/appimage/
```

Run the AppImage; it carries the dictionary packs. Optional background sync with no window, started at login:

```bash
./Lexis*.AppImage --install-agent
```

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

1. Install Xcode command line tools (`xcode-select --install`), Rust (`rustup`), Node 20+.
2. Get the code: `git clone https://github.com/lehoangan02/DictionaryClaude && cd DictionaryClaude`.
3. Copy the packs from Ubuntu (they are not in git):
   ```bash
   mkdir -p data/packs
   scp -P 22 'lhan@lhan.duckdns.org:/home/lhan/Documents/AILab/DictionaryClaude/data/packs/*' data/packs/
   scp -P 22 lhan@lhan.duckdns.org:~/lexis-pairing/mac.txt ~/Desktop/lexis-mac-code.txt
   ```
4. Build: `cargo install tauri-cli --version "^2" --locked`, then `cd app/src-tauri && cargo tauri icon icons/source.png && cargo tauri build`. Move `Lexis.app` to Applications.
5. Open it, **Settings → Sync server**, paste the code from `lexis-mac-code.txt` (then delete that file). Barron's quizzes arrive with the first sync.
6. Optional background sync at login: `/Applications/Lexis.app/Contents/MacOS/lexis --install-agent`.
7. Location permission lets the app read the Wi-Fi name; if you decline it the app still works by testing both routes.

Not yet built or tested on a Mac: the on-device NPU runtime (CoreML).

## 6. The phone (Galaxy S24 Ultra)

The Android build has **not been attempted** (it needs Android Studio, the NDK and a device). The steps, when you want to try:

1. Install Android Studio, the Android SDK and NDK; `rustup target add aarch64-linux-android`.
2. `cd app/src-tauri && cargo tauri android init && cargo tauri android build --apk`.
3. The packs (340 MB) are too large to bundle comfortably; copy `data/packs/*` onto the phone (the app looks in its data folder, or set `LEXIS_PACKS`). A smaller "lite" pack is a good next step.
4. Pair with the `phone` code (send it to yourself privately, not by a public channel).
5. Not yet built: the NPU runtime (QNN on the Snapdragon) and Bluetooth sync.

## 7. Optional accounts and keys

- **Merriam-Webster**: free keys at dictionaryapi.com (Collegiate Dictionary and Thesaurus), pasted in Settings → Merriam-Webster. Their terms require their logo: download it from them and save it as `app/ui/public/mw-logo.png`, then rebuild the UI. Non-commercial use only; the app never queries in bulk.
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

## 9. Publishing

The repository is public (`github.com/lehoangan02/DictionaryClaude`). Never commit `private/`, `data/`, pairing codes, API keys or anything quoted from Barron's. The local branch `checkpoints-archive` must never be pushed (see `CLAUDE.md`).
