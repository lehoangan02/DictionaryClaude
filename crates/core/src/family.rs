//! Word families: the words built from a word, built on it, and spelled from
//! the same stem, read from `family.pack`.
//!
//! The pack holds, for each word, what Wiktionary says about it (the word an
//! etymology note builds it from, "loquacious = loquacity + -ous"; its derived
//! and related terms) and, as a weaker hint, the words that share its stem by
//! spelling ("malefaction", "malefactor", "malefactress"). The spelling hints
//! are guesses, so they are kept apart from what Wiktionary states.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::lemma;

pub const SCHEMA: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE fam(key TEXT PRIMARY KEY, blob BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE stem(id INTEGER PRIMARY KEY, blob BLOB NOT NULL);
";

/// What the pack stores for one word. Pairs are (word, what was added to build it: "-ous", "un-";
/// empty when Wiktionary only lists the word among the derived terms).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Links {
    /// The words this one is built from.
    #[serde(default, rename = "u", skip_serializing_if = "Vec::is_empty")]
    pub up: Vec<(String, String)>,
    /// The words built on this one.
    #[serde(default, rename = "d", skip_serializing_if = "Vec::is_empty")]
    pub down: Vec<(String, String)>,
    /// Words Wiktionary lists as related.
    #[serde(default, rename = "r", skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    /// The group of words that share a stem.
    #[serde(default, rename = "s", skip_serializing_if = "Option::is_none")]
    pub stem: Option<u32>,
}

pub struct FamilyPack {
    conn: Connection,
    dict: Option<zstd::dict::DecoderDictionary<'static>>,
}

impl FamilyPack {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let dict: Option<Vec<u8>> = conn.query_row("SELECT v FROM meta WHERE k='zdict'", [], |r| r.get(0)).optional()?;
        Ok(FamilyPack { conn, dict: dict.map(|d| zstd::dict::DecoderDictionary::copy(&d)) })
    }

    fn unpack(&self, blob: &[u8]) -> Result<Vec<u8>> {
        Ok(match &self.dict {
            Some(d) => zstd::bulk::Decompressor::with_prepared_dictionary(d)?.decompress(blob, 4 << 20)?,
            None => zstd::bulk::decompress(blob, 4 << 20)?,
        })
    }

    /// `key` must already be a lemma key.
    pub fn links(&self, key: &str) -> Result<Option<Links>> {
        let blob: Option<Vec<u8>> = self.conn.query_row("SELECT blob FROM fam WHERE key=?", [key], |r| r.get(0)).optional()?;
        blob.map(|b| Ok(serde_json::from_slice(&self.unpack(&b)?)?)).transpose()
    }

    /// The words of one stem group.
    pub fn stem_mates(&self, id: u32) -> Result<Vec<String>> {
        let blob: Option<Vec<u8>> = self.conn.query_row("SELECT blob FROM stem WHERE id=?", [id], |r| r.get(0)).optional()?;
        blob.map(|b| Ok(serde_json::from_slice(&zstd::bulk::decompress(&b, 4 << 20)?)?)).transpose().map(Option::unwrap_or_default)
    }
}

/// A word of the family tree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Member {
    pub word: String,
    /// 0 for a root word, 1 for a word built on a root, and so on.
    pub depth: usize,
    /// The word this one is built on (none for a root).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// What was added to `from` ("-ous"); empty when Wiktionary only lists it as a derived term.
    pub via: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Family {
    /// The tree of words built from one another, roots first, each followed by what is built on it.
    pub members: Vec<Member>,
    /// Words Wiktionary lists as related but that are not in the tree.
    pub related: Vec<String>,
    /// Words that share the stem by spelling only (guesses), the closest in spelling first.
    pub stem: Vec<String>,
}

const CLIMB: usize = 6;
const DEPTH: usize = 3;
const PER_WORD: usize = 24;
const TREE: usize = 60;
const RELATED: usize = 24;
const STEM: usize = 30;

