//! The word-forms command: paradigm tables, an inflected word reaching its lemma, a typo, the word family.

use gredict_app::{Api, Config};
use gredict_core::pack::SCHEMA;
use serde_json::{json, Value};

fn tiny_wiktionary(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    let entries = [
        ("go", json!({"word": "go", "blocks": [
            {"pos": "verb", "head": "go (...)", "forms": [
                {"form": "goes", "tags": ["present", "singular", "third-person"]},
                {"form": "going", "tags": ["participle", "present"]},
                {"form": "went", "tags": ["past"]},
                {"form": "gone", "tags": ["participle", "past"]}],
             "senses": [{"gloss": "to move"}]},
            {"pos": "noun", "head": "go (plural goes)", "forms": [{"form": "goes", "tags": ["plural"]}], "senses": [{"gloss": "a turn"}]}]})),
        ("went", json!({"word": "went", "blocks": [{"pos": "verb", "senses": [{"gloss": "simple past of go", "form_of": "go"}]}]})),
        ("be", json!({"word": "be", "blocks": [{"pos": "noun", "senses": [{"gloss": "the letter B"}, {"gloss": "abbreviation of by", "form_of": "by"}]}]})),
        ("by", json!({"word": "by", "blocks": [{"pos": "adv", "head": "by (not comparable)", "senses": [{"gloss": "near"}]}]})),
        ("loquacious", json!({"word": "loquacious", "blocks": [
            {"pos": "adj", "head": "loquacious (comparative more loquacious, superlative most loquacious)", "forms": [
                {"form": "more loquacious", "tags": ["comparative"]}, {"form": "most loquacious", "tags": ["superlative"]}],
             "senses": [{"gloss": "talkative"}]}]})),
    ];
    for (key, e) in entries {
        let blob = zstd::bulk::compress(e.to_string().as_bytes(), 3).unwrap();
        conn.execute("INSERT INTO entries VALUES (?,?,?)", rusqlite::params![key, e["word"].as_str().unwrap(), blob]).unwrap();
    }
}

/// A family pack where "loquacious" is built from "loquacity" and "loquaciously" from "loquacious".
fn tiny_family(path: &std::path::Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(gredict_core::family::SCHEMA).unwrap();
    let words = [
        ("loquacity", json!({"d": [["loquacious", "-ous"]], "r": ["loquor"]})),
        ("loquacious", json!({"u": [["loquacity", "-ous"]], "d": [["loquaciously", "-ly"]], "s": 1})),
        ("loquaciously", json!({"u": [["loquacious", "-ly"]]})),
    ];
    for (key, l) in words {
        let blob = zstd::bulk::compress(l.to_string().as_bytes(), 3).unwrap();
        conn.execute("INSERT INTO fam VALUES (?,?)", rusqlite::params![key, blob]).unwrap();
    }
    let group = zstd::bulk::compress(json!(["loquacious", "loquacity", "loquaciously", "loquaciousness"]).to_string().as_bytes(), 3).unwrap();
    conn.execute("INSERT INTO stem VALUES (1, ?)", [group]).unwrap();
}

fn rows(table: &Value) -> Vec<(String, Vec<String>)> {
    table["rows"].as_array().unwrap().iter()
        .map(|r| (r["label"].as_str().unwrap().to_string(), r["forms"].as_array().unwrap().iter().map(|c| c["form"].as_str().unwrap().to_string()).collect()))
        .collect()
}

#[test]
fn forms_of_a_word_its_inflected_form_and_a_typo() {
    let dir = std::env::temp_dir().join(format!("gredict-forms-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("packs")).unwrap();
    tiny_wiktionary(&dir.join("packs/wiktionary.pack"));
    let api = Api::open(Config { data_dir: dir.join("data"), packs_dir: dir.join("packs"), private_dir: None }).unwrap();

    let go = api.call("forms", json!({"word": "Go"})).unwrap();
    let g = &go["groups"][0];
    assert_eq!(go["groups"].as_array().unwrap().len(), 1);
    assert_eq!(g["lemma"], "go");
    assert_eq!(g["via"], Value::Null);
    let verb = rows(&g["tables"][0]);
    assert_eq!(verb[1], ("Third person singular".into(), vec!["goes".into()]));
    assert_eq!(verb[3], ("Past".into(), vec!["went".into()]));
    assert_eq!(rows(&g["tables"][1])[1], ("Plural".into(), vec!["goes".into()]));

    // "went" has no table of its own: it leads to the lemma it is a form of.
    let went = api.call("forms", json!({"word": "went"})).unwrap();
    assert_eq!(went["groups"].as_array().unwrap().len(), 1);
    assert_eq!(went["groups"][0]["lemma"], "go");
    assert_eq!(went["groups"][0]["via"], "went");

    // A sense that merely points at another word ("abbreviation of by") is not an inflection of it.
    let be = api.call("forms", json!({"word": "be"})).unwrap();
    assert_eq!(be["groups"].as_array().unwrap().len(), 1);
    assert_eq!(be["groups"][0]["lemma"], "be");

    let adj = api.call("forms", json!({"word": "loquacious"})).unwrap();
    assert_eq!(rows(&adj["groups"][0]["tables"][0])[2], ("Superlative".into(), vec!["most loquacious".into()]));

    let none = api.call("forms", json!({"word": "zzzxq"})).unwrap();
    assert!(none["groups"].as_array().unwrap().is_empty());

    // No family pack yet: the tables still show and the page says what to download.
    assert_eq!(adj["family"], Value::Null);

    // Without the dictionary the command says what to download instead of showing nothing.
    let bare = Api::open_headless(&dir.join("bare")).unwrap();
    assert!(bare.call("forms", json!({"word": "go"})).unwrap_err().to_string().contains("Wiktionary"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_family_of_a_word_comes_from_the_family_pack() {
    let dir = std::env::temp_dir().join(format!("gredict-family-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("packs")).unwrap();
    tiny_wiktionary(&dir.join("packs/wiktionary.pack"));
    tiny_family(&dir.join("packs/family.pack"));
    let api = Api::open(Config { data_dir: dir.join("data"), packs_dir: dir.join("packs"), private_dir: None }).unwrap();

    let f = api.call("forms", json!({"word": "loquacious"})).unwrap()["family"].clone();
    let tree: Vec<(u64, &str, &str)> = f["members"].as_array().unwrap().iter().map(|m| (m["depth"].as_u64().unwrap(), m["word"].as_str().unwrap(), m["via"].as_str().unwrap())).collect();
    assert_eq!(tree, [(0, "loquacity", ""), (1, "loquacious", "-ous"), (2, "loquaciously", "-ly")]);
    // Each word carries what the dictionary knows of it.
    assert_eq!(f["members"][1]["pos"], json!(["adj"]));
    assert_eq!(f["members"][1]["gloss"], "talkative");
    assert_eq!(f["members"][1]["from"], "loquacity");
    assert_eq!(f["related"][0]["word"], "loquor");
    // The same-stem guess names only the word that is not already in the tree.
    assert_eq!(f["stem"].as_array().unwrap().iter().map(|w| w["word"].as_str().unwrap()).collect::<Vec<_>>(), ["loquaciousness"]);
    std::fs::remove_dir_all(&dir).ok();
}
