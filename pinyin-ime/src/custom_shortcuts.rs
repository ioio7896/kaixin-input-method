#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutCandidate {
    pub phrase: String,
    pub source_key: String,
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn unescape_phrase(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn parse_shortcuts_section(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{FEFF}');
        if line.is_empty()
            || line.starts_with('#')
            || (line.starts_with(';') && !line.contains('='))
        {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            in_section = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("shortcuts");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = normalize_key(key);
        if key.is_empty() {
            continue;
        }
        let phrases = value
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(unescape_phrase)
            .collect::<Vec<_>>();
        if phrases.is_empty() {
            continue;
        }
        out.push((key, phrases));
    }
    out
}

fn entries() -> std::sync::Arc<[(String, Vec<String>)]> {
    std::sync::Arc::clone(&crate::runtime_config::snapshot().shortcuts)
}

pub fn lookup(input: &str) -> Option<Vec<(String, f64)>> {
    let key = normalize_key(input);
    if key.is_empty() {
        return None;
    }
    for (shortcut, phrases) in entries().iter() {
        if shortcut == &key {
            let out = phrases
                .iter()
                .enumerate()
                .map(|(idx, phrase)| (phrase.clone(), 10_000.0 - idx as f64))
                .collect::<Vec<_>>();
            return Some(out);
        }
    }
    None
}

pub fn lookup_detailed(input: &str) -> Option<Vec<ShortcutCandidate>> {
    let key = normalize_key(input);
    if key.is_empty() {
        return None;
    }
    for (shortcut, phrases) in entries().iter() {
        if shortcut == &key {
            return Some(
                phrases
                    .iter()
                    .map(|phrase| ShortcutCandidate {
                        phrase: phrase.clone(),
                        source_key: shortcut.clone(),
                    })
                    .collect(),
            );
        }
    }
    None
}
