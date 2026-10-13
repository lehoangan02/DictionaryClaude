//! Photographed notes: real OCR on a generated image, and the agent flow with
//! stand-in CLIs (so tests never spend anyone's quota).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use gredict_app::{Api, Config};
use serde_json::{json, Value};

/// These tests set process-wide environment variables.
static SERIAL: Mutex<()> = Mutex::new(());

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gredict-draft-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn api(name: &str) -> Option<Api> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() {
        return None;
    }
    Some(Api::open(Config { data_dir: tmp(name), packs_dir: root.join("data/packs"), private_dir: None }).unwrap())
}

/// A photo of handwriting stand-in: black text on white, as a JPEG.
fn photo(lines: &[&str]) -> Option<String> {
    let out = tmp("img").join("p.jpg");
    let mut c = Command::new("convert");
    c.args(["-size", "1000x420", "xc:white", "-fill", "black", "-pointsize", "52"]);
    for (i, l) in lines.iter().enumerate() {
        c.args(["-annotate", &format!("+40+{}", 110 + i * 110), l]);
    }
    c.arg(&out);
    c.status().ok().filter(|s| s.success())?;
    Some(STANDARD.encode(std::fs::read(out).ok()?))
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

#[test]
fn ocr_reads_a_photo_into_a_draft_that_becomes_a_note_only_when_accepted() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("tesseract") || !have("convert") {
        return;
    }
    let Some(api) = api("ocr") else { return };
    std::env::set_var("LEXIS_OCR", "tesseract");
    let Some(img) = photo(&["abate = lessen", "lucid >< opaque"]) else { return };

    let d = api.call("draft_create", json!({"photos": [format!("data:image/jpeg;base64,{img}")], "session_date": "2026-10-11", "tags": "tutor"})).unwrap();
    let id = d["id"].as_str().unwrap().to_string();
    assert_eq!(api.call("draft_get", json!({"id": id})).unwrap()["status"], "queued");
    // Nothing reads it until the job runner does, and nothing becomes a note by itself.
    assert!(api.call("notes_list", json!({})).unwrap().as_array().unwrap().is_empty());

    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["status"], "needs_review", "{d}");
    let text = d["text"].as_str().unwrap().to_lowercase();
    assert!(text.contains("abate") && text.contains("lessen") && text.contains("lucid") && text.contains("opaque"), "OCR text: {text:?}");
    assert_eq!(d["agent_status"], "", "agents are never started automatically");
    assert!(api.call("notes_list", json!({})).unwrap().as_array().unwrap().is_empty(), "still no note");

    // The person corrects it and accepts: now it is a note and feeds the thesaurus.
    api.call("draft_update", json!({"id": id, "text": "abate = lessen = subside\nlucid >< opaque"})).unwrap();
    let r = api.call("draft_accept", json!({"id": id})).unwrap();
    assert_eq!(r["draft"]["status"], "accepted");
    let notes = api.call("notes_list", json!({})).unwrap();
    assert_eq!(notes[0]["body"], "abate = lessen = subside\nlucid >< opaque");
    assert_eq!(notes[0]["session_date"], "2026-10-11");
    assert_eq!(api.call("thesaurus", json!({"word": "subside"})).unwrap()["notes"]["same"], json!(["abate", "lessen"]));
    // A closed draft cannot be accepted twice.
    assert!(api.call("draft_accept", json!({"id": id})).is_err());
    // And it leaves the review list.
    assert!(api.call("drafts_list", json!({})).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn uploads_are_validated() {
    let Some(api) = api("validate") else { return };
    let gif = STANDARD.encode(b"GIF89a....");
    assert!(api.call("draft_create", json!({"photos": [gif]})).unwrap_err().to_string().contains("JPEG and PNG"));
    assert!(api.call("draft_create", json!({"photos": []})).is_err());
    assert!(api.call("draft_create", json!({"photos": ["!!!not base64!!!"]})).is_err());
    assert!(api.call("photo_data", json!({"name": "../../etc/passwd"})).is_err(), "names cannot escape the folder");
}

const SAME: &str = r#"{"result":"","structured_output":{"lines":[{"text":"abate = lessen","unsure":false},{"text":"lucid >< opaque","unsure":false}]}}"#;
const MISREAD: &str = r#"{"result":"","structured_output":{"lines":[{"text":"abate = lessen","unsure":false},{"text":"lucid >< opake","unsure":true,"reason":"smudged letter"}]}}"#;

fn draft(api: &Api) -> String {
    let Some(img) = photo(&["abate = lessen", "lucid >< opaque"]) else { panic!("convert missing") };
    let id = api.call("draft_create", json!({"photos": [img]})).unwrap()["id"].as_str().unwrap().to_string();
    api.call("run_jobs", json!({})).unwrap(); // OCR first, as in real use
    id
}

#[test]
fn agents_run_only_when_asked_and_resolve_their_disagreement() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("agents") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins");
    // Claude reads it right both times; agy misreads once, then agrees after seeing Claude's reading.
    let claude = script(&bins, "claude", &format!("echo '{SAME}'"));
    let agy = script(&bins, "agy", &format!("case \"$2\" in *'Another reader'*) echo '{SAME}' ;; *) echo '{MISREAD}' ;; esac"));
    std::env::set_var("LEXIS_CLAUDE_BIN", &claude);
    std::env::set_var("LEXIS_AGY_BIN", &agy);
    std::env::set_var("LEXIS_AGENT_CAP", "8");

    let id = draft(&api);
    api.call("run_jobs", json!({})).unwrap();
    assert_eq!(api.call("draft_get", json!({"id": id})).unwrap()["agent_status"], "", "no request, no agents");

    api.call("draft_agents", json!({"id": id, "mode": "both", "effort": "low"})).unwrap();
    assert_eq!(api.call("draft_get", json!({"id": id})).unwrap()["agent_status"], "requested");
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "done", "{d}");
    let a = &d["agents"];
    assert_eq!(a["claude"]["lines"][1]["text"], "lucid >< opaque");
    assert_eq!(a["agy"]["lines"][1]["text"], "lucid >< opake");
    assert_eq!(a["agy"]["lines"][1]["unsure"], true);
    assert_eq!(a["rounds"].as_array().unwrap().len(), 2, "both reconsidered once");
    assert_eq!(a["usage"]["calls"], 4);
    // After the discussion they agree on every line.
    assert!(a["merged"]["lines"].as_array().unwrap().iter().all(|l| l["status"] == "agreed"), "{a}");
    // The agents never touch the person's text or notes.
    assert_eq!(d["status"], "needs_review");
    assert!(api.call("notes_list", json!({})).unwrap().as_array().unwrap().is_empty());
    assert_eq!(api.call("agent_status", json!({})).unwrap()["used_today"], 4);
    // The scratch folder with the photos is removed.
    assert!(!api.call("agent_status", json!({})).is_err());
}

