//! The command surface. Every command takes and returns JSON so the Tauri
//! shell, the browser dev server and (later) the sync server share one path.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use anyhow::{anyhow, bail, Result};
use gredict_core::entry::Entry;
use gredict_core::notes::{self, Graph};
use gredict_core::pack::Pack;
use gredict_core::thesaurus::Thesaurus;
use gredict_core::fuzzy::Fuzzy;
use gredict_core::ipa::Pronunciations;
use gredict_core::lemma;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

use crate::barrons::Barrons;
use crate::userdb::{self, new_id, now_ms};

#[derive(Clone)]
pub struct Config {
    /// Holds `user.sqlite` and caches. Created if missing.
    pub data_dir: PathBuf,
    /// Holds `*.pack` dictionary packs.
    pub packs_dir: PathBuf,
    /// Holds `barrons.sqlite` if the owner imported the book.
    pub private_dir: Option<PathBuf>,
}

pub(crate) struct Source {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) pack: Mutex<Pack>,
    /// 0 until counted (counting scans the whole pack, so it is done off the startup path).
    pub(crate) entries: std::sync::atomic::AtomicU64,
}

/// Known pack files in display order.
const SOURCES: &[(&str, &str, &str)] = &[
    ("wiktionary", "Wiktionary", "wiktionary.pack"),
    ("wordnet", "WordNet", "wordnet.pack"),
    ("gcide", "Webster 1913", "gcide.pack"),
    ("oald", "OALD", "oald.pack"),
];

fn open_cache(dir: &std::path::Path) -> Result<Connection> {
    let c = Connection::open(dir.join("cache.sqlite"))?;
    c.execute_batch(crate::mw::CACHE_SCHEMA)?;
    Ok(c)
}

pub struct Api {
    pub(crate) sources: Vec<Source>,
    /// Bundled packs (read-only) and the downloaded ones in `<data>/packs`.
    pub(crate) packs_dir: PathBuf,
    pub(crate) pack_progress: Mutex<HashMap<String, (u64, u64)>>,
    /// Set when a pack was added or removed: the shell should reopen the Api.
    pub(crate) reload: std::sync::atomic::AtomicBool,
    pub(crate) moby: Option<Mutex<Thesaurus>>,
    /// Hot-swappable: a paired device receives the parsed book from the server.
    pub(crate) barrons: RwLock<Option<Arc<Barrons>>>,
    pub(crate) private_dir: Option<PathBuf>,
    pub(crate) db: Mutex<Connection>,
    pub(crate) device: String,
    pub(crate) rng: Mutex<u64>,
    pub(crate) data_dir: PathBuf,
    /// Device-local cache of Merriam-Webster lookups (never synced).
    pub(crate) cache: Mutex<Connection>,
    /// CMU pronunciations: IPA for words whose entry has none.
    pron: Option<Mutex<Pronunciations>>,
    /// Set by commands that change synced data; the background loop reads it.
    pub(crate) dirty: std::sync::atomic::AtomicBool,
    fuzzy: OnceLock<Option<Fuzzy>>,
    pub(crate) ai: OnceLock<std::result::Result<Arc<gredict_core::ai::Embedder>, String>>,
    fuzzy_paths: [PathBuf; 2],
    /// Synonym sets of every Barron's word, built on first use.
    barrons_syn: RwLock<Option<Arc<HashMap<String, (HashSet<String>, HashSet<String>)>>>>,
}

#[derive(Serialize)]
struct Suggestion {
    word: String,
    key: String,
    barrons: bool,
}

impl Api {
    /// User data only, no dictionaries: what the sync server needs.
    pub fn open_headless(data_dir: &std::path::Path) -> Result<Self> {
        Self::open_headless_serving(data_dir, &data_dir.join("none"))
    }

