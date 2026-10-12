//! Merriam-Webster's official Dictionary API (non-commercial key, 1,000
//! queries a day per reference work).
//!
//! Hard rules from their terms: queries are made only when the person opens a
//! word in this source (no bulk or automated pre-fetching), at most two
//! reference works (Collegiate + Thesaurus) are used, and the key never
//! leaves this module (it is not logged and not sent back to the UI).

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use gredict_core::entry::{Block, Entry, Sense, Sound};
use regex::Regex;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::now_ms;

const DAILY_LIMIT: i64 = 1000;
/// Stop a little short of the limit so a retry or two never crosses it.
const DAILY_STOP: i64 = 950;

static MARKUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{/?[a-z_]+(?:\|[^}]*)?\}").unwrap());
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(?:sx|a_link|d_link|i_link|et_link|mat|dxt)\|([^|}]*)[^}]*\}").unwrap());

/// MW's inline markup ({it}, {bc}, {sx|word||}) to plain text.
pub fn plain(s: &str) -> String {
    let s = s.replace("{bc}", ": ").replace("{ldquo}", "\u{201C}").replace("{rdquo}", "\u{201D}");
    let s = LINK.replace_all(&s, "$1");
    let s = MARKUP.replace_all(&s, "");
    s.split_whitespace().collect::<Vec<_>>().join(" ").replace(" :", ":").trim_start_matches(": ").to_string()
}

fn audio_url(file: &str) -> String {
    // MW's rule for the sub-directory: "bix", "gg", numbers/punctuation, else the first letter.
    let sub = if file.starts_with("bix") {
        "bix".to_string()
    } else if file.starts_with("gg") {
        "gg".to_string()
    } else if file.chars().next().is_some_and(|c| !c.is_alphabetic()) {
        "number".to_string()
    } else {
        file[..1].to_string()
    };
    format!("https://media.merriam-webster.com/audio/prons/en/us/mp3/{sub}/{file}.mp3")
}

/// Collegiate JSON (one object per homograph) -> normalized entry. A response
/// that is an array of strings means "not found, did you mean...".
pub fn map_collegiate(word: &str, v: &Value) -> Result<Result<Entry, Vec<String>>> {
    let arr = v.as_array().ok_or_else(|| anyhow!("unexpected reply from Merriam-Webster"))?;
    if arr.iter().all(Value::is_string) {
        return Ok(Err(arr.iter().filter_map(Value::as_str).map(String::from).collect()));
    }
    let mut blocks = Vec::new();
    let mut syl: Option<String> = None;
    // MW also returns neighbours ("japery" brings "jape"); keep the entries for
    // this very word, else those that list it as an inflected form.
    let want = word.trim().to_lowercase();
    let objs: Vec<&Value> = arr.iter().filter(|e| e.is_object()).collect();
    let headed: Vec<&Value> = objs.iter().copied().filter(|e| e["hwi"]["hw"].as_str().is_some_and(|h| h.replace('*', "").to_lowercase() == want)).collect();
    let stemmed: Vec<&Value> = objs.iter().copied().filter(|e| e["meta"]["stems"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str().is_some_and(|x| x.to_lowercase() == want)))).collect();
    // A derived word ("japery") is often only a run-on of another entry ("jape"):
    // it keeps its own part of speech, syllables and pronunciation, and borrows the
    // parent's definitions.
    let run_on = if headed.is_empty() {
        objs.iter().copied().find_map(|e| {
            e["uros"].as_array()?.iter().find(|u| u["ure"].as_str().is_some_and(|x| x.replace('*', "").to_lowercase() == want)).map(|u| (e, u))
        })
    } else {
        None
    };
    if let Some((parent, uro)) = run_on {
        let senses: Vec<Sense> = parent["shortdef"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|d| Sense { gloss: plain(d), ..Default::default() }).collect();
        let sounds: Vec<Sound> = uro["prs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Sound { ipa: p["mw"].as_str().map(|m| format!("\\{m}\\")), audio: p["sound"]["audio"].as_str().map(audio_url), tags: vec![] })
            .collect();
        let ure = uro["ure"].as_str().unwrap_or("");
        let syl = ure.contains('*').then(|| ure.replace('*', "\u{b7}"));
        let block = Block {
            pos: uro["fl"].as_str().unwrap_or("").to_string(),
            head: parent["hwi"]["hw"].as_str().map(|h| h.replace('*', "")),
            sounds,
            senses,
            ..Default::default()
        };
        if !block.senses.is_empty() {
            return Ok(Ok(Entry { word: word.to_string(), syl, blocks: vec![block] }));
        }
    }
    let chosen = if !headed.is_empty() { headed } else if !stemmed.is_empty() { stemmed } else { objs };
    for e in chosen {
        let senses: Vec<Sense> = e["shortdef"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|d| Sense { gloss: plain(d), ..Default::default() }).collect();
        if senses.is_empty() {
            continue;
        }
        let sounds: Vec<Sound> = e["hwi"]["prs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Sound {
                // MW prints respellings between backslashes.
                ipa: p["mw"].as_str().map(|m| format!("\\{m}\\")),
                audio: p["sound"]["audio"].as_str().map(audio_url),
                tags: vec![],
            })
            .collect();
        let etymology = e["et"].as_array().into_iter().flatten().filter_map(|x| x.get(1).and_then(Value::as_str)).map(plain).collect::<Vec<_>>().join(" ");
        // MW marks syllables in the headword: "lo*qua*cious".
        if syl.is_none() {
            syl = e["hwi"]["hw"].as_str().filter(|h| h.contains('*')).map(|h| h.replace('*', "\u{b7}"));
        }
        blocks.push(Block {
            pos: e["fl"].as_str().unwrap_or("").to_string(),
            head: e["hwi"]["hw"].as_str().map(|h| h.replace('*', "")),
            etymology: (!etymology.is_empty()).then_some(etymology),
            sounds,
            senses,
            ..Default::default()
        });
    }
    if blocks.is_empty() {
        return Ok(Err(vec![]));
    }
    Ok(Ok(Entry { word: word.to_string(), syl, blocks }))
}

