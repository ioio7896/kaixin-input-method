//! Rebuild the bundled lexicons and supported-character table from declared,
//! independently licensed source data.

use pinyin::ToPinyin;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CHAR_RANGES: &[(u32, u32)] = &[
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B73F),
    (0x2B740, 0x2B81F),
    (0x2B820, 0x2CEAF),
    (0x2CEB0, 0x2EBEF),
    (0x30000, 0x3134F),
    (0x31350, 0x323AF),
];

#[derive(Clone, Debug)]
struct Correction {
    phrase: Vec<char>,
    syllables: Vec<String>,
}

#[derive(Clone, Debug)]
struct PronunciationAlias {
    entry: Entry,
    label: String,
}

#[derive(Clone, Debug)]
struct Entry {
    phrase: String,
    code: String,
    freq: u64,
}

fn normalize_syllable(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace('ü', "v")
        .replace("nue", "nve")
        .replace("lue", "lve")
}

fn read_corrections(path: &Path) -> io::Result<Vec<Correction>> {
    let text = fs::read_to_string(path)?;
    let mut out = Vec::new();
    let mut seen_phrases = HashSet::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
        if !(2..=3).contains(&fields.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected <phrase>\\t<pinyin>[\\t<exact|substring>]",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        let phrase = fields[0].chars().collect::<Vec<_>>();
        let syllables = fields[1]
            .split_whitespace()
            .map(normalize_syllable)
            .collect::<Vec<_>>();
        if phrase.is_empty()
            || phrase.len() != syllables.len()
            || syllables
                .iter()
                .any(|value| !value.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: invalid phrase or pinyin",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        if !seen_phrases.insert(phrase.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: duplicate standard correction phrase",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        if let Some(scope) = fields.get(2).copied() {
            if !matches!(scope, "exact" | "substring") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{}:{}: invalid correction scope {scope:?}",
                        path.display(),
                        line_no + 1
                    ),
                ));
            }
        }
        out.push(Correction { phrase, syllables });
    }
    out.sort_by_key(|item| Reverse(item.phrase.len()));
    Ok(out)
}

fn read_core_priority_corrections(
    path: &Path,
    corrections: &[Correction],
) -> io::Result<HashSet<(String, String)>> {
    let known = corrections
        .iter()
        .map(|item| (item.phrase.iter().collect(), item.syllables.join(" ")))
        .collect::<HashSet<(String, String)>>();
    let mut priority = HashSet::new();
    for (line_no, raw) in fs::read_to_string(path)?.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
        let pair = if fields.len() == 2 {
            (
                fields[0].to_string(),
                fields[1]
                    .split_whitespace()
                    .map(normalize_syllable)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected phrase and reading",
                    path.display(),
                    line_no + 1
                ),
            ));
        };
        if !known.contains(&pair) || !priority.insert(pair) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: unknown or duplicate Core priority reading",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
    }
    Ok(priority)
}

fn collect_always_on_readings(dir: &Path) -> io::Result<HashSet<(String, String)>> {
    let mut readings = HashSet::new();
    for item in fs::read_dir(dir)? {
        let path = item?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("txt")
            || path.file_name().and_then(|name| name.to_str()) == Some("kaixin_recall.txt")
            || path.file_name().and_then(|name| name.to_str())
                == Some("life_common_3char_tail_15000.txt")
        {
            continue;
        }
        for line in fs::read_to_string(path)?.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() >= 3 && fields[2].parse::<u64>().is_ok() {
                readings.insert((
                    fields[0].trim().to_string(),
                    fields[1]
                        .split_whitespace()
                        .map(normalize_syllable)
                        .collect::<Vec<_>>()
                        .join(" "),
                ));
            }
        }
    }
    Ok(readings)
}

fn primary_pinyin(ch: char) -> Option<String> {
    ch.to_pinyin()
        .map(|value| normalize_syllable(value.plain()))
}

fn read_project_entries(path: &Path) -> io::Result<Vec<Entry>> {
    let text = fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected <phrase>\\t<pinyin>\\t<weight>",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        let syllables = fields[1].split_whitespace().collect::<Vec<_>>();
        if fields[0].chars().count() != syllables.len()
            || syllables
                .iter()
                .any(|value| !value.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}:{}: invalid pinyin", path.display(), line_no + 1),
            ));
        }
        let freq = fields[2].parse::<u64>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}:{}: invalid weight", path.display(), line_no + 1),
            )
        })?;
        entries.push(Entry {
            phrase: fields[0].to_string(),
            code: syllables.join(" ").to_ascii_lowercase(),
            freq: freq.max(1),
        });
    }
    Ok(entries)
}

