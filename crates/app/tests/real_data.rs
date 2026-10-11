//! End-to-end checks over the real packs and Barron's import. Skipped when
//! `data/packs` or `private/` have not been built on this machine.

use gredict_app::{Api, Config};
use serde_json::{json, Value};

fn api() -> Option<Api> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let packs = root.join("data/packs");
    if !packs.join("wiktionary.pack").exists() {
        return None;
    }
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let data = std::env::temp_dir().join(format!("gredict-it-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    Some(Api::open(Config { data_dir: data, packs_dir: packs, private_dir: Some(root.join("private")) }).unwrap())
}

fn words(v: &Value) -> Vec<String> {
    v["suggestions"].as_array().unwrap().iter().map(|s| s["word"].as_str().unwrap().to_string()).collect()
}

#[test]
fn search_ranks_exact_and_gre_words_first() {
    let Some(api) = api() else { return };
    let r = api.call("search", json!({"q": "abat"})).unwrap();
    let w = words(&r);
    assert!(w.contains(&"abate".to_string()), "{w:?}");
    assert_eq!(w[0], "abate", "GRE word should lead single-word results: {w:?}");
    let exact = words(&api.call("search", json!({"q": "Lucid"})).unwrap());
    assert_eq!(exact[0], "lucid");
}

#[test]
fn typos_get_suggestions() {
    let Some(api) = api() else { return };
    let r = api.call("search", json!({"q": "loquacioux"})).unwrap();
    let d: Vec<&str> = r["did_you_mean"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(d.contains(&"loquacious"), "{d:?}");
    // A recorded misspelling is its own entry that points at the right word.
    let e = api.call("entry", json!({"word": "loquatious", "record": false})).unwrap();
    assert_eq!(e["entry"]["blocks"][0]["senses"][0]["form_of"], "loquacious");
}

#[test]
fn entry_combines_sources_barrons_and_thesaurus() {
    let Some(api) = api() else { return };
    let e = api.call("entry", json!({"word": "loquacious"})).unwrap();
    assert_eq!(e["source"]["id"], "wiktionary");
    assert!(e["found_in"].as_array().unwrap().len() >= 2, "wiktionary + wordnet");
    if !e["barrons"].is_null() {
        assert_eq!(e["barrons"][0]["high_freq"], true);
        // GRE words related by meaning, from the book.
        let rel: Vec<&str> = e["thesaurus"]["barrons"].as_array().unwrap().iter().filter_map(|x| x["word"].as_str()).collect();
        assert!(rel.iter().any(|w| ["garrulous", "verbose", "laconic", "taciturn", "voluble"].contains(w)), "{rel:?}");
    }
    let syn: Vec<&str> = e["thesaurus"]["general"]["synonyms"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(syn.contains(&"talkative"), "{syn:?}");
    // Switching source shows the other pack's entry.
    let wn = api.call("entry", json!({"word": "loquacious", "source": "wordnet", "record": false})).unwrap();
    assert_eq!(wn["source"]["id"], "wordnet");
}

#[test]
fn notes_feed_the_thesaurus_and_history_records_lookups() {
    let Some(api) = api() else { return };
    api.call("note_save", json!({"body": "abate = lessen = subside\nabate >< intensify\nimbibition ⍯ osmosis\nfrugal ≈ thrifty", "session_date": "2026-10-10"})).unwrap();
    let e = api.call("entry", json!({"word": "subside"})).unwrap();
    let n = &e["thesaurus"]["notes"];
    assert_eq!(n["same"], json!(["abate", "lessen"]));
    assert_eq!(n["opposite"], json!(["intensify"]));
    let o = api.call("thesaurus", json!({"word": "osmosis"})).unwrap();
    assert_eq!(o["notes"]["different"], json!(["imbibition"]));

    let h = api.call("history_list", json!({})).unwrap();
    assert_eq!(h[0]["key"], "subside");
    // Deleting the note removes its relations from the panel.
    let id = api.call("notes_list", json!({})).unwrap()[0]["id"].as_str().unwrap().to_string();
    api.call("note_delete", json!({"id": id})).unwrap();
    assert!(api.call("thesaurus", json!({"word": "subside"})).unwrap()["notes"]["same"].as_array().unwrap().is_empty());
}

fn has_book(api: &Api) -> bool {
    api.call("quiz_catalog", json!({})).unwrap()["barrons"] == true
}

/// Answers every item of a started session, wrongly when `wrong_ids` says so.
fn play(api: &Api, s: &Value, wrong_every: usize) -> Value {
    let sid = s["id"].as_str().unwrap();
    let mut last = Value::Null;
    for (i, item) in s["items"].as_array().unwrap().iter().enumerate() {
        // Peek the expected answer through a first (recorded) answer attempt is
        // impossible, so derive it from the stored session instead.
        let rev = api.call("session_get", json!({"id": sid})).unwrap();
        let expected = rev["review"][i]["expected"].clone();
        let given = if wrong_every > 0 && i % wrong_every == 0 {
            // Deliberately wrong: pick a choice that is not the expected one.
            let wrong = item["choices"][0].as_array().unwrap().iter().find(|c| json!([c]) != expected).cloned().unwrap_or(json!("zzz"));
            if item["blanks"].as_u64().unwrap() > 1 { json!(["zzz", "zzz", "zzz"]) } else { wrong }
        } else if item["blanks"].as_u64().unwrap() > 1 || item["select"].as_u64().unwrap_or(1) > 1 {
            expected
        } else {
            expected[0].clone()
        };
        last = api.call("quiz_answer", json!({"session_id": sid, "item_id": item["id"], "given": given, "ms": 1200})).unwrap();
    }
    last
}

#[test]
fn barrons_quiz_session_is_scored_and_reviewable() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let cat = api.call("quiz_catalog", json!({})).unwrap();
    assert_eq!(cat["units"].as_array().unwrap().len(), 80);
    let s = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["u06-fill"]})).unwrap();
    assert_eq!(s["total"], 10);
    // Answers are never sent to the client up front.
    assert!(s["items"][0].get("expected").is_none());
    let done = play(&api, &s, 4); // items 0, 4, 8 answered wrongly
    assert_eq!(done["finished"], true);
    assert_eq!(done["score"], 7);
    let g = api.call("session_get", json!({"id": s["id"]})).unwrap();
    assert_eq!(g["correct"], 7);
    assert_eq!(g["review"][0]["correct"], false);
    assert_eq!(g["review"][1]["correct"], true);

    // A second answer to the same question does not change the score.
    let first = s["items"][1]["id"].clone();
    api.call("quiz_answer", json!({"session_id": s["id"], "item_id": first, "given": "nonsense"})).unwrap();
    assert_eq!(api.call("session_get", json!({"id": s["id"]})).unwrap()["correct"], 7);

    // Retrying the misses starts a session with exactly those questions.
    let retry = api.call("quiz_start", json!({"kind": "retry", "session_id": s["id"]})).unwrap();
    assert_eq!(retry["total"], 3);
}

