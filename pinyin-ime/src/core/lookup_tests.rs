use super::*;
use crate::core::{LEARN_FLAG_COMPOSED_PHRASE, LEARN_FLAG_WEAK};

#[test]
fn lookup_returns_non_primary_heteronym_readings() {
    let mut engine = PinyinEngine::with_phrase_dir(None);
    for (input, expected) in [("chong", "重"), ("yue", "乐"), ("hang", "行")] {
        let candidates = engine.lookup_tsf_candidates(input);
        assert!(
            candidates.iter().any(|candidate| candidate == expected),
            "{input} did not return {expected}; candidates={candidates:?}"
        );
    }
}

#[test]
fn xiong_does_not_offer_neng_character() {
    let mut engine = PinyinEngine::with_phrase_dir(None);
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);
    let base_candidates = engine.lookup_full_explain("xiong").0;
    assert!(!base_candidates.iter().any(|(phrase, _, _)| phrase == "能"));
    assert!(base_candidates
        .iter()
        .take(crate::core::TSF_PAGE_SIZE)
        .any(|(phrase, _, _)| phrase == "熊"));
    let neng_candidates = engine.lookup_full_explain("neng").0;
    assert!(neng_candidates
        .iter()
        .take(crate::core::TSF_PAGE_SIZE)
        .any(|(phrase, _, _)| phrase == "能"));
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut lexicon_engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    lexicon_engine.clear_user_lexicon_for_eval();
    lexicon_engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);
    let lexicon_candidates = lexicon_engine.lookup_full_explain("xiong").0;
    assert!(!lexicon_candidates
        .iter()
        .any(|(phrase, _, _)| phrase == "能"));
    assert!(lexicon_candidates
        .iter()
        .take(crate::core::TSF_PAGE_SIZE)
        .any(|(phrase, _, _)| phrase == "熊"));
}

#[test]
fn composed_polyphonic_user_phrase_is_available_on_next_lookup() {
    let mut engine = PinyinEngine::with_phrase_dir(None);
    engine.clear_user_lexicon_for_eval();

    // Simulate selecting each character from a polyphonic path, then
    // committing the phrase assembled by the TSF continuation flow.
    engine
        .learn_commit_with_flags("chong", "重", LEARN_FLAG_WEAK)
        .expect("learn first selected character");
    engine
        .learn_commit_with_flags("qing", "庆", 0)
        .expect("learn final selected character");
    engine
        .learn_commit_with_flags("chongqing", "重庆", LEARN_FLAG_COMPOSED_PHRASE)
        .expect("learn composed user phrase");

    let candidates = engine.lookup_tsf_candidates("chongqing");
    assert!(
        candidates.iter().any(|candidate| candidate == "重庆"),
        "composed phrase was not recalled: {candidates:?}"
    );
}

#[test]
fn composed_phrase_respects_hotword_front_policy_on_the_next_full_pinyin_lookup() {
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);

    engine
        .learn_commit_with_flags("shishi", "湿十", LEARN_FLAG_COMPOSED_PHRASE)
        .expect("learn composed user phrase");

    let candidates = engine.lookup_full_explain("shishi").0;
    let learned_rank = candidates
        .iter()
        .position(|(phrase, _, _)| phrase == "湿十");
    assert!(
        learned_rank.is_some(),
        "newly composed phrase was not recalled: {candidates:?}"
    );
    if crate::user_hotword_prefs::get_user_hotword_prefs().front_limit > 0 {
        assert_eq!(
            learned_rank,
            Some(0),
            "newly composed phrase was not promoted: {candidates:?}"
        );
    }
}

