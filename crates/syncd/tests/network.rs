//! Real TLS, real sockets: devices pair with a server and sync through it.

use std::sync::Arc;

use gredict_app::{Api, Config};
use gredict_syncd::{ensure_tls, serve, serve_enrolling, Enroll, Pairing, Tokens};
use serde_json::{json, Value};

/// These tests set a process-wide env var (the pretend Wi-Fi name), so they
/// must not overlap.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn tmp(name: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("gredict-net-{}-{}-{name}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn client(name: &str) -> Option<Api> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() {
        return None;
    }
    Some(Api::open(Config { data_dir: tmp(name), packs_dir: root.join("data/packs"), private_dir: None }).unwrap())
}

struct Server {
    api: Arc<Api>,
    lan: u16,
    wan: u16,
    dir: std::path::PathBuf,
    tokens: Tokens,
    cert: String,
}

fn start() -> Server {
    let dir = tmp("server");
    std::fs::create_dir_all(&dir).unwrap();
    let tls = ensure_tls(&dir.join("tls"), &["127.0.0.1".into(), "localhost".into()]).unwrap();
    let (lan, wan) = (free_port(), free_port());
    let api = Arc::new(Api::open_headless(&dir).unwrap());
    let tokens = Arc::new(Tokens::new(&dir));
    let private = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../private");
    serve(api.clone(), tokens, private, &tls, &[format!("127.0.0.1:{lan}"), format!("127.0.0.1:{wan}")]).unwrap();
    Server { api, lan, wan, dir: dir.clone(), tokens: Tokens::new(&dir), cert: tls.cert_pem }
}

fn code(s: &Server, name: &str, lan: Option<u16>, wan: Option<u16>) -> String {
    Pairing { v: 1, name: name.into(), lan: lan.map(|p| format!("127.0.0.1:{p}")), wan: wan.map(|p| format!("127.0.0.1:{p}")), token: s.tokens.issue(name).unwrap(), cert: s.cert.clone() }.encode()
}

fn bodies(a: &Api) -> Vec<String> {
    let mut v: Vec<String> = a.call("notes_list", json!({})).unwrap().as_array().unwrap().iter().map(|n| n["body"].as_str().unwrap().into()).collect();
    v.sort();
    v
}

#[test]
fn two_devices_converge_through_the_server_over_tls() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (Some(mac), Some(phone)) = (client("mac"), client("phone")) else { return };
    std::env::set_var("LEXIS_WIFI", "Anan");
    let s = start();
    mac.call("sync_pair", json!({"code": code(&s, "mac", Some(s.lan), Some(s.wan))})).unwrap();
    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(s.lan), Some(s.wan))})).unwrap();

    mac.call("note_save", json!({"body": "abate = lessen"})).unwrap();
    phone.call("note_save", json!({"body": "lucid >< opaque"})).unwrap();
    let r = mac.call("sync_now", json!({})).unwrap();
    assert_eq!(r["route"], "lan", "on the home Wi-Fi the LAN route is used");
    phone.call("sync_now", json!({})).unwrap();
    mac.call("sync_now", json!({})).unwrap();
    assert_eq!(bodies(&mac), vec!["abate = lessen", "lucid >< opaque"]);
    assert_eq!(bodies(&phone), bodies(&mac));

    // The server holds the merged data too.
    let server = Api::open_headless(&s.dir).unwrap();
    assert_eq!(bodies(&server), bodies(&mac));

    // Status for the header pill.
    let st = mac.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!(st["online"], true);
    assert_eq!(st["route"], "lan");
    assert_eq!(st["pending"], 0, "everything was pushed");
    mac.call("note_save", json!({"body": "frugal ≈ thrifty"})).unwrap();
    assert_eq!(mac.call("server_status", json!({})).unwrap()["pending"], 1);
}