    /// Like `open_headless`, and offers the pack files in `packs_dir` to paired devices.
    pub fn open_headless_serving(data_dir: &std::path::Path, packs_dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db = userdb::open(&data_dir.join("user.sqlite"))?;
        let device = Self::device_id(&db)?;
        let fuzzy_paths = [data_dir.join("none.fst"), data_dir.join("none.fst")];
        Ok(Api { sources: Vec::new(), packs_dir: packs_dir.to_path_buf(), pack_progress: Default::default(), reload: Default::default(), moby: None, barrons: RwLock::new(None), private_dir: None, db: Mutex::new(db), device, rng: Mutex::new(now_ms() as u64 | 1), data_dir: data_dir.to_path_buf(), cache: Mutex::new(open_cache(data_dir)?), pron: None, dirty: Default::default(), fuzzy: OnceLock::new(), ai: OnceLock::new(), fuzzy_paths, barrons_syn: RwLock::new(None) })
    }

    fn device_id(db: &Connection) -> Result<String> {
        match db.query_row("SELECT v FROM settings WHERE k='device'", [], |r| r.get::<_, String>(0)) {
            Ok(d) => Ok(d),
            Err(_) => {
                let d = new_id();
                db.execute("INSERT INTO settings VALUES('device', ?)", [&d])?;
                Ok(d)
            }
        }
    }