/// Adds the Thesaurus API's synonyms and antonyms to the blocks they belong to.
pub fn merge_thesaurus(entry: &mut Entry, v: &Value) {
    let Some(arr) = v.as_array() else { return };
    for t in arr.iter().filter(|t| t.is_object()) {
        let pos = t["fl"].as_str().unwrap_or("");
        let list = |k: &str| -> Vec<String> {
            t["meta"][k].as_array().into_iter().flatten().flat_map(|g| g.as_array().into_iter().flatten()).filter_map(Value::as_str).take(20).map(String::from).collect()
        };
        if let Some(b) = entry.blocks.iter_mut().find(|b| b.pos == pos) {
            b.synonyms = list("syns");
            b.antonyms = list("ants");
        }
    }
}

/// Keys shipped with the app (the owner chose to publish them), so every device
/// starts with Merriam-Webster already set up.
const DEFAULT_KEYS: &str = include_str!("../defaults/mw.json");

impl Api {
    /// Fills in the shipped keys on a device that has none, unless the person
    /// removed them on purpose. The app shells call this; tests do not.
    pub fn seed_defaults(&self) {
        if self.mw_key("collegiate").is_some() || self.setting("mw.removed").as_deref() == Some("1") {
            return;
        }
        if let Ok(v) = serde_json::from_str::<Value>(DEFAULT_KEYS) {
            for k in ["collegiate", "thesaurus"] {
                if let Some(key) = v[k].as_str().filter(|k| !k.is_empty()) {
                    let _ = self.set_setting(&format!("mw.{k}"), key);
                }
            }
        }
    }

    fn mw_key(&self, which: &str) -> Option<String> {
        self.setting(&format!("mw.{which}")).filter(|k| !k.trim().is_empty())
    }

    pub(crate) fn mw_configured(&self) -> bool {
        self.mw_key("collegiate").is_some()
    }

    fn mw_today(&self) -> String {
        format!("mw.q.{}", now_ms() / 86_400_000)
    }

