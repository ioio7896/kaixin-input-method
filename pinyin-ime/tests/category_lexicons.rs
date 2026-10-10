use pinyin_ime::lexicon_prefs::{
    category_lexicon_info, default_optional_lexicon_tag_enabled,
    discover_optional_lexicon_tags_in_lexicon_dir, optional_lexicon_group,
};
use pinyin_ime::thuocl::{
    load_dir_txt_with_profile_default_enabled, load_prebaked_lexicon, save_prebaked_lexicon,
    LexiconBuildProfile,
};
use std::path::{Path, PathBuf};

#[test]
fn imported_categories_have_discoverable_chinese_names_and_default_switches() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("shared/category_lexicons.json")).unwrap(),
    )
    .unwrap();
    let categories = manifest["categories"].as_array().unwrap();
    assert_eq!(categories.len(), 78);
    let discovered = discover_optional_lexicon_tags_in_lexicon_dir(&root.join("lexicon"));
    for category in categories {
        let tag = category["tag"].as_str().unwrap();
        let title = category["title"].as_str().unwrap();
        let group = category["group"].as_str().unwrap();
        assert_eq!(category_lexicon_info(tag), Some((title, group)));
        assert_eq!(optional_lexicon_group(tag), group);
        assert!(default_optional_lexicon_tag_enabled(tag));
        assert!(discovered.iter().any(|(found, _)| found == tag));
    }
    // Existing discipline preferences retain their old opt-in policy.
    assert!(!default_optional_lexicon_tag_enabled(
        "professional_medical"
    ));
    assert_eq!(category_lexicon_info("my_custom_dictionary"), None);
}

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let workspace_target = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        if let (Ok(path), Ok(target)) = (self.0.canonicalize(), workspace_target.canonicalize()) {
            if path.starts_with(&target) && path != target {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

#[test]
fn category_weights_and_phrase_readings_survive_both_profiles_and_prebaking() {
    let fixture = Fixture(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "category-lexicon-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )),
    );
    for directory in ["zh", "zh-ext"] {
        std::fs::create_dir_all(fixture.0.join(directory)).unwrap();
    }
    std::fs::write(
        fixture.0.join("zh/base_fixture.txt"),
        "常用表达\tchang yong biao da\t9900\n基础表达\tji chu biao da\t9000\n",
    )
    .unwrap();
    std::fs::write(
        fixture.0.join("zh-ext/category_office.txt"),
        "会议纪要\thui yi ji yao\t6100\n调休申请\ttiao xiu shen qing\t1800\n你方便接电话吗\tni fang bian jie dian hua ma\t6500\n",
    )
    .unwrap();
    for profile in [LexiconBuildProfile::Hot, LexiconBuildProfile::Standard] {
        let (lexicon, report) =
            load_dir_txt_with_profile_default_enabled(&fixture.0, profile).unwrap();
        assert!(report.files_failed.is_empty());
        assert_eq!(report.entries_indexed, 5);
        let binary = fixture.0.join("lexicon.bin");
        save_prebaked_lexicon(&binary, &lexicon).unwrap();
        let restored = load_prebaked_lexicon(&binary).unwrap();
        for (word, full, abbreviated, score) in [
            ("会议纪要", "huiyijiyao", "hyjy", 6100),
            ("调休申请", "tiaoxiushenqing", "txsq", 1800),
            ("你方便接电话吗", "nifangbianjiedianhuama", "nfbjdhm", 6500),
        ] {
            for index in [&lexicon, &restored] {
                assert!(index
                    .lookup_pinyin(full)
                    .unwrap()
                    .iter()
                    .any(|entry| entry.phrase == word && entry.freq == score));
                assert!(index
                    .lookup(abbreviated)
                    .unwrap()
                    .iter()
                    .any(|entry| entry.phrase == word && entry.freq == score));
            }
        }
    }
}