    pub fn open(cfg: Config) -> Result<Self> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let db = userdb::open(&cfg.data_dir.join("user.sqlite"))?;
        let device = Self::device_id(&db)?;
        let mut sources = Vec::new();
        let find = |file: &str| crate::packs::locate(&cfg.data_dir, &cfg.packs_dir, file);
        for (id, name, file) in SOURCES {
            let p = find(file);
            if p.exists() {
                let pack = Pack::open(&p)?;
                let entries = std::sync::atomic::AtomicU64::new(crate::packs::cached_count(&db, &p));
                sources.push(Source { id, name, pack: Mutex::new(pack), entries });
            }
        }
        // No packs is allowed: a fresh phone starts empty and downloads them.
        let moby = find("moby.pack");
        let moby = moby.exists().then(|| Thesaurus::open(&moby)).transpose()?.map(Mutex::new);
        let barrons = match cfg.private_dir.as_ref().map(|d| d.join("barrons.sqlite")) {
            Some(p) if p.exists() => Some(Arc::new(Barrons::open(&p)?)),
            _ => None,
        };
        let pron = {
            let p = find("cmudict.pack");
            p.exists().then(|| Pronunciations::open(&p)).transpose()?.map(Mutex::new)
        };
        let fuzzy_paths = [find("wiktionary.fst"), cfg.data_dir.join("wiktionary.fst")];
        Ok(Api { sources, packs_dir: cfg.packs_dir.clone(), pack_progress: Default::default(), reload: Default::default(), moby, barrons: RwLock::new(barrons), private_dir: cfg.private_dir, db: Mutex::new(db), device, rng: Mutex::new(now_ms() as u64 | 1), data_dir: cfg.data_dir.clone(), cache: Mutex::new(open_cache(&cfg.data_dir)?), pron, dirty: Default::default(), fuzzy: OnceLock::new(), ai: OnceLock::new(), fuzzy_paths, barrons_syn: RwLock::new(None) })
    }

    pub fn call(&self, cmd: &str, args: Value) -> Result<Value> {
        let out = self.dispatch(cmd, args);
        // Anything that writes synced data nudges the background sync.
        if out.is_ok() && matches!(cmd, "draft_agents" | "draft_reread" | "draft_create" | "draft_update" | "draft_accept" | "draft_reject" | "entry" | "history_star" | "history_delete" | "history_clear" | "note_save" | "note_delete" | "quiz_start" | "quiz_answer" | "session_delete") {
            self.dirty.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        out
    }

    fn dispatch(&self, cmd: &str, args: Value) -> Result<Value> {
        let a = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        let n = |k: &str, d: i64| args.get(k).and_then(Value::as_i64).unwrap_or(d);
        if self.sources.is_empty() && matches!(cmd, "search" | "entry") {
            // Merriam-Webster is looked up online and needs no pack, so it works on a device that has none yet.
            if self.mw_configured() {
                if cmd == "search" {
                    return Ok(json!({"suggestions": [], "did_you_mean": []}));
                }
                if a("source").as_deref() == Some("mw") {
                    return self.entry_mw(&req_word(&args)?, &lemma::key(&req_word(&args)?), args.get("record").and_then(Value::as_bool).unwrap_or(true));
                }
            }
            bail!("no dictionaries are installed yet: open Settings → Dictionaries to download them");
        }
        let req = |k: &str| a(k).ok_or_else(|| anyhow!("missing argument `{k}`"));
        match cmd {
            "sources" => {
                let mut v: Vec<Value> = self.sources.iter().map(|s| json!({"id": s.id, "name": s.name, "entries": Some(s.entries.load(std::sync::atomic::Ordering::Relaxed)).filter(|n| *n > 0)})).collect();
                // Merriam-Webster is looked up live (and kept), so it has no entry count.
                if self.mw_configured() {
                    v.push(json!({"id": "mw", "name": "Merriam-Webster", "entries": null}));
                }
                Ok(json!(v))
            }
            "packs_list" => self.packs_list(),
            "packs_catalog" => self.packs_catalog(),
            "pack_install" => self.pack_install(&req("id")?),
            "pack_remove" => self.pack_remove(&req("id")?),
            "ai_status" => self.ai_status(),
            "ai_test" => self.ai_test(),
            "packs_auto_status" => self.packs_auto_status(),
            "packs_auto_now" => self.packs_auto_now(),
            "pack_progress" => Ok(self.pack_progress_now()),
            "mw_status" => self.mw_status(),
            "mw_config" => self.mw_config(&args),
            "search" => self.search(&req("q")?, a("source").as_deref()),
            "entry" => self.entry(&req("word")?, a("source").as_deref(), args.get("record").and_then(Value::as_bool).unwrap_or(true)),
            "draft_create" => {
                let photos: Vec<String> = args.get("photos").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
                self.draft_create(&photos, &a("session_date").unwrap_or_default(), &a("tags").unwrap_or_default())
            }
            "drafts_list" => self.drafts_list(args.get("all").and_then(Value::as_bool).unwrap_or(false)),
            "draft_get" => self.draft_get(&req("id")?),
            "draft_update" => self.draft_update(&req("id")?, &req("text")?, a("session_date").as_deref(), a("tags").as_deref()),
            "draft_accept" => self.draft_accept(&req("id")?, a("text").as_deref()),
            "draft_reject" => self.draft_reject(&req("id")?),
            "draft_reread" => self.draft_reread(&req("id")?),
            "draft_agents" => self.draft_agents(&req("id")?, &a("mode").unwrap_or_else(|| "both".into()), &a("effort").unwrap_or_else(|| "medium".into())),
            "agent_status" => Ok(self.agent_status()),
            "run_jobs" => self.run_jobs_once(),
            "photo_data" => self.photo_data(&req("name")?),
            "audio_get" => self.audio_get(&req("url")?),
            "audio_prefetch" => self.audio_prefetch(n("limit", 20) as usize),
            "thesaurus" => Ok(self.thesaurus(&lemma::key(&req("word")?))?),
            "history_list" => self.history_list(a("q").as_deref(), n("limit", 100), n("offset", 0), args.get("starred").and_then(Value::as_bool).unwrap_or(false)),
            "history_star" => self.exec("UPDATE lookups SET starred=?2, updated=?3 WHERE id=?1", params![req("id")?, n("starred", 1), now_ms()]),
            "history_delete" => self.exec("UPDATE lookups SET deleted=1, updated=?2 WHERE id=?1", params![req("id")?, now_ms()]),
            "history_clear" => self.exec("UPDATE lookups SET deleted=1, updated=?1 WHERE starred=0 AND deleted=0", params![now_ms()]),
            "notes_list" => self.notes_list(),
            "note_save" => self.note_save(a("id"), &req("body")?, a("session_date").unwrap_or_default(), a("tags").unwrap_or_default()),
            "note_delete" => {
                let id = req("id")?;
                self.exec("UPDATE notes SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()])?;
                self.exec("DELETE FROM note_edges WHERE note_id=?1", params![id])
            }
            "quiz_catalog" => self.quiz_catalog(),
            "quiz_start" => self.quiz_start(&args),
            "quiz_answer" => self.quiz_answer(&args),
            "quiz_resume" => self.quiz_resume(&args),
            "update_check" => self.update_check(&a("current").unwrap_or_default()),
            "expand_terms" => {
                let w = self.expand_local(&req("term")?)?;
                Ok(json!({"words": w}))
            }
            "update_progress" => Ok(self.update_progress()),
            "update_download" => self.update_download(args.get("asset").unwrap_or(&Value::Null)),
            "session_get" => self.session_get(&req("id")?),
            "sessions_list" => self.sessions_list(a("q").as_deref(), n("limit", 50)),
            "session_delete" => {
                let id = req("id")?;
                self.exec("DELETE FROM session_fts WHERE session_id=?1", params![id])?;
                self.exec("UPDATE sessions SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()])
            }
            "sync_export" => self.sync_export(n("since", 0), n("limit", 5000) as usize),
            "sync_apply" => self.sync_apply(args.get("changes").unwrap_or(&Value::Null)),
            "sync_pull" => self.sync_pull(n("cursor", 0), n("limit", 2000) as usize),
            "sync_pair" => self.sync_pair(&req("code")?),
            "devices_list" => self.server_post("/v1/devices", json!({})),
            "device_revoke" => self.server_post("/v1/device/revoke", json!({"name": req("name")?})),
            "sync_unpair" => self.sync_unpair(),
            "sync_autopair" => self.sync_autopair(),
            "sync_set_autopair_host" => {
                self.set_setting("autopair.host", &req("host")?)?;
                Ok(json!({"ok": true}))
            }
            "sync_set_home_ssids" => {
                self.set_setting("sync.home_ssids", &req("ssids")?)?;
                Ok(json!({"ok": true}))
            }
            "sync_now" => self.sync_now(),
            "server_status" => self.server_status(args.get("probe").and_then(Value::as_bool).unwrap_or(false)),
            "note_parse" => Ok(json!(notes::parse(&req("body")?))),
            _ => bail!("unknown command `{cmd}`"),
        }
    }

    pub(crate) fn barrons(&self) -> Option<Arc<Barrons>> {
        self.barrons.read().unwrap().clone()
    }

    /// Opens `barrons.sqlite` from the private directory (after a download).
    pub(crate) fn reload_barrons(&self) -> Result<()> {
        let dir = self.private_dir.as_ref().ok_or_else(|| anyhow!("no private directory is configured"))?;
        let b = Barrons::open(&dir.join("barrons.sqlite"))?;
        *self.barrons.write().unwrap() = Some(Arc::new(b));
        *self.barrons_syn.write().unwrap() = None;
        Ok(())
    }

    fn exec(&self, sql: &str, p: impl rusqlite::Params) -> Result<Value> {
        let n = self.db.lock().unwrap().execute(sql, p)?;
        Ok(json!({"changed": n}))
    }

    fn source(&self, id: Option<&str>) -> &Source {
        id.and_then(|id| self.sources.iter().find(|s| s.id == id)).unwrap_or(&self.sources[0])
    }

    // ---- search -----------------------------------------------------------

    fn search(&self, q: &str, source: Option<&str>) -> Result<Value> {
        let key = lemma::key(q);
        if key.is_empty() {
            return Ok(json!({"suggestions": [], "did_you_mean": []}));
        }
        let primary = self.source(source);
        let mut words: Vec<String> = primary.pack.lock().unwrap().prefix(&key, 400)?;
        // Other sources fill gaps (e.g. a word only WordNet has).
        if words.len() < 15 {
            for s in self.sources.iter().filter(|s| s.id != primary.id) {
                words.extend(s.pack.lock().unwrap().prefix(&key, 60)?);
            }
        }
        let barrons = self.barrons();
        let mut seen = HashSet::new();
        let mut out: Vec<Suggestion> = words
            .into_iter()
            .map(|w| Suggestion { key: lemma::key(&w), barrons: false, word: w })
            .filter(|s| seen.insert(s.key.clone()))
            .map(|mut s| {
                s.barrons = barrons.as_ref().is_some_and(|b| b.contains(&s.key));
                s
            })
            .collect();
        // Exact match first, then GRE words, then single words by length.
        out.sort_by_key(|s| {
            (s.key != key, !s.barrons, s.key.contains(' '), s.key.chars().count(), s.key.clone())
        });
        out.truncate(15);
        // Typo help only when prefix search found (almost) nothing: otherwise
        // "abat" would drown the real completions in near-misses.
        let did_you_mean = if out.iter().any(|s| s.key == key) || out.len() >= 3 { Vec::new() } else { self.fuzzy(&key)? };
        Ok(json!({"suggestions": out, "did_you_mean": did_you_mean}))
    }

    /// The headword index for typo search: shipped next to the packs, or
    /// built once into the data directory on first use.
    fn fuzzy_index(&self) -> Option<&Fuzzy> {
        self.fuzzy
            .get_or_init(|| {
                if let Some(p) = self.fuzzy_paths.iter().find(|p| p.exists()) {
                    return Fuzzy::open(p).ok();
                }
                let wikt = self.sources.iter().find(|s| s.id == "wiktionary")?;
                let pack_path = crate::packs::locate(&self.data_dir, &self.packs_dir, "wiktionary.pack");
                let _ = wikt;
                Fuzzy::build(&pack_path, &self.fuzzy_paths[1]).ok()?;
                Fuzzy::open(&self.fuzzy_paths[1]).ok()
            })
            .as_ref()
    }

    /// Typo suggestions ranked by true edit distance, GRE words first.
    fn fuzzy(&self, key: &str) -> Result<Vec<String>> {
        let Some(index) = self.fuzzy_index() else { return Ok(Vec::new()) };
        let barrons = self.barrons();
        let max = match key.chars().count() {
            0..=3 => 1,
            _ => 2,
        };
        let mut scored: Vec<(usize, bool, String)> = index
            .near(key, max, 2000)?
            .into_iter()
            .filter(|w| !w.contains(' ') && w != key)
            .filter_map(|w| {
                let d = bounded_levenshtein(key, &w, max as usize)?;
                let gre = barrons.as_ref().is_some_and(|b| b.contains(&w));
                Some((d, !gre, w))
            })
            .collect();
        scored.sort();
        Ok(scored.into_iter().map(|(_, _, w)| w).take(6).collect())
    }

    // ---- entries ------------------------------------------------------------

    fn entry(&self, word: &str, source: Option<&str>, record: bool) -> Result<Value> {
        let key = lemma::key(word);
        if source == Some("mw") && self.mw_configured() {
            return self.entry_mw(word, &key, record);
        }
        let primary = self.source(source);
        let mut found_in = Vec::new();
        let mut shown: Option<(&Source, Entry)> = None;
        for s in std::iter::once(primary).chain(self.sources.iter().filter(|s| s.id != primary.id)) {
            if let Some(e) = s.pack.lock().unwrap().get(&key)? {
                found_in.push(s.id);
                if s.id == primary.id {
                    shown = Some((s, e));
                } else if shown.is_none() && found_in.len() == 1 {
                    // keep as fallback only if the requested source lacks it
                }
            }
        }
        // If the chosen source lacks the word, show the first one that has it.
        let fallback = if shown.is_none() { found_in.first().copied() } else { None };
        let shown = match (shown, fallback) {
            (Some(x), _) => Some(x),
            (None, Some(id)) => {
                let s = self.source(Some(id));
                s.pack.lock().unwrap().get(&key)?.map(|e| (s, e))
            }
            _ => None,
        };
        let barrons = self.barrons().and_then(|b| b.entries.get(&key).cloned());
        // Entries without their own IPA (WordNet, Webster) borrow the CMU one.
        let ipa_fallback: Option<String> = match (&shown, &self.pron) {
            (Some((_, e)), Some(p)) if !e.blocks.iter().any(|b| b.sounds.iter().any(|s| s.ipa.is_some())) => p.lock().unwrap().get(&key).ok().flatten(),
            _ => None,
        };
        if record && shown.is_some() {
            let display = shown.as_ref().map(|(_, e)| e.word.clone()).unwrap_or_else(|| word.to_string());
            self.record_lookup(&key, &display, shown.as_ref().map_or("", |(s, _)| s.id))?;
        }
        // Syllables: the shown entry's own, else the first other dictionary that has them.
        let syl: Option<String> = shown.as_ref().and_then(|(_, e)| e.syl.clone()).or_else(|| {
            self.sources.iter().find_map(|s| s.pack.lock().unwrap().get(&key).ok().flatten().and_then(|e| e.syl))
        });
        Ok(json!({
            "key": key,
            "syl": syl,
            "source": shown.as_ref().map(|(s, _)| json!({"id": s.id, "name": s.name})),
            "ipa_fallback": ipa_fallback,
            "roots": self.barrons().map(|b| b.roots_for(&key)).unwrap_or_default(),
            "entry": shown.map(|(_, e)| e),
            "found_in": found_in,
            "barrons": barrons,
            "thesaurus": self.thesaurus(&key)?,
        }))
    }

    fn entry_mw(&self, word: &str, key: &str, record: bool) -> Result<Value> {
        let mut out = json!({
            "key": key, "source": {"id": "mw", "name": "Merriam-Webster"}, "found_in": self.sources.iter().filter(|s| s.pack.lock().unwrap().get(key).ok().flatten().is_some()).map(|s| s.id).collect::<Vec<_>>(), "ipa_fallback": null,
            "barrons": self.barrons().and_then(|b| b.entries.get(key).cloned()), "thesaurus": self.thesaurus(key)?, "entry": null,
        });
        match self.mw_entry(word) {
            Ok(Ok(e)) => {
                if record {
                    self.record_lookup(key, &e.word, "mw")?;
                }
                out["syl"] = json!(e.syl.clone().or_else(|| self.sources.iter().find_map(|s| s.pack.lock().unwrap().get(key).ok().flatten().and_then(|x| x.syl))));
                out["entry"] = json!(e);
            }
            Ok(Err(suggestions)) => out["did_you_mean"] = json!(suggestions),
            // Offline, a bad key, the daily limit: shown in place of the entry.
            Err(e) => out["mw_error"] = json!(format!("{e:#}")),
        }
        Ok(out)
    }

    fn record_lookup(&self, key: &str, word: &str, source: &str) -> Result<()> {
        let db = self.db.lock().unwrap();
        // Looking the same word up again within a minute is one history item.
        let recent: Option<String> = db
            .query_row("SELECT id FROM lookups WHERE key=?1 AND ts > ?2 AND deleted=0 ORDER BY ts DESC LIMIT 1", params![key, now_ms() - 60_000], |r| r.get(0))
            .ok();
        match recent {
            Some(id) => db.execute("UPDATE lookups SET ts=?2, updated=?2, source=?3 WHERE id=?1", params![id, now_ms(), source])?,
            None => db.execute(
                "INSERT INTO lookups(id,ts,key,word,source,device,updated) VALUES (?,?,?,?,?,?,?)",
                params![new_id(), now_ms(), key, word, source, self.device, now_ms()],
            )?,
        };
        Ok(())
    }

    // ---- history ------------------------------------------------------------

    fn history_list(&self, q: Option<&str>, limit: i64, offset: i64, starred: bool) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let pat = format!("{}%", q.map(lemma::key).unwrap_or_default());
        let mut stmt = db.prepare(
            "SELECT id,ts,key,word,source,starred FROM lookups
             WHERE key LIKE ?1 AND deleted=0 AND (?2 = 0 OR starred = 1) ORDER BY ts DESC LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt
            .query_map(params![pat, starred as i64, limit, offset], |r| {
                Ok(json!({"id": r.get::<_, String>(0)?, "ts": r.get::<_, i64>(1)?, "key": r.get::<_, String>(2)?,
                          "word": r.get::<_, String>(3)?, "source": r.get::<_, String>(4)?, "starred": r.get::<_, i64>(5)? != 0}))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }

    // ---- notes --------------------------------------------------------------

    fn notes_list(&self) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT id,created,updated,session_date,tags,body FROM notes WHERE deleted=0 ORDER BY session_date DESC, updated DESC")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(json!({"id": r.get::<_, String>(0)?, "created": r.get::<_, i64>(1)?, "updated": r.get::<_, i64>(2)?,
                          "session_date": r.get::<_, String>(3)?, "tags": r.get::<_, String>(4)?, "body": r.get::<_, String>(5)?}))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }

    fn note_save(&self, id: Option<String>, body: &str, session_date: String, tags: String) -> Result<Value> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let now = now_ms();
        let id = match id {
            Some(id) => {
                tx.execute("UPDATE notes SET body=?2, session_date=?3, tags=?4, updated=?5, deleted=0 WHERE id=?1", params![id, body, session_date, tags, now])?;
                id
            }
            None => {
                let id = new_id();
                tx.execute("INSERT INTO notes(id,created,updated,session_date,tags,body) VALUES (?,?,?,?,?,?)", params![id, now, now, session_date, tags, body])?;
                id
            }
        };
        tx.execute("DELETE FROM note_edges WHERE note_id=?1", params![id])?;
        let edges = notes::parse(body);
        for e in &edges {
            tx.execute("INSERT INTO note_edges VALUES (?,?,?,?)", params![id, e.a, e.b, serde_json::to_value(e.rel)?.as_str()])?;
        }
        tx.commit()?;
        Ok(json!({"id": id, "relations": edges.len()}))
    }

    pub(crate) fn note_graph(&self) -> Result<Graph> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT e.a,e.b,e.rel FROM note_edges e JOIN notes n ON n.id=e.note_id WHERE n.deleted=0")?;
        let edges = stmt
            .query_map([], |r| {
                let rel: String = r.get(2)?;
                Ok(notes::Edge { a: r.get(0)?, b: r.get(1)?, rel: serde_json::from_value(Value::String(rel)).unwrap_or(notes::Rel::Similar) })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Graph::new(edges))
    }

    // ---- the GRE thesaurus --------------------------------------------------

    /// Synonyms of `key`, lowercased: `precise` from sense-level data
    /// (WordNet, Wiktionary) and `broad` from the Moby thesaurus.
    pub(crate) fn synonym_tiers(&self, key: &str) -> Result<(HashSet<String>, HashSet<String>)> {
        let mut precise = HashSet::new();
        for s in &self.sources {
            if let Some(e) = s.pack.lock().unwrap().get(key)? {
                for b in &e.blocks {
                    for w in b.synonyms.iter().chain(b.senses.iter().flat_map(|x| x.synonyms.iter())) {
                        precise.insert(lemma::key(w));
                    }
                }
            }
        }
        let mut broad = HashSet::new();
        if let Some(m) = &self.moby {
            broad.extend(m.lock().unwrap().get(key)?.into_iter().take(150).map(|w| lemma::key(&w)));
        }
        precise.remove(key);
        broad.remove(key);
        Ok((precise, broad))
    }

    /// Every synonym of either tier.
    pub(crate) fn synonym_set(&self, key: &str) -> Result<HashSet<String>> {
        let (p, b) = self.synonym_tiers(key)?;
        Ok(p.into_iter().chain(b).collect())
    }

    fn barrons_synonyms(&self) -> Arc<HashMap<String, (HashSet<String>, HashSet<String>)>> {
        if let Some(m) = self.barrons_syn.read().unwrap().clone() {
            return m;
        }
        let mut m = HashMap::new();
        if let Some(b) = self.barrons() {
            for k in b.entries.keys() {
                if let Ok(t) = self.synonym_tiers(k) {
                    m.insert(k.clone(), t);
                }
            }
        }
        let m = Arc::new(m);
        *self.barrons_syn.write().unwrap() = Some(m.clone());
        m
    }

    fn thesaurus(&self, key: &str) -> Result<Value> {
        let notes = self.note_graph()?.related(key);
        let (mine, mine_broad) = self.synonym_tiers(key)?;

        // GRE words from the book that relate to this word. A direct link in
        // sense-level data counts most; a link only in the broad Moby list
        // counts less; shared precise synonyms add a little. Overlap alone is
        // too weak a signal to list a word.
        let mut related: Vec<(usize, String)> = Vec::new();
        let barrons = self.barrons();
        if barrons.is_some() {
            for (bk, (theirs, theirs_broad)) in self.barrons_synonyms().iter() {
                if bk == key {
                    continue;
                }
                let direct = 6 * (mine.contains(bk) as usize) + 6 * (theirs.contains(key) as usize);
                let broad = 3 * (mine_broad.contains(bk) as usize) + 3 * (theirs_broad.contains(key) as usize);
                let shared = mine.intersection(theirs).count();
                let score = direct + broad + if shared >= 2 { shared } else { 0 };
                if score >= 3 {
                    related.push((score, bk.clone()));
                }
            }
            related.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
            related.truncate(10);
        }
        let barrons_related: Vec<Value> = related
            .into_iter()
            .filter_map(|(score, k)| {
                let e = barrons.as_ref()?.entries.get(&k)?.first()?;
                Some(json!({"word": e.word, "key": k, "unit": e.unit, "pos": e.pos, "definition": e.definition, "score": score}))
            })
            .collect();

        let mut general = json!({"synonyms": [], "antonyms": []});
        let mut syn: Vec<String> = Vec::new();
        let mut ant: Vec<String> = Vec::new();
        for s in &self.sources {
            if let Some(e) = s.pack.lock().unwrap().get(key)? {
                for b in &e.blocks {
                    for sense in &b.senses {
                        for w in &sense.synonyms {
                            if !syn.contains(w) {
                                syn.push(w.clone());
                            }
                        }
                        for w in &sense.antonyms {
                            if !ant.contains(w) {
                                ant.push(w.clone());
                            }
                        }
                    }
                    for w in &b.synonyms {
                        if !syn.contains(w) {
                            syn.push(w.clone());
                        }
                    }
                    for w in &b.antonyms {
                        if !ant.contains(w) {
                            ant.push(w.clone());
                        }
                    }
                }
            }
        }
        syn.truncate(24);
        ant.truncate(12);
        general["synonyms"] = json!(syn);
        general["antonyms"] = json!(ant);
        let moby: Vec<String> = match &self.moby {
            Some(m) => m.lock().unwrap().get(key)?.into_iter().take(30).collect(),
            None => Vec::new(),
        };
        Ok(json!({"notes": notes, "barrons": barrons_related, "general": general, "moby": moby}))
    }
}

/// Levenshtein distance, or `None` once it must exceed `max`.
fn bounded_levenshtein(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let v = (prev[j] + (ca != cb) as usize).min(prev[j + 1] + 1).min(cur[j] + 1);
            row_min = row_min.min(v);
            cur.push(v);
        }
        if row_min > max {
            return None;
        }
        prev = cur;
    }
    (prev[b.len()] <= max).then_some(prev[b.len()])
}


fn req_word(args: &Value) -> Result<String> {
    args.get("word").and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("missing argument `word`"))
}
