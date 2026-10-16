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
pub(crate) struct Ran {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) ok: bool,
}

pub(crate) fn run(mut cmd: Command, limit: Duration) -> Result<Ran> {
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

/// Like `run`, but hands every line the child prints to `on_line` as it arrives (for the agents' stream-json progress).
pub(crate) fn run_stream(mut cmd: Command, limit: Duration, mut on_line: impl FnMut(&str)) -> Result<Ran> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| anyhow!("could not start the program ({e})"))?;
    let out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for l in std::io::BufReader::new(out).lines().map_while(std::result::Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let mut stdout = String::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(l) => {
                on_line(&l);
                stdout.push_str(&l);
                stdout.push('\n');
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("timed out after {}s", limit.as_secs());
        }
    }
    let status = child.wait()?;
    Ok(Ran { stdout, stderr: t_err.join().unwrap_or_default(), ok: status.success() })
}

/// What a stream-json line of `claude -p` shows the owner: the tools it uses and the thoughts it says aloud,
/// each as `kind|text` (kinds: read, edit, search, run, say).
pub(crate) fn describe_event(line: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return vec![] };
    if v["type"] != "assistant" {
        return vec![];
    }
    let short = |s: &str, n: usize| -> String {
        let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if one.chars().count() > n { format!("{}…", one.chars().take(n).collect::<String>()) } else { one }
    };
    let file = |i: &Value| i["file_path"].as_str().or_else(|| i["path"].as_str()).map(|p| p.rsplit('/').next().unwrap_or(p).to_string()).unwrap_or_default();
    v["message"]["content"].as_array().map(|parts| {
        parts.iter().filter_map(|c| match c["type"].as_str()? {
            "text" => Some(format!("say|{}", short(c["text"].as_str()?, 240))).filter(|_| !c["text"].as_str().unwrap_or("").trim().is_empty()),
            "tool_use" => {
                let i = &c["input"];
                Some(match c["name"].as_str()? {
                    "Read" => format!("read|Read {}", file(i)),
                    "Edit" | "MultiEdit" => format!("edit|Edit {}", file(i)),
                    "Write" => format!("edit|Write {}", file(i)),
                    "Grep" => format!("search|Search for “{}”", short(i["pattern"].as_str().unwrap_or(""), 60)),
                    "Glob" => format!("search|Find files {}", short(i["pattern"].as_str().unwrap_or(""), 60)),
                    "Bash" => format!("run|{}", short(i["command"].as_str().unwrap_or(""), 90)),
                    // The schema's answer is the reply itself, not a step.
                    "StructuredOutput" => return None,
                    other => format!("run|{other}"),
                })
            }
            _ => None,
        }).collect()
    }).unwrap_or_default()
}

/// The final `result` line of a stream-json run (what the plain json format prints alone); the whole text if there is none.
fn final_result(stdout: &str) -> String {
    stdout.lines().rev().find(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["type"] == "result")).map(str::to_string).unwrap_or_else(|| stdout.to_string())
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

/// What Gemini is told: the same literal copy, plus the page's quirks.
const GEMINI_PROMPT: &str = "This is a photo of a page from a student's handwritten GRE vocabulary notebook, in English with some Vietnamese. \
Transcribe every line exactly as written, one note per line, in reading order, keeping the writer's own spelling and Vietnamese diacritics. \
A line that is indented under another continues that line: join it onto the line above. \
Write the relation symbols the way they are used: = (same meaning), ≈ (similar, often handwritten as a wavy equals), >< (opposite), ≠ (different). \
Keep colons, brackets, slashes, (v) (n) (adj) tags, usage notes like (of a child) and abbreviations like sth as written. \
Ignore bleed-through from the other side of the page and shadows. Put crossed-out text between ~~ marks. \
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

