//! English word prefix lookup backed by the bundled 20,000-word native lexicon.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::runtime_log::{self, RuntimeLogLevel};

const WORDS_RELATIVE_PATH: &str = "en/kaixin_common_english.txt";
const EXPECTED_WORD_COUNT: usize = 20_000;
const MAX_MATCHES_TO_RANK: usize = 4096;

static DEFAULT_LEXICON: OnceLock<Option<EnglishWordLexicon>> = OnceLock::new();

#[derive(Debug, Clone)]
struct EnglishWordEntry {
    word: Box<str>,
    lower: Box<str>,
    weight: u32,
    case_rank: u8,
}

#[derive(Debug)]
struct EnglishWordLexicon {
    entries: Vec<EnglishWordEntry>,
    source: PathBuf,
}

impl EnglishWordLexicon {
    fn from_word_rows<I>(rows: I, source: PathBuf) -> io::Result<Self>
    where
        I: IntoIterator<Item = (String, u32)>,
    {
        let mut best: HashMap<String, (String, u32)> = HashMap::new();
        for (word, weight) in rows {
            let word = word.trim();
            if word.is_empty() {
                continue;
            };
            if !is_suggestable_word(word) {
                continue;
            }
            let lower = word.to_ascii_lowercase();
            let replace = best
                .get(&lower)
                .map(|(existing, existing_weight)| {
                    weight > *existing_weight
                        || (weight == *existing_weight
                            && case_rank(word) < case_rank(existing.as_str()))
                })
                .unwrap_or(true);
            if replace {
                best.insert(lower, (word.to_string(), weight));
            }
        }

        let mut entries: Vec<_> = best
            .into_iter()
            .map(|(lower, (word, weight))| EnglishWordEntry {
                case_rank: case_rank(&word),
                word: word.into_boxed_str(),
                lower: lower.into_boxed_str(),
                weight,
            })
            .collect();
        entries.sort_by(|a, b| {
            a.lower
                .cmp(&b.lower)
                .then_with(|| b.weight.cmp(&a.weight))
                .then_with(|| a.case_rank.cmp(&b.case_rank))
                .then_with(|| a.word.cmp(&b.word))
        });
        Ok(Self { entries, source })
    }

    fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut rows = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim().trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(invalid_lexicon_line(
                    path,
                    index + 1,
                    "expected 3 tab-separated fields",
                ));
            }
            let word = fields[0];
            if !is_suggestable_word(word) || word != word.to_ascii_lowercase() {
                return Err(invalid_lexicon_line(
                    path,
                    index + 1,
                    "word must be lowercase ASCII letters",
                ));
            }
            if fields[1] != word {
                return Err(invalid_lexicon_line(
                    path,
                    index + 1,
                    "input code must match the word",
                ));
            }
            let weight = fields[2].parse::<u32>().map_err(|_| {
                invalid_lexicon_line(path, index + 1, "weight must be a positive integer")
            })?;
            if weight == 0 {
                return Err(invalid_lexicon_line(
                    path,
                    index + 1,
                    "weight must be positive",
                ));
            }
            rows.push((word.to_string(), weight));
        }
        let lexicon = Self::from_word_rows(rows, path.to_path_buf())?;
        if lexicon.entries.len() != EXPECTED_WORD_COUNT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "English lexicon must contain {EXPECTED_WORD_COUNT} unique words, found {}: {}",
                    lexicon.entries.len(),
                    path.display()
                ),
            ));
        }
        Ok(lexicon)
    }

    fn lookup_prefix(&self, prefix: &str, limit: usize) -> Vec<String> {
        if limit == 0 {
            return Vec::new();
        }
        let prefix = prefix.to_ascii_lowercase();
        if prefix.len() < 2 || !prefix.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return Vec::new();
        }

        let start = self
            .entries
            .partition_point(|entry| entry.lower.as_ref() < prefix.as_str());
        let mut matches: Vec<&EnglishWordEntry> = Vec::new();
        for entry in self.entries[start..].iter() {
            if !entry.lower.starts_with(prefix.as_str()) {
                break;
            }
            matches.push(entry);
            if matches.len() >= MAX_MATCHES_TO_RANK {
                break;
            }
        }
        matches.sort_by(|a, b| {
            a.case_rank
                .cmp(&b.case_rank)
                .then_with(|| b.weight.cmp(&a.weight))
                .then_with(|| a.word.chars().count().cmp(&b.word.chars().count()))
                .then_with(|| a.lower.cmp(&b.lower))
        });
        matches
            .into_iter()
            .take(limit)
            .map(|entry| entry.word.to_string())
            .collect()
    }
}

