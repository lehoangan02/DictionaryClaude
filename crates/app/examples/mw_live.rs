//! Looks a word up in the real Merriam-Webster API from an unpaired device
//! (no sync server involved): `cargo run -p gredict-app --example mw_live -- loquacious`
use gredict_app::{Api, Config};
use serde_json::json;

fn main() {
    let word = std::env::args().nth(1).unwrap_or_else(|| "loquacious".into());
    let dir = std::env::temp_dir().join(format!("mw-live-{}", std::process::id()));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let api = Api::open(Config { data_dir: dir, packs_dir: root.join("data/none"), private_dir: None }).unwrap();
    api.seed_defaults();
    println!("paired: {}", api.call("server_status", json!({"probe": false})).unwrap()["paired"]);
    let r = api.call("entry", json!({"word": word, "source": "mw"})).unwrap();
    println!("mw_error: {}", r["mw_error"]);
    println!("pos/senses: {:?}", r["entry"]["blocks"].as_array().map(|b| b.iter().map(|x| (x["pos"].as_str().unwrap_or("").to_string(), x["senses"].as_array().map_or(0, Vec::len))).collect::<Vec<_>>()));
    println!("ipa: {}", r["entry"]["blocks"][0]["sounds"][0]["ipa"]);
    println!("syllables: {}", r["syl"]);
    println!("synonyms: {}", r["entry"]["blocks"][0]["synonyms"]);
    println!("mw_status: {}", api.call("mw_status", json!({})).unwrap());
    let r2 = api.call("entry", json!({"word": "japery", "source": "mw"})).unwrap();
    println!("japery blocks: {:?}", r2["entry"]["blocks"].as_array().map(|b| b.iter().map(|x| x["pos"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>()));
}
