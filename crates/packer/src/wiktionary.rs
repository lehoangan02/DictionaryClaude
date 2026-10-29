//! Converts the kaikki.org Wiktionary English dump (wiktextract JSONL, one
//! line per word+part-of-speech) into a dictionary pack. The same reader
//! builds the English-to-Vietnamese pack from the Vietnamese Wiktionary
//! extract of the English words (`Flavor::Vietnamese`): same line shape, with
//! Vietnamese glosses, a Vietnamese translation under each example and
//! Vietnamese labels in `raw_tags`.
//!
//! Lines are grouped by lemma key as they stream past. A zstd dictionary is
//! trained on the first few thousand groups, which roughly halves the size of
//! the many small blobs.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::MultiGzDecoder;
use gredict_core::entry::{Block, Entry, Example, Form, Sense, Sound};
use gredict_core::{lemma, pack};
use rusqlite::{params, Connection};
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Line {
    word: String,
    pos: String,
    lang_code: String,
    senses: Vec<RSense>,
    sounds: Vec<RSound>,
    forms: Vec<RForm>,
    etymology_text: String,
    hyphenations: Vec<RHyph>,
    head_templates: Vec<RHead>,
    synonyms: Vec<RWord>,
    antonyms: Vec<RWord>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RSense {
    glosses: Vec<String>,
    tags: Vec<String>,
    raw_tags: Vec<String>,
    examples: Vec<RExample>,
    synonyms: Vec<RWord>,
    antonyms: Vec<RWord>,
    form_of: Vec<RWord>,
    alt_of: Vec<RWord>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RExample {
    text: String,
    r#ref: String,
    translation: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RSound {
    ipa: String,
    tags: Vec<String>,
    raw_tags: Vec<String>,
    mp3_url: String,
    ogg_url: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RForm {
    form: String,
    tags: Vec<String>,
    raw_tags: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RHyph {
    parts: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RHead {
    expansion: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RWord {
    word: String,
}

/// Which kaikki dump is being read.
#[derive(Clone, Copy, PartialEq)]
pub enum Flavor {
    English,
    Vietnamese,
}

impl Flavor {
    fn source(self) -> &'static str {
        match self {
            Flavor::English => "Wiktionary (kaikki.org wiktextract), CC BY-SA 4.0",
            Flavor::Vietnamese => "Vietnamese Wiktionary, English entries (kaikki.org wiktextract), CC BY-SA 4.0",
        }
    }
}

/// The accent a Vietnamese Wiktionary pronunciation label names, in the tags the UI knows.
fn accent(label: &str) -> Option<&'static str> {
    match label {
        "Mỹ" => Some("US"),
        "Anh" => Some("UK"),
        _ => None,
    }
}

const MAX_EXAMPLES: usize = 3;
const MAX_TEXT: usize = 400;
const MAX_FORMS: usize = 12;
const MAX_LIST: usize = 30;
const TRAIN_GROUPS: usize = 20_000;

fn words(v: &[RWord]) -> Vec<String> {
    let mut seen = HashSet::new();
    v.iter()
        .map(|w| w.word.trim())
        .filter(|w| !w.is_empty() && seen.insert(*w))
        .take(MAX_LIST)
        .map(String::from)
        .collect()
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_TEXT {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_TEXT).collect();
    out.push('…');
    out
}

/// Recent dumps prefix the prose with an "Etymology tree" list of ancestor
/// terms; the prose starts at the first line that reads like a sentence.
fn etymology(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let text = if raw.starts_with("Etymology tree") {
        let mut lines = raw.lines().skip(1).skip_while(|l| l.split_whitespace().count() < 5);
        let first = lines.next()?;
        std::iter::once(first).chain(lines).collect::<Vec<_>>().join("\n")
    } else {
        raw.to_string()
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn block(l: Line, flavor: Flavor) -> Block {
    let mut seen_ipa = HashSet::new();
    let mut sounds: Vec<Sound> = Vec::new();
    for s in &l.sounds {
        let audio = [&s.mp3_url, &s.ogg_url].into_iter().find(|u| !u.is_empty()).cloned();
        let ipa = (!s.ipa.is_empty()).then(|| s.ipa.clone());
        match (&ipa, &audio) {
            (None, None) => continue,
            (Some(i), _) if !seen_ipa.insert(i.clone()) => continue,
            _ => {}
        }
        if sounds.len() < 8 {
            let tags: Vec<String> = match flavor {
                Flavor::English => s.tags.iter().take(3).cloned().collect(),
                Flavor::Vietnamese => s.tags.iter().cloned().chain(s.raw_tags.iter().filter_map(|t| accent(t).map(String::from))).take(3).collect(),
            };
            sounds.push(Sound { ipa, audio, tags });
        }
    }
    let senses = l
        .senses
        .into_iter()
        .filter_map(|s| {
            let (gloss, parents) = match s.glosses.split_last() {
                Some((g, p)) => (g.clone(), p.to_vec()),
                None => return None,
            };
            let form_of = s.form_of.first().or(s.alt_of.first()).map(|w| w.word.clone()).filter(|w| !w.is_empty());
            Some(Sense {
                gloss,
                parents,
                tags: if flavor == Flavor::Vietnamese { s.tags.into_iter().chain(s.raw_tags).collect() } else { s.tags },
                examples: s
                    .examples
                    .into_iter()
                    .filter(|e| !e.text.trim().is_empty())
                    .take(MAX_EXAMPLES)
                    .map(|e| Example { text: clip(&e.text), by: (!e.r#ref.is_empty()).then(|| clip(&e.r#ref)), tr: (!e.translation.trim().is_empty()).then(|| clip(&e.translation)) })
                    .collect(),
                synonyms: words(&s.synonyms),
                antonyms: words(&s.antonyms),
                form_of,
            })
        })
        .collect();
    // Syllables, when Wiktionary hyphenates the word into several parts.
    let syl = l.hyphenations.iter().map(|h| &h.parts).find(|p| p.len() > 1 && p.iter().all(|x| !x.is_empty())).map(|p| p.join("\u{b7}"));
    Block {
        syl,
        word: None,
        pos: l.pos,
        head: l.head_templates.into_iter().map(|h| h.expansion).find(|h| !h.is_empty()),
        etymology: etymology(&l.etymology_text),
        sounds,
        forms: kept_forms(l.forms, flavor),
        senses,
        synonyms: words(&l.synonyms),
        antonyms: words(&l.antonyms),
    }
}

/// The forms worth keeping, the ones that fill the main slots first: a verb like "be" lists dozens of
/// person-by-person forms before its participles, and the cap would cut those.
fn kept_forms(forms: Vec<RForm>, flavor: Flavor) -> Vec<Form> {
    let minor = |f: &RForm| f.tags.iter().any(|t| matches!(t.as_str(), "first-person" | "second-person" | "subjunctive" | "imperative" | "alternative"));
    let mut forms: Vec<RForm> = forms
        .into_iter()
        .filter(|f| !f.form.is_empty() && f.form != "no-table-tags" && !f.tags.iter().any(|t| t == "table-tags" || t == "inflection-template"))
        .collect();
    forms.sort_by_key(minor);
    forms
        .into_iter()
        .take(MAX_FORMS)
        .map(|f| Form { form: f.form, tags: if f.tags.is_empty() && flavor == Flavor::Vietnamese { f.raw_tags } else { f.tags } })
        .collect()
}

/// The dump as lines, whether it is gzipped (the English one) or plain JSONL (the Vietnamese one).
fn open_lines(input: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(input).with_context(|| input.display().to_string())?;
    let mut magic = [0u8; 2];
    let mut head = BufReader::new(file);
    let gz = head.read(&mut magic)? == 2 && magic == [0x1f, 0x8b];
    let file = File::open(input)?;
    Ok(if gz { Box::new(BufReader::with_capacity(1 << 20, MultiGzDecoder::new(file))) } else { Box::new(BufReader::with_capacity(1 << 20, file)) })
}

pub fn build(input: &Path, out: &Path, limit: Option<usize>) -> Result<()> {
    build_flavor(input, out, limit, Flavor::English)
}

pub fn build_flavor(input: &Path, out: &Path, limit: Option<usize>, flavor: Flavor) -> Result<()> {
    for p in [out.to_path_buf(), out.with_extension("stage")] {
        if p.exists() {
            std::fs::remove_file(p)?;
        }
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // Phase 1: stage every parsed line. The dump is not grouped by lemma key
    // (case variants such as "Polish"/"polish" sit far apart), so grouping is
    // done by the database sort in phase 2 instead of by input order.
    let stage_path = out.with_extension("stage");
    let stage = Connection::open(&stage_path)?;
    stage.pragma_update(None, "journal_mode", "OFF")?;
    stage.pragma_update(None, "synchronous", "OFF")?;
    stage.execute_batch("CREATE TABLE stage(key TEXT NOT NULL, word TEXT NOT NULL, json TEXT NOT NULL); BEGIN")?;
    let reader = open_lines(input)?;
    let (mut lines, mut skipped, mut staged) = (0usize, 0usize, 0usize);
    {
        let mut ins = stage.prepare("INSERT INTO stage VALUES (?,?,?)")?;
        for raw in reader.lines() {
            let raw = raw?;
            lines += 1;
            if lines % 200_000 == 0 {
                eprintln!("  read {lines} lines, staged {staged}");
            }
            if limit.is_some_and(|n| lines > n) {
                break;
            }
            let l: Line = match serde_json::from_str(&raw) {
                Ok(l) => l,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            if l.lang_code != "en" || l.word.is_empty() || l.senses.is_empty() {
                skipped += 1;
                continue;
            }
            let word = l.word.clone();
            let b = block(l, flavor);
            if b.senses.is_empty() {
                skipped += 1;
                continue;
            }
            ins.execute(params![lemma::key(&word), word, serde_json::to_string(&b)?])?;
            staged += 1;
        }
    }
    stage.execute_batch("COMMIT")?;

    // Phase 2: one entry per key, in key order, compressed with a trained dictionary.
    eprintln!("grouping {staged} blocks...");
    let conn = Connection::open(out)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch(pack::SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    let mut stmt = stage.prepare("SELECT key, word, json FROM stage ORDER BY key, (word = key) DESC, rowid")?;
    let mut rows = stmt.query([])?;

    let mut dict: Option<zstd::dict::EncoderDictionary<'static>> = None;
    let mut pending: Vec<(String, Entry)> = Vec::new();
    let mut entries = 0usize;
    let mut put = |dict: &Option<zstd::dict::EncoderDictionary<'static>>, key: &str, e: &Entry| -> Result<()> {
        let json = serde_json::to_vec(e)?;
        let blob = match dict {
            Some(d) => zstd::bulk::Compressor::with_prepared_dictionary(d)?.compress(&json)?,
            None => zstd::bulk::compress(&json, 12)?,
        };
        conn.execute("INSERT INTO entries(key,word,blob) VALUES (?,?,?)", params![key, e.word, blob])?;
        Ok(())
    };
    let mut cur: Option<(String, Entry)> = None;
    let mut finish = |dict: &mut Option<zstd::dict::EncoderDictionary<'static>>, pending: &mut Vec<(String, Entry)>, g: (String, Entry)| -> Result<()> {
        entries += 1;
        // The entry carries the syllables; the blocks need not repeat them.
        let mut g = g;
        g.1.blocks.iter_mut().for_each(|b| b.syl = None);
        if dict.is_none() && pending.len() < TRAIN_GROUPS {
            pending.push(g);
            if pending.len() == TRAIN_GROUPS {
                let samples: Vec<Vec<u8>> = pending.iter().map(|(_, e)| serde_json::to_vec(e).unwrap()).collect();
                let raw = zstd::dict::from_samples(&samples, 112 * 1024)?;
                conn.execute("INSERT INTO meta VALUES('zdict', ?)", [&raw])?;
                *dict = Some(zstd::dict::EncoderDictionary::copy(&raw, 12));
                for (k, e) in pending.drain(..) {
                    put(dict, &k, &e)?;
                }
            }
            return Ok(());
        }
        put(dict, &g.0, &g.1)
    };
    while let Some(r) = rows.next()? {
        let (key, word, json): (String, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        let mut b: Block = serde_json::from_str(&json)?;
        match &mut cur {
            Some((k, e)) if *k == key => {
                if e.syl.is_none() && word == e.word {
                    e.syl = b.syl.clone();
                }
                if word != e.word {
                    b.word = Some(word);
                }
                e.blocks.push(b);
            }
            _ => {
                if let Some(g) = cur.take() {
                    finish(&mut dict, &mut pending, g)?;
                }
                cur = Some((key, Entry { word, syl: b.syl.clone(), blocks: vec![b] }));
            }
        }
    }
    if let Some(g) = cur.take() {
        finish(&mut dict, &mut pending, g)?;
    }
    // Inputs smaller than the training threshold stay dictionary-less.
    for (k, e) in std::mem::take(&mut pending) {
        put(&None, &k, &e)?;
    }
    conn.execute("INSERT INTO meta VALUES('source', ?)", [flavor.source()])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [entries.to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    drop(rows);
    drop(stmt);
    drop(stage);
    std::fs::remove_file(&stage_path)?;
    eprintln!("{lines} lines -> {staged} blocks -> {entries} entries ({skipped} skipped)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vietnamese_edition_keeps_glosses_translations_and_labels() {
        let dir = std::env::temp_dir().join(format!("envi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (input, out) = (dir.join("in.jsonl"), dir.join("envi.pack"));
        let lines = [
            r#"{"word":"quokka","lang_code":"en","pos":"noun","sounds":[{"ipa":"/ˈkwɒkə/","raw_tags":["Anh"]}],"forms":[{"form":"quokkas","raw_tags":["số nhiều"]}],"senses":[{"glosses":["Một loài thú nhỏ."],"raw_tags":["động vật học"],"examples":[{"text":"A quokka smiled.","translation":"Một con quokka mỉm cười."}]}]}"#,
            r#"{"word":"quokka","lang_code":"vi","pos":"noun","senses":[{"glosses":["không phải tiếng Anh"]}]}"#,
            r#"{"word":"empty","lang_code":"en","pos":"adj","senses":[{}]}"#,
        ];
        std::fs::write(&input, lines.join("\n")).unwrap();
        build_flavor(&input, &out, None, Flavor::Vietnamese).unwrap();
        let pack = gredict_core::pack::Pack::open(&out).unwrap();
        assert_eq!(pack.len().unwrap(), 1);
        let e = pack.get("quokka").unwrap().unwrap();
        let b = &e.blocks[0];
        assert_eq!(b.sounds[0].tags, ["UK"]);
        assert_eq!(b.forms[0].tags, ["số nhiều"]);
        let s = &b.senses[0];
        assert_eq!(s.gloss, "Một loài thú nhỏ.");
        assert_eq!(s.tags, ["động vật học"]);
        assert_eq!(s.examples[0].tr.as_deref(), Some("Một con quokka mỉm cười."));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_main_forms_survive_the_cap_and_template_junk_is_dropped() {
        let f = |form: &str, tags: &[&str]| RForm { form: form.into(), tags: tags.iter().map(|t| t.to_string()).collect(), raw_tags: vec![] };
        let mut forms = vec![f("glossary", &["inflection-template"]), f("be", &["infinitive"])];
        for i in 0..14 {
            forms.push(f(&format!("p{i}"), &["first-person", "present", "singular"]));
        }
        forms.push(f("is", &["present", "singular", "third-person"]));
        forms.push(f("been", &["participle", "past"]));
        let kept = kept_forms(forms, Flavor::English);
        assert_eq!(kept.len(), MAX_FORMS);
        assert_eq!(kept.iter().map(|k| k.form.as_str()).take(3).collect::<Vec<_>>(), ["be", "is", "been"]);
        assert!(kept.iter().all(|k| k.form != "glossary"));
    }
}