/// Reads a photo with Google's Gemini API. The key lives in the server's environment
/// (`GEMINI_API_KEY`), never in the repo or in the app; the photo is sent to Google.
fn ocr_gemini_at(base: &str, key: &str, model: &str, image: &Path) -> Result<String> {
    use base64::Engine;
    let bytes = std::fs::read(image)?;
    let mime = if image.extension().is_some_and(|e| e == "png") { "image/png" } else { "image/jpeg" };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(75)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into();
    let body = json!({
        "contents": [{"parts": [{"text": GEMINI_PROMPT}, {"inline_data": {"mime_type": mime, "data": base64::engine::general_purpose::STANDARD.encode(bytes)}}]}],
        "generationConfig": {"temperature": 0},
    });
    let mut resp = agent
        .post(&format!("{base}/v1beta/models/{model}:generateContent"))
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", key)
        .send(serde_json::to_vec(&body)?)
        .map_err(|e| anyhow!("Gemini is not reachable ({e})"))?;
    let status = resp.status().as_u16();
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?).unwrap_or(Value::Null);
    if status != 200 {
        bail!("Gemini said: {}", v["error"]["message"].as_str().unwrap_or("an error"));
    }
    let text: String = v["candidates"][0]["content"]["parts"].as_array().into_iter().flatten().filter_map(|p| p["text"].as_str()).collect();
    let text = text.trim().trim_start_matches("```").trim_end_matches("```").trim().to_string();
    if text.is_empty() {
        bail!("Gemini returned nothing");
    }
    Ok(join_continuations(&text))
}

/// One plain-text Gemini answer (no image), with the same busy-model retries and fallback as photo reading.
pub(crate) fn gemini_text(prompt: &str) -> Result<String> {
    let key = gemini_key().ok_or_else(|| anyhow!("no GEMINI_API_KEY is set on the server"))?;
    let base = std::env::var("LEXIS_GEMINI_URL").unwrap_or_else(|_| "https://generativelanguage.googleapis.com".into());
    let model = std::env::var("LEXIS_GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.6-flash".into());
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(40))).timeout_connect(Some(Duration::from_secs(10))).http_status_as_error(false).build().into();
    let mut last = anyhow!("no model tried");
    for m in [model.as_str(), "gemini-2.5-flash"] {
        let body = json!({"contents": [{"parts": [{"text": prompt}]}], "generationConfig": {"temperature": 0}});
        let sent = agent.post(&format!("{base}/v1beta/models/{m}:generateContent")).header("Content-Type", "application/json").header("x-goog-api-key", &key).send(serde_json::to_vec(&body)?);
        let mut resp = match sent {
            Ok(r) => r,
            Err(e) => {
                last = anyhow!("Gemini is not reachable ({e})");
                continue;
            }
        };
        let status = resp.status().as_u16();
        let v: Value = serde_json::from_str(&resp.body_mut().read_to_string()?).unwrap_or(Value::Null);
        if status != 200 {
            last = anyhow!("Gemini said: {}", v["error"]["message"].as_str().unwrap_or("an error"));
            continue;
        }
        let text: String = v["candidates"][0]["content"]["parts"].as_array().into_iter().flatten().filter_map(|p| p["text"].as_str()).collect();
        if !text.trim().is_empty() {
            return Ok(text.trim().to_string());
        }
        last = anyhow!("Gemini returned nothing");
    }
    Err(last)
}

pub(crate) fn gemini_key() -> Option<String> {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"].iter().find_map(|k| std::env::var(k).ok()).filter(|k| !k.trim().is_empty())
}