#[test]
fn applying_questions_have_a_choice_per_blank() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let s = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["u20-apply"]})).unwrap();
    let multi = s["items"].as_array().unwrap().iter().find(|i| i["blanks"] == 3).expect("a three-blank question");
    assert!(multi["choices"].as_array().unwrap().iter().all(|c| c.as_array().unwrap().len() == 3));
    let done = play(&api, &s, 0);
    assert_eq!(done["score"], done["total"], "all applying answers resolve");
}

#[test]
fn generated_word_quizzes_have_four_choices_and_one_answer() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    for mode in ["def_to_word", "word_to_def", "cloze"] {
        let s = api.call("quiz_start", json!({"kind": "words", "mode": mode, "count": 8, "unit_from": 1, "unit_to": 20})).unwrap();
        assert!(s["total"].as_u64().unwrap() >= 5, "{mode}");
        for it in s["items"].as_array().unwrap() {
            let c = it["choices"][0].as_array().unwrap();
            assert_eq!(c.len(), 4, "{mode}");
            assert!(c.iter().collect::<std::collections::HashSet<_>>().len() == 4, "choices are distinct");
        }
        if mode == "cloze" {
            assert!(s["items"][0]["prompt"].as_str().unwrap().contains("____"));
        }
        assert_eq!(play(&api, &s, 0)["score"], s["total"]);
    }
}

