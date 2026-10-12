//! Oxford lookups: asked once, kept in the migrated cache, still there after a restart.

use gredict_app::Api;
use serde_json::json;

#[test]
fn an_oxford_word_is_paid_for_once_and_survives_a_restart() {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let asked = std::thread::spawn(move || {
        let mut seen = Vec::new();
        while let Ok(Some(req)) = server.recv_timeout(std::time::Duration::from_secs(3)) {
            let id = req.headers().iter().find(|h| h.field.equiv("app_id")).map(|h| h.value.to_string());
            seen.push((req.url().to_string(), id));
            let body = if req.url().contains("/zzzz") {
                tiny_http::Response::from_string("{}").with_status_code(404)
            } else {
                tiny_http::Response::from_string(
                    json!({"results": [{"lexicalEntries": [{"lexicalCategory": {"text": "Adjective"}, "entries": [{"senses": [{"definitions": ["lacking interest or excitement"]}]}]}]}]}).to_string(),
                )
            };
            let _ = req.respond(body);
        }
        seen
    });
    std::env::set_var("OXFORD_APP_ID", "the-id");
    std::env::set_var("OXFORD_APP_KEY", "the-key");
    std::env::set_var("LEXIS_OXFORD_BASE", format!("http://127.0.0.1:{port}"));

    let dir = std::env::temp_dir().join(format!("gredict-oxford-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let api = Api::open_headless(&dir).unwrap();
        let e = api.call("entry", json!({"word": "mundane", "source": "ox", "record": false})).unwrap();
        assert_eq!(e["entry"]["blocks"][0]["senses"][0]["gloss"], "lacking interest or excitement");
        // The second look-up is answered from the kept copy.
        api.call("entry", json!({"word": "mundane", "source": "ox", "record": false})).unwrap();
        // A word Oxford does not have is kept as "none", so it is not asked again either.
        let none = api.call("entry", json!({"word": "zzzz", "source": "ox", "record": false})).unwrap();
        assert!(none["entry"].is_null());
        api.call("entry", json!({"word": "zzzz", "source": "ox", "record": false})).unwrap();
        assert_eq!(api.call("ox_status", json!({})).unwrap()["cache_entries"], 2);
    }
    // A restart (an update reopens the database and runs any new migrations): nothing is asked again.
    let api = Api::open_headless(&dir).unwrap();
    let again = api.call("entry", json!({"word": "mundane", "source": "ox", "record": false})).unwrap();
    assert_eq!(again["entry"]["blocks"][0]["pos"], "adjective");

    let seen = asked.join().unwrap();
    assert_eq!(seen.len(), 2, "one request per word, ever: {seen:?}");
    assert_eq!(seen[0].1.as_deref(), Some("the-id"));
    assert!(seen[0].0.contains("/entries/en-gb/mundane"));
}
