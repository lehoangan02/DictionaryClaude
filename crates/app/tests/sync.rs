//! Two devices and a server, no network: everything is shuttled by hand
//! through the same JSON the HTTP endpoints carry.

use gredict_app::{Api, Config};
use serde_json::{json, Value};

fn device(name: &str) -> Option<Api> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() {
        return None;
    }
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("gredict-sync-{}-{n}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Some(Api::open(Config { data_dir: dir, packs_dir: root.join("data/packs"), private_dir: Some(root.join("private")) }).unwrap())
}

/// What a client does: push everything it has, then pull from the server.
fn sync(client: &Api, server: &Api, cursor: &mut i64) {
    let out = client.call("sync_export", json!({"since": 0})).unwrap();
    server.call("sync_apply", json!({"changes": out["changes"]})).unwrap();
    loop {
        let pull = server.call("sync_pull", json!({"cursor": *cursor, "limit": 3})).unwrap();
        client.call("sync_apply", json!({"changes": pull["changes"]})).unwrap();
        *cursor = pull["cursor"].as_i64().unwrap();
        if pull["more"] != true {
            break;
        }
    }
}

fn notes(a: &Api) -> Vec<String> {
    let mut v: Vec<String> = a.call("notes_list", json!({})).unwrap().as_array().unwrap().iter().map(|n| n["body"].as_str().unwrap().to_string()).collect();
    v.sort();
    v
}

#[test]
fn notes_written_offline_on_two_devices_merge_everywhere() {
    let (Some(mac), Some(phone), Some(server)) = (device("mac"), device("phone"), device("server")) else { return };
    mac.call("note_save", json!({"body": "abate = lessen", "session_date": "2026-10-10"})).unwrap();
    phone.call("note_save", json!({"body": "lucid >< opaque", "session_date": "2026-10-11"})).unwrap();
    let (mut c1, mut c2) = (0, 0);
    sync(&mac, &server, &mut c1);
    sync(&phone, &server, &mut c2);
    sync(&mac, &server, &mut c1); // mac learns the phone's note
    assert_eq!(notes(&mac), vec!["abate = lessen", "lucid >< opaque"]);
    assert_eq!(notes(&phone), notes(&mac));
    assert_eq!(notes(&server), notes(&mac));
    // The merged notes drive the thesaurus on the device that never typed them.
    let t = phone.call("thesaurus", json!({"word": "lessen"})).unwrap();
    assert_eq!(t["notes"]["same"], json!(["abate"]));
    let t = mac.call("thesaurus", json!({"word": "opaque"})).unwrap();
    assert_eq!(t["notes"]["opposite"], json!(["lucid"]));
}

