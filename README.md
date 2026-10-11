# LadyBird — an offline GRE dictionary

A fast, offline dictionary, thesaurus and quiz app for GRE Verbal study, built with Rust, Tauri 2 and Svelte. It runs on Linux and macOS (Android planned), keeps your notes, history and quiz scores in sync between your own devices, and never needs the internet to look a word up.

## What it does

- **Three offline dictionaries** you can switch between: Wiktionary (1.36M entries), Open English WordNet and Webster's 1913. Typo-tolerant search, IPA (with a CMU-dictionary fallback) and recorded pronunciations that are cached for offline use.
- **Optional Merriam-Webster** via their free API, looked up only when you open a word (their terms forbid automated queries). You supply your own key.
- **A GRE thesaurus** that starts from *your own tutor notes* (`a = b` same, `a ≈ b` similar, `a >< b` opposite, `a ⍯ b` different/confusable), then related words from your Barron's book, root families, and general synonyms.
- **Quizzes** from Barron's *Essential Words for the GRE* (units, reviews, word roots, pretest/posttest), generated word quizzes, and quizzes from your own notes — with scores, history and meaning-aware search over past sessions.
- **Photograph your notes**: the server OCRs the photo into a draft you check and correct; for hard handwriting you can optionally ask Claude Code and/or agy to read it and compare their readings. Nothing becomes a note until you save it.
- **Multi-device sync** over TLS to a small server you run (`gredict-syncd`), choosing the home-network port or the remote port automatically, with a status indicator in the app.

## Not included

This repository contains **no** Barron's content and no dictionary data. Barron's is copyrighted: the importer (`packer barrons`) parses *your own copy* of the PDF locally into `private/`, which is git-ignored, and your devices receive the parsed result from your own server. Dictionary packs are built from open datasets (see below) with the `packer` tool.

## Data sources and licences

| Source | Licence |
|---|---|
| Wiktionary (via kaikki.org) | CC BY-SA 4.0 |
| Open English WordNet | CC BY 4.0 |
| Webster's 1913 (GCIDE), Moby Thesaurus, CMU Pronouncing Dictionary | public domain |
| Merriam-Webster API (optional) | free for personal, non-commercial use under their terms; the Merriam-Webster logo is required and is not included |

## Build and run

See [`CLAUDE.md`](CLAUDE.md) for the full command list. In short:

```bash
cargo build --release                          # packer, gredict-server, gredict-syncd
# download the datasets into data/raw, then build packs with `packer wiktionary|wordnet|moby|gcide|cmudict|fst`
(cd app/ui && npm install && npm run build)
./target/release/gredict-server                # open http://127.0.0.1:8787
cargo test --workspace                         # tests (some skip themselves without data/ or private/)
(cd app/ui && npm run e2e)                     # browser test, including two-device sync
```

The desktop/mobile shell lives in `app/src-tauri` and needs the WebKitGTK development libraries on Linux. The sync server's systemd unit and installer are in `deploy/`.

## Documents

- [`docs/SETUP.md`](docs/SETUP.md): everything that needs a person: the sync server, router, Mac, phone, accounts and keys.
- [`docs/REQUIREMENTS.md`](docs/REQUIREMENTS.md): the original request and the decisions made.
- [`docs/PLAN.md`](docs/PLAN.md): architecture, status and what is not built yet.
