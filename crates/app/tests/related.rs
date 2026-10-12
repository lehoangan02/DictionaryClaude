//! Finding a quiz session from a looser word: "boring" reaches a session on *mundane*.

use gredict_app::Api;
use serde_json::json;

#[test]
fn a_related_word_finds_the_session() {
    // A stand-in for Gemini; it answers once, then the answer must come from the cache.
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let asked = std::thread::spawn(move || {
        let mut n = 0;
        while let Ok(Some(req)) = server.recv_timeout(std::time::Duration::from_secs(4)) {
            n += 1;
            let reply = json!({"candidates": [{"content": {"parts": [{"text": "Tedious, dull,\nmundane, humdrum"}]}}]});
            let _ = req.respond(tiny_http::Response::from_string(reply.to_string()));
        }
        n
    });
    std::env::set_var("GEMINI_API_KEY", "k");
    std::env::set_var("LEXIS_GEMINI_URL", format!("http://127.0.0.1:{port}"));

    let dir = std::env::temp_dir().join(format!("gredict-related-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let api = Api::open_headless(&dir).unwrap();
    api.call("note_save", json!({"body": "mundane = prosaic\nobdurate = intransigent", "session_date": "2026-10-11", "tags": "tutor"})).unwrap();
    let s = api.call("quiz_start", json!({"kind": "notes", "count": 4})).unwrap();
    for it in s["items"].as_array().unwrap() {
        api.call("quiz_answer", json!({"session_id": s["id"], "item_id": it["id"], "given": "x"})).unwrap();
    }

    let hit = api.call("sessions_list", json!({"q": "boring"})).unwrap();
    assert_eq!(hit.as_array().unwrap().len(), 1, "boring should reach the session containing mundane");
    let again = api.call("sessions_list", json!({"q": "boring"})).unwrap();
    assert_eq!(again.as_array().unwrap().len(), 1);
    assert_eq!(api.call("sessions_list", json!({"q": "cheerful"})).unwrap().as_array().unwrap().len(), 1, "the stand-in answers any word the same way; the point is the cache below");
    assert_eq!(asked.join().unwrap(), 2, "one request for boring (second search came from the cache), one for cheerful");
}
