use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::{is_han, read_utf8, text_files, Result};

fn field<'a>(row: &'a Value, key: &str) -> Result<&'a str> {
    row[key]
        .as_str()
        .ok_or_else(|| format!("missing {key} in lexicon policy").into())
}

fn list<'a>(row: &'a Value, key: &str) -> Result<&'a [Value]> {
    row[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("missing {key} in lexicon policy").into())
}

fn data_rows(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.starts_with('#'))
}

fn normalize_reading(value: &str) -> String {
    value
        .to_lowercase()
        .replace('ü', "v")
        .replace("nue", "nve")
        .replace("lue", "lve")
}

fn check_english(root: &Path) -> Result<()> {
    let path = root.join("en/kaixin_common_english.txt");
    for required in ["en/LICENSE.wordfreq.md", "en/README.md"] {
        if fs::metadata(root.join(required))?.len() == 0 {
            return Err(format!("empty English lexicon documentation: {required}").into());
        }
    }
    let text = read_utf8(&path)?;
    let mut seen = HashSet::new();
    let mut previous = u64::MAX;
    for (index, row) in data_rows(&text) {
        let columns: Vec<_> = row.split('\t').map(str::trim).collect();
        if let [word, code, weight] = columns.as_slice() {
            if let Ok(weight) = weight.parse::<u64>() {
                if (2..=24).contains(&word.len())
                    && word.bytes().all(|c| c.is_ascii_lowercase())
                    && word == code
                    && weight > 0
                    && weight <= previous
                    && seen.insert(*word)
                {
                    previous = weight;
                    continue;
                }
            }
        }
        return Err(format!(
            "{}:{}: invalid English lexicon row",
            path.display(),
            index + 1
        )
        .into());
    }
    if seen.len() != 20_000
        || [
            "the", "this", "you", "computer", "internet", "email", "github", "python", "rust",
            "windows",
        ]
        .iter()
        .any(|word| !seen.contains(word))
    {
        return Err("common English lexicon is incomplete".into());
    }
    Ok(())
}