/// `hard` is the owner's "read again carefully" button: a stronger (slower) model goes first.
fn ocr_gemini(image: &Path, hard: bool) -> Result<String> {
    let key = gemini_key().ok_or_else(|| anyhow!("no GEMINI_API_KEY is set on the server"))?;
    let base = std::env::var("LEXIS_GEMINI_URL").unwrap_or_else(|_| "https://generativelanguage.googleapis.com".into());
    let model = std::env::var("LEXIS_GEMINI_MODEL").unwrap_or_else(|_| "gemini-3.6-flash".into());
    let strong = std::env::var("LEXIS_GEMINI_HARD_MODEL").unwrap_or_else(|_| "gemini-3.1-pro-preview".into());
    let mut order = Vec::new();
    if hard {
        order.push(strong.as_str());
    }
    order.extend([model.as_str(), "gemini-2.5-flash"]);
    // New models are often "in high demand" (503): try each twice, then the steadier older one.
    let mut last = anyhow!("no model tried");
    for m in order {
        for attempt in 0..2 {
            match ocr_gemini_at(&base, &key, m, image) {
                Ok(t) => return Ok(t),
                Err(e) => {
                    // Only "high demand" is worth a second try; a hang or any other error moves on to the next model.
                    let busy = format!("{e:#}").contains("high demand");
                    last = e;
                    if !busy {
                        break;
                    }
                    if attempt == 0 {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        }
    }
    Err(last)
}

fn ocr_one(engine: &str, image: &Path) -> Result<String> {
    match engine {
        "none" => bail!("OCR is turned off"),
        "gemini" => ocr_gemini(image, false),
        "gemini-hard" => ocr_gemini(image, true),
        // The default: Gemini when the server has a key, else a local vision model (far better
        // than tesseract at handwriting); tesseract stays as the last fallback.
        "auto" => {
            if gemini_key().is_some() {
                match ocr_gemini(image, false) {
                    Ok(t) => return Ok(t),
                    Err(e) => eprintln!("Gemini OCR failed ({e:#}); trying the local model"),
                }
            }
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

pub(crate) struct Agent {
    pub(crate) name: &'static str,
    pub(crate) bin: String,
    /// Claude only: the model and effort the owner picked for a chat message (the CLI's own default when `None`).
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    /// Claude only: a folder with the photos attached to the message, readable in addition to the working folder.
    pub(crate) extra: Option<PathBuf>,
}

impl Agent {
    fn pick(&self, c: &mut Command) {
        if let Some(m) = &self.model {
            c.args(["--model", m]);
        }
        if let Some(e) = &self.effort {
            c.args(["--effort", e]);
        }
        if let Some(d) = &self.extra {
            c.arg("--add-dir").arg(d);
        }
    }

    fn call(&self, dir: &Path, prompt: &str, effort: &str) -> Result<Vec<(String, bool, String)>> {
        match self.call_with(dir, prompt, Some(effort)) {
            // Some of agy's models take no effort setting: say nothing and use the model's own.
            Err(e) if self.name == "agy" && format!("{e:#}").contains("--effort is not supported") => self.call_with(dir, prompt, None),
            r => r,
        }
    }

    fn call_with(&self, dir: &Path, prompt: &str, effort: Option<&str>) -> Result<Vec<(String, bool, String)>> {
        let out = self.run_cli(dir, prompt, SCHEMA, effort, None)?;
        extract_lines(&out).ok_or_else(|| anyhow!("{} replied, but not in the expected form", self.name))
    }

    /// A task chat: Claude may edit files in `dir` (a private worktree on its own branch) and use git there, nothing else.
    pub(crate) fn run_work(&self, dir: &Path, prompt: &str, schema: &str, live: Option<&mut dyn FnMut(String)>) -> Result<String> {
        let mut c = Command::new(&self.bin);
        c.current_dir(dir).arg("-p").arg(prompt).args(["--output-format", if live.is_some() { "stream-json" } else { "json" }, "--json-schema", schema]);
        if live.is_some() {
            c.arg("--verbose");
        }
        c.args(["--permission-mode", "acceptEdits", "--allowedTools", "Read", "Edit", "Write", "Glob", "Grep", "Bash(git status:*)", "Bash(git diff:*)", "Bash(git log:*)", "Bash(git add:*)", "Bash(git commit:*)"]);
        c.arg("--add-dir").arg(dir).args(["--disallowedTools", "WebFetch", "WebSearch"]);
        self.pick(&mut c);
        let mut r = Self::exec(c, Duration::from_secs(1500), live).map_err(|e| anyhow!("{}: {e:#}", self.name))?;
        r.stdout = final_result(&r.stdout);
        if let Some(why) = reported_error(&r.stdout).or_else(|| (!r.ok).then(|| r.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string())) {
            bail!("{} says: {why}", self.name);
        }
        Ok(r.stdout)
    }

    /// Runs the command; with `live`, each tool use or remark of the agent is passed on as it happens.
    fn exec(c: Command, limit: Duration, live: Option<&mut dyn FnMut(String)>) -> Result<Ran> {
        match live {
            Some(f) => run_stream(c, limit, |l| describe_event(l).into_iter().for_each(&mut *f)),
            None => run(c, limit),
        }
    }

    /// One non-interactive call of the CLI in `dir`; its raw output, or what it reported as the error.
    pub(crate) fn run_cli(&self, dir: &Path, prompt: &str, schema: &str, effort: Option<&str>, live: Option<&mut dyn FnMut(String)>) -> Result<String> {
        let live = live.filter(|_| self.name == "claude");
        let mut c = Command::new(&self.bin);
        c.current_dir(dir).arg("-p").arg(prompt).args(["--output-format", if live.is_some() { "stream-json" } else { "json" }, "--json-schema", schema]);
        if live.is_some() {
            c.arg("--verbose");
        }
        match self.name {
            // Read only: the Read tool for the photos, nothing that writes or browses.
            "claude" => {
                c.args(["--allowedTools", "Read"]).arg("--add-dir").arg(dir).args(["--disallowedTools", "Bash", "Edit", "Write", "WebFetch", "WebSearch"]);
                self.pick(&mut c);
            }
            // Plan mode keeps agy from editing anything.
            _ => {
                c.args(["--mode", "plan"]);
                if let Some(e) = effort {
                    c.args(["--effort", e]);
                }
            }
        }
        let mut r = Self::exec(c, AGENT_TIMEOUT, live).map_err(|e| anyhow!("{}: {e:#}", self.name))?;
        r.stdout = final_result(&r.stdout);
        // Errors (quota reached, not signed in) come back as JSON on stdout or as text on stderr:
        // say what they say rather than guessing.
        if let Some(why) = reported_error(&r.stdout).or_else(|| (!r.ok).then(|| r.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string())) {
            bail!("{} says: {why}", self.name);
        }
        Ok(r.stdout)
    }
}

const CHAT_SCHEMA: &str = r#"{"type":"object","properties":{"reply":{"type":"string"},"text":{"type":"string"}},"required":["reply"]}"#;

/// Claude's chat answer: `reply`, and `text` when it proposes a new version of the whole note.
pub fn extract_chat(stdout: &str) -> Option<(String, Option<String>)> {
    fn find(v: &Value, depth: usize) -> Option<(String, Option<String>)> {
        if depth > 6 {
            return None;
        }
        match v {
            Value::Object(m) => {
                if let Some(r) = m.get("reply").and_then(Value::as_str) {
                    let t = m.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
                    return Some((r.trim().to_string(), t));
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

fn chat_prompt(pages: usize, text: &str, chat: &[Value]) -> String {
    let talk: String = chat
        .iter()
        .rev()
        .take(14)
        .rev()
        .filter_map(|m| Some(format!("{}: {}", if m["role"] == "user" { "STUDENT" } else { "YOU" }, m["text"].as_str()?)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are helping a GRE student check the transcription of their handwritten vocabulary notes. The photo{} {} in this folder: {}; open them with the Read tool whenever you need to look. \
         Notes use only these symbols: '=' same meaning, '≈' roughly similar, '><' opposite, '⍯' different but easily confused. \
         The transcription as it stands now (the student may have edited it): <<<\n{text}\n>>>\n\
         The conversation so far:\n{talk}\n\
         Answer the student's last message briefly and plainly. Look at the photo again when they question a word. \
         If the transcription should change, also give `text`: the complete new transcription, one note per line, changing only what was discussed. Otherwise leave `text` out. \
         Reply with JSON only: {{\"reply\":string,\"text\":string (optional)}}",
        if pages == 1 { "" } else { "s" },
        if pages == 1 { "is" } else { "are" },
        (1..=pages).map(|i| format!("page{i}.jpg")).collect::<Vec<_>>().join(", "),
    )
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
    pub(crate) fn env_or(&self, k: &str, d: &str) -> String {
        std::env::var(k).unwrap_or_else(|_| d.to_string())
    }

    pub(crate) fn agent_used_today(&self) -> i64 {
        self.setting(&format!("agents.used.{}", now_ms() / 86_400_000)).and_then(|s| s.parse().ok()).unwrap_or(0)
    }

    pub(crate) fn agent_cap(&self) -> i64 {
        self.setting("agents.cap").and_then(|s| s.parse().ok()).or_else(|| std::env::var("LEXIS_AGENT_CAP").ok().and_then(|s| s.parse().ok())).unwrap_or(50)
    }

    pub(crate) fn count_agent_call(&self) {
        let _ = self.set_setting(&format!("agents.used.{}", now_ms() / 86_400_000), &(self.agent_used_today() + 1).to_string());
    }

    /// Reads every queued draft, then serves agent requests. Safe to call often.
    pub fn run_jobs_once(&self) -> Result<Value> {
        self.run_jobs(true)
    }

    /// `with_talks` false leaves the Claude chats to `spawn_talks`, which runs each chat on its own thread.
    fn run_jobs(&self, with_talks: bool) -> Result<Value> {
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
        let chats: Vec<String> = {
            let db = self.db.lock().unwrap();
            let mut st = db.prepare("SELECT id FROM drafts WHERE agent_status='chat' AND deleted=0 AND status NOT IN ('accepted','rejected') ORDER BY updated LIMIT 2")?;
            let rows = st.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        for id in chats {
            self.job_chat(&id)?;
            agents_done += 1;
        }
        if with_talks {
            agents_done += self.job_talks()?;
        }
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
        // "Read again carefully" asked for the strong Gemini model and replaces the text.
        let hard = d["ocr_engine"] == "gemini-hard";
        let engine = if hard { "gemini-hard".to_string() } else { self.env_or("LEXIS_OCR", "auto") };
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
        let keep = !hard && d["text"].as_str().is_some_and(|t| !t.trim().is_empty());
        self.db.lock().unwrap().execute(
            "UPDATE drafts SET status='needs_review', ocr_text=?2, ocr_engine=?3, text=CASE WHEN ?4 THEN text ELSE ?2 END, error=?5, updated=?6 WHERE id=?1",
            params![id, text, engine, keep, error, now_ms()],
        )?;
        Ok(())
    }

    /// Answers the student's latest chat message about a draft, with Claude.
    fn job_chat(&self, id: &str) -> Result<()> {
        let d = self.draft_get(id)?;
        let mut chat = d["chat"].as_array().cloned().unwrap_or_default();
        let finish = |chat: &Vec<Value>, status: &str, err: &str| -> Result<()> {
            self.db.lock().unwrap().execute("UPDATE drafts SET agent_status=?2, chat=?3, error=CASE WHEN ?4<>'' THEN ?4 ELSE error END, updated=?5 WHERE id=?1", params![id, status, json!(chat).to_string(), err, now_ms()])?;
            Ok(())
        };
        self.db.lock().unwrap().execute("UPDATE drafts SET agent_status='chat_running', updated=?2 WHERE id=?1", params![id, now_ms()])?;
        let fail = |chat: &mut Vec<Value>, why: String| -> Result<()> {
            chat.push(json!({"role": "error", "text": why, "at": now_ms()}));
            finish(chat, "chat_done", "")
        };
        if self.agent_used_today() + 1 > self.agent_cap() {
            return fail(&mut chat, format!("the daily agent allowance ({}) is used up; raise it in Settings or try tomorrow", self.agent_cap()));
        }
        let photos = self.draft_photos(&d);
        if photos.is_empty() {
            return fail(&mut chat, "the photos are not on the server".into());
        }
        let scratch = self.data_dir.join("jobs").join(id);
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch)?;
        for (i, p) in photos.iter().enumerate() {
            std::fs::copy(p, scratch.join(format!("page{}.jpg", i + 1)))?;
        }
        let claude = Agent { name: "claude", bin: self.env_or("LEXIS_CLAUDE_BIN", "claude"), model: None, effort: None, extra: None };
        self.count_agent_call();
        let prompt = chat_prompt(photos.len(), d["text"].as_str().unwrap_or(""), &chat);
        let out = claude.run_cli(&scratch, &prompt, CHAT_SCHEMA, None, None).and_then(|o| extract_chat(&o).ok_or_else(|| anyhow!("claude replied, but not in the expected form")));
        let _ = std::fs::remove_dir_all(&scratch);
        match out {
            Ok((reply, text)) => {
                chat.push(json!({"role": "claude", "text": reply, "proposal": text, "at": now_ms()}));
                finish(&chat, "chat_done", "")
            }
            Err(e) => fail(&mut chat, format!("{e:#}")),
        }
    }

    /// The student writes to Claude about a draft. Nothing runs until they do (it uses the agent allowance).
    pub(crate) fn draft_chat(&self, id: &str, message: &str) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() || message.len() > 2000 {
            bail!("write a message of up to 2000 characters");
        }
        let d = self.draft_get(id)?;
        if matches!(d["status"].as_str(), Some("accepted" | "rejected")) {
            bail!("that draft is closed");
        }
        if matches!(d["agent_status"].as_str(), Some("chat" | "chat_running" | "requested" | "running")) {
            bail!("Claude is still answering");
        }
        let mut chat = d["chat"].as_array().cloned().unwrap_or_default();
        chat.push(json!({"role": "user", "text": message, "at": now_ms()}));
        self.db.lock().unwrap().execute("UPDATE drafts SET agent_status='chat', chat=?2, updated=?3 WHERE id=?1 AND deleted=0", params![id, json!(chat).to_string(), now_ms()])?;
        self.draft_get(id)
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
            .map(|(name, bin)| Agent { name: if name == "claude" { "claude" } else { "agy" }, bin, model: None, effort: None, extra: None })
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
    // Chats get their own dispatcher, so a long photo read or agent run never delays a chat.
    let chats = api.clone();
    std::thread::spawn(move || loop {
        chats.spawn_talks();
        std::thread::sleep(every);
    });
    std::thread::spawn(move || loop {
        let _ = api.run_jobs(false);
        std::thread::sleep(every);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_reads_a_photo_through_a_stand_in_server() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let seen = std::thread::spawn(move || {
            let mut req = server.recv().unwrap();
            let mut body = String::new();
            std::io::Read::read_to_string(req.as_reader(), &mut body).unwrap();
            let key = req.headers().iter().find(|h| h.field.equiv("x-goog-api-key")).map(|h| h.value.to_string());
            let reply = json!({"candidates": [{"content": {"parts": [{"text": "court (v): please so\nto get what you want\nwoo: try to get support"}]}}]});
            let url = req.url().to_string();
            let _ = req.respond(tiny_http::Response::from_string(reply.to_string()));
            (url, key, body)
        });
        let img = std::env::temp_dir().join(format!("gemini-test-{}.jpg", std::process::id()));
        std::fs::write(&img, b"\xff\xd8 not really a photo").unwrap();
        let text = ocr_gemini_at(&format!("http://127.0.0.1:{port}"), "secret-key", "gemini-test", &img).unwrap();
        // The indented second line is rejoined, and the key travels in a header, never in the URL.
        assert_eq!(text, "court (v): please so to get what you want\nwoo: try to get support");
        let (url, key, body) = seen.join().unwrap();
        assert_eq!(url, "/v1beta/models/gemini-test:generateContent");
        assert_eq!(key.as_deref(), Some("secret-key"));
        assert!(body.contains("inline_data") && body.contains("image/jpeg"));
    }

    #[test]
    fn a_busy_gemini_model_falls_back_to_2_5_flash() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let tried = std::thread::spawn(move || {
            let mut tried = Vec::new();
            for _ in 0..3 {
                let req = server.recv().unwrap();
                let url = req.url().to_string();
                let busy = !url.contains("gemini-2.5-flash");
                tried.push(url);
                let r = if busy {
                    tiny_http::Response::from_string(r#"{"error":{"message":"This model is currently experiencing high demand."}}"#).with_status_code(503)
                } else {
                    tiny_http::Response::from_string(json!({"candidates": [{"content": {"parts": [{"text": "woo: try to get support"}]}}]}).to_string())
                };
                let _ = req.respond(r);
            }
            tried
        });
        let img = std::env::temp_dir().join(format!("gemini-test3-{}.jpg", std::process::id()));
        std::fs::write(&img, b"x").unwrap();
        std::env::set_var("LEXIS_GEMINI_URL", format!("http://127.0.0.1:{port}"));
        std::env::set_var("GEMINI_API_KEY", "k");
        std::env::set_var("LEXIS_GEMINI_MODEL", "gemini-9-flash");
        let text = ocr_gemini(&img, false).unwrap();
        assert_eq!(text, "woo: try to get support");
        let tried = tried.join().unwrap();
        // The busy model is tried twice, then the fallback answers.
        assert!(tried[0].contains("gemini-9-flash") && tried[1].contains("gemini-9-flash") && tried[2].contains("gemini-2.5-flash"));
    }

    #[test]
    fn gemini_errors_do_not_leak_the_key() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            let req = server.recv().unwrap();
            let _ = req.respond(tiny_http::Response::from_string(r#"{"error":{"message":"API key not valid"}}"#).with_status_code(400));
        });
        let img = std::env::temp_dir().join(format!("gemini-test2-{}.jpg", std::process::id()));
        std::fs::write(&img, b"x").unwrap();
        let e = ocr_gemini_at(&format!("http://127.0.0.1:{port}"), "secret-key", "m", &img).unwrap_err();
        assert!(format!("{e:#}").contains("API key not valid") && !format!("{e:#}").contains("secret-key"));
    }

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

#[cfg(test)]
mod describe_tests {
    use super::*;
    #[test]
    fn shows_tools_and_remarks() {
        let l = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking at the chat box"},{"type":"tool_use","name":"Read","input":{"file_path":"/a/b/Claude.svelte"}},{"type":"tool_use","name":"Bash","input":{"command":"git status --short"}},{"type":"tool_use","name":"StructuredOutput","input":{}}]}}"#;
        assert_eq!(describe_event(l), vec!["say|Looking at the chat box", "read|Read Claude.svelte", "run|git status --short"]);
        assert!(describe_event(r#"{"type":"result"}"#).is_empty());
        assert_eq!(final_result("{\"type\":\"system\"}\n{\"type\":\"result\",\"result\":\"x\"}\n"), "{\"type\":\"result\",\"result\":\"x\"}");
    }
}
