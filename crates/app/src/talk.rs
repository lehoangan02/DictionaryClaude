//! The general chat with Claude: ask about the app, report problems, suggest changes.
//! Each chat is a synced `talks` row. The server answers with `claude -p` in a
//! read-only view of the app's source; Claude never changes anything. What it suggests
//! shows up as an idea the owner approves or dismisses; approved ideas are what a coding
//! session picks up (`talk_ideas`).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};

use crate::api::Api;
use crate::jobs::Agent;
use crate::userdb::{new_id, now_ms};

const SCHEMA: &str = r#"{"type":"object","properties":{"reply":{"type":"string"},"idea":{"type":"object","properties":{"title":{"type":"string"},"detail":{"type":"string"}},"required":["title","detail"]}},"required":["reply"]}"#;

/// Claude's answer: the reply and, when the owner asked for a change, one idea.
pub fn extract_talk(stdout: &str) -> Option<(String, Option<(String, String)>)> {
    fn find(v: &Value, depth: usize) -> Option<(String, Option<(String, String)>)> {
        if depth > 6 {
            return None;
        }
        match v {
            Value::Object(m) => {
                if let Some(r) = m.get("reply").and_then(Value::as_str) {
                    let idea = m.get("idea").and_then(|i| Some((i["title"].as_str()?.trim().to_string(), i["detail"].as_str()?.trim().to_string()))).filter(|(t, _)| !t.is_empty());
                    return Some((r.trim().to_string(), idea));
                }
                m.values().find_map(|x| find(x, depth + 1))
            }
            Value::Array(a) => a.iter().find_map(|x| find(x, depth + 1)),
            Value::String(s) => {
                let t = s.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                t.starts_with('{').then(|| serde_json::from_str::<Value>(t).ok()).flatten().and_then(|v| find(&v, depth + 1))
            }
            _ => None,
        }
    }
    serde_json::from_str::<Value>(stdout.trim()).ok().and_then(|v| find(&v, 0))
}