pub fn check(repo: &Path) -> Result<()> {
    let lexicon = repo.join("lexicon");
    check_english(&lexicon)?;
    let syllables = read_utf8(&repo.join("pinyin-ime/data/syllables.txt"))?;
    let syllables: HashSet<_> = syllables.split_whitespace().collect();
    let policy: Value =
        serde_json::from_str(&read_utf8(&repo.join("shared/lexicon_quality.json"))?)?;
    let short: Value =
        serde_json::from_str(&read_utf8(&repo.join("shared/short_phrase_quality.json"))?)?;
    let mut rejected: HashSet<_> = policy["rejected_phrases"]
        .as_object()
        .ok_or("missing rejected phrases")?
        .keys()
        .map(String::as_str)
        .collect();
    for group in short["rejected_groups"]
        .as_object()
        .ok_or("missing rejected groups")?
        .values()
    {
        for word in group.as_array().ok_or("invalid rejected phrase group")? {
            rejected.insert(word.as_str().ok_or("invalid rejected phrase")?);
        }
    }
    let retained: HashSet<_> = list(&policy, "retained_phrase_readings")?
        .iter()
        .map(|row| Ok((field(row, "phrase")?, field(row, "reading")?)))
        .collect::<Result<_>>()?;
    let corrections: HashSet<_> = list(&policy, "reading_corrections")?
        .iter()
        .map(|row| Ok((field(row, "phrase")?, field(row, "rejected")?)))
        .collect::<Result<_>>()?;
    let spellings: HashSet<_> = list(&policy, "spelling_corrections")?
        .iter()
        .map(|row| field(row, "phrase"))
        .collect::<Result<_>>()?;
    let categories: Value =
        serde_json::from_str(&read_utf8(&repo.join("shared/category_lexicons.json"))?)?;
    for row in list(&categories, "categories")? {
        let name = field(row, "file")?;
        let path = lexicon.join("zh-ext").join(name);
        if format!("{:x}", Sha256::digest(fs::read(&path)?)) != field(row, "sha256")?
            || data_rows(&read_utf8(&path)?).count() as u64
                != row["rows"].as_u64().ok_or("missing category row count")?
        {
            return Err(format!("stale category manifest for {name}").into());
        }
    }

    let read_table = read_utf8(&repo.join("pinyin-ime/data/pinyin_character_readings.txt"))?;
    let mut reference = HashMap::<char, HashSet<String>>::new();
    for (_, line) in data_rows(&read_table) {
        let (character, readings) = line
            .split_once('\t')
            .ok_or("invalid character reading row")?;
        let character = character.chars().next().ok_or("empty character")?;
        reference.insert(
            character,
            readings.split_whitespace().map(normalize_reading).collect(),
        );
    }
    for name in ["pronunciation_aliases.tsv", "pronunciation_exclusions.tsv"] {
        let text = read_utf8(&repo.join("data_sources/kaixin").join(name))?;
        for (_, line) in data_rows(&text) {
            let fields: Vec<_> = line.split('\t').collect();
            if let [character, reading, ..] = fields.as_slice() {
                if character.chars().count() == 1 {
                    let readings = reference
                        .entry(character.chars().next().unwrap())
                        .or_default();
                    let reading = normalize_reading(reading);
                    if fields.get(3) == Some(&"historical") {
                        readings.remove(&reading);
                    } else {
                        readings.insert(reading);
                    }
                }
            }
        }
    }

    let mut found = HashSet::new();
    for path in text_files(&lexicon)? {
        let relative = path.strip_prefix(&lexicon)?;
        if relative.iter().any(|part| part.eq_ignore_ascii_case("en")) {
            continue;
        }
        let single = path
            .file_name()
            .is_some_and(|name| name == "single_char_common_8105.txt");
        let text = read_utf8(&path)?;
        let mut previous = u64::MAX;
        let mut characters = HashSet::new();
        for (index, line) in data_rows(&text) {
            let bad = |reason: &str| format!("{}:{}: {reason}", relative.display(), index + 1);
            let fields: Vec<_> = line.split('\t').collect();
            let phrase = fields[0];
            if rejected.contains(phrase)
                || spellings.contains(phrase)
                || phrase.contains([
                    '（', '）', '【', '】', '《', '》', '，', '。', '！', '？', '、', '；', '：',
                    '“', '”', '‘', '’',
                ])
            {
                return Err(bad("disallowed lexicon phrase").into());
            }
            let score = fields
                .get(if fields.len() == 2 { 1 } else { 2 })
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|score| (1..=10_000).contains(score))
                .ok_or_else(|| bad("expected 1–10000 score"))?;
            if single {
                if phrase.chars().count() != 1
                    || fields.len() != 2
                    || score >= previous
                    || !characters.insert(phrase)
                {
                    return Err(bad("invalid common single-character row").into());
                }
                previous = score;
            }
            if fields.len() < 3 {
                continue;
            }
            let reading = fields[1];
            if corrections.contains(&(phrase, reading))
                || reading
                    .split_whitespace()
                    .any(|code| matches!(code, "lue" | "nue"))
                    && !relative.starts_with("core")
            {
                return Err(bad("rejected or noncanonical reading").into());
            }
            if retained.contains(&(phrase, reading)) {
                found.insert((phrase.to_owned(), reading.to_owned()));
            } else if phrase.chars().all(is_han) {
                let normalized = normalize_reading(reading);
                let codes: Vec<_> = normalized.split_whitespace().collect();
                if phrase.chars().count() != codes.len()
                    || phrase.chars().zip(codes).any(|(ch, code)| {
                        !reference.get(&ch).is_some_and(|known| known.contains(code))
                    })
                {
                    return Err(bad("word/character reading mismatch").into());
                }
            }
            if !phrase.bytes().any(|byte| byte.is_ascii_alphanumeric()) {
                let lower = reading.to_ascii_lowercase();
                let codes: Vec<_> = lower
                    .split(|ch: char| !ch.is_ascii_alphabetic())
                    .filter(|code| !code.is_empty())
                    .collect();
                let han_count = phrase.chars().filter(|&ch| is_han(ch)).count();
                if codes.iter().any(|code| !syllables.contains(code))
                    || (!codes.is_empty() && han_count > 0 && han_count != codes.len())
                {
                    return Err(bad("unknown syllable or syllable count mismatch").into());
                }
            }
        }
        if single && characters.len() != 8_105 {
            return Err("common single-character lexicon must contain 8105 rows".into());
        }
    }
    if found.len() != retained.len() || !lexicon.join("zh/single_char_common_8105.txt").is_file() {
        return Err("protected lexicon readings or common-character file missing".into());
    }
    println!("Lexicon checks passed: syllables, reviewed readings, category hashes, English and common characters");
    Ok(())
}