    fn mw_used(&self) -> i64 {
        self.setting(&self.mw_today()).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    pub(crate) fn mw_status(&self) -> Result<Value> {
        Ok(json!({
            "configured": self.mw_configured(),
            "thesaurus": self.mw_key("thesaurus").is_some(),
            "used_today": self.mw_used(), "limit": DAILY_LIMIT,
            "cache_entries": self.mw_cache().lock().unwrap().query_row("SELECT count(*) FROM mw", [], |r| r.get::<_, i64>(0)).unwrap_or(0),
            "keep_offline": self.setting("mw.keep").as_deref() != Some("0"),
        }))
    }

    /// Saves the keys (empty clears one) and the offline-cache choice.
    pub(crate) fn mw_config(&self, args: &Value) -> Result<Value> {
        for k in ["collegiate", "thesaurus"] {
            if let Some(v) = args.get(k).and_then(Value::as_str) {
                if v.trim().is_empty() {
                    self.db.lock().unwrap().execute("DELETE FROM settings WHERE k=?", [format!("mw.{k}")])?;
                    // Removed on purpose: do not bring the shipped keys back at the next start.
                    self.set_setting("mw.removed", "1")?;
                } else {
                    self.set_setting(&format!("mw.{k}"), v.trim())?;
                    self.db.lock().unwrap().execute("DELETE FROM settings WHERE k='mw.removed'", [])?;
                }
            }
        }
        if let Some(keep) = args.get("keep_offline").and_then(Value::as_bool) {
            self.set_setting("mw.keep", if keep { "1" } else { "0" })?;
        }
        if args.get("clear_cache").and_then(Value::as_bool) == Some(true) {
            self.mw_cache().lock().unwrap().execute("DELETE FROM mw", [])?;
        }
        self.mw_status()
    }

    fn mw_cache(&self) -> &std::sync::Mutex<Connection> {
        &self.cache
    }

    fn mw_cached(&self, kind: &str, key: &str) -> Option<Value> {
        let j: String = self.mw_cache().lock().unwrap().query_row("SELECT json FROM mw WHERE kind=?1 AND key=?2", params![kind, key], |r| r.get(0)).ok()?;
        serde_json::from_str(&j).ok()
    }

    fn mw_request(&self, kind: &str, word: &str) -> Result<Value> {
        let key = self.mw_key(if kind == "collegiate" { "collegiate" } else { "thesaurus" }).ok_or_else(|| anyhow!("no Merriam-Webster {kind} key is set"))?;
        if self.mw_used() >= DAILY_STOP {
            bail!("today's Merriam-Webster allowance ({DAILY_LIMIT} lookups) is nearly used up; try again tomorrow");
        }
        let base = std::env::var("LEXIS_MW_BASE").unwrap_or_else(|_| "https://www.dictionaryapi.com/api/v3/references".into());
        let agent: ureq::Agent = ureq::Agent::config_builder().user_agent("LadyBird/0.1 (personal study app)").timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false).build().into();
        let mut resp = agent
            .get(&format!("{base}/{kind}/json/{}", urlencode(word)))
            .query("key", &key)
            .call()
            // Never include the URL: it contains the key.
            .map_err(|_| anyhow!("Merriam-Webster is not reachable right now"))?;
        let used = self.mw_used() + 1;
        self.set_setting(&self.mw_today(), &used.to_string())?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().with_config().limit(4 << 20).read_to_string().map_err(|_| anyhow!("unreadable reply from Merriam-Webster"))?;
        if status != 200 {
            bail!("Merriam-Webster refused the request (status {status}); check the key in Settings");
        }
        // An invalid key comes back as a plain-text message, not JSON.
        serde_json::from_str(&text).map_err(|_| anyhow!("Merriam-Webster did not accept the key: {}", text.chars().take(80).collect::<String>()))
    }

