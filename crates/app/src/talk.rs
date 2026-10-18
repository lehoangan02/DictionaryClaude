//! The general chat with Claude: ask about the app, report problems, suggest changes.
//! Each chat is a synced `talks` row. The server answers with `claude -p` in a
//! read-only view of the app's source for the old "ask" chats; "work" chats (every new chat) have full
//! permissions on the server: Claude changes the app on a private branch, builds and tests it, and the
//! owner's Merge button brings it in. After a merge or release the server rebuilds and restarts itself
//! when its own code changed (`update_server`). Ideas from ask chats are approved or dismissed (`talk_ideas`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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

/// Without this, the model sometimes guessed a language from the owner's name or the repo and answered in Vietnamese.
/// Said last in each prompt: in the middle it was missed, and a reply to an English message came in German.
const LANGUAGE: &str = "LANGUAGE: write `reply` in English. Only when the owner's last message is written entirely in another language (for example Vietnamese) use that language instead. \
     Never choose a language from one word, a greeting or slang (\"Dude\" is English), the owner's name, e-mail or the project's files; never answer in German.";

/// Chats whose run is in flight on this server, so the job runner never starts one twice.
static IN_FLIGHT: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Merging and releasing change the server's checkout: one at a time.
static REPO: Mutex<()> = Mutex::new(());
/// Making or removing private copies (git worktrees): one at a time, but never behind a release, which holds
/// `REPO` for up to hours, so a new work chat no longer waits for it. Taken after `REPO` when both are needed.
static WORKTREES: Mutex<()> = Mutex::new(());
/// The server's counters at the previous sample: (when, cpu total, cpu idle, bytes received, bytes sent).
static LAST_LOAD: Mutex<Option<(std::time::Instant, u64, u64, u64, u64)>> = Mutex::new(None);
/// How many load samples a running chat keeps (one every few seconds).
const LOAD_KEPT: usize = 90;
/// Set once the server has installed a new build of itself: it restarts as soon as nothing is running.
static RESTART: AtomicBool = AtomicBool::new(false);
/// How many times a release is run before its failure is reported.
const RELEASE_TRIES: u32 = 3;

/// One reading of the server's load, like GNOME's System Monitor: CPU %, memory %, and network bytes/s in and out.
/// Linux only (reads /proc); `None` elsewhere and on the first call, which only sets the baseline.
fn read_load() -> Option<Value> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let cpu: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let (total, idle) = (cpu.iter().take(8).sum::<u64>(), cpu.get(3)? + cpu.get(4).unwrap_or(&0));
    let (mut rx, mut tx) = (0u64, 0u64);
    for l in std::fs::read_to_string("/proc/net/dev").ok()?.lines().skip(2) {
        let Some((name, rest)) = l.split_once(':') else { continue };
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if name.trim() != "lo" && f.len() >= 9 {
            rx += f[0];
            tx += f[8];
        }
    }
    let mem = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok());
    let used = 100.0 * (1.0 - kb("MemAvailable:")? / kb("MemTotal:")?.max(1.0));
    let now = std::time::Instant::now();
    let prev = LAST_LOAD.lock().unwrap_or_else(|e| e.into_inner()).replace((now, total, idle, rx, tx))?;
    let secs = now.duration_since(prev.0).as_secs_f64().max(0.1);
    let busy = 100.0 * (1.0 - idle.saturating_sub(prev.2) as f64 / total.saturating_sub(prev.1).max(1) as f64);
    Some(json!({"at": now_ms(), "cpu": busy.clamp(0.0, 100.0).round(), "mem": used.clamp(0.0, 100.0).round(), "rx": (rx.saturating_sub(prev.3) as f64 / secs).round(), "tx": (tx.saturating_sub(prev.4) as f64 / secs).round()}))
}

/// A message sent while Claude was busy: it waits, editable, until the running answer is done.
fn is_queued(m: &Value) -> bool {
    m["queued"] == true
}

fn prompt(msgs: &[Value]) -> String {
    let sent: Vec<&Value> = msgs.iter().filter(|m| !is_queued(m)).collect();
    let talk: String = sent
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
         Answer the owner's last message briefly and plainly. \
         When they report a bug, ask for a feature or a change, also give `idea`: a short `title` and a `detail` a programmer could act on \
         (what to change and why, naming the files if you know them). Give no idea for a plain question. \
         The conversation so far:\n{talk}\n\
         {LANGUAGE}\n\
         Reply with JSON only: {{\"reply\":string,\"idea\":{{\"title\":string,\"detail\":string}} (optional)}}"
    )
}

