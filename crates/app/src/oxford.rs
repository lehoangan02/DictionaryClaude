//! Oxford Languages' official Dictionary API (the owner's own app id and key).
//!
//! Same discipline as Merriam-Webster: nothing is shipped, the id and key are typed into Settings
//! and stay on this device (never logged, never sent to the UI again), a word is looked up only
//! when the person opens it in the Oxford tab (no bulk or background fetching), and a day's
//! counter stops lookups well short of any plan limit. Saved entries are kept on the device only
//! when the person leaves "keep words I look up" on.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use gredict_core::entry::{Block, Entry, Example, Sense, Sound};
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::now_ms;

/// A cautious daily stop; the free plans of this API are small (the owner can read theirs on the developer portal).
const DAILY_STOP: i64 = 40;

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn labels(s: &Value) -> Vec<String> {
    ["domains", "registers", "regions"].iter().flat_map(|k| s[*k].as_array().into_iter().flatten().filter_map(|x| text(&x["text"]))).collect()
}

fn words(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| text(&x["text"])).collect()
}

fn sense(s: &Value, parents: &[String]) -> Option<Sense> {
    // Some senses only point at another word ("variant of ..."): keep their short text when there is one.
    let gloss = s["definitions"].as_array().and_then(|d| d.first()).and_then(text).or_else(|| s["shortDefinitions"].as_array().and_then(|d| d.first()).and_then(text))?;
    Some(Sense {
        gloss,
        parents: parents.to_vec(),
        tags: labels(s),
        examples: s["examples"].as_array().into_iter().flatten().filter_map(|e| text(&e["text"])).map(|t| Example { text: t, by: None }).collect(),
        synonyms: words(&s["synonyms"]),
        antonyms: words(&s["antonyms"]),
        form_of: None,
    })
}

/// The API's JSON for one word -> the normalized entry. `Err(vec![])` means "no such word".
pub fn map_entries(word: &str, v: &Value) -> Result<Result<Entry, Vec<String>>> {
    let results = v["results"].as_array().ok_or_else(|| anyhow!("unexpected reply from Oxford"))?;
    let mut blocks = Vec::new();
    for r in results {
        for le in r["lexicalEntries"].as_array().into_iter().flatten() {
            let pos = le["lexicalCategory"]["text"].as_str().unwrap_or("").to_lowercase();
            let mut block = Block { pos, ..Default::default() };
            for e in le["entries"].as_array().into_iter().flatten() {
                for p in e["pronunciations"].as_array().into_iter().flatten() {
                    let dialects: Vec<String> = p["dialects"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|d| if d.contains("American") { "US".to_string() } else if d.contains("British") { "UK".to_string() } else { d.to_string() }).collect();
                    let ipa = text(&p["phoneticSpelling"]);
                    let audio = text(&p["audioFile"]);
                    if ipa.is_some() || audio.is_some() {
                        block.sounds.push(Sound { ipa, audio, tags: dialects });
                    }
                }
                if block.etymology.is_none() {
                    block.etymology = e["etymologies"].as_array().and_then(|a| a.first()).and_then(text);
                }
                for s in e["senses"].as_array().into_iter().flatten() {
                    let Some(top) = sense(s, &[]) else { continue };
                    let parent = top.gloss.clone();
                    block.senses.push(top);
                    for sub in s["subsenses"].as_array().into_iter().flatten() {
                        if let Some(x) = sense(sub, std::slice::from_ref(&parent)) {
                            block.senses.push(x);
                        }
                    }
                }
            }
            // Block-level lists: the words the senses name, once each.
            for s in &block.senses {
                for w in &s.synonyms {
                    if !block.synonyms.contains(w) {
                        block.synonyms.push(w.clone());
                    }
                }
                for w in &s.antonyms {
                    if !block.antonyms.contains(w) {
                        block.antonyms.push(w.clone());
                    }
                }
            }
            if !block.senses.is_empty() {
                blocks.push(block);
            }
        }
    }
    if blocks.is_empty() {
        return Ok(Err(vec![]));
    }
    Ok(Ok(Entry { word: word.trim().to_string(), syl: None, blocks }))
}

