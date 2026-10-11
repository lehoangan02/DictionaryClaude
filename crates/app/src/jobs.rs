//! Reading photographed notes (runs on the server).
//!
//! 1. OCR runs automatically on every new draft and always ends in review: a
//!    machine reading of handwriting is a draft, never a note.
//! 2. "Ask agents" is a separate, person-triggered step (quota is limited) that
//!    asks Claude Code and/or agy to transcribe the photos, lets them reconsider
//!    where they disagree, and offers the result line by line. Agents read the
//!    photos in an empty scratch folder with read-only access and never write
//!    to the person's notes.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde_json::{json, Value};

use crate::api::Api;
use crate::userdb::now_ms;

const AGENT_TIMEOUT: Duration = Duration::from_secs(300);
const OCR_TIMEOUT: Duration = Duration::from_secs(90);

const SCHEMA: &str = r#"{"type":"object","properties":{"lines":{"type":"array","items":{"type":"object","properties":{"text":{"type":"string"},"unsure":{"type":"boolean"},"reason":{"type":"string"}},"required":["text","unsure"]}}},"required":["lines"]}"#;

/// Output captured from a child process, killed if it overruns.
struct Ran {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn run(mut cmd: Command, limit: Duration) -> Result<Ran> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| anyhow!("could not start the program ({e})"))?;
    // Drain both pipes on threads so a chatty child cannot block on a full pipe.
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out after {}s", limit.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    Ok(Ran { stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default(), ok: status.success() })
}

/// Cleans up how OCR (and tired handwriting) renders the relation symbols.
pub fn normalize_symbols(text: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut l = line.trim().to_string();
        for (from, to) in [("—", "="), ("–", " = "), ("==", "="), ("﹦", "="), ("≃", "≈"), ("≅", "≈"), ("~=", "≈"), ("=~", "≈"), ("=/=", "⍯"), ("!=", "⍯"), ("≠", "⍯"), ("<>", "><"), ("> <", "><")] {
            l = l.replace(from, to);
        }
        // A lone tilde between words is how a hand-drawn ≈ often reads.
        l = l.replace(" ~ ", " ≈ ");
        out.push(l.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    out.join("\n").trim().to_string()
}

/// What the vision model is told. Handwriting is the point, so it is asked for a literal copy.
const VISION_PROMPT: &str = "This is a photo of a page from a student's handwritten vocabulary notebook. \
Transcribe every line exactly as written, one note per line, in reading order, keeping the writer's own spelling. \
A line that is indented under another continues that line: join it onto the line above. \
Write the relation symbols the way they are used: = (same meaning), ≈ (similar), >< (opposite), ≠ (different). \
Keep colons, brackets, slashes, Vietnamese words and abbreviations like sth as written. \
Output only the transcription, with no comments, no numbering and no markdown.";

/// Joins the indented second lines of a note back onto it ("court (v): please so to get what you want" /
/// "try to get sth"): a short lowercase line with no colon or relation symbol belongs to the line above.
pub fn join_continuations(text: &str) -> String {
    let marks = [':', '=', '≈', '≠', '⍯', '>', '<'];
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let continuation = !line.contains(marks.as_slice())
            && line.split_whitespace().count() <= 6
            && line.chars().next().is_some_and(char::is_lowercase)
            && out.last().is_some_and(|prev| prev.contains(marks.as_slice()));
        match out.last_mut() {
            Some(prev) if continuation => {
                prev.push(' ');
                prev.push_str(line);
            }
            _ => out.push(line.to_string()),
        }
    }
    out.join("\n")
}

/// Reads a photo with a local vision model served by Ollama (`ollama:<model>`).
fn ocr_ollama(model: &str, image: &Path) -> Result<String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(image)?);
    let base = std::env::var("LEXIS_OLLAMA").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(900)))
        .timeout_connect(Some(Duration::from_secs(3)))
        .http_status_as_error(false)
        .build()
        .into();
    let body = json!({
        "model": model, "stream": false, "keep_alive": "10m",
        "options": {"temperature": 0, "num_predict": 2048},
        "messages": [{"role": "user", "content": VISION_PROMPT, "images": [b64]}],
    });
    let mut resp = agent.post(&format!("{base}/api/chat")).header("Content-Type", "application/json").send(serde_json::to_vec(&body)?).map_err(|e| anyhow!("Ollama is not reachable ({e})"))?;
    let status = resp.status().as_u16();
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?).unwrap_or(Value::Null);
    if status != 200 {
        bail!("Ollama said: {}", v["error"].as_str().unwrap_or("an error"));
    }
    let text = v["message"]["content"].as_str().unwrap_or("").trim().trim_start_matches("```").trim_end_matches("```").trim().to_string();
    if text.is_empty() {
        bail!("the vision model returned nothing");
    }
    Ok(join_continuations(&text))
}

