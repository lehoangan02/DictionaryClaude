//! Open English WordNet (GWN-LMF XML) -> dictionary pack.
//!
//! Entries point at synsets; a synset carries the definition, examples and
//! member senses. Synonyms are the other members of the synset, antonyms come
//! from sense-level `antonym` relations.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use gredict_core::entry::{Block, Entry, Example, Sense};
use gredict_core::{lemma, pack};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use rusqlite::{params, Connection};

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name.as_bytes()).and_then(|a| a.unescape_value().ok()).map(|v| v.into_owned())
}

fn pos_name(code: &str) -> &'static str {
    match code {
        "n" => "noun",
        "v" => "verb",
        "a" | "s" => "adj",
        "r" => "adv",
        _ => "other",
    }
}

#[derive(Default)]
struct Synset {
    pos: String,
    definition: String,
    examples: Vec<String>,
    members: Vec<String>,
}

struct LexEntry {
    id: String,
    lemma: String,
    pos: String,
    senses: Vec<(String, String)>, // (sense id, synset id)
}

pub fn build(input: &Path, out: &Path) -> Result<()> {
    let mut reader = Reader::from_reader(BufReader::new(GzDecoder::new(File::open(input).with_context(|| input.display().to_string())?)));
    let mut buf = Vec::new();

    let mut entries: Vec<LexEntry> = Vec::new();
    let mut synsets: HashMap<String, Synset> = HashMap::new();
    let mut sense_lemma: HashMap<String, String> = HashMap::new();
    // Synset `members` reference lexical-entry ids, not sense ids.
    let mut entry_lemma: HashMap<String, String> = HashMap::new();
    let mut antonyms: HashMap<String, Vec<String>> = HashMap::new();

    let mut cur_entry: Option<LexEntry> = None;
    let mut cur_sense: Option<String> = None;
    let mut cur_syn: Option<(String, Synset)> = None;
    let mut text_into: Option<&'static str> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"LexicalEntry" => cur_entry = Some(LexEntry { id: attr(&e, "id").unwrap_or_default(), lemma: String::new(), pos: String::new(), senses: Vec::new() }),
                b"Lemma" => {
                    if let Some(c) = cur_entry.as_mut() {
                        c.lemma = attr(&e, "writtenForm").unwrap_or_default();
                        c.pos = attr(&e, "partOfSpeech").unwrap_or_default();
                    }
                }
                b"Sense" => {
                    if let (Some(c), Some(id), Some(syn)) = (cur_entry.as_mut(), attr(&e, "id"), attr(&e, "synset")) {
                        sense_lemma.insert(id.clone(), c.lemma.clone());
                        c.senses.push((id.clone(), syn));
                        cur_sense = Some(id);
                    }
                }
                b"SenseRelation" => {
                    if let (Some(s), Some("antonym"), Some(t)) = (cur_sense.as_ref(), attr(&e, "relType").as_deref(), attr(&e, "target")) {
                        antonyms.entry(s.clone()).or_default().push(t);
                    }
                }
                b"Synset" => {
                    let id = attr(&e, "id").unwrap_or_default();
                    let members = attr(&e, "members").unwrap_or_default().split_whitespace().map(String::from).collect();
                    let pos = pos_name(&attr(&e, "partOfSpeech").unwrap_or_default()).to_string();
                    cur_syn = Some((id, Synset { pos, members, ..Default::default() }));
                }
                b"Definition" => text_into = Some("definition"),
                b"Example" => text_into = Some("example"),
                _ => {}
            },
            Event::Text(t) => {
                if let (Some(into), Some((_, s))) = (text_into, cur_syn.as_mut()) {
                    let text = t.unescape()?;
                    match into {
                        "definition" => s.definition.push_str(&text),
                        _ => s.examples.push(text.into_owned()),
                    }
                }
            }
            Event::End(e) => match e.name().as_ref() {
                b"Sense" => cur_sense = None,
                b"LexicalEntry" => {
                    if let Some(c) = cur_entry.take() {
                        entry_lemma.insert(c.id.clone(), c.lemma.clone());
                        entries.push(c);
                    }
                }
                b"Synset" => {
                    if let Some((id, s)) = cur_syn.take() {
                        synsets.insert(id, s);
                    }
                }
                b"Definition" | b"Example" => text_into = None,
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    if entries.is_empty() || synsets.is_empty() {
        bail!("no entries parsed: is this a GWN-LMF file?");
    }

    // One Entry per lemma key; case variants become labelled blocks.
    let mut by_key: HashMap<String, Entry> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for le in entries {
        let senses: Vec<Sense> = le
            .senses
            .iter()
            .filter_map(|(sid, syn_id)| {
                let syn = synsets.get(syn_id)?;
                let own = lemma::key(&le.lemma);
                let mut syns: Vec<String> = Vec::new();
                for m in &syn.members {
                    if let Some(l) = entry_lemma.get(m).or_else(|| sense_lemma.get(m)).filter(|l| lemma::key(l) != own && !syns.contains(l)) {
                        syns.push(l.clone());
                    }
                }
                let ants: Vec<String> = antonyms
                    .get(sid)
                    .into_iter()
                    .flatten()
                    .filter_map(|t| sense_lemma.get(t).cloned())
                    .collect();
                Some(Sense {
                    gloss: syn.definition.trim().to_string(),
                    synonyms: syns,
                    antonyms: ants,
                    examples: syn.examples.iter().map(|t| Example { text: t.trim().trim_matches('"').to_string(), by: None }).collect(),
                    ..Default::default()
                })
            })
            .collect();
        if senses.is_empty() {
            continue;
        }
        let key = lemma::key(&le.lemma);
        let pos = pos_name(&le.pos).to_string();
        let block = Block { pos, senses, ..Default::default() };
        let e = by_key.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Entry { word: le.lemma.clone(), blocks: Vec::new() }
        });
        let mut block = block;
        if le.lemma != e.word {
            block.word = Some(le.lemma.clone());
        }
        e.blocks.push(block);
    }

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
    let samples: Vec<Vec<u8>> = order.iter().step_by((order.len() / 8000).max(1)).map(|k| serde_json::to_vec(&by_key[k]).unwrap()).collect();
    let dict = zstd::dict::from_samples(&samples, 64 * 1024)?;
    conn.execute("INSERT INTO meta VALUES('zdict', ?)", [&dict])?;
    let enc = zstd::dict::EncoderDictionary::copy(&dict, 12);
    for k in &order {
        let e = &by_key[k];
        let blob = zstd::bulk::Compressor::with_prepared_dictionary(&enc)?.compress(&serde_json::to_vec(e)?)?;
        conn.execute("INSERT INTO entries(key,word,blob) VALUES (?,?,?)", params![k, e.word, blob])?;
    }
    conn.execute("INSERT INTO meta VALUES('source', 'Open English WordNet 2024 (CC BY 4.0)')", [])?;
    conn.execute("INSERT INTO meta VALUES('entries', ?)", [order.len().to_string()])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} entries, {} synsets", order.len(), synsets.len());
    Ok(())
}
