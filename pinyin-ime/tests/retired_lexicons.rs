use pinyin_ime::lexicon_prefs::{
    discover_optional_lexicon_info, discover_optional_lexicon_tags_in_lexicon_dir,
    RETIRED_OPTIONAL_LEXICON_TAGS,
};
use pinyin_ime::thuocl::{
    load_dir_txt, load_dir_txt_with_profile_default_enabled, LexiconBuildProfile,
};
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn upgraded_tree_hides_and_skips_retired_files_but_keeps_modern_and_main_sources() {
    let fixture = Fixture(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "kaixin-retired-lexicons-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            )),
    );
    for directory in ["zh-ext", "ext", "zh"] {
        std::fs::create_dir_all(fixture.0.join(directory)).unwrap();
    }
    for tag in RETIRED_OPTIONAL_LEXICON_TAGS {
        for directory in ["zh-ext", "ext"] {
            for filename in [
                format!("{tag}.txt"),
                format!("THUOCL_{tag}.txt"),
                format!("old__THUOCL_{tag}.txt"),
            ] {
                std::fs::write(
                    fixture.0.join(directory).join(filename),
                    "旧版测试词\tjiu ban ce shi ci\t1000\n",
                )
                .unwrap();
            }
        }
    }
    for (file, text) in [
        ("zh-ext/animals.txt", "小猫\txiao mao\t1000\n"),
        ("zh-ext/my_words.txt", "自定义\tzi ding yi\t1000\n"),
        ("zh/THUOCL_animal.txt", "动物\tdong wu\t1000\n"),
        ("zh/lfie-common-3char.txt", "旧主库\tjiu zhu ku\t1000\n"),
        (
            "zh/life_common_3char_20000.txt",
            "旧词库\tjiu ci ku\t1000\n",
        ),
        (
            "zh/life_common_phrases_5to8.txt",
            "旧短语\tjiu duan yu\t1000\n",
        ),
        ("zh-ext/药物2026.txt", "旧药名\tjiu yao ming\t1000\n"),
    ] {
        std::fs::write(fixture.0.join(file), text).unwrap();
    }
    let tags = discover_optional_lexicon_tags_in_lexicon_dir(&fixture.0);
    assert_eq!(
        tags.iter().map(|(tag, _)| tag.as_str()).collect::<Vec<_>>(),
        ["animals", "my_words"],
    );
    assert_eq!(discover_optional_lexicon_info(&fixture.0).len(), 2);
    let (_, full) = load_dir_txt(&fixture.0).unwrap();
    assert_eq!(full.files_read, 3);
    assert_eq!(full.entries_indexed, 3);
    for profile in [LexiconBuildProfile::Hot, LexiconBuildProfile::Standard] {
        let (_, report) = load_dir_txt_with_profile_default_enabled(&fixture.0, profile).unwrap();
        assert_eq!(report.files_read, 3);
        assert_eq!(report.entries_indexed, 3);
    }
}

#[test]
fn migrated_main_and_drug_files_do_not_reenter_an_upgraded_tree() {
    use pinyin_ime::lexicon_prefs::{
        is_retired_packaged_lexicon_path, RETIRED_PACKAGED_LEXICON_FILES,
    };
    for relative in RETIRED_PACKAGED_LEXICON_FILES {
        assert!(is_retired_packaged_lexicon_path(
            &PathBuf::from("lexicon").join(relative)
        ));
    }
    for relative in [
        "zh/life_common_3char.txt",
        "zh/life_common_phrases.txt",
        "zh-ext/medicine.txt",
    ] {
        assert!(!is_retired_packaged_lexicon_path(
            &PathBuf::from("lexicon").join(relative)
        ));
    }
    assert_eq!(
        pinyin_ime::lexicon_prefs::optional_lexicon_group("medicine"),
        "药物名称"
    );
    assert_eq!(
        pinyin_ime::lexicon_prefs::optional_lexicon_group("technology"),
        "科技与开发"
    );
}
