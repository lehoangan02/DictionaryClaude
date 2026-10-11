//! CMU Pronouncing Dictionary (public domain) -> `cmudict.pack` of IPA.

use std::path::Path;

use anyhow::Result;
use gredict_core::{ipa, lemma};
use rusqlite::{params, Connection};

pub fn build(input: &Path, out: &Path) -> Result<()> {
    let text = String::from_utf8_lossy(&std::fs::read(input)?).into_owned();
    if out.exists() {
        std::fs::remove_file(out)?;
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d)?;
    }
    let conn = Connection::open(out)?;
    conn.execute_batch(ipa::SCHEMA)?;
    conn.execute_batch("BEGIN")?;
    let mut n = 0;
    for line in text.lines() {
        if line.starts_with(";;;") {
            continue;
        }
        let Some((word, arpa)) = line.split_once(' ') else { continue };
        // "word(2)" are alternates; the first reading is the common one.
        if word.contains('(') {
            continue;
        }
        let key = lemma::key(word);
        if conn.execute("INSERT OR IGNORE INTO pron(key,ipa) VALUES (?,?)", params![key, ipa::arpabet_to_ipa(arpa)])? > 0 {
            n += 1;
        }
    }
    conn.execute("INSERT INTO meta VALUES('source', 'CMU Pronouncing Dictionary, public domain')", [])?;
    conn.execute_batch("COMMIT; VACUUM;")?;
    eprintln!("{n} pronunciations");
    Ok(())
}
