use gredict_core::fuzzy::Fuzzy;

#[test]
fn finds_a_one_letter_typo() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/packs/wiktionary.fst");
    if !p.exists() {
        return;
    }
    let f = Fuzzy::open(&p).unwrap();
    assert!(f.len() > 1_000_000);
    let near = f.near("loquatious", 2, 2000).unwrap();
    assert!(near.contains(&"loquacious".to_string()), "{near:?}");
}
