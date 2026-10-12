//! Quizzes: Barron's exercises, generated word quizzes, quizzes from the
//! owner's notes, scored sessions, and retrieval search over past sessions.
//!
//! A session stores its fully resolved items (with answers) inside
//! `sessions.spec`, so a session can always be reviewed or retried even if the
//! book import or the notes change later. Answers never leave the server
//! side until the item is answered.

use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use gredict_core::lemma;
use gredict_core::notes::Rel;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::Api;
use crate::BarronsEntry;
use crate::userdb::{new_id, now_ms};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    id: String,
    kind: String,
    prompt: String,
    /// One list per blank; a plain question has exactly one.
    choices: Vec<Vec<String>>,
    expected: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    explanation: Option<String>,
    /// Lemma keys of the words this item exercises.
    #[serde(default)]
    words: Vec<String>,
    /// Items of one exercise share a group so the UI can title them together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    /// Reading passage shown with the question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passage: Option<String>,
}

impl Item {
    fn public(&self) -> Value {
        // Questions that ask for several answers from one list ("pick two").
        let select = if matches!(self.kind.as_str(), "se" | "multi") { self.expected.len() } else { 1 };
        json!({"id": self.id, "kind": self.kind, "prompt": self.prompt, "choices": self.choices,
               "blanks": self.choices.len(), "select": select, "group": self.group, "passage": self.passage})
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
    /// `n` distinct items of `pool` other than those in `avoid`.
    fn pick(&mut self, pool: &[String], avoid: &HashSet<&str>, n: usize) -> Vec<String> {
        let mut c: Vec<&String> = pool.iter().filter(|p| !avoid.contains(p.as_str())).collect();
        self.shuffle(&mut c);
        c.into_iter().take(n).cloned().collect()
    }
}

fn mcq(rng: &mut Rng, answer: &str, mut distractors: Vec<String>) -> Vec<String> {
    distractors.retain(|d| !d.eq_ignore_ascii_case(answer));
    distractors.push(answer.to_string());
    rng.shuffle(&mut distractors);
    distractors
}

impl Api {
    fn rng(&self) -> Rng {
        let mut s = self.rng.lock().unwrap();
        *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        Rng(*s | 1)
    }

    pub(crate) fn quiz_catalog(&self) -> Result<Value> {
        let notes = self.note_graph()?;
        let _ = notes;
        let relations: i64 = self.db.lock().unwrap().query_row(
            "SELECT count(*) FROM note_edges e JOIN notes n ON n.id=e.note_id WHERE n.deleted=0",
            [],
            |r| r.get(0),
        )?;
        let dates: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT DISTINCT session_date FROM notes WHERE deleted=0 AND session_date<>'' ORDER BY session_date DESC")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            rows
        };
        let Some(b) = self.barrons() else {
            return Ok(json!({"barrons": false, "units": [], "notes": {"relations": relations, "dates": dates}}));
        };
        let conn = b.conn.lock().unwrap();
        let mut st = conn.prepare(
            "SELECT q.id,q.unit,q.kind,q.title,count(i.id) FROM quizzes q JOIN items i ON i.quiz_id=q.id WHERE q.grp='unit' GROUP BY q.id ORDER BY q.unit,q.id",
        )?;
        let mut units: Vec<Value> = Vec::new();
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?;
        for row in rows {
            let (id, unit, kind, title, count) = row?;
            let q = json!({"id": id, "kind": kind, "title": title, "count": count});
            match units.last_mut() {
                Some(u) if u["unit"] == unit => u["quizzes"].as_array_mut().unwrap().push(q),
                _ => units.push(json!({"unit": unit, "quizzes": [q]})),
            }
        }
        // Reviews and word-root exercises that sit outside the 80 units.
        let mut groups = json!({"reviews": [], "roots": []});
        let mut st = conn.prepare("SELECT q.id,q.kind,q.title,q.grp,count(i.id) FROM quizzes q JOIN items i ON i.quiz_id=q.id WHERE q.grp<>'unit' GROUP BY q.id ORDER BY q.ord")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?;
        for row in rows {
            let (id, kind, title, grp, count) = row?;
            if let Some(list) = groups.get_mut(&grp).and_then(Value::as_array_mut) {
                list.push(json!({"id": id, "kind": kind, "title": title, "count": count}));
            }
        }
        Ok(json!({"barrons": true, "units": units, "extras": groups, "words": b.entries.values().map(Vec::len).sum::<usize>(), "notes": {"relations": relations, "dates": dates}}))
    }