#[test]
fn repeated_unselected_top_two_cool_while_selected_candidate_moves_forward() {
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);

    let before = engine.lookup_full_explain("fangzhi").0;
    assert!(before.len() >= 3, "not enough candidates: {before:?}");
    let first = before[0].0.clone();
    let second = before[1].0.clone();
    let selected = before[2].0.clone();
    let skipped = vec![first.clone(), second.clone()];

    for _ in 0..3 {
        engine
            .learn_selection_feedback("fangzhi", &selected, 2, 0, &skipped)
            .expect("record candidate feedback");
    }

    let weak = engine
        .user_lexicon
        .weak_unselected_signal("fangzhi")
        .expect("weak top-two feedback");
    for phrase in [&first, &second] {
        assert_eq!(
            weak.iter()
                .find(|entry| &entry.phrase == phrase)
                .map(|entry| entry.score),
            Some(-3),
            "top-two candidate did not accumulate three weak misses: {weak:?}"
        );
    }
    let strong = engine.user_lexicon.selection_signal("fangzhi");
    assert!(
        strong.is_none_or(|entries| entries
            .iter()
            .all(|entry| { entry.phrase != first && entry.phrase != second })),
        "top-two candidates received an immediate strong penalty: {strong:?}"
    );

    let after = engine.lookup_full_explain("fangzhi").0;
    let selected_rank = after
        .iter()
        .position(|(phrase, _, _)| phrase == &selected)
        .expect("selected candidate remains reachable");
    assert!(
        selected_rank < 2,
        "frequently selected candidate did not move ahead: selected={selected}, after={after:?}"
    );
}

#[test]
fn two_syllable_full_pinyin_keeps_two_char_words_on_later_pages() {
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);

    let candidates = engine.lookup_full_explain("shishi").0;
    let page_size = crate::candidate_prefs::get_effective_candidate_page_size()
        .clamp(3, crate::core::TSF_PAGE_SIZE);
    let two_char_per_page = candidates
        .chunks(page_size)
        .take(3)
        .map(|page| {
            page.iter()
                .filter(|(phrase, _, _)| phrase.chars().count() == 2)
                .count()
        })
        .collect::<Vec<_>>();

    assert!(
        two_char_per_page.len() >= 3 && two_char_per_page.iter().all(|count| *count >= 2),
        "two-character candidates were not distributed across pages: \
             page_size={page_size}, counts={two_char_per_page:?}, candidates={candidates:?}"
    );
}

#[test]
fn two_char_intent_page_density_matches_between_cold_and_cached_lookup() {
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);

    // 第一次查询是冷路径：完整 postprocess + 二字分页编排。
    let cold = engine.lookup_full_explain("shishi").0;
    // 第二次查询应命中 final 缓存，并按存储的两字意图编排模式重放分页。
    let cache_key = final_lookup_cache_key("shishi", engine.mode_flags, engine.cache_epoch);
    assert!(
        engine.final_lookup_cache.get(&cache_key).is_some(),
        "first lookup did not populate the final lookup cache"
    );
    let cached = engine.lookup_full_explain("shishi").0;

    let page_size = crate::candidate_prefs::get_effective_candidate_page_size()
        .clamp(3, crate::core::TSF_PAGE_SIZE);
    let two_char_per_page = |candidates: &[(String, f64, CandidateMeta)]| -> Vec<usize> {
        candidates
            .chunks(page_size)
            .take(3)
            .map(|page| {
                page.iter()
                    .filter(|(phrase, _, _)| phrase.chars().count() == 2)
                    .count()
            })
            .collect()
    };
    let cold_counts = two_char_per_page(&cold);
    let cached_counts = two_char_per_page(&cached);

    assert_eq!(
        cached_counts, cold_counts,
        "cached lookup lost the two-character page layout applied on the \
             cold path: page_size={page_size}, cold={cold_counts:?}, \
             cached={cached_counts:?}, cold_candidates={cold:?}, \
             cached_candidates={cached:?}"
    );
    assert!(
        cached_counts.len() >= 3 && cached_counts.iter().all(|count| *count >= 2),
        "cached lookup did not keep two-character candidates on later pages: \
             page_size={page_size}, counts={cached_counts:?}, candidates={cached:?}"
    );
}