fn ocr_one(engine: &str, image: &Path) -> Result<String> {
    match engine {
        "none" => bail!("OCR is turned off"),
        // The default: a local vision model reads handwriting far better than tesseract,
        // which stays as the fallback when Ollama or the model is missing.
        "auto" => {
            let model = std::env::var("LEXIS_OCR_MODEL").unwrap_or_else(|_| "qwen2.5vl:3b".into());
            ocr_ollama(&model, image).or_else(|e| {
                eprintln!("vision OCR unavailable ({e:#}); using tesseract");
                ocr_one("tesseract", image)
            })
        }
        e if e.starts_with("ollama:") => ocr_ollama(&e[7..], image),
        "tesseract" => {
            let mut c = Command::new("tesseract");
            c.arg(image).arg("stdout").args(["-l", "eng", "--psm", "6"]);
            let r = run(c, OCR_TIMEOUT)?;
            if !r.ok {
                bail!("tesseract failed: {}", r.stderr.lines().next().unwrap_or(""));
            }
            Ok(r.stdout)
        }
        // "cmd:/path/to/program arg1 {image}"
        cmd if cmd.starts_with("cmd:") => {
            let parts: Vec<String> = cmd[4..].split_whitespace().map(|p| p.replace("{image}", &image.to_string_lossy())).collect();
            let (prog, args) = parts.split_first().ok_or_else(|| anyhow!("empty OCR command"))?;
            let mut c = Command::new(prog);
            c.args(args);
            let r = run(c, OCR_TIMEOUT)?;
            if !r.ok {
                bail!("OCR command failed: {}", r.stderr.lines().next().unwrap_or(""));
            }
            Ok(r.stdout)
        }
        other => bail!("unknown OCR engine `{other}`"),
    }
}

/// An error an agent CLI reported in its JSON output (`{"status":"ERROR","error":...}`
/// or Claude Code's `is_error`), if any.
pub fn reported_error(stdout: &str) -> Option<String> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let failed = v.get("is_error").and_then(Value::as_bool) == Some(true) || v.get("status").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("error"));
    if !failed {
        return None;
    }
    let msg = v.get("error").or_else(|| v.get("result")).and_then(Value::as_str).unwrap_or("it reported an error");
    Some(msg.to_string())
}

/// The agents' reply: JSON somewhere in their output, with a `lines` array.
pub fn extract_lines(stdout: &str) -> Option<Vec<(String, bool, String)>> {
    fn find(v: &Value, depth: usize) -> Option<Vec<(String, bool, String)>> {
        if depth > 6 {
            return None;
        }
        match v {
            Value::Object(m) => {
                if let Some(arr) = m.get("lines").and_then(Value::as_array) {
                    let lines: Vec<_> = arr
                        .iter()
                        .filter_map(|l| {
                            let t = l.get("text").and_then(Value::as_str).or_else(|| l.as_str())?;
                            Some((t.trim().to_string(), l.get("unsure").and_then(Value::as_bool).unwrap_or(false), l.get("reason").and_then(Value::as_str).unwrap_or("").to_string()))
                        })
                        .filter(|l| !l.0.is_empty())
                        .collect();
                    return Some(lines);
                }
                m.values().find_map(|x| find(x, depth + 1))
            }
            Value::Array(a) => a.iter().find_map(|x| find(x, depth + 1)),
            Value::String(s) => {
                // The reply may arrive as text, possibly inside a ``` fence.
                let t = s.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
                t.starts_with('{').then(|| serde_json::from_str::<Value>(t).ok()).flatten().and_then(|v| find(&v, depth + 1))
            }
            _ => None,
        }
    }
    let t = stdout.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return find(&v, 0);
    }
    // Streaming output: one JSON object per line; the last with lines wins.
    t.lines().rev().find_map(|l| serde_json::from_str::<Value>(l).ok().and_then(|v| find(&v, 0)))
}