    // ---- building items -----------------------------------------------------

    fn barrons_items(&self, quiz_ids: &[String]) -> Result<Vec<Item>> {
        let b = self.barrons().ok_or_else(|| anyhow!("Barron's book is not imported"))?;
        let conn = b.conn.lock().unwrap();
        let mut rng = self.rng();
        let mut out = Vec::new();
        for qid in quiz_ids {
            let title: String = conn
                .query_row("SELECT title FROM quizzes WHERE id=?", [qid], |r| r.get(0))
                .map_err(|_| anyhow!("unknown quiz `{qid}`"))?;
            let mut st = conn.prepare("SELECT id,prompt,choices,answer,explanation,words,style,passage FROM items WHERE quiz_id=? ORDER BY ord")?;
            let rows: Vec<(String, String, Option<String>, String, Option<String>, String, String, Option<String>)> = st
                .query_map([qid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?
                .collect::<std::result::Result<_, _>>()?;
            for (id, prompt, choices, answer, explanation, words, style, passage) in rows {
                let words: Vec<String> = serde_json::from_str(&words).unwrap_or_default();
                // How an item is presented follows its style, not its quiz.
                let kind = match style.as_str() {
                    "match" => "matching",
                    "bank" => "fill_in",
                    "blanks" => "applying",
                    other => other,
                }
                .to_string();
                let (choices, expected) = match kind.as_str() {
                    "applying" => {
                        let blanks: Vec<Vec<String>> = serde_json::from_str(choices.as_deref().unwrap_or("[]"))?;
                        (blanks, serde_json::from_str::<Vec<String>>(&answer)?)
                    }
                    "se" | "multi" => (
                        vec![serde_json::from_str(choices.as_deref().unwrap_or("[]"))?],
                        serde_json::from_str::<Vec<String>>(&answer)?,
                    ),
                    "matching" => {
                        // Five options instead of the unit's ten keep it playable on a phone.
                        let all: Vec<String> = serde_json::from_str(choices.as_deref().unwrap_or("[]"))?;
                        let d = rng.pick(&all, &HashSet::from([answer.as_str()]), 4);
                        (vec![mcq(&mut rng, &answer, d)], vec![answer])
                    }
                    _ => (vec![serde_json::from_str(choices.as_deref().unwrap_or("[]"))?], vec![answer]),
                };
                out.push(Item { id, kind: kind.clone(), prompt, choices, expected, explanation, words, group: Some(title.clone()), passage });
            }
        }
        Ok(out)
    }

    /// Definition/word/cloze items from the book's vocabulary.
    fn word_items(&self, mode: &str, units: Option<(u32, u32)>, high_freq: bool, count: usize) -> Result<Vec<Item>> {
        let b = self.barrons().ok_or_else(|| anyhow!("Barron's book is not imported"))?;
        let mut rng = self.rng();
        let mut pool: Vec<&BarronsEntry> = b
            .entries
            .values()
            .flatten()
            .filter(|e| units.is_none_or(|(a, z)| (a..=z).contains(&e.unit)) && (!high_freq || e.high_freq))
            .collect();
        pool.sort_by_key(|e| e.key.clone());
        rng.shuffle(&mut pool);
        let all: Vec<&BarronsEntry> = b.entries.values().flatten().collect();
        let mut out = Vec::new();
        for e in pool {
            if out.len() >= count {
                break;
            }
            let same_pos: Vec<&&BarronsEntry> = all.iter().filter(|o| o.pos == e.pos && o.key != e.key).collect();
            let mut pick = |f: &dyn Fn(&BarronsEntry) -> String| -> Vec<String> {
                let pool: Vec<String> = same_pos.iter().map(|o| f(o)).collect();
                rng.pick(&pool, &HashSet::new(), 3)
            };
            let item = match mode {
                "word_to_def" => {
                    let d = pick(&|o| o.definition.clone());
                    Item { id: format!("w2d-{}", e.key), kind: "word_to_def".into(), prompt: e.word.clone(), choices: vec![mcq(&mut rng, &e.definition, d)], expected: vec![e.definition.clone()], explanation: None, words: vec![e.key.clone()], group: Some("Word → meaning".into()), passage: None }
                }
                "cloze" => {
                    let Some(sentence) = e.examples.as_array().and_then(|a| a.iter().filter_map(Value::as_str).find_map(|s| blank_word(s, &e.word))) else { continue };
                    let d = pick(&|o| o.word.clone());
                    Item { id: format!("cz-{}", e.key), kind: "cloze".into(), prompt: sentence, choices: vec![mcq(&mut rng, &e.word, d)], expected: vec![e.word.clone()], explanation: Some(format!("{} {}", e.word, e.definition)), words: vec![e.key.clone()], group: Some("Fill the blank".into()), passage: None }
                }
                _ => {
                    let d = pick(&|o| o.word.clone());
                    Item { id: format!("d2w-{}", e.key), kind: "def_to_word".into(), prompt: e.definition.clone(), choices: vec![mcq(&mut rng, &e.word, d)], expected: vec![e.word.clone()], explanation: None, words: vec![e.key.clone()], group: Some("Meaning → word".into()), passage: None }
                }
            };
            out.push(item);
        }
        if out.is_empty() {
            bail!("no words match that selection");
        }
        Ok(out)
    }

    /// Items from the owner's own notes: synonyms, opposites and confusables.
    fn note_items(&self, session_date: Option<&str>, tag: Option<&str>, count: usize) -> Result<Vec<Item>> {
        let rows: Vec<(String, String, String, String, String)> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare(
                "SELECT e.a,e.b,e.rel,n.session_date,n.body FROM note_edges e JOIN notes n ON n.id=e.note_id
                 WHERE n.deleted=0 AND (?1 IS NULL OR n.session_date=?1) AND (?2 IS NULL OR (',' || n.tags || ',') LIKE '%,' || ?2 || ',%')",
            )?;
            let rows = st
                .query_map(params![session_date, tag], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<std::result::Result<_, _>>()?;
            rows
        };
        if rows.is_empty() {
            bail!("your notes have no relations yet; write some like `abate = lessen` first");
        }
        let mut rng = self.rng();
        // Distractors come from everything in the notes, then the book.
        let mut universe: Vec<String> = rows.iter().flat_map(|r| [r.0.clone(), r.1.clone()]).collect();
        if let Some(b) = self.barrons() {
            universe.extend(b.entries.keys().cloned());
        }
        universe.sort();
        universe.dedup();
        let mut shuffled = rows.clone();
        rng.shuffle(&mut shuffled);
        let graph = self.note_graph()?;
        let mut out = Vec::new();
        for (a, b, rel, date, body) in shuffled {
            if out.len() >= count {
                break;
            }
            let rel: Rel = serde_json::from_value(Value::String(rel)).unwrap_or(Rel::Similar);
            // Ask about either end of the relation.
            let (ask, ans) = if rng.below(2) == 0 { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
            let known = graph.related(&ask);
            let related: HashSet<&str> = known
                .same.iter().chain(&known.similar).chain(&known.opposite).chain(&known.different)
                .map(String::as_str).chain([ask.as_str()]).collect();
            let d = rng.pick(&universe, &related, 3);
            let line = body.lines().find(|l| l.to_lowercase().contains(&ask) && l.to_lowercase().contains(&ans)).unwrap_or("").trim().to_string();
            let prompt = match rel {
                Rel::Same => format!("Which word means the same as “{ask}”?"),
                Rel::Similar => format!("Which word is roughly similar in meaning to “{ask}”?"),
                Rel::Opposite => format!("Which word is the opposite of “{ask}”?"),
                Rel::Different => format!("Which word is easily confused with “{ask}” but means something different?"),
            };
            let explanation = format!("From your note{}: {line}", if date.is_empty() { String::new() } else { format!(" of {date}") });
            out.push(Item {
                id: format!("nt-{}-{:?}-{}", ask, rel, ans),
                kind: "note".into(),
                prompt,
                choices: vec![mcq(&mut rng, &ans, d)],
                expected: vec![ans.clone()],
                explanation: Some(explanation),
                words: vec![ask, ans],
                group: Some("From your notes".into()),
                passage: None,
            });
        }
        Ok(out)
    }

    // ---- sessions -----------------------------------------------------------

    pub(crate) fn quiz_start(&self, req: &Value) -> Result<Value> {
        let s = |k: &str| req.get(k).and_then(Value::as_str);
        let n = |k: &str, d: usize| req.get(k).and_then(Value::as_u64).map_or(d, |v| v as usize);
        let kind = s("kind").ok_or_else(|| anyhow!("missing `kind`"))?;
        let (title, mut items) = match kind {
            "barrons" => {
                let mut ids: Vec<String> = req.get("quiz_ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
                if ids.is_empty() {
                    if let (Some(u0), Some(u1)) = (req.get("unit_from").and_then(Value::as_u64), req.get("unit_to").and_then(Value::as_u64)) {
                        let b = self.barrons().ok_or_else(|| anyhow!("Barron's book is not imported"))?;
                        let conn = b.conn.lock().unwrap();
                        let mut st = conn.prepare("SELECT id FROM quizzes WHERE unit BETWEEN ?1 AND ?2 ORDER BY unit,id")?;
                        ids = st.query_map(params![u0, u1], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                    }
                }
                if ids.is_empty() {
                    bail!("choose at least one quiz");
                }
                let title = if ids.len() == 1 { self.quiz_title(&ids[0])? } else { format!("Barron's: {} quizzes", ids.len()) };
                (title, self.barrons_items(&ids)?)
            }
            "words" => {
                let mode = s("mode").unwrap_or("def_to_word");
                let units = req.get("unit_from").and_then(Value::as_u64).zip(req.get("unit_to").and_then(Value::as_u64)).map(|(a, z)| (a as u32, z as u32));
                let hf = req.get("high_freq").and_then(Value::as_bool).unwrap_or(false);
                let label = match mode { "word_to_def" => "Word → meaning", "cloze" => "Fill the blank", _ => "Meaning → word" };
                let scope = if hf { " (high-frequency)".to_string() } else { units.map_or(String::new(), |(a, z)| format!(" (units {a}–{z})")) };
                (format!("{label}{scope}"), self.word_items(mode, units, hf, n("count", 20))?)
            }
            "notes" => {
                let date = s("session_date").filter(|d| !d.is_empty());
                let items = self.note_items(date, s("tag").filter(|t| !t.is_empty()), n("count", 15))?;
                (format!("My notes{}", date.map_or(String::new(), |d| format!(" · {d}"))), items)
            }
            "retry" => {
                let id = s("session_id").ok_or_else(|| anyhow!("missing `session_id`"))?;
                let (title, mut items) = self.session_items(id)?;
                let missed: HashSet<String> = {
                    let db = self.db.lock().unwrap();
                    let mut st = db.prepare("SELECT item_id FROM attempts WHERE session_id=? AND correct=0")?;
                    let rows = st.query_map([id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                    rows
                };
                items.retain(|i| missed.contains(&i.id));
                if items.is_empty() {
                    bail!("nothing was missed in that session");
                }
                (format!("Retry: {title}"), items)
            }
            other => bail!("unknown quiz kind `{other}`"),
        };
        if req.get("shuffle").and_then(Value::as_bool).unwrap_or(false) {
            self.rng().shuffle(&mut items);
        }
        if let Some(limit) = req.get("limit").and_then(Value::as_u64) {
            items.truncate(limit as usize);
        }
        if items.is_empty() {
            bail!("that selection has no questions");
        }
        let id = new_id();
        let spec = json!({"request": req, "items": items});
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,kind,title,spec,started,total,updated) VALUES (?,?,?,?,?,?,?)",
            params![id, kind, title, spec.to_string(), now_ms(), items.len() as i64, now_ms()],
        )?;
        Ok(json!({"id": id, "title": title, "total": items.len(), "items": items.iter().map(Item::public).collect::<Vec<_>>()}))
    }

    fn quiz_title(&self, id: &str) -> Result<String> {
        let b = self.barrons().ok_or_else(|| anyhow!("Barron's book is not imported"))?;
        let conn = b.conn.lock().unwrap();
        Ok(conn.query_row("SELECT title FROM quizzes WHERE id=?", [id], |r| r.get(0))?)
    }

    fn session_items(&self, id: &str) -> Result<(String, Vec<Item>)> {
        let db = self.db.lock().unwrap();
        let (title, spec): (String, String) = db
            .query_row("SELECT title,spec FROM sessions WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|_| anyhow!("no such session"))?;
        let v: Value = serde_json::from_str(&spec)?;
        Ok((title, serde_json::from_value(v["items"].clone())?))
    }

    pub(crate) fn quiz_answer(&self, req: &Value) -> Result<Value> {
        let sid = req.get("session_id").and_then(Value::as_str).ok_or_else(|| anyhow!("missing `session_id`"))?;
        let iid = req.get("item_id").and_then(Value::as_str).ok_or_else(|| anyhow!("missing `item_id`"))?;
        let given: Vec<String> = match req.get("given") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            _ => bail!("missing `given`"),
        };
        let ms = req.get("ms").and_then(Value::as_i64).unwrap_or(0);
        let (title, items) = self.session_items(sid)?;
        let item = items.iter().find(|i| i.id == iid).ok_or_else(|| anyhow!("that question is not in this session"))?;
        let norm = |v: &[String]| v.iter().map(|s| s.trim().to_lowercase()).collect::<Vec<_>>();
        let (mut g, mut e) = (norm(&given), norm(&item.expected));
        // "Pick two" and "select all that apply" ignore the order of the picks.
        if matches!(item.kind.as_str(), "se" | "multi") {
            g.sort();
            e.sort();
        }
        let correct = g == e;
        {
            let db = self.db.lock().unwrap();
            // Answering twice keeps the first answer.
            let exists: bool = db.query_row("SELECT count(*) FROM attempts WHERE session_id=?1 AND item_id=?2", params![sid, iid], |r| r.get::<_, i64>(0))? > 0;
            if !exists {
                db.execute(
                    "INSERT INTO attempts(id,session_id,item_id,prompt,given,expected,correct,ms,ts,updated) VALUES (?,?,?,?,?,?,?,?,?,?)",
                    params![new_id(), sid, iid, item.prompt, given.join(" | "), item.expected.join(" | "), correct as i64, ms, now_ms(), now_ms()],
                )?;
            }
        }
        let (answered, right): (i64, i64) = self.db.lock().unwrap().query_row(
            "SELECT count(*), coalesce(sum(correct),0) FROM attempts WHERE session_id=?",
            [sid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let finished = answered as usize >= items.len();
        {
            let db = self.db.lock().unwrap();
            db.execute("UPDATE sessions SET correct=?2, ended=CASE WHEN ?3 THEN coalesce(ended, ?4) ELSE ended END, updated=?4 WHERE id=?1", params![sid, right, finished, now_ms()])?;
        }
        if finished {
            self.index_session(sid, &title, &items)?;
        }
        Ok(json!({
            "correct": correct, "expected": item.expected, "explanation": item.explanation,
            "answered": answered, "total": items.len(), "score": right, "finished": finished,
        }))
    }

    /// Reopens an unfinished quiz where it was left: the same questions, which ones are
    /// already answered, and what was missed so far.
    pub(crate) fn quiz_resume(&self, req: &Value) -> Result<Value> {
        let sid = req.get("session_id").and_then(Value::as_str).ok_or_else(|| anyhow!("missing `session_id`"))?;
        let (title, items) = self.session_items(sid)?;
        let answered: Vec<(String, bool)> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT item_id, correct FROM attempts WHERE session_id=?1 ORDER BY ts")?;
            let rows = st.query_map([sid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?.collect::<std::result::Result<_, _>>()?;
            rows
        };
        let missed: Vec<Value> = answered
            .iter()
            .filter(|(_, ok)| !ok)
            .filter_map(|(id, _)| items.iter().find(|i| &i.id == id))
            .map(|i| json!({"prompt": i.prompt, "expected": i.expected, "explanation": i.explanation}))
            .collect();
        Ok(json!({
            "id": sid, "title": title, "total": items.len(), "items": items.iter().map(Item::public).collect::<Vec<_>>(),
            "answered": answered.iter().map(|(id, ok)| json!({"item_id": id, "correct": ok})).collect::<Vec<_>>(), "missed": missed,
        }))
    }

    /// Rebuilds a finished session's search document from its stored items.
    pub(crate) fn reindex_session(&self, id: &str) -> Result<()> {
        let (title, items) = self.session_items(id)?;
        self.index_session(id, &title, &items)
    }

    /// Rebuilds the full-text document of a finished session.
    fn index_session(&self, sid: &str, title: &str, items: &[Item]) -> Result<()> {
        let db = self.db.lock().unwrap();
        let wrong: HashSet<String> = {
            let mut st = db.prepare("SELECT item_id FROM attempts WHERE session_id=? AND correct=0")?;
            let rows = st.query_map([sid], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            rows
        };
        let mut body = String::from(title);
        for i in items {
            body.push('\n');
            body.push_str(&i.prompt);
            body.push(' ');
            body.push_str(&i.expected.join(" "));
            body.push(' ');
            body.push_str(&i.words.join(" "));
            if wrong.contains(&i.id) {
                body.push_str(" missed");
            }
        }
        if wrong.is_empty() {
            body.push_str(" perfect");
        }
        db.execute("DELETE FROM session_fts WHERE session_id=?", [sid])?;
        db.execute("INSERT INTO session_fts(session_id, body) VALUES (?,?)", params![sid, body])?;
        Ok(())
    }

    pub(crate) fn session_get(&self, id: &str) -> Result<Value> {
        let (title, items) = self.session_items(id)?;
        let db = self.db.lock().unwrap();
        let (kind, started, ended, total, correct): (String, i64, Option<i64>, i64, i64) = db.query_row(
            "SELECT kind,started,ended,total,correct FROM sessions WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let mut st = db.prepare("SELECT item_id,given,correct,ms FROM attempts WHERE session_id=?")?;
        let attempts: Vec<(String, String, i64, i64)> = st.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
        let review: Vec<Value> = items
            .iter()
            .map(|i| {
                let a = attempts.iter().find(|a| a.0 == i.id);
                json!({"item": i.public(), "expected": i.expected, "explanation": i.explanation,
                       "given": a.map(|a| a.1.clone()), "correct": a.map(|a| a.2 != 0), "ms": a.map(|a| a.3)})
            })
            .collect();
        Ok(json!({"id": id, "title": title, "kind": kind, "started": started, "ended": ended, "total": total, "correct": correct, "review": review}))
    }

    /// Recent sessions, or — with a query — retrieval over everything the
    /// session contained, so "stubborn" also finds a session on *obdurate*.
    pub(crate) fn sessions_list(&self, q: Option<&str>, limit: i64) -> Result<Value> {
        let row = |r: &rusqlite::Row| -> rusqlite::Result<Value> {
            Ok(json!({"id": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?,
                      "started": r.get::<_, i64>(3)?, "ended": r.get::<_, Option<i64>>(4)?,
                      "total": r.get::<_, i64>(5)?, "correct": r.get::<_, i64>(6)?, "answered": r.get::<_, i64>(7)?}))
        };
        let q = q.map(str::trim).filter(|q| !q.is_empty());
        let Some(q) = q else {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id,kind,title,started,ended,total,correct,(SELECT count(*) FROM attempts a WHERE a.session_id=sessions.id) FROM sessions WHERE deleted=0 ORDER BY started DESC LIMIT ?")?;
            let rows = st.query_map([limit], row)?.collect::<std::result::Result<Vec<_>, _>>()?;
            return Ok(json!(rows));
        };

        // Intent words, then thesaurus expansion of what is left.
        const STOP: &[&str] = &["a", "an", "the", "i", "me", "my", "of", "on", "in", "to", "for", "and", "or", "about", "with", "words", "word", "got", "get", "had", "have", "that", "which", "sessions", "session", "quiz", "quizzes", "all", "were", "was", "is", "are"];
        let toks: Vec<String> = q.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_lowercase).collect();
        let wants_wrong = toks.iter().any(|t| matches!(t.as_str(), "wrong" | "missed" | "incorrect" | "mistake" | "mistakes" | "failed" | "errors"));
        let wants_perfect = toks.iter().any(|t| matches!(t.as_str(), "perfect" | "flawless"));
        let terms: Vec<String> = toks
            .into_iter()
            .filter(|t| !STOP.contains(&t.as_str()) && !matches!(t.as_str(), "wrong" | "missed" | "incorrect" | "mistake" | "mistakes" | "failed" | "errors" | "perfect" | "flawless"))
            .collect();

        let quote = |t: &str| format!("\"{}\"", t.replace('"', ""));
        let mut scored: Vec<(String, f64)> = Vec::new();
        let mut run = |match_expr: &str, weight: f64, scored: &mut Vec<(String, f64)>| -> Result<()> {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT session_id, bm25(session_fts) FROM session_fts WHERE session_fts MATCH ? ORDER BY bm25(session_fts) LIMIT 200")?;
            let rows = st.query_map([match_expr], |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))?;
            for r in rows.flatten() {
                // bm25 is negative; more negative is better.
                match scored.iter_mut().find(|(id, _)| *id == r.0) {
                    Some(e) => e.1 += -r.1 * weight,
                    None => scored.push((r.0, -r.1 * weight)),
                }
            }
            Ok(())
        };
        if !terms.is_empty() {
            run(&terms.iter().map(|t| quote(t)).collect::<Vec<_>>().join(" OR "), 1.0, &mut scored)?;
            let mut expanded: Vec<String> = Vec::new();
            for t in &terms {
                let mut syn: Vec<String> = self.synonym_set(&lemma::key(t)).unwrap_or_default().into_iter().filter(|s| !s.contains(' ')).collect();
                syn.sort();
                expanded.extend(syn.into_iter().take(150));
            }
            // Gemini's list of close words (cached; empty when offline), so "boring" reaches "mundane".
            let mut related: Vec<String> = Vec::new();
            for t in terms.iter().take(3) {
                related.extend(self.related_words(t).into_iter().filter(|w| !w.contains(' ')));
            }
            related.sort();
            related.dedup();
            if !related.is_empty() {
                run(&related.iter().map(|t| quote(t)).collect::<Vec<_>>().join(" OR "), 0.8, &mut scored)?;
            }
            expanded.sort();
            expanded.dedup();
            if !expanded.is_empty() {
                run(&expanded.iter().map(|t| quote(t)).collect::<Vec<_>>().join(" OR "), 0.4, &mut scored)?;
            }
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        // Meaning search (when the AI model is installed): fuse the keyword
        // ranking with the semantic one, so "stubborn" also finds sessions
        // about obdurate words that no synonym list links.
        if !terms.is_empty() {
            let semantic = self.semantic_sessions(&terms.join(" "));
            if !semantic.is_empty() {
                let mut fused: Vec<(String, f64)> = Vec::new();
                let mut add = |id: &str, w: f64| match fused.iter_mut().find(|(i, _)| i == id) {
                    Some(e) => e.1 += w,
                    None => fused.push((id.to_string(), w)),
                };
                for (rank, (id, _)) in scored.iter().enumerate() {
                    add(id, 1.0 / (60.0 + rank as f64));
                }
                for (rank, (id, _)) in semantic.iter().enumerate() {
                    add(id, 1.0 / (60.0 + rank as f64));
                }
                fused.sort_by(|a, b| b.1.total_cmp(&a.1));
                scored = fused;
            }
        }

        let db = self.db.lock().unwrap();
        let mut out = Vec::new();
        let candidate_ids: Vec<String> = if terms.is_empty() {
            // Only intent words ("sessions I got wrong"): everything, newest first.
            let mut st = db.prepare("SELECT id FROM sessions WHERE deleted=0 ORDER BY started DESC LIMIT 500")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            rows
        } else {
            scored.into_iter().map(|(id, _)| id).collect()
        };
        for id in candidate_ids {
            let v = db.query_row("SELECT id,kind,title,started,ended,total,correct,(SELECT count(*) FROM attempts a WHERE a.session_id=sessions.id) FROM sessions WHERE id=?1 AND deleted=0", [&id], row);
            let Ok(v) = v else { continue };
            let (total, correct) = (v["total"].as_i64().unwrap_or(0), v["correct"].as_i64().unwrap_or(0));
            let finished = v["ended"].as_i64().is_some();
            if (wants_wrong && !(finished && correct < total)) || (wants_perfect && !(finished && correct == total)) {
                continue;
            }
            out.push(v);
            if out.len() as i64 >= limit {
                break;
            }
        }
        Ok(json!(out))
    }
}

/// Replaces the first whole-word occurrence of `word` (or a close inflection
/// sharing its stem) in `sentence` with a blank, or `None` when it is absent.
fn blank_word(sentence: &str, word: &str) -> Option<String> {
    let w = word.to_lowercase();
    let stem: String = w.chars().take(w.chars().count().saturating_sub(3).max(4)).collect();
    let mut out = String::new();
    let mut done = false;
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut String, done: &mut bool| {
        if !cur.is_empty() {
            if !*done && cur.to_lowercase().starts_with(&stem) {
                out.push_str("____");
                *done = true;
            } else {
                out.push_str(cur);
            }
            cur.clear();
        }
    };
    for c in sentence.chars() {
        if c.is_alphabetic() || c == '\'' || c == '’' {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out, &mut done);
            out.push(c);
        }
    }
    flush(&mut cur, &mut out, &mut done);
    done.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::blank_word;

    #[test]
    fn blanks_inflected_forms() {
        assert_eq!(blank_word("The storm abated at dawn.", "abate").unwrap(), "The storm ____ at dawn.");
        assert_eq!(blank_word("Her loquacious aunt spoke.", "loquacious").unwrap(), "Her ____ aunt spoke.");
        assert!(blank_word("Nothing here.", "abate").is_none());
    }
}