fn invalid_lexicon_line(path: &Path, line: usize, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "invalid English lexicon row {}:{line}: {reason}",
            path.display()
        ),
    )
}

pub fn lookup_prefix(prefix: &str, limit: usize) -> Vec<String> {
    let Some(lexicon) = DEFAULT_LEXICON.get_or_init(load_default_lexicon).as_ref() else {
        return Vec::new();
    };
    lexicon.lookup_prefix(prefix, limit)
}

fn load_default_lexicon() -> Option<EnglishWordLexicon> {
    let Some(path) = discover_default_words_path() else {
        runtime_log::log_engine(
            RuntimeLogLevel::Basic,
            "srf_english_words",
            "bundled 20,000-word English lexicon not found",
        );
        return None;
    };
    match EnglishWordLexicon::load(&path) {
        Ok(lexicon) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "srf_english_words",
                format!(
                    "loaded entries={} source={}",
                    lexicon.entries.len(),
                    lexicon.source.display()
                ),
            );
            Some(lexicon)
        }
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_english_words",
                format!("failed to load {}: {err}", path.display()),
            );
            None
        }
    }
}

fn discover_default_words_path() -> Option<PathBuf> {
    candidate_words_paths()
        .into_iter()
        .find(|path| path.is_file())
}

fn candidate_words_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    };

    if let Some(path) = std::env::var_os("SRF_ENGLISH_WORDS_PATH").map(PathBuf::from) {
        push_unique(path);
    }
    if let Some(dir) = std::env::var_os("SRF_LEXICON_DIR").map(PathBuf::from) {
        push_unique(dir.join(WORDS_RELATIVE_PATH));
    }
    if let Some(dir) = crate::core::default_phrase_lexicon_dir() {
        push_unique(dir.join(WORDS_RELATIVE_PATH));
    }
    if let Ok(cwd) = std::env::current_dir() {
        push_unique(cwd.join("lexicon").join(WORDS_RELATIVE_PATH));
        push_unique(cwd.join("..").join("lexicon").join(WORDS_RELATIVE_PATH));
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(8) {
            push_unique(ancestor.join("lexicon").join(WORDS_RELATIVE_PATH));
            push_unique(ancestor.join(WORDS_RELATIVE_PATH));
        }
    }
    paths
}

fn is_suggestable_word(word: &str) -> bool {
    let len = word.chars().count();
    if !(2..=48).contains(&len) {
        return false;
    }
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    if word.ends_with('-') || word.ends_with('\'') {
        return false;
    }
    word.chars()
        .all(|ch| ch.is_ascii_alphabetic() || ch == '\'' || ch == '-')
}

fn case_rank(word: &str) -> u8 {
    if word.chars().all(|ch| !ch.is_ascii_uppercase()) {
        return 0;
    }
    if word
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
        && word
            .chars()
            .skip(1)
            .all(|ch| !ch.is_ascii_uppercase() || !ch.is_ascii_alphabetic())
    {
        return 1;
    }
    if word.chars().all(|ch| !ch.is_ascii_lowercase()) {
        return 3;
    }
    2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_lookup_prefers_weight_then_length() {
        let lexicon = EnglishWordLexicon::from_word_rows(
            [
                ("computer".to_string(), 100u32),
                ("compute".to_string(), 110),
                ("compiler".to_string(), 90),
            ],
            PathBuf::from("test.txt"),
        )
        .unwrap();
        assert_eq!(
            lexicon.lookup_prefix("comp", 3),
            vec!["compute", "computer", "compiler"]
        );
    }

    #[test]
    fn bundled_common_english_word_list_is_available() {
        let words = lookup_prefix("th", 8);
        assert_eq!(words.first().map(String::as_str), Some("the"), "{words:?}");
        assert!(words.iter().any(|word| word == "this"), "{words:?}");
    }
}
