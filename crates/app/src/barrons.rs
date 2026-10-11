//! Read access to the locally imported Barron's book (`private/barrons.sqlite`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct BarronsEntry {
    pub word: String,
    pub key: String,
    pub unit: u32,
    pub pos: String,
    pub high_freq: bool,
    pub definition: String,
    pub examples: Value,
    pub notes: Value,
    pub terms: Value,
}

pub struct Barrons {
    pub(crate) conn: Mutex<Connection>,
    /// Entries by lemma key (a few headwords share a key, e.g. noun and verb).
    pub entries: HashMap<String, Vec<BarronsEntry>>,
}

impl Barrons {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut entries: HashMap<String, Vec<BarronsEntry>> = HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT headword,key,unit,pos,high_freq,definition,examples,notes,terms FROM entries ORDER BY id",
            )?;
            let rows = stmt.query_map([], |r| {
                let j = |i: usize| -> Value { serde_json::from_str(&r.get::<_, String>(i).unwrap_or_default()).unwrap_or(Value::Null) };
                Ok(BarronsEntry {
                    word: r.get(0)?,
                    key: r.get(1)?,
                    unit: r.get(2)?,
                    pos: r.get(3)?,
                    high_freq: r.get::<_, i64>(4)? != 0,
                    definition: r.get(5)?,
                    examples: j(6),
                    notes: j(7),
                    terms: j(8),
                })
            })?;
            for e in rows {
                let e = e?;
                entries.entry(e.key.clone()).or_default().push(e);
            }
        }
        Ok(Barrons { conn: Mutex::new(conn), entries })
    }

    /// Root families (from the book's word-root lists) that contain `key`.
    pub fn roots_for(&self, key: &str) -> Vec<Value> {
        let conn = self.conn.lock().unwrap();
        let Ok(mut st) = conn.prepare("SELECT DISTINCT r.id,r.forms,r.meaning,r.lang FROM roots r JOIN root_words w ON w.root_id=r.id WHERE w.key=?1 ORDER BY r.id") else { return vec![] };
        let roots: Vec<(i64, String, String, String)> = st.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map(|r| r.flatten().collect()).unwrap_or_default();
        roots
            .into_iter()
            .map(|(id, forms, meaning, lang)| {
                let words: Vec<Value> = conn
                    .prepare("SELECT word,key,definition FROM root_words WHERE root_id=?1 ORDER BY rowid")
                    .and_then(|mut w| w.query_map([id], |r| Ok(serde_json::json!({"word": r.get::<_, String>(0)?, "key": r.get::<_, String>(1)?, "definition": r.get::<_, String>(2)?}))).map(|r| r.flatten().collect()))
                    .unwrap_or_default();
                serde_json::json!({"forms": forms, "meaning": meaning, "lang": lang, "words": words})
            })
            .collect()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
}