fn same_line(a: &str, b: &str) -> bool {
    let n = |s: &str| s.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    n(a) == n(b)
}

/// Lines both agree on are settled; the rest are offered as choices.
pub fn merge(versions: &[(&str, Vec<String>)]) -> Value {
    match versions {
        [] => json!({"lines": [], "alternatives": []}),
        [(who, lines)] => json!({"lines": lines.iter().map(|l| json!({"status": "single", "text": l, "options": [{"from": who, "text": l}]})).collect::<Vec<_>>(), "alternatives": []}),
        [(wa, a), (wb, b), ..] if a.len() == b.len() => {
            let lines: Vec<Value> = a
                .iter()
                .zip(b)
                .map(|(x, y)| {
                    if same_line(x, y) {
                        json!({"status": "agreed", "text": x, "options": [{"from": "both", "text": x}]})
                    } else {
                        json!({"status": "conflict", "text": x, "options": [{"from": wa, "text": x}, {"from": wb, "text": y}]})
                    }
                })
                .collect();
            json!({"lines": lines, "alternatives": []})
        }
        // Different line counts cannot be aligned honestly: offer each whole version.
        vs => json!({"lines": [], "alternatives": vs.iter().map(|(w, l)| json!({"from": w, "text": l.join("\n")})).collect::<Vec<_>>()}),
    }
}

struct Agent {
    name: &'static str,
    bin: String,
}

impl Agent {
    fn call(&self, dir: &Path, prompt: &str, effort: &str) -> Result<Vec<(String, bool, String)>> {
        let mut c = Command::new(&self.bin);
        c.current_dir(dir).arg("-p").arg(prompt).args(["--output-format", "json", "--json-schema", SCHEMA]);
        match self.name {
            // Read only: the Read tool for the photos, nothing that writes or browses.
            "claude" => {
                c.args(["--allowedTools", "Read"]).arg("--add-dir").arg(dir).args(["--disallowedTools", "Bash", "Edit", "Write", "WebFetch", "WebSearch"]);
            }
            // Plan mode keeps agy from editing anything.
            _ => {
                c.args(["--mode", "plan", "--effort", effort]);
            }
        }
        let r = run(c, AGENT_TIMEOUT).map_err(|e| anyhow!("{}: {e:#}", self.name))?;
        // Errors (quota reached, not signed in) come back as JSON on stdout or as text on stderr:
        // say what they say rather than guessing.
        if let Some(why) = reported_error(&r.stdout).or_else(|| (!r.ok).then(|| r.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string())) {
            bail!("{} says: {why}", self.name);
        }
        extract_lines(&r.stdout).ok_or_else(|| anyhow!("{} replied, but not in the expected form", self.name))
    }
}

fn first_prompt(ocr: &str, pages: usize) -> String {
    format!(
        "You are reading photographed handwritten vocabulary notes for a GRE student. The photo{} {} in this folder: {}. \
         Open each with the Read tool and transcribe it exactly, one relation or note per line, in reading order. \
         Use only these symbols between words: '=' same meaning, '≈' roughly similar, '><' opposite, '⍯' different but easily confused (for example imbibition ⍯ osmosis). \
         Keep every word spelled exactly as written; do not correct, add or define anything. Mark a line unsure if any word or symbol is hard to read, and say why briefly. \
         A machine draft that may be wrong: <<<\n{ocr}\n>>>\n\
         Reply with JSON only: {{\"lines\":[{{\"text\":string,\"unsure\":boolean,\"reason\":string}}]}}",
        if pages == 1 { "" } else { "s" },
        if pages == 1 { "is" } else { "are" },
        (1..=pages).map(|i| format!("page{i}.jpg")).collect::<Vec<_>>().join(", "),
    )
}

fn second_prompt(mine: &[String], theirs: &[String]) -> String {
    format!(
        "Another reader transcribed the same photos differently. Look at the photos again and reconsider every line where you differ. \
         Keep your reading where you are sure of it; adopt theirs where they are right.\n\
         YOUR TRANSCRIPTION:\n<<<\n{}\n>>>\nTHEIR TRANSCRIPTION:\n<<<\n{}\n>>>\n\
         Reply with JSON only, the final lines: {{\"lines\":[{{\"text\":string,\"unsure\":boolean,\"reason\":string}}]}}",
        mine.join("\n"),
        theirs.join("\n"),
    )
}