#[test]
fn the_latest_edit_wins_and_deletions_propagate() {
    let (Some(mac), Some(phone), Some(server)) = (device("mac"), device("phone"), device("server")) else { return };
    let id = mac.call("note_save", json!({"body": "obdurate = stubborn"})).unwrap()["id"].as_str().unwrap().to_string();
    let (mut c1, mut c2) = (0, 0);
    sync(&mac, &server, &mut c1);
    sync(&phone, &server, &mut c2);
    assert_eq!(notes(&phone), vec!["obdurate = stubborn"]);

    // Both edit while offline; the phone edits later.
    mac.call("note_save", json!({"id": id, "body": "obdurate = stubborn = adamant"})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    phone.call("note_save", json!({"id": id, "body": "obdurate = stubborn = inflexible"})).unwrap();
    sync(&mac, &server, &mut c1);
    sync(&phone, &server, &mut c2);
    sync(&mac, &server, &mut c1);
    assert_eq!(notes(&mac), vec!["obdurate = stubborn = inflexible"], "later edit wins on every device");
    assert_eq!(notes(&mac), notes(&phone));

    // A deletion on one device removes the note (and its relations) everywhere.
    phone.call("note_delete", json!({"id": id})).unwrap();
    sync(&phone, &server, &mut c2);
    sync(&mac, &server, &mut c1);
    assert!(notes(&mac).is_empty());
    assert!(mac.call("thesaurus", json!({"word": "obdurate"})).unwrap()["notes"]["same"].as_array().unwrap().is_empty());
}

#[test]
fn replays_and_stale_copies_change_nothing() {
    let (Some(mac), Some(server)) = (device("mac"), device("server")) else { return };
    mac.call("note_save", json!({"body": "frugal ≈ thrifty"})).unwrap();
    let out = mac.call("sync_export", json!({"since": 0})).unwrap();
    let first = server.call("sync_apply", json!({"changes": out["changes"]})).unwrap();
    assert_eq!(first["applied"]["notes"], 1);
    let again = server.call("sync_apply", json!({"changes": out["changes"]})).unwrap();
    assert_eq!(again["applied"]["notes"], 0, "idempotent");

    // A stale copy of a row never overwrites a newer one.
    let mut stale = out["changes"].clone();
    stale["notes"][0]["updated"] = json!(1);
    stale["notes"][0]["body"] = json!("old text");
    let r = server.call("sync_apply", json!({"changes": stale})).unwrap();
    assert_eq!(r["applied"]["notes"], 0);
    assert_eq!(notes(&server), vec!["frugal ≈ thrifty"]);

    // Unknown tables/columns from a newer peer are ignored, not fatal.
    let mut future = out["changes"].clone();
    future["notes"][0]["updated"] = json!(i64::MAX / 2);
    future["notes"][0]["brand_new_column"] = json!("x");
    future["holograms"] = json!([{"id": "z", "updated": 1}]);
    server.call("sync_apply", json!({"changes": future})).unwrap();
}

#[test]
fn history_version_moves_when_a_lookup_arrives_by_sync() {
    let (Some(mac), Some(phone), Some(server)) = (device("mac"), device("phone"), device("server")) else { return };
    let v = |a: &Api| a.call("history_version", json!({})).unwrap()["v"].as_i64().unwrap();
    let (mut c1, mut c2) = (0, 0);
    sync(&mac, &server, &mut c1);
    let before = v(&mac);
    phone.call("entry", json!({"word": "garrulous"})).unwrap();
    sync(&phone, &server, &mut c2);
    sync(&mac, &server, &mut c1);
    let after = v(&mac);
    assert!(after > before, "the UI's History poll sees a lookup made on another device");
    sync(&mac, &server, &mut c1);
    assert_eq!(v(&mac), after, "a sync that brings nothing new leaves it alone");
}

#[test]
fn quiz_history_and_search_follow_the_user() {
    let (Some(mac), Some(phone), Some(server)) = (device("mac"), device("phone"), device("server")) else { return };
    if mac.call("quiz_catalog", json!({})).unwrap()["barrons"] != true {
        return;
    }
    // Take a quiz on the mac and miss some questions.
    let s = mac.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["u03-sense"]})).unwrap();
    for item in s["items"].as_array().unwrap() {
        mac.call("quiz_answer", json!({"session_id": s["id"], "item_id": item["id"], "given": "Sense"})).unwrap();
    }
    mac.call("history_list", json!({})).unwrap();
    mac.call("entry", json!({"word": "alacrity"})).unwrap();
    let (mut c1, mut c2) = (0, 0);
    sync(&mac, &server, &mut c1);
    sync(&phone, &server, &mut c2);

    let sessions = phone.call("sessions_list", json!({})).unwrap();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(sessions[0]["total"], 5);
    let review = phone.call("session_get", json!({"id": s["id"]})).unwrap();
    assert_eq!(review["review"].as_array().unwrap().len(), 5);
    assert!(review["review"].as_array().unwrap().iter().all(|r| !r["given"].is_null()), "attempts arrived too");
    // Retrieval search works on the phone over a session it never ran.
    let hit = phone.call("sessions_list", json!({"q": "sense nonsense"})).unwrap();
    assert!(!hit.as_array().unwrap().is_empty());
    // History merged as well.
    let h = phone.call("history_list", json!({})).unwrap();
    assert_eq!(h[0]["key"], "alacrity");

    // Deleting the session on the phone removes it from the mac.
    phone.call("session_delete", json!({"id": s["id"]})).unwrap();
    sync(&phone, &server, &mut c2);
    sync(&mac, &server, &mut c1);
    assert!(mac.call("sessions_list", json!({})).unwrap().as_array().unwrap().is_empty());
}

#[test]
fn a_fresh_device_catches_up_from_zero_in_pages() {
    let (Some(a), Some(b), Some(server)) = (device("a"), device("b"), device("server")) else { return };
    for i in 0..12 {
        a.call("note_save", json!({"body": format!("word{i} = term{i}")})).unwrap();
    }
    let (mut ca, mut cb) = (0, 0);
    sync(&a, &server, &mut ca);
    sync(&b, &server, &mut cb); // pages of 3
    assert_eq!(notes(&b).len(), 12);
    let again: Value = server.call("sync_pull", json!({"cursor": cb})).unwrap();
    assert_eq!(again["more"], false);
    assert!(again["changes"]["notes"].as_array().unwrap().is_empty(), "nothing new after catching up");
}