fn read_pronunciation_aliases(path: &Path) -> io::Result<Vec<PronunciationAlias>> {
    let text = fs::read_to_string(path)?;
    let mut aliases = Vec::new();
    let mut seen = HashSet::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: expected <phrase>\\t<pinyin>\\t<weight>\\t<primary|alternate|colloquial|historical>",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        let phrase = fields[0];
        let syllables = fields[1]
            .split_whitespace()
            .map(normalize_syllable)
            .collect::<Vec<_>>();
        if phrase.is_empty()
            || phrase.chars().count() != syllables.len()
            || syllables
                .iter()
                .any(|value| !value.chars().all(|ch| ch.is_ascii_alphabetic()))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: invalid phrase or pinyin",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        let code = syllables.join(" ");
        if !seen.insert((phrase.to_string(), code.clone())) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}:{}: duplicate phrase/pinyin pair",
                    path.display(),
                    line_no + 1
                ),
            ));
        }
        let freq = fields[2].parse::<u64>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}:{}: invalid weight", path.display(), line_no + 1),
            )
        })?;
        let label = match fields[3] {
            "primary" | "alternate" | "colloquial" | "historical" => fields[3],
            value => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{}:{}: invalid pronunciation label {value:?}",
                        path.display(),
                        line_no + 1
                    ),
                ));
            }
        };
        aliases.push(PronunciationAlias {
            entry: Entry {
                phrase: phrase.to_string(),
                code,
                freq: freq.max(1),
            },
            label: label.to_string(),
        });
    }
    Ok(aliases)
}

/// 把人工权重归一化到 1..=10000 万分制（与 MAX_LEXICON_FREQ 约定一致）。
/// 按词语长度分组映射：不同长度的词频分布不可直接比较；组内按原权重的
/// 唯一值排名（并列保持并列）映射到 10000..=1，相对顺序不变。运行时的
/// 百分位校准以排名为输入，因此归一化不改变最终候选顺序。
fn normalize_lexicon_weights(entries: &mut [Entry]) {
    let mut by_len: BTreeMap<usize, Vec<&mut Entry>> = BTreeMap::new();
    for entry in entries.iter_mut() {
        by_len
            .entry(entry.phrase.chars().count())
            .or_default()
            .push(entry);
    }
    for group in by_len.values_mut() {
        if group.len() <= 1 {
            if let Some(entry) = group.first_mut() {
                entry.freq = entry.freq.max(1).min(10_000);
            }
            continue;
        }
        group.sort_by(|left, right| right.freq.cmp(&left.freq));
        let mut unique: Vec<u64> = Vec::new();
        for entry in group.iter() {
            let value = entry.freq.max(1);
            if unique.last() != Some(&value) {
                unique.push(value);
            }
        }
        let last = unique.len().saturating_sub(1);
        for entry in group.iter_mut() {
            let value = entry.freq.max(1);
            let rank = unique
                .iter()
                .position(|&candidate| candidate == value)
                .unwrap_or(last);
            // 10000 - rank*(9999/last)，rank 0 → 10000，rank last → 1。
            entry.freq = (10_000 - rank.saturating_mul(9_999) / last.max(1)) as u64;
        }
    }
}

fn write_entries(path: &Path, title: &str, source: &str, entries: &[Entry]) -> io::Result<()> {
    let mut text = format!(
        "# 开心输入法原生词库：{title}\n# 格式：词语<TAB>拼音<TAB>权重\n# Source: {source}, generated by build_clean_lexicons\n"
    );
    for entry in entries {
        text.push_str(&entry.phrase);
        text.push('\t');
        text.push_str(&entry.code);
        text.push('\t');
        text.push_str(&entry.freq.to_string());
        text.push('\n');
    }
    fs::write(path, text)
}

