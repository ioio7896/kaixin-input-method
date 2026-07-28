use pinyin::ToPinyinMulti;
use std::collections::HashMap;

fn normalize_pinyin_key(key: &str) -> String {
    key.replace('ü', "v")
        .replace("nue", "nve")
        .replace("lue", "lve")
}

fn pinyin_plain_options(c: char) -> Vec<String> {
    let Some(multi) = c.to_pinyin_multi() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for py in multi {
        let plain = normalize_pinyin_key(&py.plain().to_ascii_lowercase());
        if plain.is_empty() || out.iter().any(|seen| seen == &plain) {
            continue;
        }
        out.push(plain);
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
    use super::build_dict;

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
}
