//! Flat word -> synonyms packs (Moby). Lookups return the stored list as-is.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub const SCHEMA: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE syn(key TEXT PRIMARY KEY, word TEXT NOT NULL, list BLOB NOT NULL) WITHOUT ROWID;
";

pub struct Thesaurus {
    conn: Connection,
}

impl Thesaurus {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Thesaurus { conn: Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)? })
    }

    pub fn get(&self, key: &str) -> Result<Vec<String>> {
        let blob: Option<Vec<u8>> =
            self.conn.query_row("SELECT list FROM syn WHERE key=?", [key], |r| r.get(0)).optional()?;
        match blob {
            Some(b) => Ok(serde_json::from_slice(&zstd::bulk::decompress(&b, 16 << 20)?)?),
            None => Ok(Vec::new()),
        }
    }
}