#[test]
fn exact_full_pinyin_single_syllable_keeps_cold_chars_reachable() {
    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);

    for (input, cold_chars) in [
        ("chang", &["昶", "氅", "鲳"][..]),
        ("da", &["龘", "妲"][..]),
        ("shang", &["熵", "殇"][..]),
        ("wang", &["罔", "辋"][..]),
        ("si", &["兕", "巳"][..]),
    ] {
        let candidates = engine.lookup_full_explain(input).0;
        let phrases: Vec<String> = candidates.iter().map(|(p, _, _)| p.clone()).collect();
        for cold in cold_chars {
            assert!(
                phrases.iter().any(|phrase| phrase == cold),
                "{input} did not list cold single char {cold}; candidates={phrases:?}"
            );
        }
    }

    // 精确单音节的完整候选池应容纳字典内全部单字（不再按高频场景截断
    // 到 128）；这里断言字典中每个单字在完整结果里都可翻页到达。
    for input in ["chang", "da", "shang", "wang", "si", "mo"] {
        let candidates = engine.lookup_full_explain(input).0;
        let phrases: Vec<String> = candidates.iter().map(|(p, _, _)| p.clone()).collect();
        let dict_chars = engine.dict.get(input);
        let missing = dict_chars.map(|chars| {
            chars
                .iter()
                .filter(|ch| !phrases.contains(&ch.to_string()))
                .map(|ch| ch.to_string())
                .collect::<Vec<_>>()
        });
        assert!(
            missing.as_ref().is_none_or(|m| m.is_empty()),
            "{input} full-pinyin candidate list dropped dict chars: \
                 missing={missing:?} total={} dict={}",
            phrases.len(),
            dict_chars.map_or(0, Vec::len)
        );
    }

    // 首屏仍保持小批量：首批候选（用户翻页前）只给单字页，完整加载由
    // 翻页触发，交互成本不变。
    assert_eq!(engine.lookup_first_page("si").len(), 9);
}

#[test]
fn cold_extension_words_are_tail_only_for_non_full_input() {
    let mut lexicon = AbbrevLexicon::default();
    assert!(lexicon.insert_parsed_with_layer(
        ThuoclEntry {
            phrase: "高频词".to_string(),
            freq: 9_000,
            code: Some("gao pin ci".to_string()),
        },
        LexiconLayer::Base,
    ));
    assert!(lexicon.insert_parsed_with_layer(
        ThuoclEntry {
            phrase: "冷门词".to_string(),
            freq: 3_500,
            code: Some("leng men ci".to_string()),
        },
        LexiconLayer::Ext,
    ));

    let cold = RankedCandidate {
        phrase: "冷门词".to_string(),
        score: 100.0,
        meta: CandidateMeta::legacy(ABBREV_CANDIDATE_META).with_source_layer(LexiconLayer::Ext),
    };
    let hot = RankedCandidate {
        phrase: "高频词".to_string(),
        score: 1.0,
        meta: CandidateMeta::legacy(ABBREV_CANDIDATE_META).with_source_layer(LexiconLayer::Base),
    };
    let mut abbreviated = vec![cold.clone(), hot.clone()];
    demote_cold_non_full_lexicon_candidates(
        &mut abbreviated,
        InputIntent::ShortAbbrev,
        Some(&lexicon),
    );
    assert_eq!(
        abbreviated
            .iter()
            .map(|item| item.phrase.as_str())
            .collect::<Vec<_>>(),
        ["高频词", "冷门词"]
    );

    let mut full_pinyin = vec![cold, hot];
    demote_cold_non_full_lexicon_candidates(
        &mut full_pinyin,
        InputIntent::FullPinyin,
        Some(&lexicon),
    );
    assert_eq!(full_pinyin[0].phrase, "冷门词");

    let repo_lexicon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&repo_lexicon));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(crate::core::MODE_JIANPIN | crate::core::MODE_MIXED_PINYIN);
    assert_eq!(engine.input_intent("wj"), InputIntent::ShortAbbrev);
    // Use an entry that is actually part of the generated 500-item animal
    // extension lexicon.  "弯棘" lives beyond that source's configured
    // limit and therefore made this test depend on stale generated data.
    let abbreviated = engine.lookup_tsf_candidates("hpyw");
    let cold_position = abbreviated.iter().position(|phrase| phrase == "虎皮鹦鹉");
    assert!(
        cold_position.is_none_or(|position| position >= crate::core::TSF_PAGE_SIZE),
        "cold extension phrase entered the short-input page: {abbreviated:?}"
    );
    let full_pinyin = engine.lookup_tsf_candidates("hupiyingwu");
    assert!(
        full_pinyin.iter().any(|phrase| phrase == "虎皮鹦鹉"),
        "full pinyin could not recall the cold extension phrase: {full_pinyin:?}"
    );
}
