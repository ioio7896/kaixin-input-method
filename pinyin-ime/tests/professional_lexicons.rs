use pinyin_ime::lexicon_prefs::{
    default_optional_lexicon_tag_enabled, discover_optional_lexicon_tags_in_lexicon_dir,
    optional_lexicon_group, optional_lexicon_path_tag,
};
use std::path::Path;

#[test]
fn professional_vocabularies_are_independent_optional_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lexicon");
    let discovered = discover_optional_lexicon_tags_in_lexicon_dir(&root);
    for discipline in ["medical", "law", "math", "physics", "chemistry", "biology",
        "history", "geography", "philosophy", "finance", "architecture", "computing", "music"] {
        let tag = format!("professional_{discipline}");
        let path = root.join("zh-ext").join(format!("{tag}.txt"));
        assert_eq!(optional_lexicon_path_tag(&path), Some(tag.clone()));
        assert!(discovered.iter().any(|(found, _)| found == &tag));
        assert!(!default_optional_lexicon_tag_enabled(&tag));
        assert_eq!(optional_lexicon_group(&tag), "专业领域");
        let text = std::fs::read_to_string(path).unwrap();
        let rows: Vec<_> = text.lines().filter(|line| !line.starts_with('#') && !line.is_empty()).collect();
        assert!(rows.len() >= 60);
        for row in rows {
            let fields: Vec<_> = row.split('\t').collect();
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0].chars().count(), fields[1].split_whitespace().count());
        }
    }
}