#[test]
fn notes_quiz_and_session_retrieval() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let none = api.call("quiz_start", json!({"kind": "notes"}));
    assert!(none.is_err(), "no relations yet");
    api.call("note_save", json!({"body": "obdurate = intransigent\nobdurate >< flexible\nimbibition ⍯ osmosis", "session_date": "2026-10-11", "tags": "tutor"})).unwrap();
    let s = api.call("quiz_start", json!({"kind": "notes", "count": 6})).unwrap();
    assert_eq!(s["total"], 3, "one question per relation in the notes");
    assert!(s["items"][0]["prompt"].as_str().unwrap().contains('“'));
    let notes_session = play(&api, &s, 2);
    assert_eq!(notes_session["finished"], true);

    // A Barron's session about the same vocabulary, answered perfectly.
    let b = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["u60-match"]})).unwrap();
    play(&api, &b, 0);

    let all = api.call("sessions_list", json!({})).unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);

    // Lexical hit.
    let hit = api.call("sessions_list", json!({"q": "obdurate"})).unwrap();
    assert_eq!(hit.as_array().unwrap()[0]["id"], s["id"]);
    // Related-word hit: the session never says "stubborn", but the thesaurus ties it to obdurate/intransigent.
    let sem = api.call("sessions_list", json!({"q": "stubborn"})).unwrap();
    assert!(sem.as_array().unwrap().iter().any(|x| x["id"] == s["id"]), "thesaurus expansion finds related words: {sem}");
    // Intent: only sessions with mistakes.
    let wrong = api.call("sessions_list", json!({"q": "words I got wrong"})).unwrap();
    let ids: Vec<&str> = wrong.as_array().unwrap().iter().filter_map(|x| x["id"].as_str()).collect();
    assert_eq!(ids, vec![s["id"].as_str().unwrap()]);
    let perfect = api.call("sessions_list", json!({"q": "perfect"})).unwrap();
    assert_eq!(perfect.as_array().unwrap()[0]["id"], b["id"]);
}

#[test]
fn every_extra_quiz_can_be_played_to_a_perfect_score() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let cat = api.call("quiz_catalog", json!({})).unwrap();
    assert_eq!(cat["extras"]["reviews"].as_array().unwrap().len(), 4, "two cumulative reviews, pretest, posttest");
    assert_eq!(cat["extras"]["roots"].as_array().unwrap().len(), 72, "60 Root Work + 12 reviews");
    let mut played = 0;
    for g in ["reviews", "roots"] {
        for q in cat["extras"][g].as_array().unwrap() {
            let s = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": [q["id"]]})).unwrap();
            assert_eq!(s["total"], q["count"], "{}", q["id"]);
            for it in s["items"].as_array().unwrap() {
                let list = it["choices"][0].as_array().unwrap();
                assert!(list.len() >= 2, "{} has choices", q["id"]);
            }
            let done = play(&api, &s, 0);
            assert_eq!(done["score"], done["total"], "answers of {} all resolve", q["id"]);
            played += 1;
        }
    }
    assert_eq!(played, 76);
    // The cumulative review mixes matching, sense/nonsense and multiple choice.
    let s = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["ew-review"]})).unwrap();
    let kinds: std::collections::HashSet<&str> = s["items"].as_array().unwrap().iter().filter_map(|i| i["kind"].as_str()).collect();
    assert!(kinds.contains("matching") && kinds.contains("sense") && kinds.contains("mcq"), "{kinds:?}");
    assert_eq!(s["total"], 50);
    // Matching rows are cut to five options like the unit exercises.
    let m = s["items"].as_array().unwrap().iter().find(|i| i["kind"] == "matching").unwrap();
    assert_eq!(m["choices"][0].as_array().unwrap().len(), 5);
}

