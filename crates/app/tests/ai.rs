//! The AI model inside the app: status, the self-test and meaning-aware session search.

use gredict_app::{Api, Config};
use serde_json::json;

fn api(name: &str) -> Option<Api> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() || !root.join("data/packs/bge-small-en-v1.5-q.onnx").exists() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("gredict-ai-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Some(Api::open(Config { data_dir: dir, packs_dir: root.join("data/packs"), private_dir: Some(root.join("private")) }).unwrap())
}

#[test]
fn status_and_self_test_report_the_real_backend() {
    let Some(a) = api("status") else { return };
    let s = a.call("ai_status", json!({})).unwrap();
    assert_eq!(s["ready"], true);
    assert_eq!(s["backend"], "cpu", "no NPU on this machine, and the app says so");
    let t = a.call("ai_test", json!({})).unwrap();
    assert_eq!(t["dims"], 384);
}

#[test]
fn session_search_still_finds_sessions_with_the_model_on() {
    let Some(a) = api("search") else { return };
    if a.call("quiz_catalog", json!({})).unwrap()["barrons"] != true {
        return;
    }
    let s = a.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["u03-sense"]})).unwrap();
    for item in s["items"].as_array().unwrap() {
        a.call("quiz_answer", json!({"session_id": s["id"], "item_id": item["id"], "given": "Sense"})).unwrap();
    }
    let hit = a.call("sessions_list", json!({"q": "sense nonsense"})).unwrap();
    assert_eq!(hit.as_array().unwrap().len(), 1);
    // A query sharing no keywords still ranks without error (the vectors are cached afterwards).
    let again = a.call("sessions_list", json!({"q": "sentences that make no sense"})).unwrap();
    assert!(again.as_array().unwrap().len() <= 1);
}
