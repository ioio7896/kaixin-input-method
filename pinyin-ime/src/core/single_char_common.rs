use crate::dict::pinyin_plain_options;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SINGLE_CHAR_COMMON_FILE: &str = "single_char_common_8105.txt";

/// Authoritative frequency order for common single-character candidates.
///
/// This index intentionally stays separate from `AbbrevLexicon`: the general
/// lexicon keeps only a bounded head per key and merges duplicate weights from
/// pronunciation aliases. Both behaviours are useful for phrases, but they
/// must not rewrite the frequency order of the dedicated 8,105-character list.
#[derive(Debug, Default)]
pub(super) struct SingleCharCommonIndex {
    by_pinyin: HashMap<String, Vec<char>>,
    by_initial: HashMap<char, Vec<char>>,
    character_count: usize,
}

impl SingleCharCommonIndex {
    pub(super) fn from_lexicon_dir(dir: &Path) -> io::Result<Self> {
        let candidates = [
            dir.join("zh").join(SINGLE_CHAR_COMMON_FILE),
            dir.join("base").join(SINGLE_CHAR_COMMON_FILE),
            dir.join(SINGLE_CHAR_COMMON_FILE),
        ];
        let path = candidates
            .into_iter()
            .find(|path| path.is_file())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("missing {SINGLE_CHAR_COMMON_FILE} under {}", dir.display()),
                )
            })?;
        let text = fs::read_to_string(&path)?;
        Self::from_text(&text, Some(path))
    }

    fn from_text(text: &str, path: Option<PathBuf>) -> io::Result<Self> {
        let mut out = Self::default();
        let mut seen = HashSet::new();
        let mut previous_weight = None;
        for (line_index, raw) in text.lines().enumerate() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((phrase, weight)) = line.split_once('\t') else {
                return Err(invalid_row(
                    &path,
                    line_index,
                    "expected character<TAB>weight",
                ));
            };
            let mut chars = phrase.trim().chars();
            let Some(ch) = chars.next() else {
                return Err(invalid_row(&path, line_index, "missing character"));
            };
            if chars.next().is_some() {
                return Err(invalid_row(&path, line_index, "entry is not one character"));
            }
            let weight = weight
                .trim()
                .parse::<u64>()
                .map_err(|_| invalid_row(&path, line_index, "weight is not an unsigned integer"))?;
            if previous_weight.is_some_and(|previous| previous <= weight) {
                return Err(invalid_row(
                    &path,
                    line_index,
                    "weights must be strictly descending",
                ));
            }
            previous_weight = Some(weight);
            if !seen.insert(ch) {
                return Err(invalid_row(&path, line_index, "duplicate character"));
            }
            out.character_count += 1;

            let readings = pinyin_plain_options(ch);
            if readings.is_empty() {
                return Err(invalid_row(&path, line_index, "character has no pinyin"));
            }
            for key in readings {
                if key.is_empty() {
                    continue;
                }
                push_unique(out.by_pinyin.entry(key.clone()).or_default(), ch);
                if let Some(initial) = key.chars().next().filter(char::is_ascii_alphabetic) {
                    push_unique(out.by_initial.entry(initial).or_default(), ch);
                }
            }
        }
        if out.character_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} contains no single-character entries",
                    path.as_deref()
                        .map(Path::display)
                        .map(|display| display.to_string())
                        .unwrap_or_else(|| SINGLE_CHAR_COMMON_FILE.to_string())
                ),
            ));
        }
        Ok(out)
    }

    pub(super) fn pinyin_order(&self, key: &str) -> &[char] {
        self.by_pinyin.get(key).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(super) fn initial_order(&self, initial: char) -> &[char] {
        self.by_initial
            .get(&initial.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::SingleCharCommonIndex;

    #[test]
    fn excludes_historical_neng_xiong_reading() {
        let index = SingleCharCommonIndex::from_text("能\t2\n熊\t1\n", None)
            .expect("single-character index");
        assert_eq!(index.pinyin_order("neng"), &['能']);
        assert_eq!(index.pinyin_order("xiong"), &['熊']);
    }
}

fn push_unique(values: &mut Vec<char>, ch: char) {
    if !values.contains(&ch) {
        values.push(ch);
    }
}

fn invalid_row(path: &Option<PathBuf>, line_index: usize, reason: &str) -> io::Error {
    let source = path
        .as_deref()
        .map(Path::display)
        .map(|display| display.to_string())
        .unwrap_or_else(|| SINGLE_CHAR_COMMON_FILE.to_string());
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{source}:{}: {reason}", line_index + 1),
    )
}
