//! Merriam-Webster integration against a local mock of their service.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gredict_app::{Api, Config};
use serde_json::{json, Value};

static SERIAL: Mutex<()> = Mutex::new(());

struct Mock {
    hits: Arc<AtomicUsize>,
    port: u16,
}

fn mock() -> Mock {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            h.fetch_add(1, Ordering::SeqCst);
            let url = req.url().to_string();
            let body = if url.contains("key=BADKEY") {
                "Invalid API key. Not subscribed for this reference.".to_string()
            } else if url.contains("/thesaurus/") {
                json!([{"fl": "adjective", "meta": {"syns": [["garrulous", "talkative", "voluble"]], "ants": [["taciturn", "reticent"]]}}]).to_string()
            } else if url.contains("/loquacious") {
                json!([{"hwi": {"hw": "lo*qua*cious", "prs": [{"mw": "lō-ˈkwā-shəs", "sound": {"audio": "loquac01"}}]}, "fl": "adjective",
                        "et": [["text", "Latin {it}loquac-{/it}"]], "shortdef": ["given to continual talking : {sx|talkative||}"]}]).to_string()
            } else {
                json!(["loquacious", "loquaciously"]).to_string()
            };
            let _ = req.respond(tiny_http::Response::from_string(body));
        }
    });
    Mock { hits, port }
}

fn api(name: &str) -> Option<Api> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("gredict-mw-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Some(Api::open(Config { data_dir: dir, packs_dir: root.join("data/packs"), private_dir: None }).unwrap())
}

fn setup(name: &str) -> Option<(Api, Mock)> {
    let api = api(name)?;
    let m = mock();
    std::env::set_var("LEXIS_MW_BASE", format!("http://127.0.0.1:{}/api", m.port));
    Some((api, m))
}

fn names(api: &Api) -> Vec<String> {
    api.call("sources", json!({})).unwrap().as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_string()).collect()
}

#[test]
fn appears_only_with_a_key_and_maps_the_entry() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((api, m)) = setup("map") else { return };
    assert!(!names(&api).contains(&"mw".to_string()), "no key, no Merriam-Webster tab");
    api.call("mw_config", json!({"collegiate": "KEY1", "thesaurus": "KEY2"})).unwrap();
    assert!(names(&api).contains(&"mw".to_string()));

    let e = api.call("entry", json!({"word": "loquacious", "source": "mw"})).unwrap();
    assert_eq!(e["source"]["id"], "mw");
    let b = &e["entry"]["blocks"][0];
    assert_eq!(b["pos"], "adjective");
    assert_eq!(b["senses"][0]["gloss"], "given to continual talking: talkative");
    assert_eq!(b["sounds"][0]["ipa"], "\\lō-ˈkwā-shəs\\");
    assert_eq!(b["synonyms"], json!(["garrulous", "talkative", "voluble"]), "thesaurus merged in");
    assert_eq!(b["antonyms"], json!(["taciturn", "reticent"]));
    assert_eq!(m.hits.load(Ordering::SeqCst), 2, "one lookup in each of the two reference works");
    // A lookup is recorded in history like any other.
    assert_eq!(api.call("history_list", json!({})).unwrap()[0]["source"], "mw");
}

