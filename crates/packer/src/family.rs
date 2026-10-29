//! Builds `family.pack` from the kaikki.org Wiktionary dump: which words are
//! built from which (the etymology notes "X + -ous", "un- + X"), the derived
//! and related terms Wiktionary lists, and, for words with no such data, groups
//! of common words that share a stem by spelling (the ones to treat as guesses).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufRead;
use std::path::Path;

use anyhow::Result;
use gredict_core::family::{Links, SCHEMA};
use gredict_core::lemma;
use rusqlite::{params, Connection};
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Line {
    word: String,
    lang_code: String,
    senses: Vec<RSense>,
    derived: Vec<RWord>,
    related: Vec<RWord>,
    etymology_templates: Vec<RTemplate>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RSense {
    tags: Vec<String>,
    raw_tags: Vec<String>,
    form_of: Vec<RWord>,
    alt_of: Vec<RWord>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RWord {
    word: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RTemplate {
    name: String,
    args: HashMap<String, String>,
}

/// Senses with these labels do not make a word common enough to suggest by spelling alone.
const UNCOMMON: &[&str] = &[
    "obsolete", "archaic", "rare", "nonstandard", "dialectal", "nonce-word", "misspelling", "proscribed", "uncommon", "informal", "vulgar", "slang",
];

/// Endings that make one word from another; a shared stem under them hints at one family.
const ENDINGS: &[&str] = &[
    "ization", "isation", "ability", "ibility", "ically", "ation", "ition", "ution", "ative", "itive", "ician", "ancy", "ency", "ance", "ence", "able", "ible", "ical", "eous", "ious", "ress", "ment",
    "ness", "less", "ship", "ion", "ity", "ous", "ive", "ial", "ize", "ise", "ify", "ist", "ism", "ant", "ent", "ful", "ory", "ary", "ure", "ual", "ate", "age", "ess", "ee", "or", "ly", "al", "ic", "er", "y",
];

const MIN_STEM: usize = 5;
const MAX_DOWN: usize = 30;
const MAX_RELATED: usize = 30;
const MAX_GROUP: usize = 120;
const TRAIN: usize = 20_000;

#[derive(Default)]
struct Word {
    word: String,
    common: bool,
    up: Vec<(String, String)>,
    down: Vec<(String, String)>,
    related: Vec<String>,
}

/// A template argument without the `<id:...>` style modifiers, or none when it is empty.
fn arg(a: &HashMap<String, String>, i: usize) -> Option<String> {
    let v = a.get(&i.to_string())?;
    let v = v.split('<').next().unwrap_or("").trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn affix(s: &str, suffix: bool) -> String {
    let s = s.trim_matches('-');
    if suffix { format!("-{s}") } else { format!("{s}-") }
}

/// The word an etymology template builds the entry from and what it adds ("-ous", "un-"):
/// only affixation of one English word, never a compound or a borrowing.
pub fn base_of(name: &str, a: &HashMap<String, String>) -> Option<(String, String)> {
    // `{{ety|en|:af|...}}` is the same template with its name in the second argument.
    let (name, first) = match (name, a.get("2").map(String::as_str)) {
        ("ety", Some(n)) if n.starts_with(':') => (&n[1..], 3),
        ("ety", _) => return None,
        (n, _) => (n, 2),
    };
    if !matches!(name, "suffix" | "suf" | "prefix" | "pre" | "affix" | "af" | "confix" | "con") || a.get("1").map(String::as_str) != Some("en") {
        return None;
    }
    // A term in another language (`lang1=la`) is not an English word to link to.
    let mut parts: Vec<Option<String>> = Vec::new();
    let mut i = first;
    while a.contains_key(&i.to_string()) {
        let foreign = a.get(&format!("lang{}", i - first + 1)).is_some_and(|l| l != "en");
        parts.push(if foreign { None } else { arg(a, i) });
        i += 1;
    }
    let parts: Vec<String> = parts.into_iter().map(|p| p.unwrap_or_default()).collect();
    let (base, added): (&str, Vec<String>) = match name {
        "suffix" | "suf" => (parts.first()?.as_str(), parts[1..].iter().filter(|p| !p.is_empty()).map(|p| affix(p, true)).collect()),
        "prefix" | "pre" => (parts.last()?.as_str(), parts[..parts.len() - 1].iter().filter(|p| !p.is_empty()).map(|p| affix(p, false)).collect()),
        "confix" | "con" if parts.len() == 3 => (parts[1].as_str(), vec![affix(&parts[0], false), affix(&parts[2], true)]),
        "affix" | "af" => {
            let bases: Vec<&String> = parts.iter().filter(|p| !p.is_empty() && !p.starts_with('-') && !p.ends_with('-')).collect();
            let [base] = bases[..] else { return None };
            // A hyphen on a part says which side it joins; a lone prefix like "un-" is not a base.
            (base.as_str(), parts.iter().filter(|p| p.starts_with('-') || p.ends_with('-')).cloned().collect())
        }
        _ => return None,
    };
    if base.is_empty() || added.is_empty() || base.starts_with('-') || base.ends_with('-') || base.contains(' ') {
        return None;
    }
    Some((base.to_string(), added.join(" + ")))
}

/// Whether a derived term looks like one: wiktionary's lists include stray entries (a drug under "onset"),
/// and a real derived term shares its start with the word or contains it.
fn plausible(parent: &str, child: &str) -> bool {
    child.contains(parent) || parent.contains(child) || parent.chars().zip(child.chars()).take_while(|(a, b)| a == b).count() >= 3
}

/// The stems a word could share with others: itself, without a final "e", and without each ending it has.
fn stems(w: &str) -> Vec<String> {
    let mut out = vec![w.to_string()];
    if let Some(s) = w.strip_suffix('e') {
        out.push(s.to_string());
    }
    for e in ENDINGS {
        if let Some(s) = w.strip_suffix(e) {
            if s.len() >= 4 {
                out.push(s.to_string());
            }
        }
    }
    out.retain(|s| s.len() >= MIN_STEM);
    out
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Groups of common words that share a stem, as lists of words, smallest edit first.
fn stem_groups(words: &[String]) -> Vec<Vec<String>> {
    let mut parent: Vec<usize> = (0..words.len()).collect();
    let mut first: HashMap<String, usize> = HashMap::new();
    for (i, w) in words.iter().enumerate() {
        for s in stems(w) {
            match first.get(&s) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
                None => {
                    first.insert(s, i);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for i in 0..words.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(words[i].clone());
    }
    groups.into_values().filter(|g| g.len() > 1).map(|mut g| {
        g.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        g.truncate(MAX_GROUP);
        g
    }).collect()
}

pub fn build(input: &Path, out: &Path) -> Result<()> {
    // Pass 1: every English headword that is not just an inflected form of another.
    let mut words: HashMap<String, Word> = HashMap::new();
    let mut lines: Vec<Line> = Vec::new();
    let reader = crate::wiktionary::open_lines(input)?;
    let mut n = 0usize;
    for raw in reader.lines() {
        let raw = raw?;
        n += 1;
        if n % 200_000 == 0 {
            eprintln!("  read {n} lines");
        }
        let Ok(l) = serde_json::from_str::<Line>(&raw) else { continue };
        if l.lang_code != "en" || l.word.is_empty() || l.senses.is_empty() {
            continue;
        }
        let inflected = l.senses.iter().all(|s| !s.form_of.is_empty() || !s.alt_of.is_empty());
        if inflected {
            continue;
        }
        let w = words.entry(lemma::key(&l.word)).or_insert_with(|| Word { word: l.word.clone(), ..Default::default() });
        // "Polish" and "polish" share a key; the lowercase spelling is the word.
        if l.word == lemma::key(&l.word) {
            w.word = l.word.clone();
        }
        let ordinary = l.senses.iter().any(|s| s.form_of.is_empty() && s.alt_of.is_empty() && !s.tags.iter().chain(&s.raw_tags).any(|t| UNCOMMON.contains(&t.as_str())));
        w.common |= ordinary;
        if !l.derived.is_empty() || !l.related.is_empty() || !l.etymology_templates.is_empty() {
            lines.push(Line { senses: Vec::new(), ..l });
        }
    }
    // Names ("Bateman", "Togolese") only ever appear capitalised; they would bloat the families of common words.
    words.retain(|k, w| *k == w.word);
    eprintln!("{} headwords, {} lines with links", words.len(), lines.len());

    // Pass 2: links between headwords.
    let single = |w: &str| !w.is_empty() && !w.contains(char::is_whitespace);
    let mut edges: Vec<(String, String, String, bool)> = Vec::new(); // (parent key, child key, added, from a template)
    let mut related: Vec<(String, String)> = Vec::new();
    for l in &lines {
        let key = lemma::key(&l.word);
        if !words.contains_key(&key) {
            continue;
        }
        for t in &l.etymology_templates {
            if let Some((base, added)) = base_of(&t.name, &t.args) {
                let bk = lemma::key(&base);
                if bk != key && words.contains_key(&bk) {
                    edges.push((bk, key.clone(), added, true));
                }
            }
        }
        for d in l.derived.iter().filter(|d| single(&d.word)) {
            let dk = lemma::key(&d.word);
            if dk != key && words.contains_key(&dk) && plausible(&key, &dk) {
                edges.push((key.clone(), dk, String::new(), false));
            }
        }
        for r in l.related.iter().filter(|r| single(&r.word)) {
            let rk = lemma::key(&r.word);
            if rk != key && words.contains_key(&rk) {
                related.push((key.clone(), rk));
            }
        }
    }
    drop(lines);
    // A template says more than a listing: the same pair keeps its added ending.
    edges.sort_by_key(|e| e.2.is_empty());
    let mut done: HashSet<(String, String)> = HashSet::new();
    for (parent, child, added, _) in edges {
        if !done.insert((parent.clone(), child.clone())) {
            continue;
        }
        let (pw, cw) = (words[&parent].word.clone(), words[&child].word.clone());
        words.get_mut(&parent).unwrap().down.push((cw, added.clone()));
        words.get_mut(&child).unwrap().up.push((pw, added));
    }
    for (a, b) in related {
        let (aw, bw) = (words[&a].word.clone(), words[&b].word.clone());
        for (k, other) in [(&a, bw), (&b, aw)] {
            let w = words.get_mut(k).unwrap();
            if w.related.len() < MAX_RELATED && !w.related.contains(&other) {
                w.related.push(other);
            }
        }
    }
    let common: HashMap<String, bool> = words.iter().map(|(k, w)| (k.clone(), w.common)).collect();
    for w in words.values_mut() {
        // Built-by-affix words first, then the common ones, the short ones, the alphabetical ones.
        w.down.sort_by(|a, b| {
            // Endings before prefixes ("-ness" before "un-"), the common words before the rare, the short before the long.
            let rank = |x: &(String, String)| (x.1.is_empty(), !x.1.starts_with('-'), !common.get(&lemma::key(&x.0)).copied().unwrap_or(false), x.0.len());
            rank(a).cmp(&rank(b)).then(a.0.cmp(&b.0))
        });
        w.down.truncate(MAX_DOWN);
        w.up.sort_by_key(|u| u.1.is_empty());
        w.up.truncate(4);
    }

    // Pass 3: words that share a stem.
    let mut spelled: Vec<String> = words.iter().filter(|(k, w)| w.common && **k == w.word && k.len() >= 4 && k.chars().all(|c| c.is_ascii_lowercase())).map(|(k, _)| k.clone()).collect();
    spelled.sort();
    let groups = stem_groups(&spelled);
    let mut stem_of: HashMap<&str, u32> = HashMap::new();
    for (id, g) in groups.iter().enumerate() {
        for w in g {
            stem_of.insert(w, id as u32 + 1);
        }
    }

    // Write.
    for p in [out.to_path_buf(), out.with_extension("stage")] {
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(out)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch(SCHEMA)?;
    let mut keys: Vec<&String> = words.keys().collect();
    keys.sort();
    let rows: Vec<(&String, Links)> = keys
        .into_iter()
        .filter_map(|k| {
            let w = &words[k];
            let l = Links { up: w.up.clone(), down: w.down.clone(), related: w.related.clone(), stem: stem_of.get(k.as_str()).copied() };
            (!(l.up.is_empty() && l.down.is_empty() && l.related.is_empty() && l.stem.is_none())).then_some((k, l))
        })
        .collect();
    let samples: Vec<Vec<u8>> = rows.iter().step_by((rows.len() / TRAIN).max(1)).take(TRAIN).map(|(_, l)| serde_json::to_vec(l).unwrap()).collect();
    let dict = (samples.len() >= 1000).then(|| zstd::dict::from_samples(&samples, 64 * 1024)).transpose()?;
    let enc = dict.as_ref().map(|d| zstd::dict::EncoderDictionary::copy(d, 12));
    conn.execute_batch("BEGIN")?;
    if let Some(d) = &dict {
        conn.execute("INSERT INTO meta VALUES('zdict', ?)", [d])?;
    }
    for (k, l) in &rows {
        let json = serde_json::to_vec(l)?;
        let blob = match &enc {
            Some(d) => zstd::bulk::Compressor::with_prepared_dictionary(d)?.compress(&json)?,
            None => zstd::bulk::compress(&json, 12)?,
        };
        conn.execute("INSERT INTO fam(key,blob) VALUES (?,?)", params![k, blob])?;
    }
    for (id, g) in groups.iter().enumerate() {
        conn.execute("INSERT INTO stem(id,blob) VALUES (?,?)", params![id as u32 + 1, zstd::bulk::compress(&serde_json::to_vec(g)?, 12)?])?;
    }
    conn.execute("INSERT INTO meta VALUES('source', 'Wiktionary derived terms and etymology (kaikki.org wiktextract), CC BY-SA 4.0; stem groups by spelling')", [])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [rows.len().to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} words with links, {} stem groups", rows.len(), groups.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gredict_core::family::{family, FamilyPack};
    use serde_json::json;

    fn args(v: serde_json::Value) -> HashMap<String, String> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn affix_templates_name_their_base_and_what_was_added() {
        assert_eq!(base_of("suffix", &args(json!({"1": "en", "2": "malefactor", "3": "ess<id:female>"}))), Some(("malefactor".into(), "-ess".into())));
        assert_eq!(base_of("prefix", &args(json!({"1": "en", "2": "un", "3": "happy"}))), Some(("happy".into(), "un-".into())));
        assert_eq!(base_of("af", &args(json!({"1": "en", "2": "abate", "3": "-ment"}))), Some(("abate".into(), "-ment".into())));
        assert_eq!(base_of("ety", &args(json!({"1": "en", "2": ":af", "3": "loquacious", "4": "-ly<id:adverbial>"}))), Some(("loquacious".into(), "-ly".into())));
        assert_eq!(base_of("confix", &args(json!({"1": "en", "2": "en", "3": "light", "4": "ment"}))), Some(("light".into(), "en- + -ment".into())));
    }

    #[test]
    fn compounds_borrowings_and_foreign_bases_are_not_families() {
        assert_eq!(base_of("af", &args(json!({"1": "en", "2": "crime", "3": "wave"}))), None);
        assert_eq!(base_of("compound", &args(json!({"1": "en", "2": "crime", "3": "wave"}))), None);
        assert_eq!(base_of("bor", &args(json!({"1": "en", "2": "fr", "3": "crime"}))), None);
        // "loquāx" is Latin: the English word is not built from an English base.
        assert_eq!(base_of("suf", &args(json!({"1": "en", "2": "loquāx", "3": "ous", "lang1": "la"}))), None);
        assert_eq!(base_of("suffix", &args(json!({"1": "fr", "2": "mot", "3": "ier"}))), None);
    }

    #[test]
    fn words_sharing_a_stem_are_grouped_and_short_stems_are_not() {
        let w: Vec<String> = ["malefaction", "malefactor", "malefactress", "major", "mayor", "tractor", "traction", "table"].iter().map(|s| s.to_string()).collect();
        let g = stem_groups(&w);
        assert!(g.iter().any(|g| g == &["malefactor", "malefaction", "malefactress"]));
        assert!(g.iter().any(|g| g == &["tractor", "traction"]));
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn the_pack_links_a_word_to_its_family() {
        let dir = std::env::temp_dir().join(format!("family-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let line = |word: &str, extra: serde_json::Value| {
            let mut v = json!({"word": word, "lang_code": "en", "senses": [{"glosses": ["x"]}]});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v.to_string()
        };
        let lines = [
            line("malefactor", json!({"derived": [{"word": "malefactress"}, {"word": "malefactor of great wealth"}], "related": [{"word": "maleficent"}]})),
            line("malefactress", json!({"etymology_templates": [{"name": "suffix", "args": {"1": "en", "2": "malefactor", "3": "ess"}}]})),
            line("malefaction", json!({})),
            line("maleficent", json!({})),
            line("went", json!({"senses": [{"glosses": ["past of go"], "form_of": [{"word": "go"}]}]})),
        ];
        std::fs::write(dir.join("in.jsonl"), lines.join("\n")).unwrap();
        build(&dir.join("in.jsonl"), &dir.join("family.pack")).unwrap();
        let pack = FamilyPack::open(&dir.join("family.pack")).unwrap();

        let mine = pack.links("malefactor").unwrap().unwrap();
        // The template's ending wins over the bare listing of the same pair; phrases are dropped.
        assert_eq!(mine.down, [("malefactress".to_string(), "-ess".to_string())]);
        assert_eq!(mine.related, ["maleficent"]);
        assert_eq!(pack.links("malefactress").unwrap().unwrap().up, [("malefactor".to_string(), "-ess".to_string())]);
        assert!(pack.links("went").unwrap().is_none());

        let f = family("malefaction", &|k| pack.links(k), &|id| pack.stem_mates(id)).unwrap();
        assert_eq!(f.stem, ["malefactor", "malefactress"]);
        let f = family("malefactress", &|k| pack.links(k), &|id| pack.stem_mates(id)).unwrap();
        assert_eq!(f.members.iter().map(|m| m.word.as_str()).collect::<Vec<_>>(), ["malefactor", "malefactress"]);
        assert_eq!(f.related, ["maleficent"]);
        assert_eq!(f.stem, ["malefaction"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
