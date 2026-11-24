use pinyin_ime::lexicon_prefs::{
    default_optional_lexicon_tag_enabled, discover_optional_lexicon_tags_in_lexicon_dir,
    is_retired_optional_lexicon_tag, optional_lexicon_group, optional_lexicon_path_tag,
};
use std::path::Path;

#[test]
fn professional_vocabularies_are_consolidated_into_optional_domain_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lexicon");
    let discovered = discover_optional_lexicon_tags_in_lexicon_dir(&root);
    for (tag, expected_group) in [
        ("category_medical", "专业领域"),
        ("category_law", "专业领域"),
        ("category_finance", "专业领域"),
        ("category_science", "专业领域"),
        ("category_engineering", "专业领域"),
        ("category_humanities", "专业领域"),
        ("technology", "科技与开发"),
    ] {
        let path = root.join("zh-ext").join(format!("{tag}.txt"));
        assert_eq!(optional_lexicon_path_tag(&path), Some(tag.to_string()));
        assert!(discovered.iter().any(|(found, _)| found == tag), "{tag}");
        assert!(!default_optional_lexicon_tag_enabled(tag), "{tag}");
        assert_eq!(optional_lexicon_group(tag), expected_group, "{tag}");
        let text = std::fs::read_to_string(path).unwrap();
        let rows: Vec<_> = text
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .collect();
        assert!(rows.len() >= 200, "{tag}: {}", rows.len());
    }
    for legacy in [
        "professional_medical",
        "professional_law",
        "professional_math",
        "professional_physics",
        "professional_chemistry",
        "professional_biology",
        "professional_history",
        "professional_geography",
        "professional_philosophy",
        "professional_finance",
        "professional_architecture",
        "professional_computing",
        "professional_music",
    ] {
        assert!(is_retired_optional_lexicon_tag(legacy), "{legacy}");
        assert!(!default_optional_lexicon_tag_enabled(legacy), "{legacy}");
    }
}
