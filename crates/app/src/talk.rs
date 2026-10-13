//! The general chat with Claude: ask about the app, report problems, suggest changes.
//! Each chat is a synced `talks` row. The server answers with `claude -p` in a
//! read-only view of the app's source; Claude never changes anything. What it suggests
//! shows up as an idea the owner approves or dismisses; approved ideas are what a coding
//! session picks up (`talk_ideas`).

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| anyhow!("git is not available ({e})"))?;
    if !o.status.success() {
        bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn work_prompt(msgs: &[Value], branch: &str) -> String {
    let talk: String = msgs
        .iter()
        .rev()
        .take(16)
        .rev()
        .filter_map(|m| Some(format!("{}: {}", if m["role"] == "user" { "OWNER" } else { "YOU" }, m["text"].as_str()?)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are working on LadyBird, the owner's offline GRE dictionary app, at the owner's request from their phone. The current folder is a private git worktree on branch {branch}: \
         change the app here and nowhere else. Read CLAUDE.md first and follow it (never touch private/, never put copyrighted text in the repo, keep the style of the surrounding code). \
         Do the task the owner asked for in their last message, carefully and completely, and keep docs/ROADMAP.md true. \
         Do not build or run tests (they are slow; the owner checks before merging) and run no program other than git. \
         When finished, commit all your changes on this branch with git (one clear message). Never push, never switch or create branches, never merge. \
         Then reply briefly and plainly, in the language the owner used, saying what you changed and anything they should check. \
         The conversation so far:\n{talk}\n\
         Reply with JSON only: {{\"reply\":string}}"
    )
}

impl Api {
    fn work_paths(&self, id: &str) -> (String, PathBuf) {
        let short: String = id.chars().take(8).collect();
        (format!("chat/{short}"), self.data_dir.join("work").join(short))
    }

    /// (mode, branch, changes) of a chat.
    fn talk_meta(&self, id: &str) -> Result<(String, String, String)> {
        let db = self.db.lock().unwrap();
        db.query_row("SELECT mode, branch, changes FROM talks WHERE id=?1 AND deleted=0", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(|_| anyhow!("no such chat"))
    }

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
        let (mode, branch, changes) = self.talk_meta(id)?;
        let stale = self.db.lock().unwrap().query_row("SELECT updated FROM talks WHERE id=?1", [id], |r| r.get::<_, i64>(0)).unwrap_or(0);
        Ok(json!({"id": id, "title": title, "state": state, "msgs": msgs, "mode": mode, "branch": branch, "changes": changes, "updated": stale}))
    }

    pub(crate) fn talk_list(&self) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, title, state, updated, msgs, mode FROM talks WHERE deleted=0 ORDER BY updated DESC")?;
        let rows = st
            .query_map([], |r| {
                let msgs: String = r.get(4)?;
                let last = serde_json::from_str::<Vec<Value>>(&msgs).ok().and_then(|m| m.last().and_then(|x| x["text"].as_str().map(|t| t.chars().take(90).collect::<String>()))).unwrap_or_default();
                Ok(json!({"id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "state": r.get::<_, String>(2)?, "updated": r.get::<_, i64>(3)?, "last": last, "mode": r.get::<_, String>(5)?}))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(json!(rows))
    }

    pub(crate) fn talk_new(&self, mode: &str) -> Result<Value> {
        if !matches!(mode, "ask" | "work") {
            bail!("mode must be ask or work");
        }
        let id = new_id();
        let t = now_ms();
        self.db.lock().unwrap().execute("INSERT INTO talks(id,created,updated,mode) VALUES(?1,?2,?2,?3)", params![id, t, mode])?;
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
        let (_, branch, _) = self.talk_meta(id)?;
        if branch.is_empty() {
            self.db.lock().unwrap().execute("UPDATE talks SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()])?;
        } else {
            // Work not yet merged is thrown away by the server, which then removes the chat.
            let (_, _, msgs) = self.talk_load(id)?;
            self.talk_save(id, "discard", &msgs)?;
        }
        Ok(json!({"ok": true}))
    }

    /// Asks the server to merge (or throw away) the work of a task chat.
    pub(crate) fn talk_finish(&self, id: &str, how: &str) -> Result<Value> {
        if !matches!(how, "merge" | "discard") {
            bail!("how must be merge or discard");
        }
        let (_, state, msgs) = self.talk_load(id)?;
        let (_, branch, _) = self.talk_meta(id)?;
        if branch.is_empty() || !state.is_empty() {
            bail!("there is nothing to {how} right now");
        }
        self.talk_save(id, how, &msgs)?;
        self.talk_get(id)
    }

    /// A message nobody answered (the server was away or too old): ask again.
    pub(crate) fn talk_retry(&self, id: &str) -> Result<Value> {
        let (_, state, msgs) = self.talk_load(id)?;
        if !matches!(state.as_str(), "asked" | "running") {
            bail!("nothing is waiting");
        }
        self.talk_save(id, "asked", &msgs)?;
        self.talk_get(id)
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
            let mut st = db.prepare("SELECT id FROM talks WHERE state IN ('asked','merge','discard') AND deleted=0 ORDER BY updated LIMIT 3")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        let mut ran = 0;
        for id in ids {
            let state = self.talk_load(&id)?.1;
            match state.as_str() {
                "asked" => ran += self.job_talk(&id)? as usize,
                how => self.job_finish(&id, how)?,
            }
        }
        Ok(ran)
    }

    /// Merges the work of a task chat into the app's main branch, or throws it away. Server side.
    fn job_finish(&self, id: &str, how: &str) -> Result<()> {
        let (title, _, mut msgs) = self.talk_load(id)?;
        let (_, branch, _) = self.talk_meta(id)?;
        let path = self.work_paths(id).1;
        let say = |msgs: &mut Vec<Value>, role: &str, text: String| msgs.push(json!({"role": role, "text": text, "at": now_ms()}));
        let Some(repo) = self.repo_dir() else {
            say(&mut msgs, "error", "the app's source is not on this server".into());
            return self.talk_save(id, "", &msgs);
        };
        let _ = git(&repo, &["worktree", "remove", "--force", &path.to_string_lossy()]);
        let done: Result<String> = (|| {
            if how == "merge" {
                if !git(&repo, &["status", "--porcelain"])?.is_empty() {
                    bail!("the app's folder on the server has changes that are not committed yet; try again after they are");
                }
                if let Err(e) = git(&repo, &["merge", "--no-ff", "-m", &format!("Merge chat work: {title}"), &branch]) {
                    let _ = git(&repo, &["merge", "--abort"]);
                    bail!("it does not merge cleanly ({e}); the work is still on branch {branch}");
                }
                let _ = git(&repo, &["branch", "-d", &branch]);
                Ok("Merged into the app's main branch. It goes out with the next release.".into())
            } else {
                let _ = git(&repo, &["branch", "-D", &branch]);
                Ok("Discarded.".into())
            }
        })();
        match done {
            Ok(t) => {
                say(&mut msgs, "claude", t);
                self.db.lock().unwrap().execute("UPDATE talks SET branch='', changes='' WHERE id=?1", [id])?;
                self.talk_save(id, "", &msgs)?;
                // A chat deleted with unmerged work: the branch is gone now, so the chat can go too.
                if how == "discard" {
                    self.db.lock().unwrap().execute("UPDATE talks SET deleted=1, updated=?2 WHERE id=?1", params![id, now_ms()])?;
                }
            }
            Err(e) => {
                say(&mut msgs, "error", format!("{e:#}"));
                self.talk_save(id, "", &msgs)?;
            }
        }
        Ok(())
    }

    /// A task: Claude edits a private copy of the app on its own branch. True when it ran Claude.
    fn job_work(&self, id: &str, msgs: &mut Vec<Value>, claude: &Agent) -> Result<bool> {
        let fail = |msgs: &mut Vec<Value>, why: String| -> Result<bool> {
            msgs.push(json!({"role": "error", "text": why, "at": now_ms()}));
            self.talk_save(id, "", msgs)?;
            Ok(false)
        };
        let Some(repo) = self.repo_dir() else { return fail(msgs, "the app's source is not on this server, so Claude cannot change it from here".into()) };
        let (branch, path) = self.work_paths(id);
        let made: Result<()> = (|| {
            if !path.join(".git").exists() {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let p = path.to_string_lossy().to_string();
                if git(&repo, &["rev-parse", "--verify", &branch]).is_ok() {
                    git(&repo, &["worktree", "add", &p, &branch])?;
                } else {
                    git(&repo, &["worktree", "add", "-b", &branch, &p, "HEAD"])?;
                }
            }
            Ok(())
        })();
        if let Err(e) = made {
            return fail(msgs, format!("could not prepare a private copy of the app: {e:#}"));
        }
        self.db.lock().unwrap().execute("UPDATE talks SET branch=?2 WHERE id=?1", params![id, branch])?;
        self.count_agent_call();
        let out = claude.run_work(&path, &work_prompt(msgs, &branch), SCHEMA).and_then(|o| extract_talk(&o).ok_or_else(|| anyhow!("claude replied, but not in the expected form")));
        // Whatever was left uncommitted is committed too, so the summary below is complete.
        if !git(&path, &["status", "--porcelain"]).unwrap_or_default().is_empty() {
            let _ = git(&path, &["add", "-A"]);
            let _ = git(&path, &["commit", "-q", "-m", "Work from the Claude chat"]);
        }
        let base = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "HEAD".into());
        let stat = git(&repo, &["diff", "--stat", &format!("{base}...{branch}")]).unwrap_or_default();
        self.db.lock().unwrap().execute("UPDATE talks SET changes=?2 WHERE id=?1", params![id, stat])?;
        match out {
            Ok((reply, _)) => {
                msgs.push(json!({"role": "claude", "text": reply, "at": now_ms()}));
                self.talk_save(id, "", msgs)?;
                Ok(true)
            }
            Err(e) => fail(msgs, format!("{e:#}")),
        }
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
        if self.talk_meta(id)?.0 == "work" {
            return self.job_work(id, &mut msgs, &claude);
        }
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