impl Api {
    /// The id and key: this machine's own (the sync server's `oxford.cred`, an environment
    /// variable, or Settings on a Mac used on its own).
    fn ox_creds(&self) -> Option<(String, String)> {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let (id, key) = match (env("OXFORD_APP_ID"), env("OXFORD_APP_KEY")) {
            (Some(i), Some(k)) => (i, k),
            _ => (self.setting("ox.id")?, self.setting("ox.key")?),
        };
        (!id.trim().is_empty() && !key.trim().is_empty()).then(|| (id.trim().to_string(), key.trim().to_string()))
    }

    /// Oxford is offered when this machine holds a key, or is paired with a server that may.
    pub(crate) fn ox_configured(&self) -> bool {
        self.ox_creds().is_some() || self.pairing().is_some()
    }

    fn ox_today(&self) -> String {
        format!("ox.q.{}", now_ms() / 86_400_000)
    }

    fn ox_used(&self) -> i64 {
        self.setting(&self.ox_today()).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    pub(crate) fn ox_status(&self) -> Result<Value> {
        Ok(json!({
            "configured": self.ox_configured(), "own_key": self.ox_creds().is_some(), "via_server": self.ox_creds().is_none() && self.pairing().is_some(),
            "used_today": self.ox_used(), "limit": DAILY_STOP, "cache_entries": self.ref_count("oxford"),
        }))
    }

    /// Saves this machine's own id and key (both empty clears them); `clear_cache` forgets saved entries.
    pub(crate) fn ox_config(&self, args: &Value) -> Result<Value> {
        let (id, key) = (args.get("app_id").and_then(Value::as_str), args.get("app_key").and_then(Value::as_str));
        if let (Some(i), Some(k)) = (id, key) {
            if i.trim().is_empty() && k.trim().is_empty() {
                self.db.lock().unwrap().execute("DELETE FROM settings WHERE k IN ('ox.id','ox.key')", [])?;
            } else if !i.trim().is_empty() && !k.trim().is_empty() {
                self.set_setting("ox.id", i.trim())?;
                self.set_setting("ox.key", k.trim())?;
            } else {
                bail!("Oxford needs both the application id and the key");
            }
        }
        if args.get("clear_cache").and_then(Value::as_bool) == Some(true) {
            self.db.lock().unwrap().execute("DELETE FROM ref_cache WHERE kind='oxford'", [])?;
        }
        self.ox_status()
    }

    fn ox_request(&self, word: &str) -> Result<Option<Value>> {
        let (id, key) = self.ox_creds().ok_or_else(|| anyhow!("no Oxford key is set"))?;
        if self.ox_used() >= DAILY_STOP {
            bail!("today's Oxford allowance ({DAILY_STOP} lookups here) is used up; try again tomorrow");
        }
        let base = std::env::var("LEXIS_OXFORD_BASE").unwrap_or_else(|_| "https://od-api.oxforddictionaries.com/api/v2".into());
        let agent: ureq::Agent = ureq::Agent::config_builder().user_agent("LadyBird (personal study app)").timeout_global(Some(Duration::from_secs(12))).http_status_as_error(false).build().into();
        let mut resp = agent
            .get(&format!("{base}/entries/en-gb/{}", urlencode(word)))
            .query("strictMatch", "false")
            .header("app_id", &id)
            .header("app_key", &key)
            .call()
            // Never include request details: they carry the credentials.
            .map_err(|_| anyhow!("Oxford is not reachable right now"))?;
        self.set_setting(&self.ox_today(), &(self.ox_used() + 1).to_string())?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().with_config().limit(4 << 20).read_to_string().map_err(|_| anyhow!("unreadable reply from Oxford"))?;
        match status {
            200 => Ok(Some(serde_json::from_str(&body).map_err(|_| anyhow!("unreadable reply from Oxford"))?)),
            404 => Ok(None),
            401 | 403 => bail!("Oxford refused the id or key (status {status}); check them"),
            429 => bail!("Oxford says the plan's request limit is reached"),
            _ => bail!("Oxford answered with status {status}"),
        }
    }

    /// Server side of a lookup: the kept copy, else one live request that is then kept for good
    /// (a missing word is kept too, so it is never asked twice).
    pub(crate) fn ox_raw(&self, word: &str) -> Result<Value> {
        let k = gredict_core::lemma::key(word);
        if let Some(v) = self.ref_get("oxford", &k) {
            return Ok(v);
        }
        let v = match self.ox_request(&k)? {
            Some(v) => v,
            None => json!({"results": []}),
        };
        self.ref_put("oxford", &k, &v)?;
        Ok(v)
    }

    /// The entry for `word`: this machine's kept copy; else its own key; else the sync server
    /// (which keeps every answer, so each word is paid for once across all devices).
    pub(crate) fn ox_entry(&self, word: &str) -> Result<Result<Entry, Vec<String>>> {
        let v = if self.ox_creds().is_some() || self.ref_get("oxford", &gredict_core::lemma::key(word)).is_some() {
            self.ox_raw(word)?
        } else {
            let p = self.pairing().ok_or_else(|| anyhow!("no Oxford key is set and this device is not paired with the server"))?;
            let (route, ..) = self.choose_route(&p);
            let host = match route {
                Some(crate::net::Route::Lan) => p.lan.clone(),
                Some(crate::net::Route::Wan) => p.wan.clone(),
                None => bail!("the sync server is not reachable, and this word is not saved on this device yet"),
            }
            .ok_or_else(|| anyhow!("no server address"))?;
            let a = crate::net::agent(&p.cert, Duration::from_secs(30))?;
            let v = crate::net::post(&a, &host, &p.token, "/v1/oxford", &json!({"word": word}))?;
            let v = v.get("json").cloned().ok_or_else(|| anyhow!("the server sent no Oxford entry"))?;
            self.ref_put("oxford", &gredict_core::lemma::key(word), &v)?;
            v
        };
        map_entries(word, &v)
    }
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({"results": [{"lexicalEntries": [{
            "lexicalCategory": {"id": "adjective", "text": "Adjective"},
            "entries": [{
                "etymologies": ["late Middle English from Latin"],
                "pronunciations": [{"phoneticSpelling": "ˈmʌndeɪn", "audioFile": "https://example.test/mundane.mp3", "dialects": ["British English"]}],
                "senses": [{
                    "definitions": ["lacking interest or excitement; dull"],
                    "examples": [{"text": "the mundane routine of the office"}],
                    "registers": [{"text": "Formal"}],
                    "synonyms": [{"text": "humdrum"}, {"text": "tedious"}],
                    "subsenses": [{"definitions": ["of this world rather than a spiritual one"], "domains": [{"text": "Philosophy"}]}]
                }, {"shortDefinitions": ["see other"]}]
            }]
        }]}]})
    }

    #[test]
    fn maps_an_entry_with_senses_sounds_and_labels() {
        let e = map_entries("mundane", &sample()).unwrap().unwrap();
        let b = &e.blocks[0];
        assert_eq!(b.pos, "adjective");
        assert_eq!(b.etymology.as_deref(), Some("late Middle English from Latin"));
        assert_eq!(b.sounds[0].tags, vec!["UK"]);
        assert_eq!(b.senses.len(), 3);
        assert_eq!(b.senses[0].tags, vec!["Formal"]);
        assert_eq!(b.senses[0].examples[0].text, "the mundane routine of the office");
        assert_eq!(b.senses[1].parents, vec!["lacking interest or excitement; dull"]);
        assert_eq!(b.senses[1].tags, vec!["Philosophy"]);
        assert_eq!(b.synonyms, vec!["humdrum", "tedious"]);
    }

    #[test]
    fn an_empty_reply_means_no_such_word() {
        assert!(map_entries("zzz", &json!({"results": []})).unwrap().is_err());
        assert!(map_entries("zzz", &json!({"nope": 1})).is_err());
    }
}