struct Walk<'a> {
    links: &'a dyn Fn(&str) -> Result<Option<Links>>,
    /// The chain from the typed word up to its roots: explored first, so a cap never cuts the typed word off.
    path: HashSet<String>,
    placed: HashSet<String>,
    members: Vec<Member>,
}

fn shared_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

fn kids_first(path: &HashSet<String>, w: &str, total: usize) -> bool {
    total > PER_WORD && !path.contains(&lemma::key(w))
}

impl Walk<'_> {
    fn place(&mut self, word: &str, depth: usize, from: Option<&str>, via: &str) -> Result<()> {
        let key = lemma::key(word);
        if (self.members.len() >= TREE && !self.path.contains(&key)) || !self.placed.insert(key.clone()) {
            return Ok(());
        }
        self.members.push(Member { word: word.to_string(), depth, from: from.map(String::from), via: via.to_string() });
        if depth >= DEPTH && !self.path.contains(&key) {
            return Ok(());
        }
        let Some(l) = (self.links)(&key)? else { return Ok(()) };
        // Past the cap, the way to the typed word is kept and the rest is cut.
        let mut kids: Vec<&(String, String)> = l.down.iter().collect();
        kids.sort_by_key(|(w, _)| kids_first(&self.path, w, l.down.len()));
        for (w, v) in kids.into_iter().take(PER_WORD.max(self.path.len())) {
            self.place(w, depth + 1, Some(word), v)?;
        }
        Ok(())
    }
}