#[test]
fn away_from_home_uses_the_remote_port_and_vpn_cannot_fool_it() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(phone) = client("phone") else { return };
    let s = start();
    // The LAN address points at a port nothing listens on, like another network.
    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(free_port()), Some(s.wan))})).unwrap();

    std::env::set_var("LEXIS_WIFI", "Starbucks");
    let st = phone.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!((st["online"].clone(), st["route"].clone()), (json!(true), json!("wan")));
    assert!(st["probes"].as_array().unwrap().iter().all(|p| p["route"] == "wan"), "LAN is not even tried away from home");

    // Unknown Wi-Fi (permission denied): both are tried and the one that verifies wins.
    std::env::set_var("LEXIS_WIFI", "");
    let st = phone.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!(st["route"], "wan");
    assert_eq!(st["probes"].as_array().unwrap().len(), 2);

    // At home but the LAN path is dead (VPN/odd router): fall back to remote.
    std::env::set_var("LEXIS_WIFI", "Anan 5g");
    assert_eq!(phone.call("server_status", json!({"probe": true})).unwrap()["route"], "wan");
    assert_eq!(phone.call("sync_now", json!({})).unwrap()["route"], "wan");
}

#[test]
fn wrong_token_wrong_certificate_and_dead_server_are_reported_not_trusted() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(phone) = client("phone") else { return };
    std::env::set_var("LEXIS_WIFI", "Anan");
    let s = start();

    // A token the server never issued.
    let mut p = Pairing::decode(&code(&s, "phone", Some(s.lan), Some(s.wan))).unwrap();
    p.token = "not-a-real-token".into();
    phone.call("sync_pair", json!({"code": p.encode()})).unwrap();
    let st = phone.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!(st["online"], false);
    assert!(st["probes"][0]["error"].as_str().unwrap().to_lowercase().contains("unauthorized"));
    assert!(phone.call("sync_now", json!({})).is_err());

    // A different server certificate (an impostor, or a replaced server): refused.
    let impostor = start();
    let mut p = Pairing::decode(&code(&s, "phone", Some(s.lan), Some(s.wan))).unwrap();
    p.cert = impostor.cert.clone();
    phone.call("sync_pair", json!({"code": p.encode()})).unwrap();
    let st = phone.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!(st["online"], false, "a certificate that is not the paired one is never trusted");

    // Nothing listening at all.
    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(free_port()), Some(free_port()))})).unwrap();
    let st = phone.call("server_status", json!({"probe": true})).unwrap();
    assert_eq!(st["online"], false);
    let e = phone.call("sync_now", json!({})).unwrap_err().to_string();
    assert!(e.contains("not reachable"), "{e}");

    // Unpairing forgets everything about the server.
    phone.call("sync_unpair", json!({})).unwrap();
    assert_eq!(phone.call("server_status", json!({})).unwrap()["paired"], false);
    let _: Value = phone.call("notes_list", json!({})).unwrap();
}

#[test]
fn a_new_device_receives_the_barrons_book_when_it_pairs() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if !root.join("data/packs/wiktionary.pack").exists() || !root.join("private/barrons.sqlite").exists() {
        return;
    }
    std::env::set_var("LEXIS_WIFI", "Anan");
    let s = start();
    // A phone with dictionaries but no book.
    let private = tmp("phone-private");
    let phone = Api::open(Config { data_dir: tmp("phone"), packs_dir: root.join("data/packs"), private_dir: Some(private.clone()) }).unwrap();
    assert_eq!(phone.call("quiz_catalog", json!({})).unwrap()["barrons"], false);
    assert!(phone.call("entry", json!({"word": "loquacious", "record": false})).unwrap()["barrons"].is_null());

    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(s.lan), Some(s.wan))})).unwrap();
    let r = phone.call("sync_now", json!({})).unwrap();
    assert_eq!(r["barrons"], "received");
    // No restart needed: quizzes and the Barron's card work immediately.
    let cat = phone.call("quiz_catalog", json!({})).unwrap();
    assert_eq!(cat["barrons"], true);
    assert_eq!(cat["units"].as_array().unwrap().len(), 80);
    assert!(!phone.call("entry", json!({"word": "loquacious", "record": false})).unwrap()["barrons"].is_null());
    assert!(private.join("barrons.sqlite").exists());
    assert!(!private.join("barrons.sqlite.part").exists(), "no leftover partial download");

    // The next sync sees the book is current and downloads nothing.
    assert_eq!(phone.call("sync_now", json!({})).unwrap()["barrons"], "up to date");
}

