# GRE Dictionary App: Implementation Plan

## Status (2026-10-11)

**Built and tested** (`cargo test --workspace`, `npm run e2e`): three offline dictionaries (Wiktionary 1.36M, WordNet 152k, Webster 1913 108k) with typo search, IPA (CMU fallback) and cached audio; optional live Merriam-Webster (on-demand only, per its terms); the GRE thesaurus (notes graph, related Barron's words, root families); every Barron's quiz (80 units + applying, two cumulative reviews, 60 Root Work, 12 root reviews, pretest, posttest) plus generated and notes quizzes with scoring, history and retrieval search; notes grammar `= ≈ >< ⍯`; user DB with forward-only migrations (v1-v4); row-level sync over pinned TLS on ports 25 / 5025 with route probing, VPN handling and the status pill; photographed notes with OCR and manually triggered Claude/agy review; Barron's received from the server by paired devices.

**Written but not yet built (needs `sudo apt install` of the WebKitGTK dev libraries, then a Mac/Android toolchain):** the Tauri shell in `app/src-tauri`.

**Not started (need the physical devices to build and test):** on-device AI runtime on the NPU/GPU/CPU ladder (CoreML on the M2, QNN on the S24 Ultra), Bluetooth LE sync, Android packaging, FSRS spaced repetition.

**Needs the owner:** Merriam-Webster keys and logo, `sudo deploy/install-syncd.sh`, router forwarding for 5025, and a decision on a local vision model for better handwriting OCR.


## Context
The user is studying GRE Verbal on an M2 MacBook Air, an S24 Ultra (Android) and an Ubuntu box (`lhan.duckdns.org`). They want a fast, offline, portable dictionary app with:
- Oxford-style UI and switchable dictionary sources
- history and pronunciation
- a GRE-focused thesaurus built from their tutor notes and Barron's
- Barron's quizzes plus quizzes generated from the notes, with scoring, history and semantic search over past sessions
- background sync between devices
- safe data migrations across app updates

The repo `/home/lhan/Documents/AILab/DictionaryClaude` is empty, so this is a greenfield build.

**Decisions so far:**
- Stack: Tauri 2 + Rust.
- Data: open data plus the official Merriam-Webster API with entries cached for offline use. There will be no scraping of Oxford, MW or Cambridge, because it's copyrighted and against their terms; Cambridge has no free API, so it's excluded.
- Notes: currently on paper, so they'll be written inside the app from now on.
- Barron's source: `~/800 GRE words (for 700+ SAT).pdf`, which is Barron's *Essential Words for the GRE*, 4th ed., 433 pages. Its text extracts cleanly with `pdftotext -layout`.

## Step 0: Save the requirements
- Save the user's original prompt verbatim to `docs/REQUIREMENTS.md`, plus a "Decisions" section with the answers above.
- Add a memory saying it may only be edited with the user's explicit permission.
- Write `CLAUDE.md`, pointing at `docs/REQUIREMENTS.md`, with the rule "re-read before each phase; never edit without permission".
- Run `git init`. Add a `.gitignore` covering `private/` (Barron's-derived data) and `data/raw/`.

## Architecture
A Cargo workspace plus a Svelte 5 + TypeScript frontend (a static build, small and fast):

```
crates/core       domain logic: dict query, search, user DB + migrations, notes parser,
                  relation graph, quiz engine, FSRS scheduler, embeddings, sync oplog
crates/packer     CLI (runs on Ubuntu): downloads open dumps, builds dictionary packs,
                  imports Barron's PDF → private/barrons.sqlite
crates/syncd      headless sync server for Ubuntu (systemd service)
app/src-tauri     Tauri 2 shell (desktop + Android), thin commands calling core
app/ui            Svelte UI with responsive desktop/mobile layouts
```

### Dictionary data (read-only packs, replaceable on update)
- **Sources:**
  - Wiktionary English (kaikki.org wiktextract JSONL; translations dropped): definitions, IPA, examples, etymology, synonyms and antonyms
  - Open English WordNet
  - GCIDE / Webster 1913 (public domain, rich in literary vocabulary)
  - Moby Thesaurus
  - CMUdict as an IPA/ARPAbet fallback
- **Master word list first:** the union of headwords across all sources goes into a `lemmas` table, keyed by a normalized headword string. That key is stable across pack versions, and all user data references words by it, never by row IDs.
- **Pack format:** one SQLite file per source. Each entry is stored as a zstd-compressed JSON blob in a normalized entry schema. An `fst` index over headwords gives instant prefix search plus Levenshtein fuzzy search.
- **Merriam-Webster:** the user registers free Collegiate + Thesaurus API keys (non-commercial use, 1,000 calls/day).
  - When online, a lookup fetches the MW entry and caches it in `cache.sqlite`, including MW audio files.
  - ~~A background pre-warm queue~~ **Dropped (2026-10-11):** MW's terms (section 5(a)) forbid automated queries. Entries are fetched only when the user opens them, and kept offline only if the user leaves the setting on.
  - Verify the MW caching and attribution terms before shipping.
- **Source switcher:** tabs for Wiktionary | WordNet | Webster 1913 | Merriam-Webster.
- **Pronunciation:**
  - IPA comes from Wiktionary, MW or CMUdict.
  - Audio comes from MW (cached), then an optional Wikimedia Commons audio pack limited to GRE words plus the most common ~30k words.
  - Fallbacks: Web Speech TTS on macOS/Android, and `espeak-ng` on Linux if it's installed.

### Portability and battery
- Linux ships as one AppImage with the packs inside. Mac ships as one `.app` (arm64), dragged in as a single item. Android ships as an APK, with packs bundled or downloaded on first run.
- There is no visible background window. Background sync uses the same binary run as `--agent`, with no window and no dock icon: a launchd LaunchAgent on Mac and a systemd `--user` unit on Linux. It wakes rarely and is event-driven.
- Packs are memory-mapped and read lazily; nothing loads the full dictionary into RAM.

### UI (Oxford Learner's-style look, without Oxford branding or logos)
- Navy header with the search bar; large headword, italic part of speech, IPA with a speaker button.
- Numbered senses with grey italic examples; collapsible boxes for "Synonyms / GRE thesaurus", "Word origin" and "Barron's".
- Desktop layout: a sidebar for history, quizzes and notes. Mobile layout: bottom tabs (Search · History · Quiz · Notes).

### History
- A `lookups` table records word, source, time and device.
- History can be searched, filtered by date and starred.
- History is synced.

### Notes and the GRE thesaurus
- **In-app note editor** with a small grammar and live highlighting:
  - `a = b = c` means same meaning (synonyms)
  - `a ≈ b` means roughly similar
  - `a >< b` means opposite (also accept `<>` and `> <`)
  - `a ⍯ b` means different, easily confused, like imbibition vs osmosis; `≠` is accepted as the same relation
  - free text is allowed; notes carry a tutor-session date and tags
  - typing `~=` or `!=` auto-converts to `≈` or `≠` on mobile keyboards
- **Parser** (in `core`) turns each note into edges in a `relations(word_a, word_b, rel, source, note_id)` graph. Sources are note, barrons, wordnet, moby, mw and wiktionary.
- **Thesaurus panel ranking:**
  1. your notes, with transitive `=` closure
  2. Barron's words, ranked by similarity between definition embeddings, plus root families from the "300 roots" section
  3. general thesaurus results

  The panel fills up to about 10 GRE-relevant words when the notes are sparse.
- Optional later: photo OCR import of old paper notes, with a correction screen. Offline handwriting OCR is weak, so this is low priority.

### Note photo upload and AI (added mid-session by the user)
- **Upload flow:** from the phone or Mac, take or choose a photo of handwritten notes. It's uploaded to `syncd` on Ubuntu, along with an op in the oplog.
  1. A job queue runs handwriting OCR on Ubuntu.
  2. A rule-based normalizer converts symbols (`=`, `≈`, `≠`, `><`) into note grammar.
  3. The result is a *draft note* linked to the image.
  4. The draft syncs to every device. You fix it in a review screen with the image on one side and editable text on the other, then accept it.
  5. Accepting it updates the relation graph and the notes quiz.
- **Offline photos:** when Ubuntu is unreachable, photos queue on the device. If the on-device OCR model is installed, the device makes a local draft immediately; the server's result replaces it later if it's better.
- **OCR location (user chose):** Ubuntu first, with the device NPU as a fallback.
- **Capture in the app:** "Take photo" (Android camera via `<input capture>`, Mac camera via `getUserMedia`) and "Add photos" (multi-select from gallery or files). Several photos make one page set.
- **"Ask agents" escalation (user-requested, for hard cases):**
  - Triggered by a button on a draft. It never runs automatically, because agent quota is limited.
  - `syncd` runs a job on Ubuntu with the two installed CLIs. Both run headless, with no tools, read-only, inside a sandboxed temp directory holding only the image and the current draft:
    1. **Round 1:** `claude -p … --output-format json --json-schema note.schema.json` and `agy -p … --output-format json --json-schema note.schema.json`. Each independently transcribes into the note grammar and lists the lines it's unsure about.
    2. **Round 2 (only if they disagree):** each agent sees the other's version and argues line by line. Rounds are capped (default 2).
    3. **Result:** a merged proposal that marks agreed lines and lines that still conflict.
  - **In the app:** a "Discussion" view shows the photo, each agent's version, the discussion transcript and a per-line diff against the OCR draft. You accept or edit each line, then accept the note. Nothing changes your notes without your approval.
  - **Quota controls:**
    - a daily cap in Settings
    - a choice of mode: "Claude only", "agy only" or "both, with discussion"
    - an effort level (`--effort`)
    - usage shown per job
    - a crop tool so you can send only the hard region, which saves tokens
  - **Job state** (queued, running, needs_review, accepted) and the transcript are stored in `syncd` and synced, so you can start a job from the phone and review it on the Mac.
- **OCR model:** a small vision-language model, with Qwen2.5-VL-3B (4-bit, via llama.cpp) as the candidate. On Ubuntu it runs on the T400 4 GB with CUDA, falling back to the CPU. It's prompted to transcribe exactly, keeping `= ≈ ≠ > <`. The model is benchmarked on 10 real note photos before it's locked in.
- **Device AI with NPU → GPU → CPU priority (user confirmed):** all on-device inference goes through one `core::ai` runtime trait. Implementation is ONNX Runtime with execution providers chosen in this order:
  - **Mac M2:** CoreML EP (Apple Neural Engine; the compute-unit setting lets CoreML fall back to GPU) → CPU.
  - **S24 Ultra:** QNN EP (Snapdragon 8 Gen 3 Hexagon NPU) → GPU (NNAPI/XNNPACK) → CPU.
  - **Linux desktop:** CUDA if present → CPU.
- **On-device AI workloads:**
  - Semantic embeddings: a small transformer, `bge-small-en` int8, about 35 MB, replacing model2vec when an NPU is available; model2vec stays as the CPU fallback.
  - Optional on-device OCR: a lightweight handwriting recognizer exported to ONNX/CoreML.
  - Models ship as optional downloads, so the base app stays small.
  - A Settings screen shows which backend is active.

### Quizzes
**Barron's importer** (`packer barrons <pdf>`) parses the layout text into `private/barrons.sqlite`. That file stays local and is never committed or distributed. It extracts:
- 800 entries: word, part of speech, definition, example sentences, derived forms and "Terms from the Arts…" glosses
- 80 unit reviews: Matching ×10, Fill-ins ×10 (with word bank), Sense/Nonsense ×5
- Answer keys (pp. 398–416), joined to the units
- Pretest/posttest (sentence equivalence, text completion and reading comprehension), Root Work and Root Roundup exercises
- The 300 high-frequency words list and the 300 word roots

The importer runs golden tests against known pages, such as Unit 1 and the Unit 6 key.

**Generated quizzes:**
- From the word lists: definition→word, word→definition and cloze questions from example sentences.
- From the notes: "which is a synonym, antonym or ≠ of X", group-completion questions, and confusables.
- Notes-quiz mode is filtered by tutor session or tag.

**Sessions:**
- Tables: `sessions(id, kind, filter, started, ended, score)` and `attempts(session, item, answer, correct, ms)`.
- An FSRS spaced-repetition scheduler covers all items.
- History screen with score trends. You can review a past session and retry the missed items.

**Retrieval search over sessions:** hybrid search combining SQLite FTS5 (BM25) with semantic vectors.
- Vectors come from model2vec static embeddings (`potion-base-8M`, about 30 MB, runs on the CPU in microseconds with little battery use). They're fused with reciprocal rank fusion.
- Indexed text: questions, the words involved, your wrong answers and note text.
- This lets a query like "words about stubbornness I got wrong" find the right sessions.

### Personal data, sync and migrations
- **Storage:** `user.sqlite` holds history, notes, sessions, attempts, FSRS state and settings. Every mutation is also appended to an `oplog` with a hybrid logical clock and a device ID.
- **Merge rules:** last-writer-wins per field, add-wins sets, and idempotent op application.
- **Transports:**
  1. `syncd` on Ubuntu at `lhan.duckdns.org:<port>`: HTTPS with a Let's Encrypt certificate via the DuckDNS DNS-01 challenge. Devices pair with a token by scanning a QR code, or by themselves: on a known home Wi-Fi a new device asks the LAN port (`/v1/enroll`, private source addresses only, never the internet port, 20 per hour, logged) for its token and pins the certificate it gets back. Unpairing on purpose turns this off until "Pair now" is pressed.
  2. LAN peer-to-peer: devices on the same Wi-Fi or a phone hotspot find each other via mDNS.
  3. Bluetooth LE between Mac and phone, built last: `btleplug` on the Mac and a Kotlin Tauri plugin on Android.
- **Background triggers:**
  - Desktop agent: on network change and every 15 min.
  - Android: WorkManager periodic jobs (minimum 15 min) plus network-change triggers. Android restricts background Bluetooth unless a foreground service is running, so BLE sync happens when the app runs or during a short service window.
- **Migrations:**
  - Forward-only numbered migrations (`rusqlite_migration`) run at startup, after an automatic backup `user.sqlite.bak-vN`.
  - Ops are versioned with upcasters for old formats.
  - The sync protocol negotiates a version.
  - Packs are swappable because user data references lemma keys.
  - CI migrates a saved fixture DB from every released version.

### Server address selection and status indicator (user-specified)
- **Server:** `lhan.duckdns.org`. The user reserved port **25** (same network) and port **5025** (remote). One `syncd` listens on both.
- **Rule:** when the Wi-Fi SSID is `Anan` or `Anan 5g` the device is on the home network, so it tries the LAN path on port 25 first. Otherwise it uses `lhan.duckdns.org:5025`. The SSID list is editable in Settings.
- **Things found on the Ubuntu box:**
  - DuckDNS resolves correctly to the public IP (115.73.130.78).
  - `/etc/hosts` maps the name to 127.0.0.1 on the server itself only.
  - The server is on Wi-Fi at `192.168.1.7`, and the user has already reserved that address in the router. The public name may not work from inside the LAN without router hairpin NAT, so the LAN path connects to `192.168.1.7:25` directly. mDNS (`_gredict._tcp`) stays as a fallback if the address ever changes, and Settings has an override field.
  - Nothing is listening on 25 or 5025 yet.
  - Ports under 1024 need `AmbientCapabilities=CAP_NET_BIND_SERVICE` in the systemd unit. The firewall needs rules for 25 (LAN only) and 5025, and the router forwards 5025.
- **Probe, don't trust the SSID:**
  - Reading the SSID needs a location or nearby-Wi-Fi permission (macOS 14+, Android 13+). If it's denied, the app falls back to probing both paths.
  - A probe is a TLS connect with the pinned server key and a signed challenge on `/hello`, with a short timeout (about 1.5 s). The first valid answer wins, so a captive portal or a hijacked DNS answer can't impersonate the server.
  - The choice is re-evaluated on every network-change event and at the start of each sync.
- **VPN handling:**
  - Detect VPN interfaces (`utun`, `tun`, `wg`, `ppp`, Tailscale) and a default route through them.
  - While a VPN is active, an SSID match doesn't prove LAN traffic stays local. Probe LAN with a short timeout, then fall back to `:5025`.
  - If both fail while a VPN is active, show "VPN may be blocking" with a hint to allow local network access or split tunneling.
- **Status indicator in the UI:** a pill in the header.
  - Green: "Server · Home" (LAN, port 25).
  - Blue: "Server · Remote" (port 5025).
  - Amber: "VPN active" (reachable or blocked).
  - Grey: "Offline · N changes pending".
  - Tapping it opens a panel with the current route, latency, last sync time, pending changes, queue status of OCR and agent jobs, a "Test now" button and the last probe results.
  - `syncd` provides `/health` (version, uptime, job queues). An offline server never blocks the app.

## Phases (each ends with a working build)
1. Scaffold, Step 0, and `packer`: download dumps, build the master word list and packs, and run the Barron's importer.
2. Desktop app: search, entry view, source switch, history and pronunciation (Linux AppImage first).
3. Notes editor, parser, relation graph and the GRE thesaurus panel.
4. Quizzes: Barron's and notes quizzes, sessions, scoring, FSRS and hybrid search.
5. MW API with caching and the pre-warm queue.
6. Sync: oplog, `syncd` on Ubuntu, LAN mDNS and the background agents.
7. Mac build (on the M2 or a GitHub Actions macOS runner) and Android build with the mobile layout.
8. Bluetooth LE sync.
9. Note photo upload (take or add photos), the OCR job on Ubuntu, the draft-review screen, the "Ask agents" Claude + agy discussion job and its review UI, and the NPU/GPU/CPU AI runtime on Mac and Android.

## Already done (before re-planning)
- `git init` in the repo.
- `docs/REQUIREMENTS.md` created (verbatim prompt + decisions). It still needs the user's new "update:" line appended; that line is the user's own text.
- `docs/PLAN.md` mirror and `.gitignore` (`private/`, `data/raw/`, `data/packs/`, build outputs).
- The PDF moved to `private/barrons-essential-words-4e.pdf`.
- Toolchain check: Node 22, sqlite3 and pdftotext are present. Rust is **not** installed; the first implementation step installs it via rustup (user-level, `~/.cargo`). pnpm is not installed either; use npm.

## Still to do in Step 0
- Append to `docs/REQUIREMENTS.md`, with permission granted:
  - the user's "update:" line, word for word: *update: there should be a process of uploading the image of notes to the app server/the ubuntu machine to update the apps as well, any AI things of the app should run on NPU of mac and s24u, fallback to gpu then gpu later*
  - the agent-review request: *"i also want to have the option if triggered for the agents to chat about it and let me verify the update (should implement in the app it self, like add photos, take photos), because agent quota is not infinite but for hard cases it's necessary"*
  - Decisions entries: NPU → GPU → CPU; OCR on Ubuntu first with device fallback; manually triggered Claude + agy discussion that you verify in the app.
- Refresh the `docs/PLAN.md` mirror.
- Write `CLAUDE.md`.
- Save a memory about the requirements-edit rule.

## Verification
- `cargo test --workspace`: parser golden tests (Barron's Unit 1 entries and review, answer keys matched to every unit), notes grammar tests, an oplog merge property test (any delivery order gives the same state), and migration fixture tests.
- `packer stats`: lemma counts per source, with spot checks of words like *abate*, *loquacious* and *taciturn* in every pack.
- Run the AppImage on Ubuntu and check lookup latency under 50 ms, cold start under 1 s and idle CPU at about 0%. On the Mac, check energy use in Activity Monitor.
- Sync: edit on two devices while offline, reconnect through `syncd` and through the LAN, then check that the states match. Upgrade from an older build's DB and confirm the data is intact.

## Open items for implementation
- ~~The port to reserve on Ubuntu~~: resolved, ports 25 (LAN) and 5025 (remote).
- MW API keys (the user registers them at dictionaryapi.com).
