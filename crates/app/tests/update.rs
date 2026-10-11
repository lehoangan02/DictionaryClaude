//! Self-update against a local stand-in for GitHub's releases.

use gredict_app::Api;
use serde_json::json;
use sha2::{Digest, Sha256};

/// The tests point the app at their own server through an environment variable, so they must not overlap.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serve(good_sum: bool) -> u16 {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let file = b"pretend this is an installer".to_vec();
    let sum: String = Sha256::digest(&file).iter().map(|b| format!("{b:02x}")).collect();
    let sum = if good_sum { sum } else { "0".repeat(64) };
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let url = req.url().to_string();
            let body: Vec<u8> = if url.starts_with("/latest") {
                let asset = |name: &str| json!({"name": name, "size": 28, "browser_download_url": format!("http://127.0.0.1:{port}/dl/{name}")});
                let names = ["LadyBird-0.7.0-macos-arm64.dmg", "LadyBird-0.7.0-x86_64.AppImage", "LadyBird-0.7.0-android-arm64.apk"];
                let mut assets: Vec<_> = names.iter().map(|n| asset(n)).collect();
                assets.extend(names.iter().filter(|n| !n.ends_with(".apk")).map(|n| asset(&format!("{n}.sha256"))));
                json!({"tag_name": "v0.7.0", "body": "What changed", "html_url": "https://example.test", "assets": assets}).to_string().into_bytes()
            } else if url.ends_with(".sha256") {
                format!("{sum}  file\n").into_bytes()
            } else {
                file.clone()
            };
            let _ = req.respond(tiny_http::Response::from_data(body));
        }
    });
    port
}

fn api(name: &str) -> Api {
    let d = std::env::temp_dir().join(format!("gredict-update-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    Api::open_headless(&d).unwrap()
}

#[test]
fn finds_a_newer_release_and_downloads_it_verified() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let port = serve(true);
    std::env::set_var("LEXIS_UPDATE_URL", format!("http://127.0.0.1:{port}/latest"));
    let a = api("ok");
    let up = a.call("update_check", json!({"current": "0.6.0"})).unwrap();
    assert_eq!(up["newer"], true);
    assert_eq!(up["latest"], "0.7.0");
    let asset = up["asset"].clone();
    assert!(asset["name"].as_str().unwrap().starts_with("LadyBird-0.7.0-"));
    let got = a.call("update_download", json!({"asset": asset})).unwrap();
    assert!(std::fs::read(got["path"].as_str().unwrap()).unwrap().starts_with(b"pretend"));
    // Already up to date: nothing newer.
    assert_eq!(a.call("update_check", json!({"current": "0.7.0"})).unwrap()["newer"], false);
    // A name that tries to leave the download folder is refused.
    assert!(a.call("update_download", json!({"asset": {"name": "../evil", "url": asset["url"]}})).is_err());
}

#[test]
fn a_download_that_fails_its_checksum_is_discarded() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let port = serve(false);
    std::env::set_var("LEXIS_UPDATE_URL", format!("http://127.0.0.1:{port}/latest"));
    let a = api("bad");
    let up = a.call("update_check", json!({"current": "0.6.0"})).unwrap();
    if up["asset"]["sha256_url"].is_null() {
        return; // Android has no checksum file
    }
    let err = a.call("update_download", json!({"asset": up["asset"]})).unwrap_err().to_string();
    assert!(err.contains("checksum"), "{err}");
}
