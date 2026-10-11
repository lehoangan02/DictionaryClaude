//! Moby Thesaurus (public domain): "headword,syn1,syn2,..." per line ->
//! `thesaurus.pack` with one zstd'd JSON list of synonyms per lemma key.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use gredict_core::{lemma, thesaurus};
use rusqlite::{params, Connection};

pub fn build(input: &Path, out: &Path) -> Result<()> {
    // Latin-1 safe: the file has a few non-UTF-8 bytes.
    let text = String::from_utf8_lossy(&std::fs::read(input)?).into_owned();
    let mut map: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for line in text.lines() {
        let mut parts = line.split(',').map(str::trim).filter(|p| !p.is_empty());
        let Some(head) = parts.next() else { continue };
        let e = map.entry(lemma::key(head)).or_insert_with(|| (head.to_string(), Vec::new()));
        for p in parts {
            if !e.1.iter().any(|x| x == p) {
                e.1.push(p.to_string());
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
    conn.execute_batch(thesaurus::SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    for (k, (word, list)) in &map {
        let blob = zstd::bulk::compress(&serde_json::to_vec(list)?, 12)?;
        conn.execute("INSERT INTO syn(key,word,list) VALUES (?,?,?)", params![k, word, blob])?;
    }
    conn.execute("INSERT INTO meta VALUES('source', 'Moby Thesaurus II (public domain)')", [])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{} headwords", map.len());
    Ok(())
}