impl Api {
    fn env_or(&self, k: &str, d: &str) -> String {
        std::env::var(k).unwrap_or_else(|_| d.to_string())
    }

    fn agent_used_today(&self) -> i64 {
        self.setting(&format!("agents.used.{}", now_ms() / 86_400_000)).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    fn agent_cap(&self) -> i64 {
        self.setting("agents.cap").and_then(|s| s.parse().ok()).or_else(|| std::env::var("LEXIS_AGENT_CAP").ok().and_then(|s| s.parse().ok())).unwrap_or(8)
    }

    fn count_agent_call(&self) {
        let _ = self.set_setting(&format!("agents.used.{}", now_ms() / 86_400_000), &(self.agent_used_today() + 1).to_string());
    }

    /// Reads every queued draft, then serves agent requests. Safe to call often.
    pub fn run_jobs_once(&self) -> Result<Value> {
        let (mut ocr_done, mut agents_done) = (0, 0);
        let queued: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id FROM drafts WHERE status='queued' AND deleted=0 ORDER BY created LIMIT 5")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        for id in queued {
            self.job_ocr(&id)?;
            ocr_done += 1;
        }
        let asked: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id FROM drafts WHERE agent_status='requested' AND deleted=0 AND status NOT IN ('accepted','rejected') ORDER BY updated LIMIT 2")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        for id in asked {
            self.job_agents(&id)?;
            agents_done += 1;
        }
        Ok(json!({"ocr": ocr_done, "agents": agents_done}))
    }

    fn draft_photos(&self, d: &Value) -> Vec<PathBuf> {
        let dir = self.photos_dir().unwrap_or_default();
        d["photos"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| crate::drafts::valid_photo_name(n)).map(|n| dir.join(n)).filter(|p| p.exists()).collect()
    }

    fn job_ocr(&self, id: &str) -> Result<()> {
        self.draft_set(id, "status='processing'", params![now_ms(), id])?;
        let d = self.draft_get(id)?;
        let photos = self.draft_photos(&d);
        let engine = self.env_or("LEXIS_OCR", "auto");
        let (mut pages, mut error) = (Vec::new(), String::new());
        if photos.is_empty() {
            error = "the photos have not arrived on the server yet".into();
        }
        for p in &photos {
            match ocr_one(&engine, p) {
                Ok(t) => pages.push(normalize_symbols(&t)),
                Err(e) => error = format!("{e:#}"),
            }
        }
        let text = pages.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n");
        // Whatever the OCR found is the starting text, unless the person already typed some.
        let keep = d["text"].as_str().is_some_and(|t| !t.trim().is_empty());
        self.db.lock().unwrap().execute(
            "UPDATE drafts SET status='needs_review', ocr_text=?2, ocr_engine=?3, text=CASE WHEN ?4 THEN text ELSE ?2 END, error=?5, updated=?6 WHERE id=?1",
            params![id, text, engine, keep, error, now_ms()],
        )?;
        Ok(())
    }

    fn job_agents(&self, id: &str) -> Result<()> {
        let d = self.draft_get(id)?;
        let req = d["agents"]["request"].clone();
        let mode = req["mode"].as_str().unwrap_or("both").to_string();
        let effort = req["effort"].as_str().unwrap_or("medium").to_string();
        let set = |status: &str, agents: &Value, err: &str| -> Result<()> {
            self.db.lock().unwrap().execute("UPDATE drafts SET agent_status=?2, agents=?3, error=CASE WHEN ?4<>'' THEN ?4 ELSE error END, updated=?5 WHERE id=?1", params![id, status, agents.to_string(), err, now_ms()])?;
            Ok(())
        };
        let mut agents = json!({"request": req, "usage": {"calls": 0}, "rounds": []});
        set("running", &agents, "")?;

        let wanted: Vec<Agent> = [("claude", self.env_or("LEXIS_CLAUDE_BIN", "claude")), ("agy", self.env_or("LEXIS_AGY_BIN", "agy"))]
            .into_iter()
            .filter(|(n, _)| mode == "both" || mode == *n)
            .map(|(name, bin)| Agent { name: if name == "claude" { "claude" } else { "agy" }, bin })
            .collect();
        // Each round of each agent is one call against the daily allowance.
        let needed = (wanted.len() * if wanted.len() > 1 { 2 } else { 1 }) as i64;
        if self.agent_used_today() + wanted.len() as i64 > self.agent_cap() {
            let msg = format!("the daily agent allowance ({}) is used up; raise it in Settings or try tomorrow", self.agent_cap());
            return set("failed", &agents, &msg);
        }

        // A scratch folder holding only the photos.
        let scratch = self.data_dir.join("jobs").join(id);
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch)?;
        let photos = self.draft_photos(&d);
        if photos.is_empty() {
            return set("failed", &agents, "the photos are not on the server");
        }
        for (i, p) in photos.iter().enumerate() {
            std::fs::copy(p, scratch.join(format!("page{}.jpg", i + 1)))?;
        }
        let ocr = d["ocr_text"].as_str().unwrap_or("").to_string();
        let prompt = first_prompt(&ocr, photos.len());

        let mut first: Vec<(&str, Vec<String>)> = Vec::new();
        let mut errors = Vec::new();
        for a in &wanted {
            self.count_agent_call();
            agents["usage"]["calls"] = json!(agents["usage"]["calls"].as_i64().unwrap_or(0) + 1);
            match a.call(&scratch, &prompt, &effort) {
                Ok(lines) => {
                    agents[a.name] = json!({"lines": lines.iter().map(|(t, u, r)| json!({"text": t, "unsure": u, "reason": r})).collect::<Vec<_>>()});
                    first.push((a.name, lines.into_iter().map(|l| l.0).collect()));
                }
                Err(e) => {
                    agents[a.name] = json!({"error": format!("{e:#}")});
                    errors.push(format!("{e:#}"));
                }
            }
            set("running", &agents, "")?;
        }

        // Where two agents differ, each reconsiders once, seeing the other's reading.
        let mut finals: Vec<(&str, Vec<String>)> = first.clone();
        if let [(wa, a), (wb, b)] = &first[..] {
            let differs = a.len() != b.len() || a.iter().zip(b).any(|(x, y)| !same_line(x, y));
            if differs && self.agent_used_today() + 2 <= self.agent_cap() {
                for (agent, mine, theirs) in [(&wanted[0], a, b), (&wanted[1], b, a)] {
                    self.count_agent_call();
                    agents["usage"]["calls"] = json!(agents["usage"]["calls"].as_i64().unwrap_or(0) + 1);
                    match agent.call(&scratch, &second_prompt(mine, theirs), &effort) {
                        Ok(lines) => {
                            agents["rounds"].as_array_mut().unwrap().push(json!({"from": agent.name, "round": 2, "lines": lines.iter().map(|(t, u, r)| json!({"text": t, "unsure": u, "reason": r})).collect::<Vec<_>>()}));
                            if let Some(slot) = finals.iter_mut().find(|f| f.0 == agent.name) {
                                slot.1 = lines.into_iter().map(|l| l.0).collect();
                            }
                        }
                        Err(e) => errors.push(format!("round 2 {e:#}")),
                    }
                }
                let _ = (wa, wb);
            }
        }
        let _ = needed;
        agents["merged"] = merge(&finals);
        let _ = std::fs::remove_dir_all(&scratch);
        if finals.is_empty() {
            return set("failed", &agents, &errors.join("; "));
        }
        let note = errors.join("; ");
        set("done", &agents, &note)
    }

    /// The person asks the agents to read a draft. Nothing runs until they do.
    pub(crate) fn draft_agents(&self, id: &str, mode: &str, effort: &str) -> Result<Value> {
        if !matches!(mode, "claude" | "agy" | "both") {
            bail!("choose Claude, agy or both");
        }
        let d = self.draft_get(id)?;
        if d["agent_status"] == "running" || d["agent_status"] == "requested" {
            bail!("the agents are already working on this draft");
        }
        let req = json!({"request": {"mode": mode, "effort": effort, "at": now_ms()}});
        self.db.lock().unwrap().execute(
            "UPDATE drafts SET agent_status='requested', agents=?2, updated=?3 WHERE id=?1 AND deleted=0 AND status NOT IN ('accepted','rejected')",
            params![id, req.to_string(), now_ms()],
        )?;
        self.draft_get(id)
    }

    /// Allowance status for the Settings screen (the server's counters).
    pub(crate) fn agent_status(&self) -> Value {
        json!({"used_today": self.agent_used_today(), "cap": self.agent_cap()})
    }
}

