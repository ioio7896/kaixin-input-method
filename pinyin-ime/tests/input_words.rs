use pinyin_ime::core::{PinyinEngine, MODE_DATE_AUTO_FORMAT, MODE_JIANPIN, MODE_MIXED_PINYIN};
use std::path::Path;

#[test]
fn ordinary_pinyin_does_not_trigger_utility_shortcuts() {
    let lexicon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(MODE_DATE_AUTO_FORMAT | MODE_JIANPIN | MODE_MIXED_PINYIN);
    for (reading, expected) in [("tuige", "退格"), ("huiche", "回车"), ("xingqi", "星期")] {
        let (candidates, error) = engine.lookup_full(reading);
        assert_eq!(error, None, "{reading}");
        assert_eq!(
            candidates.first().map(|c| c.0.as_str()),
            Some(expected),
            "{reading}"
        );
    }
    for (reading, expected) in [("enter", "Enter"), ("backspace", "Backspace")] {
        let (candidates, _) = engine.lookup_full(reading);
        assert_eq!(
            candidates.first().map(|c| c.0.as_str()),
            Some(expected),
            "{reading}"
        );
    }
}
