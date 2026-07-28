use std::path::PathBuf;

use pinyin_ime::core::PinyinEngine;

fn lexicon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon")
}

#[test]
fn hangzhou_metro_place_readings_reach_the_interactive_candidate_page() {
    let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir()));
    engine.clear_user_lexicon_for_eval();

    for (input, expected) in [
        ("pengbu", "彭埠"),
        ("shouxiang", "受降"),
        ("tiaoxi", "苕溪"),
        ("jianqiaolaojie", "笕桥老街"),
    ] {
        let candidates = engine.lookup_full(input).0;
        assert!(
            candidates
                .iter()
                .take(3)
                .any(|candidate| candidate.0 == expected),
            "{expected} should be in the first three candidates for {input}: {candidates:?}"
        );
    }
}
