# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Source of truth

- `docs/REQUIREMENTS.md` holds the owner's original request (verbatim) plus agreed decisions. **Re-read it before starting each phase. Never edit it without the owner's explicit permission**: propose the change and ask first.
- `docs/PLAN.md` is the approved implementation plan (architecture, phases, verification). Keep it in sync when the plan changes with approval.

## Project

An offline GRE-focused dictionary app (Tauri 2 + Rust core + Svelte UI) for Linux and macOS first, then Android. It adds a notes-driven GRE thesaurus, Barron's and notes quizzes, hybrid search over quiz sessions, multi-device sync through `syncd` on the owner's Ubuntu box, and note-photo OCR with an optional, manually triggered review by Claude + agy agents.

Planned layout (see `docs/PLAN.md`):
- `crates/core`: all domain logic
- `crates/packer`: data build CLI
- `crates/syncd`: Ubuntu sync/OCR/agent-job server
- `app/src-tauri`: thin Tauri shell
- `app/ui`: Svelte frontend

## Hard rules

- **No scraping** of Oxford, Merriam-Webster or Cambridge. Dictionary content comes only from open datasets (Wiktionary/kaikki, Open English WordNet, GCIDE, Moby, CMUdict) and the official MW API, cached.
- **Merriam-Webster API terms:** non-commercial, 1,000 queries/day per reference work, at most two reference works, and *no automated or recorded queries* (their section 5(a)). So MW is looked up only when the user opens a word in that tab, never pre-fetched in bulk; the key is kept in `settings` and is never logged or returned to the UI; the official MW logos (`app/ui/public/mw-logo-*.png`, downloaded unmodified from their brand page) must stay visible on MW entries. The owner chose to ship the two free keys in `crates/app/defaults/mw.json` (public repo); app shells call `seed_defaults()`, tests do not.
- **Barron's material is copyrighted.** The PDF and everything derived from it live in `private/` (gitignored). Never commit, publish or embed it in distributable builds.
- **The UI mimics the Oxford Learner's look without Oxford names or logos.**
- **User data references words by normalized lemma key, never by pack row IDs**, so dictionary packs stay swappable.
- **User DB schema changes are new forward-only numbered migrations.** Never edit a shipped migration; changes to oplog op formats need an upcaster.
- **Agent jobs (`claude -p` / `agy -p`) run only when the user triggers them**, because quota is limited. They never write user notes directly; the user approves every change.
- **On-device AI uses the NPU, then the GPU, then the CPU**, through the single `core::ai` runtime.

## Checkpoints

The owner wants git checkpoints. Commit at the end of each milestone (a parser finished, a phase step working), with tests passing and only source files staged: `private/`, `data/raw/` and build output are gitignored and must stay out. Name commits `Checkpoint N: <what works>`.

The repo is published at https://github.com/lehoangan02/DictionaryClaude (public). Work on `main`; push only when the owner asks. The local branch `checkpoints-archive` holds the original 18 checkpoint commits and must **never be pushed**: their early test fixtures quoted the Barron's book (since replaced with invented text). Never put book text, answer keys or passages in tests, docs or commit messages.

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
./target/release/packer fst data/packs/wiktionary.pack data/packs/wiktionary.fst
./target/release/packer barrons private/barrons-essential-words-4e.pdf --out private/barrons.sqlite   # validates every unit/key; refuses to write on any problem
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
