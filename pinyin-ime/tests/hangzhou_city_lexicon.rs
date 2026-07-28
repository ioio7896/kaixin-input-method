use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use pinyin_ime::thuocl::load_dir_txt;

const HANGZHOU_CITY_LEXICONS: [(&str, usize); 7] = [
    ("hangzhou_admin.txt", 300),
    ("hangzhou_transport.txt", 145),
    ("hangzhou_public_services.txt", 165),
    ("hangzhou_landmarks_life.txt", 165),
    ("hangzhou_business.txt", 85),
    ("hangzhou_food_culture.txt", 75),
    ("hangzhou_local_culture.txt", 65),
];

fn lexicon_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("lexicon")
}

fn native_entries(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("read Hangzhou city lexicon");
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn hangzhou_city_lexicons_keep_the_curated_thousand_entry_budget() {
    let ext_dir = lexicon_dir().join("ext");
    let mut total = 0;
    let mut unique_phrases = HashSet::new();
    for (file_name, expected) in HANGZHOU_CITY_LEXICONS {
        let entries = native_entries(&ext_dir.join(file_name));
        let actual = entries.len();
        assert_eq!(actual, expected, "unexpected entry count for {file_name}");
        for entry in entries {
            let phrase = entry.split('\t').next().expect("dictionary phrase");
            assert!(
                unique_phrases.insert(phrase.to_owned()),
                "duplicate Hangzhou city phrase: {phrase}"
            );
        }
        total += actual;
    }
    assert_eq!(total, 1_000);
    assert_eq!(unique_phrases.len(), 1_000);
}

#[test]
fn representative_hangzhou_city_terms_are_indexed_by_explicit_pinyin() {
    let (lexicon, _) = load_dir_txt(&lexicon_dir()).expect("load repository lexicon");
    for (input, expected) in [
        ("hangzhoushiyuhangqu", "杭州市余杭区"),
        ("hangzhouxizhan", "杭州西站"),
        ("zhedaeryuan", "浙大二院"),
        ("liangzhuguchengyizhi", "良渚古城遗址"),
        ("alibabaxixiyuanqu", "阿里巴巴西溪园区"),
        ("congbaohui", "葱包桧"),
        ("xiaoyaer", "小伢儿"),
    ] {
        let entries = lexicon
            .lookup_pinyin(input)
            .unwrap_or_else(|| panic!("missing pinyin key {input}"));
        assert!(
            entries.iter().any(|entry| entry.phrase == expected),
            "{expected} should be indexed under {input}: {entries:?}"
        );
    }
}
