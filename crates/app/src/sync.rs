//! Row-level, last-writer-wins sync of the user's own data.
//!
//! The synced tables hold mostly append-only records (history, attempts) and
//! small edited documents (notes, session summaries). Each row has a text id,
//! an `updated` timestamp (ms) and a `deleted` tombstone; merging keeps the row
//! with the larger `updated`, so applying the same batch twice, or batches in
//! any order, converges to the same state. Derived data (note relations, the
//! session search index) is rebuilt from the merged rows, never synced.

use anyhow::{bail, Result};
use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::{params, params_from_iter, Connection};
use serde_json::{json, Map, Value};

use crate::api::Api;

/// Parents before children so a batch applies cleanly.
pub const TABLES: [&str; 6] = ["notes", "lookups", "sessions", "attempts", "drafts", "talks"];

/// What a guest's private store on the server keeps: their chats (the server answers them), nothing else. Their notes,
/// history and quizzes stay on their own device.
const GUEST_TABLES: [&str; 1] = ["talks"];

fn columns(db: &Connection, table: &str) -> Result<Vec<String>> {
    let mut st = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = st.query_map([], |r| r.get::<_, String>(1))?.collect::<std::result::Result<_, _>>()?;
    Ok(cols)
}

fn to_json(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(_) => Value::Null,
    }
}

fn to_sql(v: &Value) -> Sql {
    match v {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(*b as i64),
        Value::Number(n) => n.as_i64().map(Sql::Integer).unwrap_or_else(|| Sql::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => Sql::Text(s.clone()),
        other => Sql::Text(other.to_string()),
    }
}

impl Api {
    fn tables(&self) -> &'static [&'static str] {
        if self.guest { &GUEST_TABLES } else { &TABLES }
    }

    fn rows_where(&self, db: &Connection, table: &str, clause: &str, p: impl rusqlite::Params) -> Result<Vec<Value>> {
        let cols = columns(db, table)?;
        let mut st = db.prepare(&format!("SELECT {} FROM {table} {clause}", cols.join(",")))?;
        let rows = st
            .query_map(p, |r| {
                let mut m = Map::new();
                for (i, c) in cols.iter().enumerate() {
                    m.insert(c.clone(), to_json(r.get_ref(i)?));
                }
                Ok(Value::Object(m))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Rows changed at or after `since` (ms). Overlap is harmless: applying a
    /// row twice is a no-op.
    pub(crate) fn sync_export(&self, since: i64, limit: usize) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut out = Map::new();
        let mut budget = limit;
        for &t in self.tables() {
            let rows = self.rows_where(&db, t, "WHERE updated >= ?1 ORDER BY updated LIMIT ?2", params![since, budget as i64])?;
            budget = budget.saturating_sub(rows.len());
            out.insert(t.to_string(), Value::Array(rows));
        }
        Ok(json!({"changes": out, "device": self.device}))
    }

    /// Rows after server sequence `cursor`, oldest first.
    pub(crate) fn sync_pull(&self, cursor: i64, limit: usize) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT tbl,id,seq FROM sync_seq WHERE seq > ?1 ORDER BY seq LIMIT ?2")?;
        let keys: Vec<(String, String, i64)> = st
            .query_map(params![cursor, limit as i64 + 1], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let more = keys.len() > limit;
        let keys = &keys[..keys.len().min(limit)];
        let mut out: Map<String, Value> = self.tables().iter().map(|t| (t.to_string(), json!([]))).collect();
        for (t, id, _) in keys {
            if !self.tables().contains(&t.as_str()) {
                continue;
            }
            for row in self.rows_where(&db, t, "WHERE id = ?1", params![id])? {
                out.get_mut(t).and_then(Value::as_array_mut).unwrap().push(row);
            }
        }
        let next = keys.last().map_or(cursor, |k| k.2);
        Ok(json!({"changes": out, "cursor": next, "more": more}))
    }

    /// Merges `changes` ({table: [row]}) into the local database.
    pub(crate) fn sync_apply(&self, changes: &Value) -> Result<Value> {
        let Some(obj) = changes.as_object() else { bail!("`changes` must be an object of tables") };
        let mut applied = Map::new();
        let mut touched_notes: Vec<String> = Vec::new();
        let mut touched_sessions: Vec<String> = Vec::new();
        {
            let mut db = self.db.lock().unwrap();
            // Rows may arrive before their parents in a later batch.
            db.pragma_update(None, "foreign_keys", false)?;
            let tx = db.transaction()?;
            for &t in self.tables() {
                let Some(rows) = obj.get(t).and_then(Value::as_array) else { continue };
                let cols = columns(&tx, t)?;
                let mut n = 0;
                for row in rows {
                    let Some(m) = row.as_object() else { continue };
                    let (Some(id), Some(updated)) = (m.get("id").and_then(Value::as_str), m.get("updated").and_then(Value::as_i64)) else { continue };
                    // Unknown columns (from a newer peer) are ignored; missing ones take defaults.
                    let present: Vec<&String> = cols.iter().filter(|c| m.contains_key(*c)).collect();
                    let sets: Vec<String> = present.iter().filter(|c| c.as_str() != "id").map(|c| format!("{c}=excluded.{c}")).collect();
                    let sql = format!(
                        "INSERT INTO {t}({}) VALUES ({}) ON CONFLICT(id) DO UPDATE SET {} WHERE excluded.updated > {t}.updated",
                        present.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(","),
                        vec!["?"; present.len()].join(","),
                        sets.join(","),
                    );
                    let changed = tx.execute(&sql, params_from_iter(present.iter().map(|c| to_sql(&m[*c]))))?;
                    if changed > 0 {
                        n += 1;
                        let _ = updated;
                        match t {
                            "notes" => touched_notes.push(id.to_string()),
                            "sessions" => touched_sessions.push(id.to_string()),
                            _ => {}
                        }
                    }
                }
                applied.insert(t.to_string(), json!(n));
            }
            tx.commit()?;
            db.pragma_update(None, "foreign_keys", true)?;
        }
        self.rebuild_derived(&touched_notes, &touched_sessions)?;
        Ok(json!({"applied": applied}))
    }

    /// Recomputes note relations and the session search index for merged rows.
    fn rebuild_derived(&self, notes: &[String], sessions: &[String]) -> Result<()> {
        for id in notes {
            let (body, deleted): (String, i64) = {
                let db = self.db.lock().unwrap();
                db.query_row("SELECT body, deleted FROM notes WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?
            };
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction()?;
            tx.execute("DELETE FROM note_edges WHERE note_id=?1", [id])?;
            if deleted == 0 {
                for e in gredict_core::notes::parse(&body) {
                    tx.execute("INSERT INTO note_edges VALUES (?,?,?,?)", params![id, e.a, e.b, serde_json::to_value(e.rel)?.as_str()])?;
                }
            }
            tx.commit()?;
        }
        for id in sessions {
            let (deleted, ended): (i64, Option<i64>) = {
                let db = self.db.lock().unwrap();
                db.query_row("SELECT deleted, ended FROM sessions WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?)))?
            };
            if deleted != 0 || ended.is_none() {
                self.db.lock().unwrap().execute("DELETE FROM session_fts WHERE session_id=?", [id])?;
            } else {
                // A session whose items we cannot read simply stays unindexed.
                let _ = self.reindex_session(id);
            }
        }
        Ok(())
    }
}