#[test]
fn photographed_notes_travel_to_the_server_get_read_and_reach_another_device() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let have = |t: &str| std::process::Command::new(t).arg("--version").output().is_ok();
    if !have("tesseract") || !have("convert") {
        return;
    }
    let (Some(phone), Some(mac)) = (client("phone"), client("mac")) else { return };
    std::env::set_var("LEXIS_WIFI", "Anan");
    std::env::set_var("LEXIS_OCR", "tesseract");
    let s = start();
    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(s.lan), Some(s.wan))})).unwrap();
    mac.call("sync_pair", json!({"code": code(&s, "mac", Some(s.lan), Some(s.wan))})).unwrap();

    // The phone photographs a page.
    let img = tmp("photo").join("p.jpg");
    std::fs::create_dir_all(img.parent().unwrap()).unwrap();
    assert!(std::process::Command::new("convert").args(["-size", "1000x300", "xc:white", "-fill", "black", "-pointsize", "52", "-annotate", "+40+120", "obdurate = intransigent"]).arg(&img).status().unwrap().success());
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&img).unwrap());
    let id = phone.call("draft_create", json!({"photos": [b64], "session_date": "2026-10-11"})).unwrap()["id"].as_str().unwrap().to_string();

    // Syncing sends the photo first, then the row that names it.
    let r = phone.call("sync_now", json!({})).unwrap();
    assert_eq!(r["photos"], 1);
    assert_eq!(phone.call("sync_now", json!({})).unwrap()["photos"], 0, "sent once");
    assert_eq!(s.api.call("draft_get", json!({"id": id})).unwrap()["status"], "queued");

    // The server reads it.
    s.api.call("run_jobs", json!({})).unwrap();
    let on_server = s.api.call("draft_get", json!({"id": id})).unwrap();
    assert_eq!(on_server["status"], "needs_review");
    assert!(on_server["text"].as_str().unwrap().to_lowercase().contains("obdurate"), "{on_server}");

    // The Mac, which never saw the photo, receives the draft and can view the photo.
    mac.call("sync_now", json!({})).unwrap();
    let list = mac.call("drafts_list", json!({})).unwrap();
    assert_eq!(list[0]["id"], id.as_str());
    assert_eq!(list[0]["status"], "needs_review");
    let name = list[0]["photos"][0].as_str().unwrap();
    let pd = mac.call("photo_data", json!({"name": name})).unwrap();
    assert!(pd["data_url"].as_str().unwrap().starts_with("data:image/jpeg;base64,"));

    // It is corrected and accepted on the Mac; the note reaches the phone.
    mac.call("draft_update", json!({"id": id, "text": "obdurate = intransigent = adamant"})).unwrap();
    mac.call("draft_accept", json!({"id": id})).unwrap();
    mac.call("sync_now", json!({})).unwrap();
    phone.call("sync_now", json!({})).unwrap();
    let notes = phone.call("notes_list", json!({})).unwrap();
    assert_eq!(notes[0]["body"], "obdurate = intransigent = adamant");
    assert_eq!(phone.call("drafts_list", json!({})).unwrap().as_array().unwrap().len(), 0, "the draft is closed everywhere");
    assert_eq!(phone.call("thesaurus", json!({"word": "adamant"})).unwrap()["notes"]["same"], json!(["intransigent", "obdurate"]));
}

