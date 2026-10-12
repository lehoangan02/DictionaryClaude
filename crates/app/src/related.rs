//! "Words close in meaning to this one", for finding a quiz session from a looser word than the
//! one it contains ("boring" -> a session on *mundane*). Gemini on the sync server answers; every
//! answer is cached on the device that asked, so each word costs one request ever.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::api::Api;

const ASK: &str = "List 25 English words or short phrases closest in meaning to the word \"{W}\" that a student preparing for the GRE might meet \
(synonyms, near-synonyms and strongly related adjectives or nouns, in the same part of speech where possible). \
Output only the words, lowercase, comma-separated, with no numbering and no explanation.";

/// Server side: at most this many fresh (uncached) questions to Gemini per hour.
static BUDGET: std::sync::Mutex<(i64, u32)> = std::sync::Mutex::new((0, 0));

fn clean_term(t: &str) -> Result<String> {
    let t = t.trim().to_lowercase();
    if t.is_empty() || t.chars().count() > 40 || !t.chars().all(|c| c.is_alphabetic() || matches!(c, ' ' | '-' | '\'')) {
        bail!("not a word");
    }
    Ok(t)
}

fn parse_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(|c| c == ',' || c == '\n')
        .map(|w| w.trim().trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
        .filter(|w| !w.is_empty() && w.chars().count() <= 30)
        .collect();
    out.dedup();
    out.truncate(40);
    out
}

impl Api {
    fn expand_cached(&self, term: &str) -> Option<Vec<String>> {
        if let Some(v) = self.ref_get("expand", term) {
            return serde_json::from_value(v).ok();
        }
        // Answers saved by the first build (cache.sqlite) move into the migrated table on first use.
        let j: String = self.cache.lock().unwrap().query_row("SELECT words FROM expand WHERE term=?", [term], |r| r.get(0)).ok()?;
        let words: Vec<String> = serde_json::from_str(&j).ok()?;
        let _ = self.ref_put("expand", term, &json!(words));
        Some(words)
    }

    fn expand_store(&self, term: &str, words: &[String]) {
        let _ = self.ref_put("expand", term, &json!(words));
    }

    /// Where Gemini is reachable (the sync server has the key): ask it. Cached on this machine.
    pub(crate) fn expand_local(&self, term: &str) -> Result<Vec<String>> {
        let term = clean_term(term)?;
        if let Some(w) = self.expand_cached(&term) {
            return Ok(w);
        }
        {
            let mut b = BUDGET.lock().unwrap();
            let hour = crate::userdb::now_ms() / 3_600_000;
            if b.0 != hour {
                *b = (hour, 0);
            }
            if b.1 >= 120 {
                bail!("too many new words this hour");
            }
            b.1 += 1;
        }
        let words = parse_words(&crate::jobs::gemini_text(&ASK.replace("{W}", &term))?);
        if words.is_empty() {
            bail!("no related words came back");
        }
        self.expand_store(&term, &words);
        Ok(words)
    }

    /// Related words for a search term: the cache, else Gemini directly (on the server) or through
    /// the sync server (on a device). Offline or unpaired it quietly returns nothing.
    pub(crate) fn related_words(&self, term: &str) -> Vec<String> {
        let Ok(term) = clean_term(term) else { return vec![] };
        if let Some(w) = self.expand_cached(&term) {
            return w;
        }
        if crate::jobs::gemini_key().is_some() {
            return self.expand_local(&term).unwrap_or_default();
        }
        match self.expand_via_server(&term) {
            Ok(w) => {
                self.expand_store(&term, &w);
                w
            }
            Err(_) => vec![],
        }
    }

    fn expand_via_server(&self, term: &str) -> Result<Vec<String>> {
        let p = self.pairing().ok_or_else(|| anyhow!("not paired"))?;
        let (route, ..) = self.choose_route(&p);
        let host = match route {
            Some(crate::net::Route::Lan) => p.lan.clone(),
            Some(crate::net::Route::Wan) => p.wan.clone(),
            None => bail!("the sync server is not reachable"),
        }
        .ok_or_else(|| anyhow!("no address"))?;
        let a = crate::net::agent(&p.cert, Duration::from_secs(45))?;
        let v = crate::net::post(&a, &host, &p.token, "/v1/expand", &json!({"term": term}))?;
        Ok(v["words"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_cleaned_and_odd_terms_refused() {
        assert_eq!(parse_words("Tedious, dull,\n  mundane. , banal"), vec!["tedious", "dull", "mundane", "banal"]);
        assert!(clean_term("boring").is_ok());
        assert!(clean_term("drop table; --").is_err());
        assert!(clean_term("").is_err());
    }
}

impl Api {
    /// A kept answer from an outside service (see migration 5 of the user database).
    pub(crate) fn ref_get(&self, kind: &str, key: &str) -> Option<Value> {
        let j: String = self.db.lock().unwrap().query_row("SELECT json FROM ref_cache WHERE kind=?1 AND key=?2", [kind, key], |r| r.get(0)).ok()?;
        serde_json::from_str(&j).ok()
    }

    pub(crate) fn ref_put(&self, kind: &str, key: &str, v: &Value) -> Result<()> {
        self.db.lock().unwrap().execute("INSERT OR REPLACE INTO ref_cache(kind,key,json,fetched) VALUES (?1,?2,?3,?4)", rusqlite::params![kind, key, v.to_string(), crate::userdb::now_ms()])?;
        Ok(())
    }

    pub(crate) fn ref_count(&self, kind: &str) -> i64 {
        self.db.lock().unwrap().query_row("SELECT count(*) FROM ref_cache WHERE kind=?", [kind], |r| r.get(0)).unwrap_or(0)
    }
}