/// The family of `word`: climb to the words it is built from, then list everything built on them.
pub fn family(word: &str, links: &dyn Fn(&str) -> Result<Option<Links>>, stem: &dyn Fn(u32) -> Result<Vec<String>>) -> Result<Family> {
    let key = lemma::key(word);
    let mut seen: HashSet<String> = HashSet::from([key.clone()]);
    let mut path = seen.clone();
    let mut roots: Vec<String> = Vec::new();
    let mut level = vec![word.to_string()];
    for step in 0..CLIMB {
        let mut next = Vec::new();
        for w in &level {
            let ups = links(&lemma::key(w))?.map(|l| l.up).unwrap_or_default();
            // A word built by adding an affix climbs to its base; one that is only listed among a word's derived terms
            // takes that word as its root, but a root's own listings (compounds) never extend the climb.
            let built: Vec<&String> = ups.iter().filter(|(_, via)| !via.is_empty()).map(|(b, _)| b).collect();
            // (A word that merely contains another, like "go-go" for "go", is not that word's root.)
            let listed = |b: &&String| b.len() <= w.len() && shared_prefix(b, w) >= 4;
            let parents: Vec<&String> = if !built.is_empty() { built } else if step == 0 { ups.iter().map(|(b, _)| b).filter(listed).take(2).collect() } else { Vec::new() };
            let fresh: Vec<&String> = parents.into_iter().filter(|b| seen.insert(lemma::key(b))).collect();
            if fresh.is_empty() {
                if !roots.contains(w) {
                    roots.push(w.clone());
                }
            }
            for b in fresh {
                path.insert(lemma::key(b));
                next.push(b.clone());
            }
        }
        level = next;
        if level.is_empty() {
            break;
        }
    }
    for w in level {
        if !roots.contains(&w) {
            roots.push(w);
        }
    }
    roots.truncate(3);

    let mut walk = Walk { links, path, placed: HashSet::new(), members: Vec::new() };
    for r in &roots {
        walk.place(r, 0, None, "")?;
    }
    let Walk { placed, members, .. } = walk;

    // What Wiktionary calls related, from the word and its roots.
    let mut related: Vec<String> = Vec::new();
    let mut stems: Vec<u32> = Vec::new();
    for w in std::iter::once(word).chain(roots.iter().map(String::as_str)) {
        if let Some(l) = links(&lemma::key(w))? {
            for r in l.related {
                let k = lemma::key(&r);
                if !placed.contains(&k) && k != key && !related.iter().any(|x| lemma::key(x) == k) {
                    related.push(r);
                }
            }
            stems.extend(l.stem.filter(|s| !stems.contains(s)));
        }
    }

    let mut guess: Vec<String> = Vec::new();
    for id in stems {
        for w in stem(id)? {
            let k = lemma::key(&w);
            if !placed.contains(&k) && k != key && !guess.iter().any(|x| lemma::key(x) == k) && !related.iter().any(|x| lemma::key(x) == k) {
                guess.push(w);
            }
        }
    }
    let shared = |w: &String| shared_prefix(w, &key);
    related.sort_by_key(|w| std::cmp::Reverse(shared(w)));
    related.truncate(RELATED);
    guess.sort_by(|a, b| shared(b).cmp(&shared(a)).then(a.len().abs_diff(key.len()).cmp(&b.len().abs_diff(key.len()))).then(a.cmp(b)));
    guess.truncate(STEM);

    Ok(Family { members, related, stem: guess })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pair(w: &str, v: &str) -> (String, String) {
        (w.to_string(), v.to_string())
    }

    fn graph() -> HashMap<String, Links> {
        let mut g = HashMap::new();
        g.insert("loquacity".into(), Links { down: vec![pair("loquacious", "-ous")], related: vec!["loquacious".into(), "loquor".into()], ..Default::default() });
        g.insert("loquacious".into(), Links { up: vec![pair("loquacity", "-ous")], down: vec![pair("loquaciously", "-ly"), pair("loquaciousness", "-ness")], stem: Some(1), ..Default::default() });
        g.insert("loquaciously".into(), Links { up: vec![pair("loquacious", "-ly")], ..Default::default() });
        g.insert("loquaciousness".into(), Links { up: vec![pair("loquacious", "-ness")], ..Default::default() });
        g.insert("malefactor".into(), Links { down: vec![pair("malefactress", "-ess"), pair("malefactee", "")], stem: Some(2), ..Default::default() });
        g.insert("malefactress".into(), Links { up: vec![pair("malefactor", "-ess")], stem: Some(2), ..Default::default() });
        g.insert("malefactee".into(), Links { up: vec![pair("malefactor", "")], ..Default::default() });
        g
    }

    fn run(word: &str) -> Family {
        let g = graph();
        let stems: HashMap<u32, Vec<String>> = HashMap::from([
            (1, vec!["loquacious".into(), "loquacity".into(), "loquaciously".into()]),
            (2, vec!["malefaction".into(), "malefactor".into(), "malefactress".into()]),
        ]);
        family(word, &|k| Ok(g.get(k).cloned()), &|id| Ok(stems.get(&id).cloned().unwrap_or_default())).unwrap()
    }

    fn shape(f: &Family) -> Vec<(usize, &str, &str)> {
        f.members.iter().map(|m| (m.depth, m.word.as_str(), m.via.as_str())).collect()
    }

    #[test]
    fn a_word_climbs_to_its_root_and_lists_everything_built_on_it() {
        let f = run("loquaciously");
        assert_eq!(shape(&f), [(0, "loquacity", ""), (1, "loquacious", "-ous"), (2, "loquaciously", "-ly"), (2, "loquaciousness", "-ness")]);
        assert_eq!(f.members[1].from.as_deref(), Some("loquacity"));
        // Related words already in the tree are not repeated; "loquor" is new.
        assert_eq!(f.related, ["loquor"]);
        assert!(f.stem.is_empty());
    }

    #[test]
    fn a_listed_derived_term_takes_its_lister_as_root() {
        let f = run("malefactee");
        assert_eq!(shape(&f), [(0, "malefactor", ""), (1, "malefactress", "-ess"), (1, "malefactee", "")]);
    }

    #[test]
    fn words_that_only_share_a_stem_are_guesses_kept_apart() {
        let f = run("malefactor");
        assert_eq!(shape(&f), [(0, "malefactor", ""), (1, "malefactress", "-ess"), (1, "malefactee", "")]);
        assert_eq!(f.stem, ["malefaction"]);
    }

    #[test]
    fn an_unknown_word_has_no_family() {
        assert_eq!(run("zzz"), Family { members: vec![Member { word: "zzz".into(), depth: 0, from: None, via: String::new() }], ..Default::default() });
    }
}
