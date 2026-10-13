//! The general chat with Claude: ask about the app, report problems, suggest changes.
//! One synced conversation (`talks` row "main"). The server answers with `claude -p` in a
//! read-only view of the app's source; Claude never changes anything. What it suggests
//! shows up as an idea the owner approves or dismisses; approved ideas are what a coding
//! session picks up (`talk_ideas`).

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};

use crate::api::Api;
use crate::jobs::Agent;
use crate::userdb::now_ms;

const ID: &str = "main";
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

    fn talk_load(&self) -> Result<(String, Vec<Value>)> {
        let db = self.db.lock().unwrap();
        let t = now_ms();
        db.execute("INSERT OR IGNORE INTO talks(id,created,updated) VALUES(?1,?2,?2)", params![ID, t])?;
        let (state, msgs): (String, String) = db.query_row("SELECT state, msgs FROM talks WHERE id=?1", [ID], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok((state, serde_json::from_str(&msgs).unwrap_or_default()))
    }

    fn talk_save(&self, state: &str, msgs: &[Value]) -> Result<()> {
        self.db.lock().unwrap().execute("UPDATE talks SET state=?2, msgs=?3, deleted=0, updated=?4 WHERE id=?1", params![ID, state, json!(msgs).to_string(), now_ms()])?;
        Ok(())
    }

    pub(crate) fn talk_get(&self) -> Result<Value> {
        let (state, msgs) = self.talk_load()?;
        Ok(json!({"state": state, "msgs": msgs}))
    }

    pub(crate) fn talk_send(&self, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() || message.len() > 2000 {
            bail!("write a message of up to 2000 characters");
        }
        let (state, mut msgs) = self.talk_load()?;
        if matches!(state.as_str(), "asked" | "running") {
            bail!("Claude is still answering");
        }
        msgs.push(json!({"role": "user", "text": message, "at": now_ms()}));
        self.talk_save("asked", &msgs)?;
        self.talk_get()
    }

    /// Approve or dismiss the idea in the message made at `at`.
    pub(crate) fn talk_idea(&self, at: i64, decision: &str) -> Result<Value> {
        if !matches!(decision, "approved" | "dismissed" | "proposed") {
            bail!("decision must be approved, dismissed or proposed");
        }
        let (state, mut msgs) = self.talk_load()?;
        let m = msgs.iter_mut().find(|m| m["at"] == at && m["idea"].is_object()).ok_or_else(|| anyhow!("no such idea"))?;
        m["idea"]["status"] = json!(decision);
        self.talk_save(&state, &msgs)?;
        self.talk_get()
    }

    pub(crate) fn talk_clear(&self) -> Result<Value> {
        let (state, msgs) = self.talk_load()?;
        if matches!(state.as_str(), "asked" | "running") {
            bail!("Claude is still answering");
        }
        // Approved ideas outlive the conversation: keep their messages.
        let keep: Vec<Value> = msgs.into_iter().filter(|m| m["idea"]["status"] == "approved").collect();
        self.talk_save("", &keep)?;
        self.talk_get()
    }

    /// Every approved idea, oldest first: what a coding session should do next.
    pub(crate) fn talk_ideas(&self) -> Result<Value> {
        let (_, msgs) = self.talk_load()?;
        Ok(json!(msgs.iter().filter(|m| m["idea"]["status"] == "approved").map(|m| json!({"at": m["at"], "title": m["idea"]["title"], "detail": m["idea"]["detail"]})).collect::<Vec<_>>()))
    }

    /// Serves a waiting chat message (server side). True when it ran Claude.
    pub(crate) fn job_talk(&self) -> Result<bool> {
        let (state, mut msgs) = self.talk_load()?;
        if state != "asked" {
            return Ok(false);
        }
        self.talk_save("running", &msgs)?;
        let fail = |msgs: &mut Vec<Value>, why: String| -> Result<bool> {
            msgs.push(json!({"role": "error", "text": why, "at": now_ms()}));
            self.talk_save("", msgs)?;
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
                self.talk_save("", &msgs)?;
                Ok(true)
            }
            Err(e) => fail(&mut msgs, format!("{e:#}")),
        }
    }
}
