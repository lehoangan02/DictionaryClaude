//! Writes the parsed book into `private/barrons.sqlite`.
//!
//! `entries` hold the vocabulary; `quizzes`/`items` hold every exercise in a
//! uniform shape the app can run directly: a prompt, optional choices, and the
//! correct answer. Fill-ins and sense items are one item per sentence;
//! matching is one item per word with the unit's definitions as choices.

use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::json;

use super::entries::Entry;
use super::exercises::{Key, Review};
use super::extras::{ExtraQuiz, Root};

const SCHEMA: &str = "
CREATE TABLE entries(
  id INTEGER PRIMARY KEY, unit INTEGER NOT NULL, page INTEGER, headword TEXT NOT NULL,
  key TEXT NOT NULL, pos TEXT NOT NULL, high_freq INTEGER NOT NULL, definition TEXT NOT NULL,
  examples TEXT NOT NULL, notes TEXT NOT NULL, terms TEXT NOT NULL);
CREATE INDEX entries_key ON entries(key);
CREATE TABLE quizzes(
  id TEXT PRIMARY KEY, unit INTEGER NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
  grp TEXT NOT NULL DEFAULT 'unit', ord INTEGER NOT NULL DEFAULT 0);
CREATE TABLE roots(id INTEGER PRIMARY KEY, forms TEXT NOT NULL, meaning TEXT NOT NULL, lang TEXT NOT NULL);
CREATE TABLE root_words(root_id INTEGER NOT NULL REFERENCES roots(id), word TEXT NOT NULL, key TEXT NOT NULL, definition TEXT NOT NULL);
CREATE INDEX root_words_key ON root_words(key);
CREATE TABLE items(
  id TEXT PRIMARY KEY, quiz_id TEXT NOT NULL REFERENCES quizzes(id), ord INTEGER NOT NULL,
  prompt TEXT NOT NULL, choices TEXT, answer TEXT NOT NULL, explanation TEXT,
  words TEXT NOT NULL, style TEXT NOT NULL DEFAULT 'mcq', passage TEXT);
CREATE INDEX items_quiz ON items(quiz_id, ord);
";

pub fn write(path: &Path, entries: &[Entry], reviews: &[(Review, Key)], extras: &[ExtraQuiz], roots: &[Root]) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut db = Connection::open(path)?;
    db.execute_batch(SCHEMA)?;
    let tx = db.transaction()?;
    for e in entries {
        tx.execute(
            "INSERT INTO entries(unit,page,headword,key,pos,high_freq,definition,examples,notes,terms)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            params![
                e.unit, e.page, e.headword, gredict_core::lemma::key(&e.headword), e.pos,
                e.high_freq, e.definition, json!(e.examples).to_string(),
                json!(e.notes).to_string(), json!(e.terms).to_string()
            ],
        )?;
    }
    for (r, k) in reviews {
        let u = r.unit;
        let mut quiz = |suffix: &str, kind: &str, title: &str| -> Result<String> {
            let id = format!("u{u:02}-{suffix}");
            tx.execute("INSERT INTO quizzes(id,unit,kind,title) VALUES (?,?,?,?)", params![id, u, kind, title])?;
            Ok(id)
        };
        let mut item = |quiz_id: &str, style: &str, ord: usize, prompt: &str, choices: Option<String>, answer: &str, expl: Option<&str>, words: Vec<String>| -> Result<()> {
            tx.execute(
                "INSERT INTO items(id,quiz_id,ord,prompt,choices,answer,explanation,words,style) VALUES (?,?,?,?,?,?,?,?,?)",
                params![format!("{quiz_id}-{ord:02}"), quiz_id, ord, prompt, choices, answer, expl, json!(words).to_string(), style],
            )?;
            Ok(())
        };

        let q = quiz("match", "matching", &format!("Unit {u}: Matching"))?;
        let defs: std::collections::HashMap<char, &String> = r.matching_choices.iter().map(|(c, d)| (*c, d)).collect();
        let choices = json!(r.matching_choices.iter().map(|(_, d)| d).collect::<Vec<_>>()).to_string();
        for (n, w) in &r.matching_words {
            let letter = k.matching.iter().find(|(m, _)| m == n).map(|(_, c)| *c).unwrap_or('?');
            let answer = defs.get(&letter).map(|s| s.as_str()).unwrap_or("");
            item(&q, "match", *n as usize, w, Some(choices.clone()), answer, None, vec![gredict_core::lemma::key(w)])?;
        }

        let q = quiz("fill", "fill_in", &format!("Unit {u}: Fill-ins"))?;
        let bank = json!(r.bank).to_string();
        for (n, s) in &r.fill_ins {
            let answer = k.fill_ins.iter().find(|(m, _)| m == n).map(|(_, a)| a.as_str()).unwrap_or("");
            item(&q, "bank", *n as usize, s, Some(bank.clone()), answer, None, vec![gredict_core::lemma::key(answer)])?;
        }

        let q = quiz("sense", "sense", &format!("Unit {u}: Sense or Nonsense"))?;
        for (n, s) in &r.sense {
            let a = k.sense.iter().find(|(m, _)| m == n).map(|(_, c)| *c).unwrap_or('?');
            item(&q, "sense", *n as usize, s, Some(r#"["Sense","Nonsense"]"#.into()), if a == 'S' { "Sense" } else { "Nonsense" }, None, vec![])?;
        }

        if !r.applying.is_empty() {
            let q = quiz("apply", "applying", &format!("Units {}\u{2013}{u}: Applying Your Knowledge", u - 9))?;
            for a in &r.applying {
                item(&q, "blanks", a.number as usize, &a.stem, Some(json!(a.blanks).to_string()), &json!(a.answers).to_string(), Some(&a.explanation),
                    a.answers.iter().map(|w| gredict_core::lemma::key(w)).collect())?;
            }
        }
    }
    for (ord, q) in extras.iter().enumerate() {
        tx.execute(
            "INSERT INTO quizzes(id,unit,kind,title,grp,ord) VALUES (?,0,?,?,?,?)",
            params![q.id, q.kind, q.title, q.group, ord as i64],
        )?;
        for it in &q.items {
            tx.execute(
                "INSERT INTO items(id,quiz_id,ord,prompt,choices,answer,explanation,words,style,passage) VALUES (?,?,?,?,?,?,?,?,?,?)",
                params![
                    format!("{}-{:03}", q.id, it.ord), q.id, it.ord, it.prompt,
                    it.choices.as_ref().map(|c| c.to_string()), it.answer, it.explanation,
                    json!(it.words).to_string(), it.style, it.passage
                ],
            )?;
        }
    }
    for (i, r) in roots.iter().enumerate() {
        tx.execute("INSERT INTO roots VALUES (?,?,?,?)", params![i as i64 + 1, r.forms, r.meaning, r.lang])?;
        for (w, d) in &r.words {
            tx.execute("INSERT INTO root_words VALUES (?,?,?,?)", params![i as i64 + 1, w, gredict_core::lemma::key(w), d])?;
        }
    }
    tx.commit()?;
    Ok(())
}
