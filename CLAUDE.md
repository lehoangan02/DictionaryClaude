# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Machines (owner's standing decision, 2026-10-11)

**The Mac (M2 MacBook Air, this checkout) is the machine the owner works on from now on.** Develop, test, commit and run `scripts/release-mac.sh` here (Rust, Node, `gh`, `tauri-cli` and, for the phone, the Android SDK/NDK are installed on it). The Ubuntu box (`192.168.1.7`, `lhan.duckdns.org`) runs `gredict-syncd`, builds the Linux AppImage and the signed Android APK (`scripts/release.sh`), and holds the packs, Barron's and the APK signing key (`~/.lexis-keys/`, never copy it into git). **Whenever something needs the Ubuntu side, SSH into it yourself instead of asking the owner to do it there**: `ssh -p 22 lhan@lhan.duckdns.org` on the home network, `ssh -p 2222 lhan@lhan.duckdns.org` when away or when Cloudflare WARP (1.1.1.1) is on; the repo is at `~/Documents/AILab/DictionaryClaude`. Pull before working there, push from the Mac, and tell the owner what you ran. The Mac's SSH key must be authorized on the box (`ssh-copy-id`); if login is refused, say so and ask the owner to run it once with `!`.

## Source of truth

- `docs/REQUIREMENTS.md` holds the owner's original request (verbatim) plus agreed decisions. **Re-read it before starting each phase. Never edit it without the owner's explicit permission**: propose the change and ask first.
- `docs/PLAN.md` is the approved implementation plan (architecture, phases, verification). Keep it in sync when the plan changes with approval.
- `docs/ROADMAP.md` lists known issues, unverified things and upcoming features. Update it whenever you fix, find or plan something, so the owner never has to remind you.

## Project

An offline GRE-focused dictionary app (Tauri 2 + Rust core + Svelte UI) for Linux and macOS first, then Android. It adds a notes-driven GRE thesaurus, Barron's and notes quizzes, hybrid search over quiz sessions, multi-device sync through `syncd` on the owner's Ubuntu box, and note-photo OCR with an optional, manually triggered review by Claude + agy agents.

## Hard rules

- **No scraping** of Oxford, Merriam-Webster or Cambridge. Dictionary content comes only from open datasets (Wiktionary/kaikki, Open English WordNet, GCIDE, Moby, CMUdict) and the official MW API, cached.
- **Merriam-Webster API terms:** non-commercial, 1,000 queries/day per reference work, at most two reference works, and *no automated or recorded queries* (their section 5(a)). So MW is looked up only when the user opens a word in that tab, never pre-fetched in bulk; the key is kept in `settings` and is never logged or returned to the UI; the official MW logos (`app/ui/public/mw-logo-*.png`, downloaded unmodified from their brand page) must stay visible on MW entries. The owner chose to ship the two free keys in `crates/app/defaults/mw.json` (public repo); app shells call `seed_defaults()`, tests do not.
- **Barron's material is copyrighted.** The PDF and everything derived from it live in `private/` (gitignored). Never commit, publish or embed it in distributable builds.
- **OALD is Oxford's content (the owner's personal extraction): never bundled into any build or release.** `oald.pack` and `oald-audio.pack` are served by the owner's sync server and downloaded on request (Settings → Downloads, home or remote). The AppImage bundles an explicit allow-list of packs in `app/src-tauri/tauri.conf.json`: a new pack is not bundled until someone adds it there on purpose. (v0.9.0 and v0.10.0 AppImages shipped with `oald.pack` inside; fixed from the next release.)
- **The UI mimics the Oxford Learner's look without Oxford names or logos.**
- **User data references words by normalized lemma key, never by pack row IDs**, so dictionary packs stay swappable.
- **User DB schema changes are new forward-only numbered migrations.** Anything the server keeps for good (related-word lists, future caches) lives in a migrated table, never in an ad-hoc `CREATE IF NOT EXISTS`, so updates never lose it. Never edit a shipped migration; changes to oplog op formats need an upcaster.
- **Agent jobs (`claude -p` / `agy -p`) run only when the user triggers them**, because quota is limited. They never write user notes directly; the user approves every change.
- **On-device AI uses the NPU, then the GPU, then the CPU**, through the single `core::ai` runtime.

## Checkpoints

The owner wants git checkpoints. Commit at the end of each milestone (a parser finished, a phase step working), with tests passing and only source files staged: `private/`, `data/raw/` and build output are gitignored and must stay out. Name commits `Checkpoint N: <what works>`.

The repo is published at https://github.com/lehoangan02/DictionaryClaude (public). Work on `main`; push only when the owner asks. The local branch `checkpoints-archive` holds the original 18 checkpoint commits and must **never be pushed**: their early test fixtures quoted the Barron's book (since replaced with invented text). Never put book text, answer keys or passages in tests, docs or commit messages.

## Releases (standing rule from the owner)