/// Runs the job queue in the background (the sync server does this).
pub fn spawn_job_runner(api: Arc<Api>, every: Duration) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        let _ = api.run_jobs_once();
        std::thread::sleep(every);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_cleanup() {
        assert_eq!(normalize_symbols("abate — lessen\nlucid <> opaque\nfrugal ~ thrifty\nimbibition != osmosis"), "abate = lessen\nlucid >< opaque\nfrugal ≈ thrifty\nimbibition ⍯ osmosis");
        assert_eq!(normalize_symbols("  a   =  b  \n\n"), "a = b");
    }

    #[test]
    fn finds_lines_in_whatever_shape_the_agent_answers() {
        let direct = r#"{"lines":[{"text":"abate = lessen","unsure":false}]}"#;
        assert_eq!(extract_lines(direct).unwrap()[0].0, "abate = lessen");
        // Claude Code wraps the reply; the JSON may be structured_output or a fenced string.
        let wrapped = json!({"type": "result", "result": "```json\n{\"lines\":[{\"text\":\"a ≈ b\",\"unsure\":true,\"reason\":\"smudge\"}]}\n```"}).to_string();
        let l = extract_lines(&wrapped).unwrap();
        assert_eq!((l[0].0.as_str(), l[0].1, l[0].2.as_str()), ("a ≈ b", true, "smudge"));
        let structured = json!({"result": "", "structured_output": {"lines": [{"text": "x = y", "unsure": false}]}}).to_string();
        assert_eq!(extract_lines(&structured).unwrap()[0].0, "x = y");
        assert!(extract_lines("I could not read the image").is_none());
    }

    #[test]
    fn surfaces_the_error_an_agent_reports() {
        let quota = r#"{"conversation_id":"x","status":"ERROR","response":"","error":"Individual quota reached. Resets in 165h15m5s."}"#;
        assert_eq!(reported_error(quota).unwrap(), "Individual quota reached. Resets in 165h15m5s.");
        assert!(reported_error(r#"{"type":"result","is_error":false,"result":"{}"}"#).is_none());
        assert_eq!(reported_error(r#"{"is_error":true,"result":"Not logged in"}"#).unwrap(), "Not logged in");
        assert!(reported_error("plain text").is_none());
    }

    #[test]
    fn merging_marks_agreement_and_conflict() {
        let m = merge(&[("claude", vec!["abate = lessen".into(), "lucid >< opaque".into()]), ("agy", vec!["Abate  = lessen".into(), "lucid >< opake".into()])]);
        assert_eq!(m["lines"][0]["status"], "agreed");
        assert_eq!(m["lines"][1]["status"], "conflict");
        assert_eq!(m["lines"][1]["options"][1]["text"], "lucid >< opake");
        // Different line counts are offered whole instead of mis-aligned.
        let m = merge(&[("claude", vec!["a".into()]), ("agy", vec!["a".into(), "b".into()])]);
        assert_eq!(m["alternatives"].as_array().unwrap().len(), 2);
        assert!(m["lines"].as_array().unwrap().is_empty());
    }
}

#[cfg(test)]
mod join_tests {
    use super::join_continuations;

    #[test]
    fn indented_continuations_rejoin_their_note() {
        let raw = "court (v): please so to get what you want\ntry to get sth\nwoo: try to get the support of somebody\ndynamism: energy and enthusiasm\nto make thing happen/\nsucceed\nscornful of ≈ contempt ≈ disrespectful\nlaggardly: slow";
        assert_eq!(
            join_continuations(raw),
            "court (v): please so to get what you want try to get sth\nwoo: try to get the support of somebody\ndynamism: energy and enthusiasm to make thing happen/ succeed\nscornful of ≈ contempt ≈ disrespectful\nlaggardly: slow"
        );
    }

    #[test]
    fn a_capitalised_or_long_line_stays_separate() {
        assert_eq!(join_continuations("a = b\nFeign this and that\nc = d"), "a = b\nFeign this and that\nc = d");
        assert_eq!(join_continuations("a = b\none two three four five six seven\nc = d").lines().count(), 3);
    }
}