#[test]
fn a_device_downloads_dictionaries_from_the_server_resumes_and_checks_them() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // A fake pack: any SQLite-headed file will do for the transfer.
    let dir = tmp("packsrv");
    let packs = dir.join("packs");
    std::fs::create_dir_all(&packs).unwrap();
    let mut body = b"SQLite format 3\0".to_vec();
    body.extend((0..3_000_000u32).map(|i| (i % 251) as u8));
    std::fs::write(packs.join("wordnet.pack"), &body).unwrap();
    std::fs::write(packs.join("notes.txt"), "not offered").unwrap();

    let tls = ensure_tls(&dir.join("tls"), &["127.0.0.1".into(), "localhost".into()]).unwrap();
    let (lan, wan) = (free_port(), free_port());
    let api = Arc::new(Api::open_headless_serving(&dir, &packs).unwrap());
    let tokens = Arc::new(Tokens::new(&dir));
    serve(api, tokens, dir.join("private"), &tls, &[format!("127.0.0.1:{lan}"), format!("127.0.0.1:{wan}")]).unwrap();
    let s = Server { api: Arc::new(Api::open_headless(&dir.join("x")).unwrap()), lan, wan, dir: dir.clone(), tokens: Tokens::new(&dir), cert: tls.cert_pem };

    let phone_dir = tmp("phone");
    let phone = Api::open(Config { data_dir: phone_dir.clone(), packs_dir: tmp("nopacks"), private_dir: None }).unwrap();
    // A brand-new device has no dictionaries and says so plainly.
    assert!(phone.call("search", json!({"q": "abate"})).unwrap_err().to_string().contains("no dictionaries are installed"));
    phone.call("sync_pair", json!({"code": code(&s, "phone", Some(lan), Some(wan))})).unwrap();

    let list = phone.call("packs_catalog", json!({})).unwrap();
    let wn = list["packs"].as_array().unwrap().iter().find(|p| p["id"] == "wordnet").unwrap().clone();
    assert_eq!(wn["on_server"], true);
    assert_eq!(wn["downloaded"], false);
    let own = phone.call("packs_list", json!({})).unwrap();
    assert_eq!(own["paired"], true);
    // An interrupted earlier download resumes from what it already has.
    std::fs::create_dir_all(phone_dir.join("packs")).unwrap();
    std::fs::write(phone_dir.join("packs/wordnet.pack.part"), &body[..1_000_000]).unwrap();
    let r = phone.call("pack_install", json!({"id": "wordnet"})).unwrap();
    assert_eq!(std::fs::read(phone_dir.join("packs/wordnet.pack")).unwrap(), body);
    assert_eq!(r["reload"], true);
    let list = phone.call("packs_list", json!({})).unwrap();
    let wn = list["packs"].as_array().unwrap().iter().find(|p| p["id"] == "wordnet").unwrap();
    assert_eq!(wn["downloaded"], true);
    assert_eq!(wn["update"], false);
    // Removing frees it again.
    phone.call("pack_remove", json!({"id": "wordnet"})).unwrap();
    let list = phone.call("packs_list", json!({})).unwrap();
    assert_eq!(list["packs"].as_array().unwrap().iter().find(|p| p["id"] == "wordnet").unwrap()["downloaded"], false);
    // Only pack files are offered; a bad name is refused.
    assert!(phone.call("pack_install", json!({"id": "../../etc/passwd"})).is_err());
}

#[test]
fn a_device_joins_by_itself_on_the_home_port_only() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("LEXIS_WIFI", "Anan");
    let dir = tmp("auto-server");
    std::fs::create_dir_all(&dir).unwrap();
    let tls = ensure_tls(&dir.join("tls"), &["127.0.0.1".into(), "localhost".into()]).unwrap();
    let (lan, wan) = (format!("127.0.0.1:{}", free_port()), format!("127.0.0.1:{}", free_port()));
    let server = Arc::new(Api::open_headless(&dir).unwrap());
    let enroll = Some((Arc::new(Enroll::new(&lan, &wan, &tls.cert_pem)), lan.clone()));
    serve_enrolling(server, Arc::new(Tokens::new(&dir)), dir.join("none"), &tls, &[lan.clone(), wan.clone()], enroll).unwrap();

    // The internet-facing port never enrols, whoever asks.
    let outside = Api::open_headless(&tmp("auto-outside")).unwrap();
    outside.call("sync_set_autopair_host", json!({"host": wan})).unwrap();
    assert!(outside.call("sync_autopair", json!({})).is_err());
    assert_eq!(outside.call("server_status", json!({})).unwrap()["paired"], false);

    // The home port does, and the result is a working pairing.
    let phone = Api::open_headless(&tmp("auto-phone")).unwrap();
    phone.call("sync_set_autopair_host", json!({"host": lan})).unwrap();
    assert_eq!(phone.call("sync_autopair", json!({})).unwrap()["paired"], true);
    phone.call("note_save", json!({"body": "abate = lessen"})).unwrap();
    assert_eq!(phone.call("sync_now", json!({})).unwrap()["route"], "lan");

    // A second device with a similar name gets its own token; nobody is replaced.
    let tablet = Api::open_headless(&tmp("auto-tablet")).unwrap();
    tablet.call("sync_set_autopair_host", json!({"host": lan})).unwrap();
    tablet.call("sync_autopair", json!({})).unwrap();
    assert_eq!(Tokens::new(&dir).list().len(), 2);
    phone.call("sync_now", json!({})).unwrap();

    // Unpairing on purpose turns automatic pairing off.
    phone.call("sync_unpair", json!({})).unwrap();
    assert_eq!(phone.call("server_status", json!({})).unwrap()["auto"], false);
}
