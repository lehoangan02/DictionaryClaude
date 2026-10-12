//! OALD (Oxford Advanced Learner's Dictionary) extracted data → dictionary pack.
//!
//! Reads the 100 unlocked entries from oald_dictionary.json, maps them into the
//! common Entry schema, and writes a standard `.pack` file.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use gredict_core::entry::{Block, Entry, Example, Sense, Sound};
use gredict_core::{lemma, pack};
use rusqlite::{params, Connection};
use serde::Deserialize;

// ── JSON shapes from oald_dictionary.json ──────────────────────────────

#[derive(Deserialize)]
struct RawEntry {
    headword: String,
    #[serde(default)]
    pos: Vec<String>,
    #[serde(default)]
    ipa_uk: Option<String>,
    #[serde(default)]
    ipa_us: Option<String>,
    #[serde(default)]
    senses: Vec<RawSense>,
    #[serde(default)]
    idioms: Vec<serde_json::Value>,
    #[serde(default)]
    word_origin: Option<String>,
}

#[derive(Deserialize)]
struct RawSense {
    definition: String,
    #[serde(default)]
    grammar: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    examples: Vec<String>,
}

// ── Sound map: headword → IPA + audio paths ────────────────────────────

#[derive(Deserialize)]
struct RawSound {
    #[serde(default)]
    ipa_uk: Option<String>,
    #[serde(default)]
    ipa_us: Option<String>,
    #[serde(default)]
    sound_uk: Option<String>,
    #[serde(default)]
    sound_us: Option<String>,
}

fn pos_normalize(p: &str) -> String {
    match p.to_lowercase().as_str() {
        "noun" => "noun",
        "verb" => "verb",
        "adjective" | "adj" => "adj",
        "adverb" | "adv" => "adv",
        "preposition" => "prep",
        "conjunction" => "conj",
        "determiner" => "det",
        "exclamation" | "interjection" => "excl",
        "pronoun" => "pron",
        "number" => "num",
        _ => return p.to_lowercase(),
    }
    .to_string()
}

fn make_sounds(ipa_uk: Option<&str>, ipa_us: Option<&str>) -> Vec<Sound> {
    let mut out = Vec::new();
    if let Some(ipa) = ipa_uk.filter(|s| !s.is_empty()) {
        out.push(Sound { ipa: Some(ipa.to_string()), tags: vec!["UK".into()], ..Default::default() });
    }
    if let Some(ipa) = ipa_us.filter(|s| !s.is_empty()) {
        out.push(Sound { ipa: Some(ipa.to_string()), tags: vec!["US".into()], ..Default::default() });
    }
    out
}

pub fn build(dir: &Path, out: &Path) -> Result<()> {
    let dict_path = dir.join("json").join("oald_dictionary.json");
    let raws: Vec<RawEntry> = serde_json::from_reader(
        std::fs::File::open(&dict_path).with_context(|| dict_path.display().to_string())?,
    )?;
    if raws.is_empty() {
        bail!("no entries in oald_dictionary.json");
    }

    // Optional: sound map for extra IPA data (covers the same ~100 words).
    let sound_map: HashMap<String, RawSound> = {
        let sm_path = dir.join("json").join("oald_sound_map.json");
        if sm_path.exists() {
            serde_json::from_reader(std::fs::File::open(&sm_path)?)?
        } else {
            HashMap::new()
        }
    };

    let mut by_key: HashMap<String, Entry> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for raw in &raws {
        let key = lemma::key(&raw.headword);

        // Build pronunciation from the entry itself, falling back to sound_map.
        let sm = sound_map.get(&raw.headword);
        let ipa_uk = raw.ipa_uk.as_deref().or_else(|| sm.and_then(|s| s.ipa_uk.as_deref()));
        let ipa_us = raw.ipa_us.as_deref().or_else(|| sm.and_then(|s| s.ipa_us.as_deref()));
        let sounds = make_sounds(ipa_uk, ipa_us);

        // One block per POS.
        let pos_list: Vec<&str> = if raw.pos.is_empty() { vec!["other"] } else { raw.pos.iter().map(String::as_str).collect() };

        let senses: Vec<Sense> = raw
            .senses
            .iter()
            .map(|s| {
                let tags: Vec<String> = s
                    .labels
                    .iter()
                    .map(|l| l.trim_matches(|c: char| c == '(' || c == ')').to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                Sense {
                    gloss: s.definition.clone(),
                    tags,
                    examples: s
                        .examples
                        .iter()
                        .map(|t| Example { text: t.clone(), by: None })
                        .collect(),
                    ..Default::default()
                }
            })
            .collect();

        let all_senses = senses;
        if all_senses.is_empty() {
            continue;
        }

        let block = Block {
            pos: pos_normalize(pos_list[0]),
            sounds,
            senses: all_senses,
            etymology: raw.word_origin.clone(),
            ..Default::default()
        };

        let e = by_key.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Entry { word: raw.headword.clone(), syl: None, blocks: Vec::new() }
        });
        e.blocks.push(block);
    }

    if order.is_empty() {
        bail!("zero entries survived mapping");
    }

    // ── Write the pack ─────────────────────────────────────────────────
    if out.exists() {
        std::fs::remove_file(out)?;
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(out)?;
    conn.pragma_update(None, "journal_mode", "OFF")?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    conn.execute_batch(pack::SCHEMA)?;
    conn.execute_batch("BEGIN")?;

    order.sort();

    // Train a zstd dictionary on the entries (small corpus, but it still helps).
    let samples: Vec<Vec<u8>> = order
        .iter()
        .step_by((order.len() / 80).max(1))
        .map(|k| serde_json::to_vec(&by_key[k]).unwrap())
        .collect();
    let dict = zstd::dict::from_samples(&samples, 64 * 1024)?;
    conn.execute("INSERT INTO meta VALUES('zdict', ?)", [&dict])?;

    let enc = zstd::dict::EncoderDictionary::copy(&dict, 12);
    for k in &order {
        let e = &by_key[k];
        let blob = zstd::bulk::Compressor::with_prepared_dictionary(&enc)?.compress(&serde_json::to_vec(e)?)?;
        conn.execute("INSERT INTO entries(key,word,blob) VALUES (?,?,?)", params![k, e.word, blob])?;
    }

    conn.execute("INSERT INTO meta VALUES('source', 'OALD (personal extraction)')", [])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [order.len().to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} entries written to {}", order.len(), out.display());
    Ok(())
}