fn collect_existing_lexicon_frequencies(lexicon_dir: &Path) -> io::Result<BTreeMap<String, u64>> {
    fn visit(path: &Path, frequencies: &mut BTreeMap<String, u64>) -> io::Result<()> {
        for item in fs::read_dir(path)? {
            let item = item?;
            let item_path = item.path();
            if item_path.is_dir() {
                if !item_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("en"))
                {
                    visit(&item_path, frequencies)?;
                }
                continue;
            }
            let Some(name) = item_path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if item_path.extension().and_then(|ext| ext.to_str()) != Some("txt")
                || name.eq_ignore_ascii_case("kaixin_polyphone.txt")
                || name.eq_ignore_ascii_case("kaixin_pronunciation_aliases.txt")
                || name.eq_ignore_ascii_case("kaixin_recall.txt")
                || name.eq_ignore_ascii_case("life_common_3char_tail_15000.txt")
            {
                continue;
            }
            let text = fs::read_to_string(&item_path)?;
            for raw in text.lines() {
                let line = raw.trim().trim_start_matches('\u{feff}');
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let fields = line.split('\t').map(str::trim).collect::<Vec<_>>();
                let Some(phrase) = fields.first().copied().filter(|value| !value.is_empty()) else {
                    continue;
                };
                let Some(freq) = fields
                    .iter()
                    .skip(1)
                    .rev()
                    .find_map(|value| value.parse::<u64>().ok())
                else {
                    continue;
                };
                frequencies
                    .entry(phrase.to_string())
                    .and_modify(|stored| *stored = (*stored).max(freq))
                    .or_insert(freq);
            }
        }
        Ok(())
    }

    let mut frequencies = BTreeMap::new();
    visit(lexicon_dir, &mut frequencies)?;
    Ok(frequencies)
}

fn write_polyphone_lexicon(
    path: &Path,
    corrections: &[Correction],
    source_frequency: &BTreeMap<String, u64>,
    core_priority: &HashSet<(String, String)>,
    always_on_readings: &HashSet<(String, String)>,
) -> io::Result<(Vec<Entry>, Vec<Entry>)> {
    let mut entries = corrections
        .iter()
        .map(|rule| {
            let phrase = rule.phrase.iter().collect::<String>();
            Entry {
                freq: source_frequency.get(&phrase).copied().unwrap_or(9_999),
                phrase,
                code: rule.syllables.join(" "),
            }
        })
        .collect::<Vec<_>>();
    normalize_lexicon_weights(&mut entries);
    entries.sort_by(|left, right| left.phrase.cmp(&right.phrase));
    let mut core = Vec::new();
    let mut recall = Vec::new();
    for entry in &entries {
        let pair = (entry.phrase.clone(), entry.code.clone());
        if core_priority.contains(&pair) {
            core.push(entry.clone());
        } else if !always_on_readings.contains(&pair) {
            let mut low_priority = entry.clone();
            low_priority.freq = low_priority.freq.min(1_500);
            recall.push(low_priority);
        }
    }
    write_entries(
        path,
        "需要 Core 优先级的多音词",
        "多音词校正表与 Core 优先级清单",
        &core,
    )?;
    Ok((entries, recall))
}

fn write_pronunciation_alias_lexicon(
    path: &Path,
    aliases: &[PronunciationAlias],
) -> io::Result<Vec<Entry>> {
    let mut aliases = aliases.to_vec();
    let mut alias_entries = aliases
        .iter()
        .map(|alias| alias.entry.clone())
        .collect::<Vec<_>>();
    normalize_lexicon_weights(&mut alias_entries);
    for (alias, entry) in aliases.iter_mut().zip(alias_entries) {
        alias.entry = entry;
    }
    aliases.sort_by(|left, right| {
        left.entry
            .phrase
            .cmp(&right.entry.phrase)
            .then_with(|| right.entry.freq.cmp(&left.entry.freq))
            .then_with(|| left.entry.code.cmp(&right.entry.code))
    });
    let mut text = String::from(
        "# 开心输入法可接受读音与多音字表\n# 格式：词语或单字<TAB>无声调拼音<TAB>独立权重<TAB>读音类别\n# 同一个词语或单字可以按不同拼音出现多次；类别参与运行时候选排序。\n",
    );
    for alias in &aliases {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            alias.entry.phrase, alias.entry.code, alias.entry.freq, alias.label
        ));
    }
    fs::write(path, text)?;
    Ok(aliases.into_iter().map(|alias| alias.entry).collect())
}

