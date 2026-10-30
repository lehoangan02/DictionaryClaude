//! The thesaurus: WordNet meanings from `wnthes.pack`, Moby ranked instead of cut alphabetically,
//! and an inflected form reaching the thesaurus of its lemma.

use gredict_app::{Api, Config};
use gredict_core::thesaurus::SCHEMA;
use serde_json::{json, Value};

fn put(path: &std::path::Path, rows: &[(&str, Value)]) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    for (key, list) in rows {
        let blob = zstd::bulk::compress(list.to_string().as_bytes(), 3).unwrap();
        conn.execute("INSERT INTO syn VALUES (?,?,?)", rusqlite::params![key, key, blob]).unwrap();
    }
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array().unwrap().iter().filter_map(Value::as_str).collect()
}

#[test]
fn meanings_ranked_moby_and_inflected_forms() {
    let dir = std::env::temp_dir().join(format!("gredict-thes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("packs")).unwrap();

    put(&dir.join("packs/wnthes.pack"), &[(
        "spread",
        json!([{"pos": "verb", "gloss": "cause to become widely known", "s": ["circulate", "broadcast"], "b": ["publicize"], "n": ["sow"]}]),
    )]);
    // Moby is alphabetical: "zeta" comes last but names "spread" back, "alpha" does not, "broadcast" is WordNet's.
    let mut moby: Vec<String> = (0..60).map(|i| format!("filler{i:02}")).collect();
    moby.extend(["alpha", "broadcast", "zeta"].map(String::from));
    moby.sort();
    put(&dir.join("packs/moby.pack"), &[("spread", json!(moby)), ("zeta", json!(["spread"])), ("alpha", json!(["other"]))]);
    // "spreads" is an inflected form whose lemma the dictionary knows.
    let conn = rusqlite::Connection::open(dir.join("packs/wiktionary.pack")).unwrap();
    conn.execute_batch(gredict_core::pack::SCHEMA).unwrap();
    let e = json!({"word": "spreads", "blocks": [{"pos": "verb", "senses": [{"gloss": "third-person singular of spread", "form_of": "spread"}]}]});
    conn.execute("INSERT INTO entries VALUES (?,?,?)", rusqlite::params!["spreads", "spreads", zstd::bulk::compress(e.to_string().as_bytes(), 3).unwrap()]).unwrap();
    drop(conn);

    let api = Api::open(Config { data_dir: dir.join("data"), packs_dir: dir.join("packs"), private_dir: None }).unwrap();
    let t = api.call("thesaurus", json!({"word": "spread"})).unwrap();
    let s = &t["senses"][0];
    assert_eq!(s["gloss"], "cause to become widely known");
    assert_eq!(strings(&s["synonyms"]), ["circulate", "broadcast"]);
    assert_eq!(strings(&s["broader"]), ["publicize"]);
    assert_eq!(strings(&s["narrower"]), ["sow"]);
    assert_eq!(t["via"], Value::Null);

    // WordNet's word first, then the one Moby names back, before the alphabet.
    let m = strings(&t["moby"]);
    assert_eq!(&m[..3], ["broadcast", "zeta", "alpha"], "{m:?}");

    let f = api.call("thesaurus", json!({"word": "spreads"})).unwrap();
    assert_eq!(f["via"]["word"], "spread");
    assert_eq!(f["senses"][0]["gloss"], "cause to become widely known");
}