#[test]
fn the_daily_allowance_stops_the_discussion_and_blocks_new_requests() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("cap") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins2");
    std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, "claude", &format!("echo '{SAME}'")));
    std::env::set_var("LEXIS_AGY_BIN", script(&bins, "agy", &format!("echo '{MISREAD}'")));
    std::env::set_var("LEXIS_AGENT_CAP", "3");

    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "both"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    // Two first-round calls fit under the cap of 3; the two-call discussion does not.
    assert_eq!(d["agents"]["usage"]["calls"], 2);
    assert!(d["agents"]["rounds"].as_array().unwrap().is_empty());
    let conflict = d["agents"]["merged"]["lines"].as_array().unwrap().iter().filter(|l| l["status"] == "conflict").count();
    assert_eq!(conflict, 1, "the unresolved line is left for the person");

    // A second draft cannot even start: 2 + 2 > 3.
    let id2 = draft(&api);
    api.call("draft_agents", json!({"id": id2, "mode": "both"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d2 = api.call("draft_get", json!({"id": id2})).unwrap();
    assert_eq!(d2["agent_status"], "failed");
    assert!(d2["error"].as_str().unwrap().contains("allowance"), "{d2}");
    // The person can still read and accept it by hand.
    api.call("draft_update", json!({"id": id2, "text": "abate = lessen"})).unwrap();
    assert!(api.call("draft_accept", json!({"id": id2})).is_ok());
}

#[test]
fn one_broken_agent_does_not_sink_the_other() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("broken") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins3");
    std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, "claude", &format!("echo '{SAME}'")));
    std::env::set_var("LEXIS_AGY_BIN", script(&bins, "agy", "echo 'I could not open the image'"));
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "both"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "done");
    assert!(d["agents"]["agy"]["error"].as_str().unwrap().contains("not in the expected form"));
    assert_eq!(d["agents"]["merged"]["lines"][0]["status"], "single", "Claude's reading stands alone");
    assert!(d["error"].as_str().unwrap().contains("agy"));

    // The real failure that happens: an agent whose quota is used up says so, and that is what we show.
    std::env::set_var("LEXIS_AGY_BIN", script(&bins, "agy-quota", r#"echo '{"status":"ERROR","response":"","error":"Individual quota reached. Resets in 165h."}'; exit 3"#));
    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "both"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "done", "Claude still delivers");
    let why = d["agents"]["agy"]["error"].as_str().unwrap();
    assert!(why.contains("agy says: Individual quota reached"), "{why}");
    assert!(!why.contains("expected form"));

    // A program that is not even installed is reported, not fatal.
    std::env::set_var("LEXIS_AGY_BIN", "/nonexistent/agy");
    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "agy"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d: Value = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "failed");
    assert!(d["error"].as_str().unwrap().contains("could not start"), "{d}");
}