#[test]
fn words_you_looked_up_are_kept_for_offline_unless_you_say_no() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((api, m)) = setup("cache") else { return };
    api.call("mw_config", json!({"collegiate": "KEY1"})).unwrap();
    api.call("entry", json!({"word": "loquacious", "source": "mw", "record": false})).unwrap();
    assert_eq!(m.hits.load(Ordering::SeqCst), 1);
    // Point at a dead address: the saved copy still answers.
    std::env::set_var("LEXIS_MW_BASE", "http://127.0.0.1:1/api");
    let e = api.call("entry", json!({"word": "loquacious", "source": "mw", "record": false})).unwrap();
    assert_eq!(e["entry"]["blocks"][0]["pos"], "adjective");
    assert_eq!(api.call("mw_status", json!({})).unwrap()["cache_entries"], 1);
    // A word never looked up, while offline, explains itself.
    let miss = api.call("entry", json!({"word": "obdurate", "source": "mw", "record": false})).unwrap();
    assert!(miss["entry"].is_null() && miss["mw_error"].as_str().unwrap().contains("not reachable"));

    // Opting out of keeping entries means every lookup asks again.
    std::env::set_var("LEXIS_MW_BASE", format!("http://127.0.0.1:{}/api", m.port));
    api.call("mw_config", json!({"keep_offline": false, "clear_cache": true})).unwrap();
    let before = m.hits.load(Ordering::SeqCst);
    api.call("entry", json!({"word": "loquacious", "source": "mw", "record": false})).unwrap();
    api.call("entry", json!({"word": "loquacious", "source": "mw", "record": false})).unwrap();
    assert_eq!(m.hits.load(Ordering::SeqCst) - before, 2);
    assert_eq!(api.call("mw_status", json!({})).unwrap()["cache_entries"], 0);
}

#[test]
fn suggestions_bad_keys_and_the_daily_limit() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((api, _m)) = setup("limits") else { return };
    api.call("mw_config", json!({"collegiate": "KEY1", "keep_offline": false})).unwrap();
    let s = api.call("entry", json!({"word": "loquatious", "source": "mw", "record": false})).unwrap();
    assert!(s["entry"].is_null());
    assert_eq!(s["did_you_mean"], json!(["loquacious", "loquaciously"]));

    // A rejected key is reported without ever echoing the key.
    api.call("mw_config", json!({"collegiate": "BADKEY"})).unwrap();
    let bad = api.call("entry", json!({"word": "abate", "source": "mw", "record": false})).unwrap();
    let msg = bad["mw_error"].as_str().unwrap();
    assert!(msg.contains("did not accept the key"), "{msg}");
    assert!(!msg.contains("BADKEY") && !bad.to_string().contains("BADKEY"));
    assert!(!api.call("mw_status", json!({})).unwrap().to_string().contains("BADKEY"), "keys are never sent back");

    // The app stops short of the 1,000-a-day allowance.
    api.call("mw_config", json!({"collegiate": "KEY1"})).unwrap();
    let mut refused = None;
    for i in 0..1100 {
        let r = api.call("entry", json!({"word": format!("word{i}"), "source": "mw", "record": false})).unwrap();
        if let Some(e) = r["mw_error"].as_str() {
            refused = Some((i, e.to_string()));
            break;
        }
    }
    let (i, e) = refused.expect("limit reached");
    assert!((945..=955).contains(&i), "stopped at {i}");
    assert!(e.contains("nearly used up"));
    let st: Value = api.call("mw_status", json!({})).unwrap();
    assert!(st["used_today"].as_i64().unwrap() >= 950);
}

#[test]
fn shipped_keys_fill_in_automatically_and_removal_sticks() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some((api, _m)) = setup("defaults") else { return };
    assert!(!names(&api).contains(&"mw".to_string()), "a bare Api has no keys");
    api.seed_defaults();
    assert!(names(&api).contains(&"mw".to_string()), "the app shells start with Merriam-Webster set up");
    let st = api.call("mw_status", json!({})).unwrap();
    assert_eq!((st["configured"].clone(), st["thesaurus"].clone()), (json!(true), json!(true)));
    assert!(!st.to_string().contains("97e9e4a1"), "keys are still never sent to the UI");

    // Removing them on purpose is respected on the next start.
    api.call("mw_config", json!({"collegiate": "", "thesaurus": ""})).unwrap();
    api.seed_defaults();
    assert!(!names(&api).contains(&"mw".to_string()));
    // Typing a key of your own brings it back, and it wins over the shipped one.
    api.call("mw_config", json!({"collegiate": "MINE"})).unwrap();
    api.seed_defaults();
    assert!(names(&api).contains(&"mw".to_string()));
}