/// Models and effort levels the chat may ask for.
pub const MODELS: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The model and effort the owner chose on their latest message.
fn last_pick(msgs: &[Value]) -> (Option<String>, Option<String>) {
    let m = msgs.iter().rev().find(|m| m["role"] == "user" && !is_queued(m));
    let get = |k: &str, ok: &[&str]| m.and_then(|m| m[k].as_str()).filter(|v| ok.contains(v)).map(str::to_string);
    (get("model", &MODELS), get("effort", &EFFORTS))
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| anyhow!("git is not available ({e})"))?;
    if !o.status.success() {
        bail!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn work_prompt(msgs: &[Value], branch: &str, repo: &Path) -> String {
    let sent: Vec<&Value> = msgs.iter().filter(|m| !is_queued(m)).collect();
    let talk: String = sent
        .iter()
        .rev()
        .take(16)
        .rev()
        .filter_map(|m| Some(format!("{}: {}", if m["role"] == "user" { "OWNER" } else { "YOU" }, m["text"].as_str()?)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are working on LadyBird, the owner's offline GRE dictionary app, at the owner's request from their phone. You run on the owner's own Ubuntu server \
         with full permissions; it also runs the app's sync server (gredict-syncd) and builds the releases. The owner is the app's only developer: when they ask for something, \
         do it yourself; never tell them to run a command you can run. \
         The current folder is a private git worktree on branch {branch}: make code changes here. The server's main checkout is {repo} (branch main): \
         work there whenever the owner asks or it is in the way (inspect it, undo leftovers of a failed release, commit what belongs there), but put code changes on your branch. \
         Read CLAUDE.md first and follow it (never commit private/ or copyrighted text, never print keys or tokens, keep the style of the surrounding code). \
         If the owner only asks a question or chats, just answer and change nothing. Otherwise do what they asked in their last message, carefully and completely, and keep docs/ROADMAP.md true. \
         Check your work before you commit: `cargo check` / `cargo test -p <crate>` for Rust (chats share a build folder, so the first build is slow), `npm run check` in app/ui \
         for the interface (node_modules is linked in). Do not run scripts/release.sh, push, or restart the sync server: the owner's buttons do that. \
         When you changed files, commit all your changes on this branch (one clear message). Never switch branches and never merge your branch: the owner's Merge button merges it, \
         the server then rebuilds and restarts itself when its own code (crates/) changed, and Release publishes the apps. \
         Then reply briefly and plainly, saying what you changed, what you checked, and anything they should look at. \
         The conversation so far:\n{talk}\n\
         {LANGUAGE}\n\
         Reply with JSON only: {{\"reply\":string}}",
        repo = repo.display()
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

    /// Saves the chat. Queued messages are the owner's own and may change while a run is in flight,
    /// so whatever the database holds of them wins over the copy the caller loaded earlier.
    fn talk_save(&self, id: &str, state: &str, msgs: &[Value]) -> Result<()> {
        let db = self.db.lock().unwrap();
        let held: String = db.query_row("SELECT msgs FROM talks WHERE id=?1", [id], |r| r.get(0)).unwrap_or_default();
        let mut all: Vec<Value> = msgs.iter().filter(|m| !is_queued(m)).cloned().collect();
        all.extend(serde_json::from_str::<Vec<Value>>(&held).unwrap_or_default().into_iter().filter(is_queued));
        db.execute("UPDATE talks SET state=?2, msgs=?3, updated=?4 WHERE id=?1", params![id, state, json!(all).to_string(), now_ms()])?;
        Ok(())
    }

    /// Changes the messages of a chat without touching its state, under one lock so a run finishing meanwhile cannot be undone.
    fn talk_edit_msgs(&self, id: &str, f: impl FnOnce(&str, &mut Vec<Value>) -> Result<()>) -> Result<()> {
        let db = self.db.lock().unwrap();
        let (state, held): (String, String) = db.query_row("SELECT state, msgs FROM talks WHERE id=?1 AND deleted=0", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|_| anyhow!("no such chat"))?;
        let mut msgs: Vec<Value> = serde_json::from_str(&held).unwrap_or_default();
        f(&state, &mut msgs)?;
        db.execute("UPDATE talks SET msgs=?2, updated=?3 WHERE id=?1", params![id, json!(msgs).to_string(), now_ms()])?;
        Ok(())
    }

    /// Changes the text of a queued message.
    pub(crate) fn talk_queue_edit(&self, id: &str, at: i64, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.len() > 2000 {
            bail!("write a message of up to 2000 characters");
        }
        self.talk_edit_msgs(id, |_, msgs| {
            let m = msgs.iter_mut().find(|m| m["at"] == at && is_queued(m)).ok_or_else(|| anyhow!("that message is no longer queued"))?;
            if message.is_empty() && m["photos"].as_array().map_or(true, |p| p.is_empty()) {
                bail!("a queued message needs text or a photo; remove it instead");
            }
            m["text"] = json!(message);
            Ok(())
        })?;
        self.talk_get(id)
    }

    /// Takes a message out of the queue.
    pub(crate) fn talk_queue_remove(&self, id: &str, at: i64) -> Result<Value> {
        self.talk_edit_msgs(id, |_, msgs| {
            let n = msgs.len();
            msgs.retain(|m| !(m["at"] == at && is_queued(m)));
            if msgs.len() == n {
                bail!("that message is no longer queued");
            }
            Ok(())
        })?;
        self.talk_get(id)
    }

    /// Sends the oldest queued message now that nothing is running.
    pub(crate) fn talk_next(&self, id: &str) -> Result<Value> {
        self.talk_edit_msgs(id, |state, msgs| {
            if !state.is_empty() {
                bail!("Claude is still busy");
            }
            let m = msgs.iter_mut().find(|m| is_queued(m)).ok_or_else(|| anyhow!("nothing is queued"))?;
            if let Some(o) = m.as_object_mut() {
                o.remove("queued");
            }
            m["at"] = json!(now_ms());
            Ok(())
        })?;
        self.db.lock().unwrap().execute("UPDATE talks SET state='asked' WHERE id=?1 AND state=''", [id])?;
        self.talk_get(id)
    }

    pub(crate) fn talk_get(&self, id: &str) -> Result<Value> {
        let (title, state, msgs) = self.talk_load(id)?;
        let (mode, branch, changes) = self.talk_meta(id)?;
        let stale = self.db.lock().unwrap().query_row("SELECT updated FROM talks WHERE id=?1", [id], |r| r.get::<_, i64>(0)).unwrap_or(0);
        // When the current run began: the question being answered, or (for merge/release) the moment it was asked for.
        // A release or merge keeps its own start: the load samples refresh `updated` every few seconds.
        let began = msgs.iter().find(|m| m["role"] == "activity").and_then(|m| m["began"].as_i64());
        let started = if matches!(state.as_str(), "asked" | "running") { msgs.iter().rev().find(|m| m["role"] == "user" && !is_queued(m)).and_then(|m| m["at"].as_i64()).unwrap_or(stale) } else { began.unwrap_or(stale) };
        Ok(json!({"id": id, "title": title, "state": state, "msgs": msgs, "mode": mode, "branch": branch, "changes": changes, "updated": stale, "started": started}))
    }

    pub(crate) fn talk_list(&self) -> Result<Value> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, title, state, updated, msgs, mode FROM talks WHERE deleted=0 ORDER BY updated DESC")?;
        let rows = st
            .query_map([], |r| {
                let msgs: String = r.get(4)?;
                let last = serde_json::from_str::<Vec<Value>>(&msgs).ok().and_then(|m| m.iter().rev().find_map(|x| x["text"].as_str().map(|t| t.chars().take(90).collect::<String>()))).unwrap_or_default();
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

    pub(crate) fn talk_send(&self, id: &str, message: &str, photos: &[String], model: Option<&str>, effort: Option<&str>) -> Result<Value> {
        if model.is_some_and(|m| !MODELS.contains(&m)) || effort.is_some_and(|e| !EFFORTS.contains(&e)) {
            bail!("unknown model or effort");
        }
        let message = message.trim();
        if message.len() > 2000 || (message.is_empty() && photos.is_empty()) || photos.len() > 4 {
            bail!("write a message of up to 2000 characters, with up to 4 photos");
        }
        let (title, state, mut msgs) = self.talk_load(id)?;
        let busy = matches!(state.as_str(), "asked" | "running");
        if !busy && !state.is_empty() {
            bail!("Claude is busy with a merge or a release; wait for it to finish");
        }
        let names = photos.iter().map(|p| self.photo_save(p)).collect::<Result<Vec<_>>>()?;
        let mut m = json!({"role": "user", "text": message, "at": now_ms()});
        if !names.is_empty() {
            m["photos"] = json!(names);
        }
        if let Some(v) = model {
            m["model"] = json!(v);
        }
        if let Some(v) = effort {
            m["effort"] = json!(v);
        }
        if busy {
            // Claude is answering: the message waits in the queue (editable) and goes out when the answer is in.
            m["queued"] = json!(true);
            self.talk_edit_msgs(id, |state, msgs| {
                if !matches!(state, "asked" | "running") {
                    bail!("Claude has just finished; send the message again");
                }
                msgs.push(m);
                Ok(())
            })?;
            return self.talk_get(id);
        }
        msgs.push(m);
        if title.is_empty() {
            let t: String = if message.is_empty() { "Photo".into() } else { message.chars().take(40).collect() };
            self.db.lock().unwrap().execute("UPDATE talks SET title=?2 WHERE id=?1", params![id, t])?;
        }
        // Typing "release it" is the owner's own trigger for a release: no agent call is needed.
        if let Some(how) = release_request(message).filter(|_| self.talk_meta(id).is_ok_and(|(_, branch, _)| branch.is_empty())) {
            self.talk_save(id, how, &msgs)?;
            return self.talk_get(id);
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
        if !matches!(how, "merge" | "discard" | "release_patch" | "release_minor") {
            bail!("how must be merge, discard, release_patch or release_minor");
        }
        let (_, state, mut msgs) = self.talk_load(id)?;
        let (_, branch, _) = self.talk_meta(id)?;
        let release = how.starts_with("release");
        if !state.is_empty() || (branch.is_empty() != release) {
            bail!("there is nothing to {how} right now");
        }
        if release {
            let offer = msgs.iter_mut().rev().find(|m| m["offer"] == "release").ok_or_else(|| anyhow!("there is nothing to release"))?;
            offer["offer"] = json!("used");
        }
        self.talk_save(id, how, &msgs)?;
        self.talk_get(id)
    }

    /// Gives up waiting for a chat that never finished (the server away, crashed or hung): the chat is usable again.
    /// A run that is somehow still alive on the server may yet add its result.
    pub(crate) fn talk_stop(&self, id: &str) -> Result<Value> {
        let (_, state, mut msgs) = self.talk_load(id)?;
        if state.is_empty() {
            bail!("nothing is waiting");
        }
        msgs.retain(|m| m["role"] != "activity");
        msgs.push(json!({"role": "error", "text": "Stopped waiting. If the server is still working on it, the result may yet appear.", "at": now_ms()}));
        self.talk_save(id, "", &msgs)?;
        self.talk_get(id)
    }

    /// Server start: a run that was in progress died with the old process. Say so instead of waiting for ever.
    pub fn recover_talks(&self) -> Result<usize> {
        let ids: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id FROM talks WHERE state IN ('running','releasing') AND deleted=0")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        for id in &ids {
            let (_, _, mut msgs) = self.talk_load(id)?;
            msgs.retain(|m| m["role"] != "activity");
            msgs.push(json!({"role": "error", "text": "The server restarted while this was running. Send the message again.", "at": now_ms()}));
            self.talk_save(id, "", &msgs)?;
        }
        Ok(ids.len())
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

    /// Copies the photos of the latest message into a scratch folder Claude may read.
    /// Returns (folder, the sentence for the prompt; empty without photos).
    fn attach_photos(&self, id: &str, msgs: &[Value]) -> (Option<PathBuf>, String) {
        let names: Vec<String> = msgs.iter().rev().find(|m| m["role"] == "user" && !is_queued(m)).map(crate::drafts::talk_photos).unwrap_or_default();
        if names.is_empty() {
            return (None, String::new());
        }
        let dir = self.data_dir.join("jobs").join(format!("talk-{}", id.chars().take(8).collect::<String>()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut paths = vec![];
        if std::fs::create_dir_all(&dir).is_ok() {
            for (i, n) in names.iter().filter(|n| crate::drafts::valid_photo_name(n)).enumerate() {
                let to = dir.join(format!("photo{}.{}", i + 1, n.rsplit('.').next().unwrap_or("jpg")));
                if self.photos_dir().is_ok_and(|d| std::fs::copy(d.join(n), &to).is_ok()) {
                    paths.push(to);
                }
            }
        }
        let note = if paths.is_empty() {
            "\nThe owner attached photos but they have not reached this server; say so.".to_string()
        } else {
            format!("\nThe owner attached these photos to their last message; look at them with the Read tool: {}.", paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
        };
        (Some(dir), note)
    }

    /// Shows what Claude is doing: the steps so far ride along as a temporary last message of the running chat.
    fn talk_progress(&self, id: &str, msgs: &[Value], steps: &[Value]) {
        let mut shown = msgs.to_vec();
        let load = self.talk_load(id).ok().and_then(|(_, _, held)| held.into_iter().find(|m| m["role"] == "activity")).map(|m| m["load"].clone()).unwrap_or(json!([]));
        shown.push(json!({"role": "activity", "steps": steps, "load": load, "at": now_ms()}));
        let _ = self.talk_save(id, "running", &shown);
    }

    /// Serves waiting chat messages (server side). Returns how many ran Claude.
    pub(crate) fn job_talks(&self) -> Result<usize> {
        let mut ran = 0;
        for id in self.talks_waiting()? {
            ran += self.serve_talk(&id)?;
        }
        Ok(ran)
    }

    fn talks_waiting(&self) -> Result<Vec<String>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id FROM talks WHERE state IN ('asked','merge','discard','release_patch','release_minor') AND deleted=0 ORDER BY updated")?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(rows)
    }

    /// Serves one chat that is waiting. Returns 1 when it ran Claude.
    fn serve_talk(&self, id: &str) -> Result<usize> {
        let state = self.talk_load(id)?.1;
        Ok(match state.as_str() {
            "asked" => {
                let ran = self.job_talk(id)? as usize;
                // Messages queued meanwhile go out one after another.
                let _ = self.talk_next(id);
                ran
            }
            how if how.starts_with("release") => {
                let _repo = REPO.lock().unwrap_or_else(|e| e.into_inner());
                self.job_release(id, how)?;
                0
            }
            how => {
                let _repo = REPO.lock().unwrap_or_else(|e| e.into_inner());
                let _copies = WORKTREES.lock().unwrap_or_else(|e| e.into_inner());
                self.job_finish(id, how)?;
                0
            }
        })
    }

    /// Starts every waiting chat on its own thread, so a slow answer in one chat never holds up the others.
    /// A chat already being served is left alone; the runner calls this every few seconds.
    pub fn spawn_talks(self: &Arc<Self>) {
        self.sample_load();
        self.restart_if_idle();
        for id in self.talks_waiting().unwrap_or_default() {
            {
                let mut busy = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
                if busy.contains(&id) {
                    continue;
                }
                busy.push(id.clone());
            }
            let api = self.clone();
            std::thread::spawn(move || {
                let _ = api.serve_talk(&id);
                IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x != &id);
            });
        }
    }

    /// Adds a reading of the server's load to every chat that is running or releasing, so the phone can draw
    /// live graphs and the owner sees whether the server is compiling, building, uploading or downloading.
    fn sample_load(&self) {
        let Some(sample) = read_load() else { return };
        let ids: Vec<String> = {
            let db = self.db.lock().unwrap();
            let Ok(mut st) = db.prepare("SELECT id FROM talks WHERE state IN ('running','releasing') AND deleted=0") else { return };
            let rows = st.query_map([], |r| r.get(0)).map(|r| r.flatten().collect()).unwrap_or_default();
            rows
        };
        for id in ids {
            let _ = self.talk_edit_msgs(&id, |state, msgs| {
                if !matches!(state, "running" | "releasing") {
                    return Ok(());
                }
                let i = match msgs.iter().position(|m| m["role"] == "activity") {
                    Some(i) => i,
                    None => {
                        msgs.push(json!({"role": "activity", "steps": [], "load": [], "began": now_ms(), "at": now_ms()}));
                        msgs.len() - 1
                    }
                };
                let mut load = msgs[i]["load"].as_array().cloned().unwrap_or_default();
                load.push(sample.clone());
                let old = load.len().saturating_sub(LOAD_KEPT);
                msgs[i]["load"] = json!(load.split_off(old));
                Ok(())
            });
        }
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
                // A release that failed midway leaves its version bump behind; that is never worth keeping,
                // so it is undone here (unless a release is running right now).
                if !git(&repo, &["status", "--porcelain"])?.is_empty() {
                    let releasing: i64 = self.db.lock().unwrap()
                        .query_row("SELECT COUNT(*) FROM talks WHERE state='releasing' AND deleted=0", [], |r| r.get(0))
                        .unwrap_or(0);
                    if releasing == 0 {
                        for f in ["app/src-tauri/tauri.conf.json", "app/src-tauri/Cargo.toml", "app/src-tauri/Cargo.lock", "Cargo.lock"] {
                            let _ = git(&repo, &["checkout", "--", f]);
                        }
                    }
                }
                let left = git(&repo, &["status", "--short"])?;
                if !left.is_empty() {
                    bail!("the app's folder on the server has changes that are not committed yet; try again after they are:\n{left}");
                }
                if let Err(e) = git(&repo, &["merge", "--no-ff", "-m", &format!("Merge chat work: {title}"), &branch]) {
                    let _ = git(&repo, &["merge", "--abort"]);
                    bail!("it does not merge cleanly ({e}); the work is still on branch {branch}");
                }
                let _ = git(&repo, &["branch", "-d", &branch]);
                Ok("Merged into the app's main branch on the server. Press Release to build and publish it.".into())
            } else {
                let _ = git(&repo, &["branch", "-D", &branch]);
                Ok("Discarded.".into())
            }
        })();
        match done {
            Ok(t) => {
                say(&mut msgs, "claude", t);
                if how == "merge" {
                    msgs.last_mut().unwrap()["offer"] = json!("release");
                }
                self.db.lock().unwrap().execute("UPDATE talks SET branch='', changes='' WHERE id=?1", [id])?;
                self.talk_save(id, "", &msgs)?;
                if how == "merge" {
                    self.update_server(id, &repo);
                }
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

    /// Builds and publishes a release of what is merged (`scripts/release.sh --publish`). Server side;
    /// it takes many minutes, and the chat shows `releasing` until it ends. Only the owner's button starts it.
    fn job_release(&self, id: &str, how: &str) -> Result<()> {
        let (_, _, mut msgs) = self.talk_load(id)?;
        self.talk_save(id, "releasing", &msgs)?;
        let result: Result<String> = (|| {
            let repo = self.repo_dir().ok_or_else(|| anyhow!("the app's source is not on this server"))?;
            let tag = git(&repo, &["describe", "--tags", "--abbrev=0", "--match", "v*"])?;
            let n: Vec<u32> = tag.trim_start_matches('v').split('.').filter_map(|p| p.parse().ok()).collect();
            if n.len() != 3 || n[0] != 0 {
                bail!("the last tag {tag} is not 0.MINOR.PATCH");
            }
            let version = if how == "release_minor" { format!("0.{}.0", n[1] + 1) } else { format!("0.{}.{}", n[1], n[2] + 1) };
            let tail = |s: &str| s.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            // A failed run is tried again (a flaky test or a network hiccup should not need the owner),
            // but only while nothing was tagged: after the tag, a rerun would refuse or publish twice.
            for attempt in 1..=RELEASE_TRIES {
                let mut c = Command::new("bash");
                c.current_dir(&repo).arg("scripts/release.sh").arg(&version).arg("--publish");
                let r = crate::jobs::run(c, std::time::Duration::from_secs(5400))?;
                if r.ok {
                    return Ok(format!("Released {version}. The apps offer it as an update; the Mac build is attached a few minutes later."));
                }
                let why = format!("the release {version} failed:\n{}\n{}", tail(&r.stdout), tail(&r.stderr));
                let tagged = git(&repo, &["rev-parse", "-q", "--verify", &format!("refs/tags/v{version}")]).is_ok();
                // The script sets the version before building; undo that so the next run (or the next merge) starts from a clean tree.
                if !tagged {
                    let _ = git(&repo, &["checkout", "--", "app/src-tauri/tauri.conf.json", "app/src-tauri/Cargo.toml", "app/src-tauri/Cargo.lock"]);
                }
                if tagged || attempt == RELEASE_TRIES {
                    bail!("{why}");
                }
                let _ = self.talk_edit_msgs(id, |_, m| {
                    let at = m.iter().position(|x| x["role"] == "activity").unwrap_or(m.len());
                    m.insert(at, json!({"role": "claude", "text": format!("{why}\n\nTrying again ({} of {RELEASE_TRIES}).", attempt + 1), "at": now_ms()}));
                    Ok(())
                });
            }
            unreachable!()
        })();
        let released = result.is_ok();
        // Keep the retry notes written meanwhile; the load graphs go once the release ends.
        if let Ok((_, _, now)) = self.talk_load(id) {
            msgs = now;
        }
        msgs.retain(|m| m["role"] != "activity");
        match result {
            Ok(t) => msgs.push(json!({"role": "claude", "text": t, "at": now_ms()})),
            Err(e) => {
                if let Some(m) = msgs.iter_mut().rev().find(|m| m["offer"] == "used") {
                    m["offer"] = json!("release");
                }
                msgs.push(json!({"role": "error", "text": format!("{e:#}"), "at": now_ms()}));
            }
        }
        self.talk_save(id, "", &msgs)?;
        if released {
            if let Some(repo) = self.repo_dir() {
                self.update_server(id, &repo);
            }
        }
        Ok(())
    }

    /// After a merge or release: when the server's own code changed since the build it runs, rebuild `gredict-syncd`,
    /// put it over `LEXIS_SELF_UPDATE` (set only by the systemd unit, so tests never do this) and restart once idle.
    /// What happened is added to the chat.
    fn update_server(&self, id: &str, repo: &Path) {
        let Some(bin) = std::env::var_os("LEXIS_SELF_UPDATE").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) else { return };
        let stamp = bin.with_extension("commit");
        let Ok(head) = git(repo, &["rev-parse", "HEAD"]) else { return };
        let built = std::fs::read_to_string(&stamp).unwrap_or_default().trim().to_string();
        // `git diff --quiet` fails when something differs.
        if !built.is_empty() && git(repo, &["diff", "--quiet", &built, &head, "--", "crates", "Cargo.toml", "Cargo.lock"]).is_ok() {
            return;
        }
        let say = |role: &str, text: String| {
            let _ = self.talk_edit_msgs(id, |_, m| {
                m.push(json!({"role": role, "text": text, "at": now_ms()}));
                Ok(())
            });
        };
        let done: Result<()> = (|| {
            let mut c = Command::new("cargo");
            c.current_dir(repo).env("PATH", crate::jobs::dev_path()).env_remove("CARGO_TARGET_DIR").args(["build", "--release", "-p", "gredict-syncd"]);
            let r = crate::jobs::run(c, std::time::Duration::from_secs(3600))?;
            if !r.ok {
                bail!("{}", r.stderr.lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
            }
            // Renamed over the running program, which keeps running until the restart.
            let new = bin.with_extension("new");
            std::fs::copy(repo.join("target/release/gredict-syncd"), &new)?;
            std::fs::rename(&new, &bin)?;
            std::fs::write(&stamp, &head)?;
            Ok(())
        })();
        match done {
            Ok(()) => {
                RESTART.store(true, Ordering::SeqCst);
                say("claude", "The server's own code changed, so it rebuilt itself and restarts as soon as no chat is running; the app reconnects by itself.".into());
            }
            Err(e) => say("error", format!("The server could not rebuild itself with this change, so it still runs the old code:\n{e:#}")),
        }
    }

    /// Ends the process so systemd starts the new build (`Restart=on-failure`), once no chat, merge, release or agent review runs.
    fn restart_if_idle(&self) {
        if !RESTART.load(Ordering::SeqCst) || !IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            return;
        }
        let busy: i64 = self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT (SELECT COUNT(*) FROM talks WHERE state<>'' AND deleted=0) + (SELECT COUNT(*) FROM drafts WHERE agent_status IN ('running','chat_running') AND deleted=0)", [], |r| r.get(0))
            .unwrap_or(1);
        if busy == 0 {
            eprintln!("restarting to run the new build");
            std::process::exit(75);
        }
    }

    /// A task: Claude edits a private copy of the app on its own branch. True when it ran Claude.
    fn job_work(&self, id: &str, msgs: &mut Vec<Value>, claude: &Agent, attached: &str) -> Result<bool> {
        let fail = |msgs: &mut Vec<Value>, why: String| -> Result<bool> {
            msgs.push(json!({"role": "error", "text": why, "at": now_ms()}));
            self.talk_save(id, "", msgs)?;
            Ok(false)
        };
        let Some(repo) = self.repo_dir() else { return fail(msgs, "the app's source is not on this server, so Claude cannot change it from here".into()) };
        let (branch, path) = self.work_paths(id);
        let made: Result<()> = (|| {
            let _copies = WORKTREES.lock().unwrap_or_else(|e| e.into_inner());
            if !path.join(".git").exists() {
                std::fs::create_dir_all(path.parent().unwrap())?;
                let p = path.to_string_lossy().to_string();
                if git(&repo, &["rev-parse", "--verify", &branch]).is_ok() {
                    git(&repo, &["worktree", "add", &p, &branch])?;
                } else {
                    git(&repo, &["worktree", "add", "-b", &branch, &p, "HEAD"])?;
                }
            }
            // So `npm run check` works in the copy without a fresh install (node_modules is not in git).
            let nm = path.join("app/ui/node_modules");
            if !nm.exists() && repo.join("app/ui/node_modules").is_dir() {
                let _ = std::os::unix::fs::symlink(repo.join("app/ui/node_modules"), &nm);
            }
            Ok(())
        })();
        if let Err(e) = made {
            return fail(msgs, format!("could not prepare a private copy of the app: {e:#}"));
        }
        self.db.lock().unwrap().execute("UPDATE talks SET branch=?2 WHERE id=?1", params![id, branch])?;
        self.count_agent_call();
        let mut steps: Vec<Value> = vec![];
        let mut live = |s: String| {
            let (k, t) = s.split_once('|').unwrap_or(("run", &s));
            steps.push(json!({"kind": k, "text": t, "at": now_ms()}));
            if steps.len() > 80 {
                steps.remove(0);
            }
            self.talk_progress(id, msgs, &steps);
        };
        let out = claude.run_work(&path, &[repo.as_path()], &self.data_dir.join("work").join("target"), &(work_prompt(msgs, &branch, &repo) + attached), SCHEMA, Some(&mut live)).and_then(|o| extract_talk(&o).ok_or_else(|| anyhow!("claude replied, but not in the expected form")));
        // Whatever was left uncommitted is committed too, so the summary below is complete.
        if !git(&path, &["status", "--porcelain"]).unwrap_or_default().is_empty() {
            let _ = git(&path, &["add", "-A"]);
            let _ = git(&path, &["commit", "-q", "-m", "Work from the Claude chat"]);
        }
        let base = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|_| "HEAD".into());
        let stat = git(&repo, &["diff", "--stat", &format!("{base}...{branch}")]).unwrap_or_default();
        if stat.is_empty() {
            // A plain question, or a run that failed before changing anything: leave no branch or private copy behind.
            let _copies = WORKTREES.lock().unwrap_or_else(|e| e.into_inner());
            let _ = git(&repo, &["worktree", "remove", "--force", &path.to_string_lossy()]);
            let _ = git(&repo, &["branch", "-D", &branch]);
            self.db.lock().unwrap().execute("UPDATE talks SET branch='', changes='' WHERE id=?1", [id])?;
        } else {
            self.db.lock().unwrap().execute("UPDATE talks SET changes=?2 WHERE id=?1", params![id, stat])?;
        }
        match out {
            Ok((reply, _)) => {
                let mut m = json!({"role": "claude", "text": reply, "at": now_ms()});
                if !steps.is_empty() {
                    m["steps"] = json!(steps);
                }
                msgs.push(m);
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
        let (dir, scratch) = match self.repo_dir() {
            Some(d) => (d, false),
            None => {
                let d = self.data_dir.join("jobs").join("talk");
                std::fs::create_dir_all(&d)?;
                (d, true)
            }
        };
        let (model, effort) = last_pick(&msgs);
        let (attach_dir, note) = self.attach_photos(id, &msgs);
        let claude = Agent { name: "claude", bin: self.env_or("LEXIS_CLAUDE_BIN", "claude"), model, effort, extra: attach_dir.clone() };
        let done = |r: Result<bool>| {
            if let Some(d) = &attach_dir {
                let _ = std::fs::remove_dir_all(d);
            }
            r
        };
        if self.talk_meta(id)?.0 == "work" {
            if self.agent_used_today() + 1 > self.agent_cap() {
                let why = format!("the daily agent allowance ({}) is used up; raise it in Settings or try tomorrow", self.agent_cap());
                if let Some(d) = &attach_dir {
                    let _ = std::fs::remove_dir_all(d);
                }
                return fail(&mut msgs, why);
            }
            return done(self.job_work(id, &mut msgs, &claude, &note));
        }
        let mut p = prompt(&msgs) + &note;
        if scratch {
            p.push_str("\nThe source is NOT available on this server: say so if asked about code, and answer from general knowledge.");
        }
        let mut steps: Vec<Value> = vec![];
        let mut live = |s: String| {
            let (k, t) = s.split_once('|').unwrap_or(("run", &s));
            steps.push(json!({"kind": k, "text": t, "at": now_ms()}));
            if steps.len() > 80 {
                steps.remove(0);
            }
            self.talk_progress(id, &msgs, &steps);
        };
        let out = claude.run_cli(&dir, &p, SCHEMA, None, Some(&mut live)).and_then(|o| extract_talk(&o).ok_or_else(|| anyhow!("claude replied, but not in the expected form")));
        if let Some(d) = &attach_dir {
            let _ = std::fs::remove_dir_all(d);
        }
        match out {
            Ok((reply, idea)) => {
                let mut m = json!({"role": "claude", "text": reply, "at": now_ms()});
                if !steps.is_empty() {
                    m["steps"] = json!(steps);
                }
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

/// A message that only asks for a release ("Release it", "please release as a new feature").
/// "new feature" or "minor" releases a new feature, anything else a fix.
fn release_request(message: &str) -> Option<&'static str> {
    let t = message.trim().trim_end_matches(['.', '!']).to_lowercase();
    if t.split_whitespace().count() > 8 || !t.split_whitespace().any(|w| w == "release" || w == "publish") {
        return None;
    }
    if t.contains('?') || t.contains("don't") || t.contains("do not") || t.contains("not ") {
        return None;
    }
    Some(if t.contains("feature") || t.contains("minor") { "release_minor" } else { "release_patch" })
}

#[cfg(test)]
mod release_request_tests {
    use super::release_request;
    #[test]
    fn recognises_short_release_orders() {
        assert_eq!(release_request("Release it"), Some("release_patch"));
        assert_eq!(release_request("please release this as a new feature!"), Some("release_minor"));
        assert_eq!(release_request("Can you change the release script?"), None);
        assert_eq!(release_request("don't release yet"), None);
    }
}