fn add_supported_char(ch: char, seen: &mut HashSet<char>, ordered: &mut Vec<char>) {
    if primary_pinyin(ch).is_some() && seen.insert(ch) {
        ordered.push(ch);
    }
}

fn write_character_table(path: &Path, prioritized: &[Entry]) -> io::Result<usize> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for entry in prioritized {
        for ch in entry.phrase.chars() {
            add_supported_char(ch, &mut seen, &mut ordered);
        }
    }
    for &(start, end) in CHAR_RANGES {
        for value in start..=end {
            if let Some(ch) = char::from_u32(value) {
                add_supported_char(ch, &mut seen, &mut ordered);
            }
        }
    }
    let mut text = String::with_capacity(ordered.len() * 4);
    for ch in &ordered {
        text.push(*ch);
        text.push('\n');
    }
    fs::write(path, text)?;
    Ok(ordered.len())
}

fn main() -> io::Result<()> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent().unwrap_or(&manifest);
    let correction_path = repo.join("data_sources/kaixin/polyphone_corrections.tsv");
    let core_priority_path = repo.join("data_sources/kaixin/core_priority_corrections.tsv");
    let pronunciation_alias_path = repo.join("data_sources/kaixin/pronunciation_aliases.tsv");
    let common_phrase_path = repo.join("data_sources/kaixin/common_phrases.tsv");
    let recall_phrase_path = repo.join("data_sources/kaixin/recall_phrases.tsv");
    let lexicon_dir = repo.join("lexicon");
    let core_dir = lexicon_dir.join("core");
    let zh_dir = lexicon_dir.join("zh");
    fs::create_dir_all(&core_dir)?;
    fs::create_dir_all(&zh_dir)?;

    let corrections = read_corrections(&correction_path)?;
    let core_priority = read_core_priority_corrections(&core_priority_path, &corrections)?;
    let always_on_readings = collect_always_on_readings(&zh_dir)?;
    let pronunciation_aliases = read_pronunciation_aliases(&pronunciation_alias_path)?;

    let mut project_entries = read_project_entries(&common_phrase_path)?;
    normalize_lexicon_weights(&mut project_entries);
    write_entries(
        &core_dir.join("kaixin_explicit.txt"),
        "开心输入法基础候选与异读",
        "开心输入法自维护基础候选",
        &project_entries,
    )?;

    let mut source_frequency = collect_existing_lexicon_frequencies(&lexicon_dir)?;
    for entry in &project_entries {
        source_frequency
            .entry(entry.phrase.clone())
            .and_modify(|freq: &mut u64| *freq = (*freq).max(entry.freq))
            .or_insert(entry.freq);
    }
    let (polyphones, mut recall_entries) = write_polyphone_lexicon(
        &core_dir.join("kaixin_polyphone.txt"),
        &corrections,
        &source_frequency,
        &core_priority,
        &always_on_readings,
    )?;
    recall_entries.extend(read_project_entries(&recall_phrase_path)?.into_iter().map(
        |mut entry| {
            entry.freq = (entry.freq / 10_000).clamp(1, 1_500);
            entry
        },
    ));
    recall_entries.sort_by(|left, right| {
        left.phrase
            .cmp(&right.phrase)
            .then_with(|| left.code.cmp(&right.code))
    });
    recall_entries.dedup_by(|left, right| left.phrase == right.phrase && left.code == right.code);
    write_entries(
        &zh_dir.join("kaixin_recall.txt"),
        "始终启用的低优先级读音",
        "多音词与基础候选来源",
        &recall_entries,
    )?;
    let pronunciation_alias_entries = write_pronunciation_alias_lexicon(
        &core_dir.join("kaixin_pronunciation_aliases.txt"),
        &pronunciation_aliases,
    )?;
    let mut prioritized = project_entries.clone();
    prioritized.extend(polyphones);
    prioritized.extend(recall_entries);
    prioritized.extend(pronunciation_alias_entries);
    let char_count = write_character_table(
        &manifest.join("data/pinyin_supported_chars.txt"),
        &prioritized,
    )?;

    println!(
        "generated {} AI-maintained entries, {} corrections, {} pronunciation aliases and {} supported characters",
        project_entries.len(),
        corrections.len(),
        pronunciation_aliases.len(),
        char_count
    );
    Ok(())
}