Every time the app changes, package it and publish it as a GitHub release. **The owner left versioning to Claude:** use `0.MINOR.PATCH` (new feature or visible redesign: bump MINOR; fix: bump PATCH). Run `scripts/release.sh <version> --publish` from a clean, committed tree: it tests, bumps `app/src-tauri/tauri.conf.json` and `Cargo.toml`, builds the AppImage, saves it and `latest.json` under `~/lexis-releases`, tags `v<version>`, pushes and creates the GitHub release. Only Linux x86_64 (and the Android APK, if the SDK is present) is built there. Releases never contain Barron's (`private/` is not bundled).

**macOS is part of every release (standing rule, do not wait to be reminded).** Publishing triggers `.github/workflows/release-mac.yml`, which builds the Apple-silicon DMG, checks it is ad-hoc signed (`signingIdentity: "-"` in `tauri.conf.json`; without it macOS calls a downloaded app "damaged") and attaches `LadyBird-<version>-macos-arm64.dmg` to the release. When the owner asks to rebuild or replace the Mac build, run `scripts/release-mac.sh v<version>` on the Mac (needs `. ~/.cargo/env`, `gh auth login` and `cargo install tauri-cli --version "^2" --locked`); it uploads with `--clobber`. Never remove the signing identity. Release notes tell users to run `xattr -dr com.apple.quarantine /Applications/LadyBird.app` if macOS still objects (no paid Apple account, so no notarization).

## Commands

Rust is installed with rustup (`. ~/.cargo/env`). The UI is Svelte 5 + Vite in `app/ui`.

```bash
cargo test --workspace                 # unit + integration tests (several skip themselves when data/ or private/ is absent)
cargo test -p gredict-app --test sync  # one test file; add a test name after `--` to run a single test
cargo build --release                  # packer, gredict-server, gredict-syncd

# Data (inputs in data/raw, outputs in data/packs and private/ — all gitignored)
./target/release/packer wiktionary data/raw/kaikki.org-dictionary-English.jsonl.gz data/packs/wiktionary.pack
./target/release/packer wordnet data/raw/english-wordnet-2024.xml.gz data/packs/wordnet.pack
./target/release/packer moby data/raw/mthesaur.txt data/packs/moby.pack
./target/release/packer viwiktionary data/raw/viwiktionary-english.jsonl data/packs/envi.pack          # English -> Vietnamese (kaikki.org Vietnamese Wiktionary, English entries)
./target/release/packer family data/raw/kaikki.org-dictionary-English.jsonl.gz data/packs/family.pack                  # word families for the Forms tab (20 MB, about 25 s)
./target/release/packer fst data/packs/wiktionary.pack data/packs/wiktionary.fst
./target/release/packer barrons private/barrons-essential-words-4e.pdf --out private/barrons.sqlite   # validates every unit/key; refuses to write on any problem
./target/release/packer oald ../OALData data/packs/oald.pack                                           # OALD dictionary (adds oald:<name> recording references when OALData/json/oald_audio.json exists)
python3 ../OALData/tools/build_oald_audio.py                                                           # OALD recordings -> OALData/audio/*.mp3 + json/oald_audio.json (about 45 min)
./target/release/packer oald-audio ../OALData private/oald-audio.pack                                  # the recordings pack: keep it in private/, everything in data/packs/ is bundled into the AppImage
./target/release/packer lookup data/packs/wiktionary.pack loquacious                                   # inspect an entry

# Run the app in a browser (same JSON API the Tauri shell will use)
(cd app/ui && npm install && npm run build)
./target/release/gredict-server        # http://127.0.0.1:8787, data in data/user
(cd app/ui && npm run check)           # svelte-check
(cd app/ui && npm run e2e)             # hermetic browser test: own server + two "devices" + sync server, screenshots in /tmp/shots

# Sync server (see deploy/ for the systemd unit; needs sudo to bind port 25)
./target/release/gredict-syncd --data data/syncd pair <device-name>   # prints a pairing code
```

## Architecture notes

- `crates/core`: `Entry` (the one schema every dictionary source maps into), `Pack` reader (zstd-dictionary-compressed JSON in SQLite), `Fuzzy` (FST + Levenshtein typo index), `lemma::key`, and the `notes` grammar (`=` same, `≈` similar, `><` opposite, `⍯`/`≠` different) with its relation graph.
- `crates/app`: `Api::call(cmd, json)` is the only surface; Tauri, `gredict-server` and the sync server all go through it. `userdb` holds forward-only migrations; `quiz` builds Barron's, generated and notes quizzes plus FTS session search; `sync` merges rows by last-writer-wins; `net` picks the LAN or remote route by *verifying* a pinned-TLS handshake.
- `crates/packer`: data builders. `barrons/` parses `pdftotext -layout` output; golden tests in `barrons/mod.rs` lock the shape of the whole book.
- Photographed notes: `drafts` rows (synced) + photo files (sent separately). The server's `jobs.rs` runs OCR on every new draft; agents (`claude -p`, `agy -p`) run *only* when the person presses the button, read the photos in an empty scratch folder, and never write notes. Tests must never call the real CLIs: use `LEXIS_CLAUDE_BIN` / `LEXIS_AGY_BIN` stand-ins (the e2e script does).
- Barron's quiz items carry a `style` (match, bank, mcq, sense, tf, blanks, se, multi) that decides how the player renders them; answers never reach the client before an answer is submitted.
