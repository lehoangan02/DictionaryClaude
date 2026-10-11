//! Read side of a dictionary pack: one SQLite file per source holding
//! zstd-compressed JSON entries keyed by lemma key, plus a trained zstd
//! dictionary in `meta`. Opened read-only; never loaded into RAM as a whole.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use zstd::dict::DecoderDictionary;

use crate::entry::Entry;

pub const SCHEMA: &str = "
CREATE TABLE meta(k TEXT PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE entries(key TEXT PRIMARY KEY, word TEXT NOT NULL, blob BLOB NOT NULL) WITHOUT ROWID;
";

pub struct Pack {
    conn: Connection,
    dict: Option<DecoderDictionary<'static>>,
}

impl Pack {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening pack {}", path.display()))?;
        let dict: Option<Vec<u8>> =
            conn.query_row("SELECT v FROM meta WHERE k='zdict'", [], |r| r.get(0)).optional()?;
        Ok(Pack { conn, dict: dict.map(|d| DecoderDictionary::copy(&d)) })
    }

    pub fn meta(&self, k: &str) -> Result<Option<String>> {
        let v: Option<Vec<u8>> =
            self.conn.query_row("SELECT v FROM meta WHERE k=?", [k], |r| r.get(0)).optional()?;
        Ok(v.map(|v| String::from_utf8_lossy(&v).into_owned()))
    }

    pub fn len(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// `key` must already be a lemma key (see [`crate::lemma::key`]).
    pub fn get(&self, key: &str) -> Result<Option<Entry>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .query_row("SELECT blob FROM entries WHERE key=?", [key], |r| r.get(0))
            .optional()?;
        blob.map(|b| self.decode(&b)).transpose()
    }

    /// Headwords starting with `prefix` (a lemma key prefix), in key order.
    pub fn prefix(&self, prefix: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT word FROM entries WHERE key >= ?1 AND key < ?2 ORDER BY key LIMIT ?3",
        )?;
        let upper = format!("{prefix}\u{10FFFF}");
        let rows = stmt.query_map(rusqlite::params![prefix, upper, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn decode(&self, blob: &[u8]) -> Result<Entry> {
        let raw = match &self.dict {
            Some(d) => zstd::bulk::Decompressor::with_prepared_dictionary(d)?.decompress(blob, 64 << 20)?,
            None => zstd::bulk::decompress(blob, 64 << 20)?,
        };
        Ok(serde_json::from_slice(&raw)?)
    }
}