#[test]
fn pretest_has_pick_two_blanks_and_reading_passages() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let s = api.call("quiz_start", json!({"kind": "barrons", "quiz_ids": ["pretest"]})).unwrap();
    assert_eq!(s["total"], 20);
    let items = s["items"].as_array().unwrap();
    // 1-4: sentence equivalence, pick two of six.
    assert!(items[..4].iter().all(|i| i["kind"] == "se" && i["select"] == 2 && i["choices"][0].as_array().unwrap().len() == 6));
    // Multi-blank text completion has a column per blank.
    assert!(items.iter().any(|i| i["kind"] == "applying" && i["blanks"] == 3));
    // 15-20: reading comprehension with the passage attached.
    for i in &items[14..] {
        let p = i["passage"].as_str().expect("passage");
        assert!(p.contains("(5)"), "line numbers kept");
    }
    assert!(items[19]["choices"][0].as_array().unwrap().len() >= 10, "the sentence question lists the passage sentences");

    // Order does not matter for pick-two; one right and one wrong is wrong.
    let sid = s["id"].as_str().unwrap();
    let rev = api.call("session_get", json!({"id": sid})).unwrap();
    let exp = rev["review"][0]["expected"].as_array().unwrap().clone();
    let swapped = json!([exp[1], exp[0]]);
    let r = api.call("quiz_answer", json!({"session_id": sid, "item_id": items[0]["id"], "given": swapped})).unwrap();
    assert_eq!(r["correct"], true);
    let other = items[1]["choices"][0].as_array().unwrap();
    let exp1 = api.call("session_get", json!({"id": sid})).unwrap()["review"][1]["expected"].as_array().unwrap().clone();
    let wrong = other.iter().find(|c| !exp1.contains(c)).unwrap();
    let r = api.call("quiz_answer", json!({"session_id": sid, "item_id": items[1]["id"], "given": [exp1[0], wrong]})).unwrap();
    assert_eq!(r["correct"], false);
}

#[test]
fn entries_show_the_root_family_from_the_book() {
    let Some(api) = api() else { return };
    if !has_book(&api) {
        return;
    }
    let e = api.call("entry", json!({"word": "loquacious", "record": false})).unwrap();
    let r = &e["roots"][0];
    assert_eq!(r["forms"], "LOC/LOG/LOQU");
    assert!(r["meaning"].as_str().unwrap().contains("speak"));
    assert!(r["words"].as_array().unwrap().iter().any(|w| w["key"] == "loquacious"));
    assert!(r["words"].as_array().unwrap().len() >= 4);
    // A word outside the book's root lists simply has none.
    assert!(api.call("entry", json!({"word": "serendipity", "record": false})).unwrap()["roots"].as_array().unwrap().is_empty());
}

#[test]
fn headwords_come_with_syllables_from_whichever_dictionary_has_them() {
    let Some(api) = api() else { return };
    // Wiktionary has none for this word; Webster 1913 supplies them.
    let e = api.call("entry", json!({"word": "loquacious", "record": false})).unwrap();
    assert_eq!(e["syl"], "lo·qua·cious");
    // Wiktionary's own hyphenation is used when it has one.
    let d = api.call("entry", json!({"word": "dictionary", "record": false})).unwrap();
    assert!(d["syl"].as_str().unwrap().contains('·'), "{d}");
    // No source knows a made-up word: no syllables, no entry.
    assert!(api.call("entry", json!({"word": "zzqxv", "record": false})).unwrap()["syl"].is_null());
}