    /// The entry for `word` from Merriam-Webster: the saved copy if there is
    /// one, otherwise one live lookup (which is then saved when allowed).
    pub(crate) fn mw_entry(&self, word: &str) -> Result<Result<Entry, Vec<String>>> {
        let k = gredict_core::lemma::key(word);
        let keep = self.setting("mw.keep").as_deref() != Some("0");
        let (coll, thes) = match self.mw_cached("collegiate", &k) {
            Some(c) => (c, self.mw_cached("thesaurus", &k)),
            None => {
                let c = self.mw_request("collegiate", &k)?;
                let t = if self.mw_key("thesaurus").is_some() { self.mw_request("thesaurus", &k).ok() } else { None };
                if keep {
                    let db = self.mw_cache().lock().unwrap();
                    db.execute("INSERT OR REPLACE INTO mw(kind,key,json,fetched) VALUES ('collegiate',?1,?2,?3)", params![k, c.to_string(), now_ms()])?;
                    if let Some(t) = &t {
                        db.execute("INSERT OR REPLACE INTO mw(kind,key,json,fetched) VALUES ('thesaurus',?1,?2,?3)", params![k, t.to_string(), now_ms()])?;
                    }
                }
                (c, t)
            }
        };
        let mut mapped = map_collegiate(word, &coll)?;
        if let (Ok(e), Some(t)) = (&mut mapped, &thes) {
            merge_thesaurus(e, t);
        }
        Ok(mapped)
    }
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub(crate) const CACHE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS mw(kind TEXT NOT NULL, key TEXT NOT NULL, json TEXT NOT NULL, fetched INTEGER NOT NULL, PRIMARY KEY(kind, key)); CREATE TABLE IF NOT EXISTS expand(term TEXT PRIMARY KEY, words TEXT NOT NULL, at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS session_vec(session_id TEXT PRIMARY KEY, hash TEXT NOT NULL, vec BLOB NOT NULL)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_on_word_gets_its_own_pronunciation_and_parts() {
        let v = json!([{"hwi": {"hw": "jape", "prs": [{"mw": "ˈjāp"}]}, "fl": "verb", "shortdef": ["to joke"],
            "uros": [{"ure": "jap*er", "fl": "noun", "prs": [{"mw": "ˈjā-pər"}]}, {"ure": "jap*ery", "fl": "noun", "prs": [{"mw": "ˈjā-p(ə-)rē"}]}]}]);
        let e = map_collegiate("japery", &v).unwrap().unwrap();
        assert_eq!(e.syl.as_deref(), Some("jap·ery"));
        assert_eq!(e.blocks[0].pos, "noun");
        assert_eq!(e.blocks[0].head.as_deref(), Some("jape"));
        assert_eq!(e.blocks[0].sounds[0].ipa.as_deref(), Some("\\ˈjā-p(ə-)rē\\"));
    }

    #[test]
    fn neighbours_are_dropped() {
        let v = json!([
            {"hwi": {"hw": "jape", "prs": [{"mw": "ˈjāp"}]}, "fl": "verb", "shortdef": ["to joke"]},
            {"hwi": {"hw": "jap*ery"}, "fl": "noun", "shortdef": ["jesting talk"]}
        ]);
        let e = map_collegiate("japery", &v).unwrap().unwrap();
        assert_eq!(e.blocks.len(), 1);
        assert_eq!(e.blocks[0].pos, "noun");
        assert!(e.blocks[0].sounds.is_empty());
    }

    #[test]
    fn strips_markup() {
        assert_eq!(plain("{bc}given to {it}continual{/it} talking"), "given to continual talking");
        assert_eq!(plain("a {sx|talkative||} person: {a_link|garrulous}"), "a talkative person: garrulous");
    }

    #[test]
    fn audio_subdirectory_rules() {
        assert!(audio_url("loquac01").ends_with("/mp3/l/loquac01.mp3"));
        assert!(audio_url("bixby001").ends_with("/mp3/bix/bixby001.mp3"));
        assert!(audio_url("gg000001").ends_with("/mp3/gg/gg000001.mp3"));
        assert!(audio_url("1abc").ends_with("/mp3/number/1abc.mp3"));
    }

    #[test]
    fn maps_entries_and_suggestions() {
        let v = json!([{"hwi": {"hw": "lo*qua*cious", "prs": [{"mw": "lō-ˈkwā-shəs", "sound": {"audio": "loquac01"}}]}, "fl": "adjective",
                        "et": [["text", "Latin {it}loquac-{/it}"]], "shortdef": ["given to continual talking : talkative"]}]);
        let e = map_collegiate("loquacious", &v).unwrap().unwrap();
        assert_eq!(e.blocks[0].head.as_deref(), Some("loquacious"));
        assert_eq!(e.blocks[0].pos, "adjective");
        assert_eq!(e.blocks[0].sounds[0].ipa.as_deref(), Some("\\lō-ˈkwā-shəs\\"));
        assert!(e.blocks[0].sounds[0].audio.as_deref().unwrap().contains("/l/loquac01"));
        assert_eq!(e.blocks[0].etymology.as_deref(), Some("Latin loquac-"));
        let miss = map_collegiate("loquatious", &json!(["loquacious", "loquaciously"])).unwrap();
        assert_eq!(miss.unwrap_err(), vec!["loquacious", "loquaciously"]);
    }
}
