//! Word -> related-words packs: Moby (a flat synonym list per word) and the WordNet thesaurus (related words
//! grouped by meaning). Lookups return what the pack stores, as-is.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE syn(key TEXT PRIMARY KEY, word TEXT NOT NULL, list BLOB NOT NULL) WITHOUT ROWID;
";

/// One meaning of a word in the WordNet thesaurus (`wnthes.pack`) and the words WordNet relates to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThesSense {
    pub pos: String,
    pub gloss: String,
    /// The other words of the same meaning.
    #[serde(default, rename = "s", skip_serializing_if = "Vec::is_empty")]
    pub synonyms: Vec<String>,
    /// Close in meaning without being the same ("similar to", "see also").
    #[serde(default, rename = "l", skip_serializing_if = "Vec::is_empty")]
    pub similar: Vec<String>,
    /// More general words ("grow" for "proliferate").
    #[serde(default, rename = "b", skip_serializing_if = "Vec::is_empty")]
    pub broader: Vec<String>,
    /// More specific words.
    #[serde(default, rename = "n", skip_serializing_if = "Vec::is_empty")]
    pub narrower: Vec<String>,
    #[serde(default, rename = "o", skip_serializing_if = "Vec::is_empty")]
    pub opposite: Vec<String>,
}

pub struct Thesaurus {
    conn: Connection,
}

impl Thesaurus {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Thesaurus { conn: Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? })
    }

    fn load<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Vec<T>> {
        let blob: Option<Vec<u8>> =
            self.conn.query_row("SELECT list FROM syn WHERE key=?", [key], |r| r.get(0)).optional()?;
        match blob {
            Some(b) => Ok(serde_json::from_slice(&zstd::bulk::decompress(&b, 16 << 20)?)?),
            None => Ok(Vec::new()),
        }
    }

    pub fn get(&self, key: &str) -> Result<Vec<String>> {
        self.load(key)
    }

    /// The meanings of a word in `wnthes.pack`, most common first.
    pub fn senses(&self, key: &str) -> Result<Vec<ThesSense>> {
        self.load(key)
    }
}