/// Uses the real `claude` and `agy` (and so a little of their quota): run by hand with
/// `cargo test -p gredict-app --test drafts real_agents -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_agents_read_a_generated_photo_and_the_discussion_resolves() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(api) = api("real-agents") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    std::env::remove_var("LEXIS_CLAUDE_BIN");
    std::env::remove_var("LEXIS_AGY_BIN");
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "both", "effort": "low"})).unwrap();
    let t = std::time::Instant::now();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    println!("{} s\n{}", t.elapsed().as_secs(), serde_json::to_string_pretty(&d["agents"]).unwrap());
    assert_eq!(d["agent_status"], "done", "{d}");
}

#[test]
fn agy_is_retried_without_effort_when_its_model_does_not_take_one() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("effort") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins3");
    std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, "claude", &format!("echo '{SAME}'")));
    let agy = script(&bins, "agy", &format!("case \"$*\" in *--effort*) echo '{{\"status\":\"ERROR\",\"error\":\"invalid model selection: --effort is not supported for the current model\"}}' ;; *) echo '{SAME}' ;; esac"));
    std::env::set_var("LEXIS_AGY_BIN", agy);
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    let id = draft(&api);
    api.call("draft_agents", json!({"id": id, "mode": "both", "effort": "high"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "done", "{d}");
    assert!(d["agents"]["agy"]["error"].is_null(), "{d}");
    assert_eq!(d["agents"]["agy"]["lines"][1]["text"], "lucid >< opaque");
}

#[test]
fn claude_chat_answers_each_message_proposes_text_and_never_edits_it() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("chat") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins4");
    // The stand-in answers the second message with a proposed text and echoes that it saw the first one.
    let claude = script(
        &bins,
        "claude",
        r#"case "$2" in *'STUDENT: swap them'*) printf '%s' '{"structured_output":{"reply":"Done, swapped.","text":"lucid >< opaque\nabate = lessen"}}' ;; *) echo '{"structured_output":{"reply":"Looks right to me."}}' ;; esac"#,
    );
    std::env::set_var("LEXIS_CLAUDE_BIN", &claude);
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    let id = draft(&api);
    assert_eq!(api.call("draft_get", json!({"id": id})).unwrap()["chat"], json!([]));
    assert!(api.call("draft_chat", json!({"id": id, "message": "  "})).is_err(), "an empty message is refused");

    api.call("draft_chat", json!({"id": id, "message": "is line one right?"})).unwrap();
    assert!(api.call("draft_chat", json!({"id": id, "message": "again"})).is_err(), "one question at a time");
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["agent_status"], "chat_done", "{d}");
    assert_eq!(d["chat"][0]["role"], "user");
    assert_eq!(d["chat"][1]["role"], "claude");
    assert_eq!(d["chat"][1]["text"], "Looks right to me.");
    assert!(d["chat"][1]["proposal"].is_null());

    let before = d["text"].clone();
    api.call("draft_chat", json!({"id": id, "message": "swap them"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["chat"][3]["proposal"], "lucid >< opaque\nabate = lessen");
    assert_eq!(d["text"], before, "a suggestion never changes the person's text by itself");
    assert_eq!(api.call("agent_status", json!({})).unwrap()["used_today"], 2);
    assert!(api.call("notes_list", json!({})).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn a_chat_that_cannot_run_says_why_in_the_chat() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !have("convert") {
        return;
    }
    let Some(api) = api("chat-err") else { return };
    std::env::set_var("LEXIS_OCR", "none");
    let bins = tmp("bins5");
    std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, "claude", r#"echo '{"status":"ERROR","error":"quota reached"}'; exit 3"#));
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    let id = draft(&api);
    api.call("draft_chat", json!({"id": id, "message": "hello"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let d = api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(d["chat"][1]["role"], "error");
    assert!(d["chat"][1]["text"].as_str().unwrap().contains("quota reached"), "{d}");
    // And the person can try again.
    assert!(api.call("draft_chat", json!({"id": id, "message": "again"})).is_ok());
}

#[test]
fn the_general_chat_answers_proposes_ideas_and_keeps_only_approved_ones() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(api) = api("talk") else { return };
    let bins = tmp("bins6");
    let repo = tmp("repo");
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/ROADMAP.md"), "x").unwrap();
    std::env::set_var("LEXIS_REPO", &repo);
    std::env::set_var("LEXIS_CLAUDE_BIN", script(&bins, "claude", r#"case "$2" in *'OWNER: make it blue'*) printf '%s' '{"structured_output":{"reply":"Noted.","idea":{"title":"Blue theme","detail":"Add a blue accent."}}}' ;; *) echo '{"structured_output":{"reply":"It stores words by lemma."}}' ;; esac"#));
    std::env::set_var("LEXIS_AGENT_CAP", "8");
    assert!(api.call("talk_list", json!({})).unwrap().as_array().unwrap().is_empty());
    let c1 = api.call("talk_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    let c2 = api.call("talk_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(api.call("talk_send", json!({"id": c1, "message": " "})).is_err());
    api.call("talk_send", json!({"id": c1, "message": "how are words stored?"})).unwrap();
    assert!(api.call("talk_send", json!({"id": c1, "message": "again"})).is_err(), "one question at a time in a chat");
    api.call("run_jobs", json!({})).unwrap();
    let t = api.call("talk_get", json!({"id": c1})).unwrap();
    assert_eq!(t["state"], "");
    assert_eq!(t["title"], "how are words stored?");
    assert_eq!(t["msgs"][1]["text"], "It stores words by lemma.");
    assert!(t["msgs"][1]["idea"].is_null());

    api.call("talk_send", json!({"id": c2, "message": "make it blue"})).unwrap();
    api.call("run_jobs", json!({})).unwrap();
    let t = api.call("talk_get", json!({"id": c2})).unwrap();
    assert_eq!(t["msgs"][1]["idea"]["status"], "proposed");
    assert_eq!(api.call("talk_get", json!({"id": c1})).unwrap()["msgs"].as_array().unwrap().len(), 2, "chats are separate");
    assert!(api.call("talk_ideas", json!({})).unwrap().as_array().unwrap().is_empty());
    let at = t["msgs"][1]["at"].as_i64().unwrap();
    api.call("talk_idea", json!({"id": c2, "at": at, "decision": "approved"})).unwrap();
    assert_eq!(api.call("talk_ideas", json!({})).unwrap()[0]["title"], "Blue theme");
    assert_eq!(api.call("talk_list", json!({})).unwrap().as_array().unwrap().len(), 2);
    api.call("talk_delete", json!({"id": c1})).unwrap();
    assert_eq!(api.call("talk_list", json!({})).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(api.call("agent_status", json!({})).unwrap()["used_today"], 2);
    std::env::remove_var("LEXIS_REPO");
}
