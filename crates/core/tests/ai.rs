//! Runs the real embedding model when it has been downloaded into data/packs.

use gredict_core::ai::{cosine, Embedder};

fn open() -> Option<Embedder> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/packs");
    let (m, t) = (root.join("bge-small-en-v1.5-q.onnx"), root.join("bge-small-en-v1.5-tokenizer.json"));
    (m.exists() && t.exists()).then(|| Embedder::open(&m, &t).unwrap())
}

#[test]
fn similar_meanings_are_closer_than_unrelated_ones() {
    let Some(e) = open() else { return };
    let v = e.embed(&["stubborn", "obdurate: refusing to change one's mind", "a recipe for lemon cake"]).unwrap();
    assert_eq!(v[0].len(), 384);
    let near = cosine(&v[0], &v[1]);
    let far = cosine(&v[0], &v[2]);
    assert!(near > far + 0.1, "near {near} far {far}");
    // Vectors are unit length.
    assert!((cosine(&v[0], &v[0]) - 1.0).abs() < 1e-3);
}

#[test]
fn the_runtime_says_what_it_runs_on() {
    let Some(e) = open() else { return };
    let t = e.self_test().unwrap();
    println!("backend {} first {:.1} ms, steady {:.1} ms", t.backend.name(), t.first_ms, t.per_run_ms);
    // This machine has no NPU: the CPU must be what is reported.
    assert_eq!(t.backend.id(), "cpu");
    assert_eq!(t.dims, 384);
}
