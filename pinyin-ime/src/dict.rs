use pinyin::ToPinyin;
use std::collections::HashMap;
use std::sync::OnceLock;

const PRONUNCIATION_ALIASES: &str =
    include_str!("../../data_sources/kaixin/pronunciation_aliases.tsv");

fn normalize_pinyin_key(key: &str) -> String {
    key.replace('ü', "v")
        .replace("nue", "nve")
        .replace("lue", "lve")
}

fn pronunciation_aliases() -> &'static HashMap<char, Vec<String>> {
    static ALIASES: OnceLock<HashMap<char, Vec<String>>> = OnceLock::new();
    ALIASES.get_or_init(|| {
        let mut aliases: HashMap<char, Vec<String>> = HashMap::new();
        for line in PRONUNCIATION_ALIASES.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split('\t');
            let Some(phrase) = fields.next() else {
                continue;
            };
            let mut chars = phrase.chars();
            let Some(ch) = chars.next() else {
                continue;
            };
            if chars.next().is_some() {
                continue;
            }
            let Some(reading) = fields.next() else {
                continue;
            };
            let _weight = fields.next();
            let category = fields.next().unwrap_or("alternate");
            if category.eq_ignore_ascii_case("historical") {
                continue;
            }
            let reading = normalize_pinyin_key(&reading.to_ascii_lowercase());
            if reading.is_empty() {
                continue;
            }
            let values = aliases.entry(ch).or_default();
            if !values.iter().any(|value| value == &reading) {
                values.push(reading);
            }
        }
        aliases
    })
}

/// Return the modern Mandarin readings used by the input engine.
///
/// `pinyin` is generated from a merged historical/etymological table, so its
/// multi-reading iterator also contains obsolete readings such as `xiong` for
/// 能 and `neng` for 而.  Keep the primary reading from the package and add
/// only the project's explicitly accepted modern single-character aliases.
pub(crate) fn pinyin_plain_options(c: char) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(py) = c.to_pinyin() {
        let plain = normalize_pinyin_key(&py.plain().to_ascii_lowercase());
        if !plain.is_empty() {
            out.push(plain);
        }
    }
    if let Some(aliases) = pronunciation_aliases().get(&c) {
        for alias in aliases {
            if !out.iter().any(|seen| seen == alias) {
                out.push(alias.clone());
            }
        }
    }
    out
}

pub fn build_dict(chars: &str) -> HashMap<String, Vec<char>> {
    let mut m: HashMap<String, Vec<char>> = HashMap::new();
    for c in chars
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\r' && *ch != '\n')
    {
        for s in pinyin_plain_options(c) {
            m.entry(s).or_default().push(c);
        }
    }
    for v in m.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    m
}

#[cfg(test)]
mod tests {
    use super::{build_dict, pinyin_plain_options};

    #[test]
    fn build_dict_indexes_heteronym_readings() {
        let dict = build_dict("重");
        assert!(dict.get("zhong").is_some_and(|chars| chars.contains(&'重')));
        assert!(dict.get("chong").is_some_and(|chars| chars.contains(&'重')));
    }

    #[test]
    fn build_dict_uses_v_form_for_umlaut_syllables() {
        let dict = build_dict("\u{8650}\u{7565}");
        assert!(dict
            .get("nve")
            .is_some_and(|chars| chars.contains(&'\u{8650}')));
        assert!(dict
            .get("lve")
            .is_some_and(|chars| chars.contains(&'\u{7565}')));
        assert!(!dict.contains_key("nue"));
        assert!(!dict.contains_key("lue"));
    }

    #[test]
    fn build_dict_excludes_historical_neng_xiong_reading() {
        let dict = build_dict("能熊");
        assert!(dict.get("neng").is_some_and(|chars| chars.contains(&'能')));
        assert!(dict.get("xiong").is_some_and(|chars| chars.contains(&'熊')));
        assert!(!dict.get("xiong").is_some_and(|chars| chars.contains(&'能')));
    }

    #[test]
    fn excludes_historical_single_character_readings_from_common_collisions() {
        let historical = [
            ('而', "neng"),
            ('是', "ti"),
            ('不', "fou"),
            ('也', "yi"),
            ('他', "tuo"),
            ('年', "ning"),
            ('个', "gan"),
            ('前', "jian"),
            ('听', "yin"),
            ('她', "jie"),
            ('它', "tuo"),
            ('能', "xiong"),
        ];
        for (ch, reading) in historical {
            assert!(
                !pinyin_plain_options(ch)
                    .iter()
                    .any(|value| value == reading),
                "unexpected historical reading {ch} -> {reading}"
            );
        }
    }

    #[test]
    fn keeps_project_approved_modern_single_character_aliases() {
        assert!(pinyin_plain_options('重').contains(&"chong".to_string()));
        assert!(pinyin_plain_options('行').contains(&"hang".to_string()));
        assert!(pinyin_plain_options('着').contains(&"zhao".to_string()));
    }
}
