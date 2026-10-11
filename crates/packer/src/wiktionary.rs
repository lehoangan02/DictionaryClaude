//! Converts the kaikki.org Wiktionary English dump (wiktextract JSONL, one
//! line per word+part-of-speech) into a dictionary pack.
//!
//! Lines are grouped by lemma key as they stream past. A zstd dictionary is
//! trained on the first few thousand groups, which roughly halves the size of
//! the many small blobs.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
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
    head_templates: Vec<RHead>,
    synonyms: Vec<RWord>,
    antonyms: Vec<RWord>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RSense {
    glosses: Vec<String>,
    tags: Vec<String>,
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
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RSound {
    ipa: String,
    tags: Vec<String>,
    mp3_url: String,
    ogg_url: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RForm {
    form: String,
    tags: Vec<String>,
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

fn block(l: Line) -> Block {
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
            sounds.push(Sound { ipa, audio, tags: s.tags.iter().take(3).cloned().collect() });
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
                tags: s.tags,
                examples: s
                    .examples
                    .into_iter()
                    .filter(|e| !e.text.trim().is_empty())
                    .take(MAX_EXAMPLES)
                    .map(|e| Example { text: clip(&e.text), by: (!e.r#ref.is_empty()).then(|| clip(&e.r#ref)) })
                    .collect(),
                synonyms: words(&s.synonyms),
                antonyms: words(&s.antonyms),
                form_of,
            })
        })
        .collect();
    Block {
        word: None,
        pos: l.pos,
        head: l.head_templates.into_iter().map(|h| h.expansion).find(|h| !h.is_empty()),
        etymology: etymology(&l.etymology_text),
        sounds,
        forms: l
            .forms
            .into_iter()
            .filter(|f| !f.form.is_empty() && f.form != "no-table-tags" && !f.tags.iter().any(|t| t == "table-tags"))
            .take(MAX_FORMS)
            .map(|f| Form { form: f.form, tags: f.tags })
            .collect(),
        senses,
        synonyms: words(&l.synonyms),
        antonyms: words(&l.antonyms),
    }
}

pub fn build(input: &Path, out: &Path, limit: Option<usize>) -> Result<()> {
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
    let reader = BufReader::with_capacity(1 << 20, MultiGzDecoder::new(File::open(input).with_context(|| input.display().to_string())?));
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
            let b = block(l);
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
                if word != e.word {
                    b.word = Some(word);
                }
                e.blocks.push(b);
            }
            _ => {
                if let Some(g) = cur.take() {
                    finish(&mut dict, &mut pending, g)?;
                }
                cur = Some((key, Entry { word, blocks: vec![b] }));
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
    conn.execute("INSERT INTO meta VALUES('source', 'Wiktionary (kaikki.org wiktextract), CC BY-SA 4.0')", [])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [entries.to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    drop(rows);
    drop(stmt);
    drop(stage);
    std::fs::remove_file(&stage_path)?;
    eprintln!("{lines} lines -> {staged} blocks -> {entries} entries ({skipped} skipped)");
    Ok(())
}
