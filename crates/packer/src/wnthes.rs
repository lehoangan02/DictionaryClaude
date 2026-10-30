//! Open English WordNet -> thesaurus pack, grouped by meaning.
//!
//! For every meaning of a word (WordNet lists the common ones first) the pack keeps the other words of
//! the synset, the words of "similar to" / "see also" synsets, the more general words (hypernyms), a few
//! more specific ones (hyponyms) and the antonyms. Unlike the flat Moby lists these say *which* meaning a
//! related word belongs to, which is what a thesaurus is for.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use gredict_core::thesaurus::{self, ThesSense};
use gredict_core::lemma;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use rusqlite::{params, Connection};

const MAX_SENSES: usize = 8;
const MAX_SIMILAR: usize = 12;
const MAX_BROADER: usize = 6;
const MAX_NARROWER: usize = 14;

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
    members: Vec<String>,        // lexical-entry ids
    rels: Vec<(String, String)>, // (relType, target synset)
}

struct LexEntry {
    id: String,
    lemma: String,
    senses: Vec<(String, String, Vec<String>)>, // (sense id, synset id, antonym sense ids)
}

/// Adds `words` to `into` unless they are the headword or already listed anywhere in this meaning.
fn add(into: &mut Vec<String>, words: impl IntoIterator<Item = String>, seen: &mut Vec<String>, cap: usize) {
    for w in words {
        if into.len() >= cap {
            break;
        }
        if !seen.contains(&w) {
            seen.push(w.clone());
            into.push(w);
        }
    }
}

pub fn build(input: &Path, out: &Path) -> Result<()> {
    let mut reader = Reader::from_reader(BufReader::new(GzDecoder::new(File::open(input).with_context(|| input.display().to_string())?)));
    let mut buf = Vec::new();

    let mut entries: Vec<LexEntry> = Vec::new();
    let mut synsets: HashMap<String, Synset> = HashMap::new();
    let mut sense_lemma: HashMap<String, String> = HashMap::new();
    let mut entry_lemma: HashMap<String, String> = HashMap::new();

    let mut cur_entry: Option<LexEntry> = None;
    let mut cur_syn: Option<(String, Synset)> = None;
    let mut in_definition = false;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                b"LexicalEntry" => cur_entry = Some(LexEntry { id: attr(&e, "id").unwrap_or_default(), lemma: String::new(), senses: Vec::new() }),
                b"Lemma" => {
                    if let Some(c) = cur_entry.as_mut() {
                        c.lemma = attr(&e, "writtenForm").unwrap_or_default();
                    }
                }
                b"Sense" => {
                    if let (Some(c), Some(id), Some(syn)) = (cur_entry.as_mut(), attr(&e, "id"), attr(&e, "synset")) {
                        sense_lemma.insert(id.clone(), c.lemma.clone());
                        c.senses.push((id, syn, Vec::new()));
                    }
                }
                b"SenseRelation" => {
                    if let (Some(c), Some("antonym"), Some(t)) = (cur_entry.as_mut(), attr(&e, "relType").as_deref(), attr(&e, "target")) {
                        if let Some(s) = c.senses.last_mut() {
                            s.2.push(t);
                        }
                    }
                }
                b"Synset" => {
                    let id = attr(&e, "id").unwrap_or_default();
                    let members = attr(&e, "members").unwrap_or_default().split_whitespace().map(String::from).collect();
                    let pos = pos_name(&attr(&e, "partOfSpeech").unwrap_or_default()).to_string();
                    cur_syn = Some((id, Synset { pos, members, ..Default::default() }));
                }
                b"SynsetRelation" => {
                    if let (Some((_, s)), Some(r), Some(t)) = (cur_syn.as_mut(), attr(&e, "relType"), attr(&e, "target")) {
                        s.rels.push((r, t));
                    }
                }
                b"Definition" => in_definition = true,
                _ => {}
            },
            Event::Text(t) => {
                if let (true, Some((_, s))) = (in_definition, cur_syn.as_mut()) {
                    s.definition.push_str(&t.unescape()?);
                }
            }
            Event::End(e) => match e.name().as_ref() {
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
                b"Definition" => in_definition = false,
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }
    if entries.is_empty() || synsets.is_empty() {
        bail!("no entries parsed: is this a GWN-LMF file?");
    }

    let lemmas_of = |syn_id: &str| -> Vec<String> {
        synsets.get(syn_id).map(|s| s.members.iter().filter_map(|m| entry_lemma.get(m).or_else(|| sense_lemma.get(m)).cloned()).collect()).unwrap_or_default()
    };
    let related = |syn: &Synset, kinds: &[&str]| -> Vec<String> {
        syn.rels.iter().filter(|(r, _)| kinds.contains(&r.as_str())).flat_map(|(_, t)| lemmas_of(t)).collect()
    };
    // Single words before phrases: "mushroom" says more than "grow in number".
    let plain_first = |mut v: Vec<String>| -> Vec<String> {
        v.sort_by_key(|w| w.contains(' ') || w.contains('-'));
        v
    };

    let mut by_key: HashMap<String, (String, Vec<ThesSense>)> = HashMap::new();
    for le in &entries {
        let own = lemma::key(&le.lemma);
        for (_, syn_id, ants) in &le.senses {
            let Some(syn) = synsets.get(syn_id) else { continue };
            // Names of people and places only drown the common words.
            if le.lemma.chars().next().is_some_and(char::is_uppercase) {
                continue;
            }
            let mut seen = vec![own.clone()];
            let mut t = ThesSense { pos: syn.pos.clone(), gloss: syn.definition.trim().to_string(), ..Default::default() };
            let mates: Vec<String> = syn.members.iter().filter_map(|m| entry_lemma.get(m)).filter(|l| lemma::key(l) != own).cloned().collect();
            add(&mut t.synonyms, mates.into_iter().map(|w| w.to_lowercase()), &mut seen, usize::MAX);
            add(&mut t.similar, plain_first(related(syn, &["similar", "also", "entails", "causes"])).into_iter().map(|w| w.to_lowercase()), &mut seen, MAX_SIMILAR);
            add(&mut t.opposite, ants.iter().filter_map(|a| sense_lemma.get(a)).cloned().map(|w| w.to_lowercase()), &mut seen, usize::MAX);
            add(&mut t.broader, plain_first(related(syn, &["hypernym", "instance_hypernym"])).into_iter().map(|w| w.to_lowercase()), &mut seen, MAX_BROADER);
            add(&mut t.narrower, plain_first(related(syn, &["hyponym"])).into_iter().map(|w| w.to_lowercase()), &mut seen, MAX_NARROWER);
            if t.synonyms.is_empty() && t.similar.is_empty() && t.broader.is_empty() && t.narrower.is_empty() && t.opposite.is_empty() {
                continue;
            }
            let e = by_key.entry(own.clone()).or_insert_with(|| (le.lemma.clone(), Vec::new()));
            if e.1.len() < MAX_SENSES {
                e.1.push(t);
            }
        }
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
    conn.execute_batch(thesaurus::SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    let mut keys: Vec<&String> = by_key.keys().collect();
    keys.sort();
    for k in &keys {
        let (word, senses) = &by_key[*k];
        let blob = zstd::bulk::compress(&serde_json::to_vec(senses)?, 12)?;
        conn.execute("INSERT INTO syn(key,word,list) VALUES (?,?,?)", params![k, word, blob])?;
    }
    conn.execute("INSERT INTO meta VALUES('source', 'Open English WordNet 2024 (CC BY 4.0), related words by meaning')", [])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} headwords", keys.len());
    Ok(())
}