fn prompt(msgs: &[Value]) -> String {
    let talk: String = msgs
        .iter()
        .rev()
        .take(16)
        .rev()
        .filter_map(|m| Some(format!("{}: {}", if m["role"] == "user" { "OWNER" } else { "YOU" }, m["text"].as_str()?)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are the assistant inside LadyBird, the owner's offline GRE dictionary app. You are in a chat with the owner, who uses the app on a phone. \
         The app's source is the current folder (read it with the Read tool; docs/ROADMAP.md lists known issues and plans, CLAUDE.md the rules). \
         Do not open anything under private/ or any file with keys or tokens. You cannot change anything and must not claim to. \
         Answer the owner's last message briefly and plainly, in the language they used. \
         When they report a bug, ask for a feature or a change, also give `idea`: a short `title` and a `detail` a programmer could act on \
         (what to change and why, naming the files if you know them). Give no idea for a plain question. \
         The conversation so far:\n{talk}\n\
         Reply with JSON only: {{\"reply\":string,\"idea\":{{\"title\":string,\"detail\":string}} (optional)}}"
    )
}

impl Api {
    fn repo_dir(&self) -> Option<PathBuf> {
        let mut c: Vec<PathBuf> = vec![];
        if let Ok(p) = std::env::var("LEXIS_REPO") {
            c.push(p.into());
        }
        if let Ok(p) = std::env::current_dir() {
            c.push(p);
        }
        if let Ok(h) = std::env::var("HOME") {
            c.push(PathBuf::from(h).join("Documents/AILab/DictionaryClaude"));
        }
        c.into_iter().find(|p| p.join("docs/ROADMAP.md").is_file())
    }

    /// (title, state, messages) of a chat.
    fn talk_load(&self, id: &str) -> Result<(String, String, Vec<Value>)> {
        let db = self.db.lock().unwrap();
        let (title, state, msgs): (String, String, String) = db
            .query_row("SELECT title, state, msgs FROM talks WHERE id=?1 AND deleted=0", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|_| anyhow!("no such chat"))?;
        Ok((title, state, serde_json::from_str(&msgs).unwrap_or_default()))
    }

    fn talk_save(&self, id: &str, state: &str, msgs: &[Value]) -> Result<()> {
        self.db.lock().unwrap().execute("UPDATE talks SET state=?2, msgs=?3, updated=?4 WHERE id=?1", params![id, state, json!(msgs).to_string(), now_ms()])?;
        Ok(())
    }

    pub(crate) fn talk_get(&self, id: &str) -> Result<Value> {
        let (title, state, msgs) = self.talk_load(id)?;
        Ok(json!({"id": id, "title": title, "state": state, "msgs": msgs}))
    }

    pub(crate) fn talk_list(&self) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, title, state, updated, msgs FROM talks WHERE deleted=0 ORDER BY updated DESC")?;
        let rows = st
            .query_map([], |r| {
                let msgs: String = r.get(4)?;
                let last = serde_json::from_str::<Vec<Value>>(&msgs).ok().and_then(|m| m.last().and_then(|x| x["text"].as_str().map(|t| t.chars().take(90).collect::<String>()))).unwrap_or_default();
                Ok(json!({"id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "state": r.get::<_, String>(2)?, "updated": r.get::<_, i64>(3)?, "last": last}))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }

    pub(crate) fn talk_new(&self) -> Result<Value> {
        let id = new_id();
        let t = now_ms();
        self.db.lock().unwrap().execute("INSERT INTO talks(id,created,updated) VALUES(?1,?2,?2)", params![id, t])?;
        self.talk_get(&id)
    }

    pub(crate) fn talk_send(&self, id: &str, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() || message.len() > 2000 {
            bail!("write a message of up to 2000 characters");
        }
        let (title, state, mut msgs) = self.talk_load(id)?;
        if matches!(state.as_str(), "asked" | "running") {
            bail!("Claude is still answering");
        }
        msgs.push(json!({"role": "user", "text": message, "at": now_ms()}));
        if title.is_empty() {
            let t: String = message.chars().take(40).collect();
            self.db.lock().unwrap().execute("UPDATE talks SET title=?2 WHERE id=?1", params![id, t])?;
        }
        self.talk_save(id, "asked", &msgs)?;
        self.talk_get(id)
    }

    /// Approve or dismiss the idea in the message made at `at`.
    pub(crate) fn talk_idea(&self, id: &str, at: i64, decision: &str) -> Result<Value> {
        if !matches!(decision, "approved" | "dismissed" | "proposed") {
            bail!("decision must be approved, dismissed or proposed");
        }
        let (_, state, mut msgs) = self.talk_load(id)?;
        let m = msgs.iter_mut().find(|m| m["at"] == at && m["idea"].is_object()).ok_or_else(|| anyhow!("no such idea"))?;
        m["idea"]["status"] = json!(decision);
        self.talk_save(id, &state, &msgs)?;
        self.talk_get(id)
    }

    pub(crate) fn talk_delete(&self, id: &str) -> Result<Value> {
        let (_, state, _) = self.talk_load(id)?;
        if matches!(state.as_str(), "asked" | "running") {
            bail!("Claude is still answering");
        }
        self.db.lock().unwrap().execute("UPDATE talks SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()])?;
        Ok(json!({"ok": true}))
    }

    /// Every approved idea of every chat, oldest first: what a coding session should do next.
    pub(crate) fn talk_ideas(&self) -> Result<Value> {
        let mut out = vec![];
        for c in self.talk_list()?.as_array().cloned().unwrap_or_default() {
            let (_, _, msgs) = self.talk_load(c["id"].as_str().unwrap_or(""))?;
            out.extend(msgs.iter().filter(|m| m["idea"]["status"] == "approved").map(|m| json!({"at": m["at"], "title": m["idea"]["title"], "detail": m["idea"]["detail"]})));
        }
        out.sort_by_key(|v| v["at"].as_i64());
        Ok(json!(out))
    }

    /// Serves waiting chat messages (server side). Returns how many ran Claude.
    pub(crate) fn job_talks(&self) -> Result<usize> {
        let ids: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id FROM talks WHERE state='asked' AND deleted=0 ORDER BY updated LIMIT 3")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        let mut ran = 0;
        for id in ids {
            ran += self.job_talk(&id)? as usize;
        }
        Ok(ran)
    }

    /// Answers the latest message of a chat with Claude, read-only. True when it ran Claude.
    fn job_talk(&self, id: &str) -> Result<bool> {
        let (_, _, mut msgs) = self.talk_load(id)?;
        self.talk_save(id, "running", &msgs)?;
        let fail = |msgs: &mut Vec<Value>, why: String| -> Result<bool> {
            msgs.push(json!({"role": "error", "text": why, "at": now_ms()}));
            self.talk_save(id, "", msgs)?;
            Ok(false)
        };
        if self.agent_used_today() + 1 > self.agent_cap() {
            let why = format!("the daily agent allowance ({}) is used up; raise it in Settings or try tomorrow", self.agent_cap());
            return fail(&mut msgs, why);
        }
        let (dir, scratch) = match self.repo_dir() {
            Some(d) => (d, false),
            None => {
                let d = self.data_dir.join("jobs").join("talk");
                std::fs::create_dir_all(&d)?;
                (d, true)
            }
        };
        let claude = Agent { name: "claude", bin: self.env_or("LEXIS_CLAUDE_BIN", "claude") };
        self.count_agent_call();
        let mut p = prompt(&msgs);
        if scratch {
            p.push_str("\nThe source is NOT available on this server: say so if asked about code, and answer from general knowledge.");
        }
        let out = claude.run_cli(&dir, &p, SCHEMA, None).and_then(|o| extract_talk(&o).ok_or_else(|| anyhow!("claude replied, but not in the expected form")));
        match out {
            Ok((reply, idea)) => {
                let mut m = json!({"role": "claude", "text": reply, "at": now_ms()});
                if let Some((title, detail)) = idea {
                    m["idea"] = json!({"title": title, "detail": detail, "status": "proposed"});
                }
                msgs.push(m);
                self.talk_save(id, "", &msgs)?;
                Ok(true)
            }
            Err(e) => fail(&mut msgs, format!("{e:#}")),
        }
    }
}
