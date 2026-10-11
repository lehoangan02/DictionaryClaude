//! A quiz left half way can be picked up again where it stopped.

use gredict_app::Api;
use serde_json::json;

#[test]
fn an_unfinished_quiz_resumes_with_its_progress() {
    let dir = std::env::temp_dir().join(format!("gredict-resume-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let api = Api::open_headless(&dir).unwrap();
    api.call("note_save", json!({"body": "obdurate = intransigent\nobdurate >< flexible\nimbibition ⍯ osmosis", "session_date": "2026-10-11", "tags": "tutor"})).unwrap();
    let s = api.call("quiz_start", json!({"kind": "notes", "count": 6})).unwrap();
    let total = s["total"].as_i64().unwrap();
    assert_eq!(total, 3);

    // Answer the first question wrongly, then leave.
    let first = &s["items"][0];
    let r = api.call("quiz_answer", json!({"session_id": s["id"], "item_id": first["id"], "given": "definitely not this"})).unwrap();
    assert_eq!(r["correct"], false);
    let listed = api.call("sessions_list", json!({})).unwrap();
    assert_eq!(listed[0]["ended"], json!(null), "still open");
    assert_eq!(listed[0]["answered"], 1);

    // Resuming returns the same questions, what is answered and what was missed.
    let again = api.call("quiz_resume", json!({"session_id": s["id"]})).unwrap();
    assert_eq!(again["total"], total);
    assert_eq!(again["items"], s["items"]);
    assert_eq!(again["answered"].as_array().unwrap().len(), 1);
    assert_eq!(again["answered"][0]["item_id"], first["id"]);
    assert_eq!(again["answered"][0]["correct"], false);
    assert_eq!(again["missed"].as_array().unwrap().len(), 1);
    assert!(again["missed"][0]["expected"].as_array().is_some_and(|e| !e.is_empty()));

    // Finishing the rest closes the session as usual.
    for it in again["items"].as_array().unwrap().iter().skip(1) {
        api.call("quiz_answer", json!({"session_id": s["id"], "item_id": it["id"], "given": "x"})).unwrap();
    }
    let listed = api.call("sessions_list", json!({})).unwrap();
    assert!(listed[0]["ended"].is_number());
}
