//! OALD (Oxford Advanced Learner's Dictionary) extracted data → dictionary pack.
//!
//! Reads oald_dictionary.json, maps the entries into the common Entry schema, and
//! writes a standard `.pack` file. When `json/oald_audio.json` exists (see
//! OALData/tools/build_oald_audio.py), every entry's UK and US sound also names its
//! recording as `oald:<name>`; the recordings themselves live in `oald-audio.pack`
//! (see `oald_audio.rs`), so the dictionary stays small and works without them.

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

// ── json/oald_audio.json: which recording belongs to which entry ───────

#[derive(Deserialize)]
pub(crate) struct AudioEntry {
    pub h: String,
    pub uk: Option<String>,
    pub us: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AudioFiles {
    pub uk: Vec<String>,
    pub us: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct AudioIndex {
    /// One per entry of oald_dictionary.json, in the same order.
    pub entries: Vec<AudioEntry>,
    /// Recording names; position n is `audio/<accent>/<n>.mp3`.
    pub files: AudioFiles,
}

pub(crate) fn same_word(a: &str, b: &str) -> bool {
    let clean = |s: &str| s.chars().filter(|c| !matches!(c, '·' | '\u{200d}')).collect::<String>().trim().to_string();
    clean(a) == clean(b)
}

pub(crate) fn read_audio_index(dir: &Path) -> Result<Option<AudioIndex>> {
    let path = dir.join("json").join("oald_audio.json");
    if !path.exists() {
        return Ok(None);
    }
    let idx: AudioIndex = serde_json::from_reader(std::fs::File::open(&path)?).with_context(|| path.display().to_string())?;
    Ok(Some(idx))
}

/// The sound a recording is played from, e.g. `oald:laconic#_gb_2`.
fn recording(name: &Option<String>) -> Option<String> {
    name.as_ref().map(|n| format!("oald:{n}"))
}

/// The word origin, unless the extraction only kept a placeholder (a row of dots) with no words in it.
fn word_origin(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|t| t.chars().any(char::is_alphabetic)).map(str::to_string)
}

fn make_sounds(ipa_uk: Option<&str>, ipa_us: Option<&str>, audio_uk: Option<String>, audio_us: Option<String>) -> Vec<Sound> {
    let mut out = Vec::new();
    for (ipa, audio, tag) in [(ipa_uk, audio_uk, "UK"), (ipa_us, audio_us, "US")] {
        let ipa = ipa.filter(|s| !s.is_empty()).map(str::to_string);
        if ipa.is_some() || audio.is_some() {
            out.push(Sound { ipa, audio, tags: vec![tag.into()] });
        }
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

    let audio = read_audio_index(dir)?;
    if let Some(a) = &audio {
        if a.entries.len() != raws.len() || a.entries.iter().zip(&raws).any(|(x, r)| !same_word(&x.h, &r.headword)) {
            bail!("json/oald_audio.json does not match oald_dictionary.json (rebuild it with build_oald_audio.py)");
        }
    }

    let mut by_key: HashMap<String, Entry> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (n, raw) in raws.iter().enumerate() {
        let key = lemma::key(&raw.headword);

        // Build pronunciation from the entry itself, falling back to sound_map.
        let sm = sound_map.get(&raw.headword);
        let ipa_uk = raw.ipa_uk.as_deref().or_else(|| sm.and_then(|s| s.ipa_uk.as_deref()));
        let ipa_us = raw.ipa_us.as_deref().or_else(|| sm.and_then(|s| s.ipa_us.as_deref()));
        let (audio_uk, audio_us) = audio.as_ref().map_or((None, None), |a| (recording(&a.entries[n].uk), recording(&a.entries[n].us)));
        let sounds = make_sounds(ipa_uk, ipa_us, audio_uk, audio_us);

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
                        .map(|t| Example { text: t.clone(), by: None, tr: None })
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
            etymology: word_origin(&raw.word_origin),
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
