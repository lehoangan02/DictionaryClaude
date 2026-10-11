//! Typo-tolerant headword index: an FST over every lemma key of a pack,
//! queried with a Levenshtein automaton. A typo anywhere in the word is found
//! in milliseconds without scanning the pack.

use std::path::Path;

use anyhow::Result;
use fst::automaton::Levenshtein;
use fst::{IntoStreamer, Set, SetBuilder, Streamer};
use rusqlite::{Connection, OpenFlags};

pub struct Fuzzy {
    set: Set<Vec<u8>>,
}

impl Fuzzy {
    /// Builds the index from a pack's keys (already sorted bytewise by SQLite).
    pub fn build(pack: &Path, out: &Path) -> Result<usize> {
        let conn = Connection::open_with_flags(pack, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tmp = out.with_extension("tmp");
        let mut b = SetBuilder::new(std::io::BufWriter::new(std::fs::File::create(&tmp)?))?;
        let mut n = 0;
        let mut stmt = conn.prepare("SELECT key FROM entries ORDER BY key")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let k: String = r.get(0)?;
            b.insert(k)?;
            n += 1;
        }
        b.finish()?;
        std::fs::rename(tmp, out)?;
        Ok(n)
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Fuzzy { set: Set::new(std::fs::read(path)?)? })
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Keys within `max` edits of `key`, at most `limit`, in key order.
    pub fn near(&self, key: &str, max: u32, limit: usize) -> Result<Vec<String>> {
        let lev = Levenshtein::new(key, max)?;
        let mut s = self.set.search(lev).into_stream();
        let mut out = Vec::new();
        while let Some(k) = s.next() {
            out.push(String::from_utf8_lossy(k).into_owned());
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}
