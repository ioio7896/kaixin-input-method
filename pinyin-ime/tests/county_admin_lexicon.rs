use std::path::PathBuf;

use pinyin_ime::{core::PinyinEngine, thuocl::load_dir_txt};

fn lexicon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon")
}

#[test]
fn county_admin_full_pinyin_is_indexed_with_explicit_place_reading() {
    let (lexicon, _) = load_dir_txt(&lexicon_dir()).expect("load repository lexicon");
    let entries = lexicon
        .lookup_pinyin("atushi")
        .expect("阿图什 should be indexed under a tu shi");
    assert!(entries.iter().any(|entry| entry.phrase == "阿图什"));
}

#[test]
fn county_admin_full_pinyin_reaches_the_interactive_candidate_page() {
    let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir()));
    engine.clear_user_lexicon_for_eval();
    let candidates = engine.lookup_full("atushi").0;
    assert!(
        candidates
            .iter()
            .take(3)
            .any(|candidate| candidate.0 == "阿图什"),
        "阿图什 should be in the first three candidates: {candidates:?}"
    );
}
