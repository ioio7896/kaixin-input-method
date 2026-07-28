use crate::text_norm::normalize_pinyin_line;
use std::collections::{BTreeSet, HashSet, VecDeque};

const LONG_VARIANT_REDUCE_INPUT_LEN: usize = 8;
const VERY_LONG_VARIANT_REDUCE_INPUT_LEN: usize = 12;
const PHONETIC_CONFUSION_MAX_EXTRA: usize = 6;
const INPUT_TYPO_MAX_EXTRA: usize = 8;
const FUZZY_VARIANT_MAX_EXTRA: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseOptions {
    pub fuzzy_enabled: bool,
    pub double_pinyin_enabled: bool,
    pub fuzzy_pairs: crate::fuzzy_prefs::FuzzyPairs,
    /// 当启用纠错时，额外生成一组保守的音节级音近变体（如 ian/iang、r/l 等），
    /// 与 fuzzy 音近分开标识，便于后续按纠错档位控制展示与学习。
    pub correction_enabled: bool,
}

#[derive(Clone)]
struct ParseSnapshot {
    input: String,
    options: ParseOptions,
    result: Result<Vec<ParseVariant>, String>,
}

/// Bounded composition-local parse snapshots. Adjacent typing naturally
/// records every prefix, so Backspace restores the previous parse without
/// rebuilding alternate/fuzzy paths. New suffixes are still parsed normally;
/// their decoder and mixed-pinyin lattices reuse their own incremental state.
pub(crate) struct IncrementalParseCache {
    snapshots: VecDeque<ParseSnapshot>,
    capacity: usize,
}

impl Default for IncrementalParseCache {
    fn default() -> Self {
        Self {
            snapshots: VecDeque::new(),
            capacity: 48,
        }
    }
}

impl IncrementalParseCache {
    pub(crate) fn clear(&mut self) {
        self.snapshots.clear();
    }

    pub(crate) fn parse(
        &mut self,
        input: &str,
        syllable_set: &HashSet<String>,
        options: ParseOptions,
    ) -> Result<Vec<ParseVariant>, String> {
        if let Some(index) = self
            .snapshots
            .iter()
            .position(|snapshot| snapshot.input == input && snapshot.options == options)
        {
            let snapshot = self.snapshots.remove(index).expect("parse snapshot exists");
            let result = snapshot.result.clone();
            self.snapshots.push_front(snapshot);
            return result;
        }

        let result = parse_input_variants_detailed(input, syllable_set, options);
        self.snapshots.push_front(ParseSnapshot {
            input: input.to_string(),
            options,
            result: result.clone(),
        });
        while self.snapshots.len() > self.capacity {
            self.snapshots.pop_back();
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, input: &str, options: ParseOptions) -> bool {
        self.snapshots
            .iter()
            .any(|snapshot| snapshot.input == input && snapshot.options == options)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseVariantSource {
    Exact,
    AlternateSplit,
    /// zh/z、in/ing 等音节级混淆（由 fuzzy 配置驱动）
    SyllableConfusion,
    /// 由纠错档位驱动的音节级音近变体（如 ian/iang、uan/uang、r/l 等）
    PhoneticTypo,
    KeyboardTypo,
    Fuzzy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseVariant {
    pub syllables: Vec<String>,
    pub tail: Option<String>,
    pub source: ParseVariantSource,
    pub corrected_input: Option<String>,
}

fn strip_tone_digits(s: &str) -> String {
    s.chars().filter(|ch| !matches!(ch, '1'..='5')).collect()
}

pub fn normalize_compact_pinyin_aliases(s: &str) -> String {
    s.replace("nue", "nve").replace("lue", "lve")
}

/// Normalize to lowercase, drop tone digits, and treat `ü` as `v`.
pub fn normalize_token(s: &str) -> String {
    let normalized = strip_tone_digits(&s.to_lowercase().replace('ü', "v"));
    normalize_compact_pinyin_aliases(&normalized)
}

pub fn load_syllables(text: &str) -> HashSet<String> {
    use crate::text_norm::strip_bom_str;
    text.lines()
        .map(|l| strip_bom_str(l).trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect()
}

pub fn prefix_is_valid_syllable(prefix: &str, syllable_set: &HashSet<String>) -> bool {
    !prefix.is_empty() && syllable_set.iter().any(|s| s.starts_with(prefix))
}

pub fn predict_syllable_completions(
    prefix: &str,
    syllable_set: &HashSet<String>,
    limit: usize,
) -> Vec<String> {
    predict_syllable_completions_by_score(prefix, syllable_set, limit, |_| 0.0)
}

/// 按外部打分函数对音节补全做概率序排序；分数相同时按字典序稳定化。
pub fn predict_syllable_completions_by_score<F>(
    prefix: &str,
    syllable_set: &HashSet<String>,
    limit: usize,
    mut score_fn: F,
) -> Vec<String>
where
    F: FnMut(&str) -> f64,
{
    if !prefix_is_valid_syllable(prefix, syllable_set) {
        return Vec::new();
    }
    let mut v: Vec<String> = syllable_set
        .iter()
        .filter(|s| s.starts_with(prefix))
        .cloned()
        .collect();
    v.sort_by(|a, b| {
        score_fn(b)
            .partial_cmp(&score_fn(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    v.truncate(limit);
    v
}

pub fn split_syllables_with_tail(
    input: &str,
    syllable_set: &HashSet<String>,
) -> Result<(Vec<String>, Option<String>), String> {
    split_syllables_with_tail_mode(input, syllable_set, ParseOptions::default())
}

pub fn split_syllables_with_tail_mode(
    input: &str,
    syllable_set: &HashSet<String>,
    options: ParseOptions,
) -> Result<(Vec<String>, Option<String>), String> {
    let variants = parse_input_variants(input, syllable_set, options)?;
    variants
        .into_iter()
        .next()
        .ok_or_else(|| "no valid syllable parse".to_string())
}

pub fn parse_input_variants_detailed(
    input: &str,
    syllable_set: &HashSet<String>,
    options: ParseOptions,
) -> Result<Vec<ParseVariant>, String> {
    let normalized = normalize_pinyin_line(input);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err("please input pinyin".into());
    }

    let mut variants: Vec<ParseVariant> = Vec::new();
    let mut base_err = None;

    if options.double_pinyin_enabled {
        if let Ok((syllables, tail)) = split_double_pinyin_with_tail(trimmed, syllable_set) {
            variants.push(ParseVariant {
                syllables,
                tail,
                source: ParseVariantSource::Exact,
                corrected_input: None,
            });
        }
    }

    match split_full_pinyin_with_tail(trimmed, syllable_set) {
        Ok((syllables, tail)) => variants.push(ParseVariant {
            syllables,
            tail,
            source: ParseVariantSource::Exact,
            corrected_input: None,
        }),
        Err(e) => base_err = Some(e),
    }

    dedup_variants(&mut variants);

    if !options.double_pinyin_enabled {
        merge_alternate_continuous_splittings(&mut variants, trimmed, syllable_set);
        dedup_variants(&mut variants);
    }

    let compact_latin_len = compact_latin_input_len(trimmed);
    let (phonetic_max_extra, typo_max_extra, fuzzy_max_extra) =
        variant_limits_for_input_len(compact_latin_len);

    merge_phonetic_confusion_variants(
        &mut variants,
        trimmed,
        syllable_set,
        options.fuzzy_pairs,
        options.correction_enabled,
        phonetic_max_extra,
    );
    dedup_variants(&mut variants);

    merge_input_typo_variants(&mut variants, trimmed, syllable_set, typo_max_extra);

    if options.fuzzy_enabled {
        variants =
            expand_fuzzy_variants(variants, syllable_set, options.fuzzy_pairs, fuzzy_max_extra);
    }

    dedup_variants(&mut variants);

    if variants.is_empty() {
        Err(base_err.unwrap_or_else(|| "no valid syllable parse".to_string()))
    } else {
        Ok(variants)
    }
}

fn compact_latin_input_len(input: &str) -> usize {
    input.chars().filter(|ch| ch.is_ascii_alphabetic()).count()
}

fn variant_limits_for_input_len(input_len: usize) -> (usize, usize, usize) {
    if input_len >= VERY_LONG_VARIANT_REDUCE_INPUT_LEN {
        (
            PHONETIC_CONFUSION_MAX_EXTRA / 3,
            INPUT_TYPO_MAX_EXTRA / 4,
            FUZZY_VARIANT_MAX_EXTRA / 4,
        )
    } else if input_len >= LONG_VARIANT_REDUCE_INPUT_LEN {
        (
            PHONETIC_CONFUSION_MAX_EXTRA / 2,
            INPUT_TYPO_MAX_EXTRA / 2,
            FUZZY_VARIANT_MAX_EXTRA / 2,
        )
    } else {
        (
            PHONETIC_CONFUSION_MAX_EXTRA,
            INPUT_TYPO_MAX_EXTRA,
            FUZZY_VARIANT_MAX_EXTRA,
        )
    }
}

pub fn parse_input_variants(
    input: &str,
    syllable_set: &HashSet<String>,
    options: ParseOptions,
) -> Result<Vec<(Vec<String>, Option<String>)>, String> {
    parse_input_variants_detailed(input, syllable_set, options).map(|variants| {
        variants
            .into_iter()
            .map(|variant| (variant.syllables, variant.tail))
            .collect()
    })
}

/// 音节分隔：撇号、连字符及常见 Unicode 连字符 → 空白。
fn map_syllable_separator_chars(c: char) -> char {
    match c {
        '\'' | '\u{2019}' | '\u{2018}' | '\u{02BC}' => ' ',
        '-' | '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}' => ' ',
        c => c,
    }
}

fn split_full_pinyin_with_tail(
    trimmed: &str,
    syllable_set: &HashSet<String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let normalized: String = trimmed.chars().map(map_syllable_separator_chars).collect();
    let t = normalized.trim();
    if t.chars().any(char::is_whitespace) {
        split_with_whitespace(t, syllable_set)
    } else {
        split_continuous(t, syllable_set)
    }
}

fn split_with_whitespace(
    trimmed: &str,
    syllable_set: &HashSet<String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return Err("no valid syllable".into());
    }
    let n = parts.len();
    let mut out = Vec::new();
    for (idx, w) in parts.iter().enumerate() {
        let t = normalize_token(w);
        if t.is_empty() {
            continue;
        }
        let is_last = idx == n - 1;
        if !is_last {
            if !syllable_set.contains(&t) {
                return Err(format!("unknown syllable: {}", t));
            }
            out.push(t);
        } else if syllable_set.contains(&t) {
            out.push(t);
        } else if prefix_is_valid_syllable(&t, syllable_set) {
            return Ok((out, Some(t)));
        } else {
            return Err(format!("unknown syllable or prefix: {}", t));
        }
    }
    Ok((out, None))
}

fn split_continuous(
    trimmed: &str,
    syllable_set: &HashSet<String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let s = normalize_token(trimmed);
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let mut found = None;
        for len in (1..=6).rev() {
            if i + len > b.len() {
                continue;
            }
            let piece =
                std::str::from_utf8(&b[i..i + len]).map_err(|_| "invalid utf-8".to_string())?;
            if syllable_set.contains(piece) {
                found = Some((piece.to_string(), len));
                break;
            }
        }
        if let Some((sy, len)) = found {
            out.push(sy);
            i += len;
        } else {
            let tail = std::str::from_utf8(&b[i..]).unwrap_or_default();
            if prefix_is_valid_syllable(tail, syllable_set) {
                return Ok((out, Some(tail.to_string())));
            }
            return Err(format!("cannot split at tail: {}", tail));
        }
    }
    Ok((out, None))
}

/// 枚举连续串的所有合法「完整」音节切分（如 `xian` → `xian` 与 `xi+an`），用于消歧常用词（西安等）。
fn enumerate_complete_splittings(
    compact: &str,
    syllable_set: &HashSet<String>,
    max_results: usize,
) -> Vec<Vec<String>> {
    let s = normalize_token(compact);
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return Vec::new();
    }
    let b = s.as_bytes();
    let mut out = Vec::new();

    fn dfs(
        pos: usize,
        b: &[u8],
        syllable_set: &HashSet<String>,
        cur: Vec<String>,
        out: &mut Vec<Vec<String>>,
        max_results: usize,
    ) {
        if out.len() >= max_results || pos > b.len() {
            return;
        }
        if pos == b.len() {
            out.push(cur);
            return;
        }
        let max_len = 6.min(b.len() - pos);
        for len in 1..=max_len {
            let piece = match std::str::from_utf8(&b[pos..pos + len]) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if syllable_set.contains(piece) {
                let mut next = cur.clone();
                next.push(piece.to_string());
                dfs(pos + len, b, syllable_set, next, out, max_results);
                if out.len() >= max_results {
                    return;
                }
            }
        }
    }

    dfs(0, b, syllable_set, Vec::new(), &mut out, max_results);
    out
}

/// 是否整段为「无空白」的连续拉丁拼音（可做多切分枚举）；显式分音节（空格/'/-）时不做枚举。
fn is_continuous_latin_pinyin(trimmed: &str) -> bool {
    let mapped: String = trimmed.chars().map(map_syllable_separator_chars).collect();
    !mapped.is_empty()
        && !mapped.chars().any(|c| c.is_whitespace())
        && trimmed.chars().all(|c| {
            c.is_ascii_alphabetic()
                || matches!(
                    c,
                    '\'' | '-'
                        | '\u{2019}'
                        | '\u{2018}'
                        | '\u{02BC}'
                        | '\u{2010}'
                        | '\u{2011}'
                        | '\u{2013}'
                        | '\u{2014}'
                )
        })
}

fn merge_alternate_continuous_splittings(
    variants: &mut Vec<ParseVariant>,
    trimmed: &str,
    syllable_set: &HashSet<String>,
) {
    if !is_continuous_latin_pinyin(trimmed) {
        return;
    }
    let compact = normalize_token(trimmed);
    if compact.chars().count() > 36 {
        return;
    }
    let alts = enumerate_complete_splittings(&compact, syllable_set, 14);
    let primary = variants.first().and_then(|variant| {
        (variant.tail.is_none() && !variant.syllables.is_empty())
            .then_some(variant.syllables.clone())
    });
    if alts.is_empty() || (alts.len() == 1 && primary.as_ref() == Some(&alts[0])) {
        return;
    }

    for alt in alts {
        if primary.as_ref() == Some(&alt) {
            continue;
        }
        variants.push(ParseVariant {
            syllables: alt,
            tail: None,
            source: ParseVariantSource::AlternateSplit,
            corrected_input: None,
        });
    }
}

/// 在连续全拼上，对「已切分」的音节逐个尝试 zh/z、in/ing 等合法替换，生成高优先级解析变体。
fn merge_phonetic_confusion_variants(
    variants: &mut Vec<ParseVariant>,
    trimmed: &str,
    syllable_set: &HashSet<String>,
    pairs: crate::fuzzy_prefs::FuzzyPairs,
    correction_enabled: bool,
    max_extra: usize,
) {
    if max_extra == 0 || !is_continuous_latin_pinyin(trimmed) {
        return;
    }
    let Some(base) = variants
        .iter()
        .find(|v| {
            v.source == ParseVariantSource::Exact && v.tail.is_none() && !v.syllables.is_empty()
        })
        .or_else(|| variants.first())
        .filter(|v| v.tail.is_none())
        .cloned()
    else {
        return;
    };
    let base_syls = base.syllables;
    if base_syls.is_empty() {
        return;
    }

    let mut seen: BTreeSet<(Vec<String>, Option<String>)> = BTreeSet::new();
    for v in variants.iter() {
        seen.insert((v.syllables.clone(), v.tail.clone()));
    }

    let mut added = 0usize;
    for i in 0..base_syls.len() {
        if added >= max_extra {
            break;
        }

        // fuzzy 驱动的音近对（与旧行为兼容）
        let fuzzy_choices = fuzzy_alternatives(&base_syls[i], syllable_set, pairs);
        for alt in fuzzy_choices {
            if alt == base_syls[i] {
                continue;
            }
            let mut syls = base_syls.clone();
            syls[i] = alt;
            if !seen.insert((syls.clone(), None)) {
                continue;
            }
            let corrected = syls.join("");
            variants.push(ParseVariant {
                syllables: syls,
                tail: None,
                source: ParseVariantSource::SyllableConfusion,
                corrected_input: Some(corrected),
            });
            added += 1;
            if added >= max_extra {
                break;
            }
        }

        // correction 档位驱动的扩展音近对
        if correction_enabled {
            for alt in correction_phonetic_transforms(&base_syls[i], syllable_set) {
                if alt == base_syls[i] {
                    continue;
                }
                let mut syls = base_syls.clone();
                syls[i] = alt;
                if !seen.insert((syls.clone(), None)) {
                    continue;
                }
                let corrected = syls.join("");
                variants.push(ParseVariant {
                    syllables: syls,
                    tail: None,
                    source: ParseVariantSource::PhoneticTypo,
                    corrected_input: Some(corrected),
                });
                added += 1;
                if added >= max_extra {
                    break;
                }
            }
        }
    }
}

fn merge_input_typo_variants(
    variants: &mut Vec<ParseVariant>,
    trimmed: &str,
    syllable_set: &HashSet<String>,
    max_extra: usize,
) {
    if max_extra == 0 || !is_continuous_latin_pinyin(trimmed) {
        return;
    }
    let compact = normalize_token(trimmed);
    if compact.len() < 2 || compact.len() > 24 {
        return;
    }
    let mut seen = BTreeSet::new();
    for candidate in single_edit_input_variants(&compact) {
        if let Ok((syllables, tail)) = split_full_pinyin_with_tail(&candidate, syllable_set) {
            if seen.insert((syllables.clone(), tail.clone())) {
                variants.push(ParseVariant {
                    syllables,
                    tail,
                    source: ParseVariantSource::KeyboardTypo,
                    corrected_input: Some(candidate),
                });
                if seen.len() >= max_extra {
                    break;
                }
            }
        }
    }
}

fn single_edit_input_variants(compact: &str) -> Vec<String> {
    let chars: Vec<char> = compact.chars().collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for idx in 0..chars.len() {
        let mut candidate = chars.clone();
        candidate.remove(idx);
        let text: String = candidate.into_iter().collect();
        if seen.insert(text.clone()) {
            out.push(text);
        }
    }

    for idx in 0..=chars.len() {
        for alt in typo_insertion_chars() {
            let mut candidate = chars.clone();
            candidate.insert(idx, *alt);
            let text: String = candidate.into_iter().collect();
            if seen.insert(text.clone()) {
                out.push(text);
            }
        }
    }

    for idx in 0..chars.len() {
        for alt in keyboard_neighbor_chars(chars[idx]) {
            if *alt == chars[idx] {
                continue;
            }
            let mut candidate = chars.clone();
            candidate[idx] = *alt;
            let text: String = candidate.into_iter().collect();
            if seen.insert(text.clone()) {
                out.push(text);
            }
        }
    }

    for idx in 0..chars.len().saturating_sub(1) {
        if chars[idx] == chars[idx + 1] {
            continue;
        }
        let mut candidate = chars.clone();
        candidate.swap(idx, idx + 1);
        let text: String = candidate.into_iter().collect();
        if seen.insert(text.clone()) {
            out.push(text);
        }
    }

    out
}

fn keyboard_neighbor_chars(ch: char) -> &'static [char] {
    match ch {
        'a' => &['q', 'w', 's', 'z'],
        'b' => &['v', 'g', 'h', 'n'],
        'c' => &['x', 'd', 'f', 'v'],
        'd' => &['w', 'e', 's', 'x', 'c', 'f'],
        'e' => &['w', 'r', 's', 'd', 'f'],
        'f' => &['d', 'r', 't', 'g', 'v', 'c'],
        'g' => &['f', 't', 'y', 'h', 'b', 'v'],
        'h' => &['g', 'y', 'u', 'j', 'n', 'b'],
        'i' => &['u', 'o', 'j', 'k', 'l'],
        'j' => &['h', 'u', 'i', 'k', 'm', 'n'],
        'k' => &['j', 'i', 'o', 'l', 'm'],
        'l' => &['k', 'o', 'p'],
        'm' => &['n', 'j', 'k'],
        'n' => &['b', 'h', 'j', 'm'],
        'o' => &['i', 'p', 'k', 'l'],
        'p' => &['o', 'l'],
        'q' => &['w', 'a'],
        'r' => &['e', 't', 'd', 'f'],
        's' => &['a', 'w', 'e', 'd', 'x', 'z'],
        't' => &['r', 'y', 'f', 'g'],
        'u' => &['y', 'i', 'h', 'j', 'k'],
        'v' => &['c', 'f', 'g', 'b'],
        'w' => &['q', 'e', 'a', 's', 'd'],
        'x' => &['z', 's', 'd', 'c'],
        'y' => &['t', 'u', 'g', 'h', 'j'],
        'z' => &['a', 's', 'x'],
        _ => &[],
    }
}

fn typo_insertion_chars() -> &'static [char] {
    &[
        'a', 'e', 'i', 'o', 'u', 'n', 'g', 'h', 'r', 'y', 'w', 'm', 'l', 'b', 'p', 'f', 'd', 't',
        'k', 'j', 'q', 'x', 'z', 'c', 's', 'v',
    ]
}

/// 预编辑内 Ctrl+左右：音节边界（字符索引，非 UTF-16）。
pub fn syllable_boundary_char_indices(reading: &str, syllable_set: &HashSet<String>) -> Vec<usize> {
    let normalized = normalize_pinyin_line(reading);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return vec![0];
    }
    let Ok((syls, tail)) = split_full_pinyin_with_tail(trimmed, syllable_set) else {
        let end = trimmed.chars().count();
        return vec![0, end];
    };

    let chars: Vec<char> = trimmed.chars().collect();
    let mut boundaries = vec![0usize];
    let mut ci = 0usize;

    for syl in &syls {
        let need = syl.chars().count();
        for _ in 0..need {
            while ci < chars.len() && !chars[ci].is_ascii_alphabetic() {
                ci += 1;
            }
            if ci < chars.len() {
                ci += 1;
            }
        }
        boundaries.push(ci.min(chars.len()));
    }

    if tail.is_some() {
        boundaries.push(chars.len());
    } else if boundaries.last().copied() != Some(chars.len()) {
        boundaries.push(chars.len());
    }

    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

/// TSF 侧 UTF-16 码元偏移（BMP 拼音安全；与 `normalize_pinyin_line` 后的 trim 对齐）。
pub fn syllable_boundary_offsets_utf16(reading: &str, syllable_set: &HashSet<String>) -> Vec<u32> {
    let normalized = normalize_pinyin_line(reading);
    let trimmed = normalized.trim();
    syllable_boundary_char_indices(reading, syllable_set)
        .into_iter()
        .map(|idx| {
            trimmed
                .chars()
                .take(idx)
                .map(|c| c.len_utf16() as u32)
                .sum()
        })
        .collect()
}

fn split_double_pinyin_with_tail(
    trimmed: &str,
    syllable_set: &HashSet<String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let compact: Vec<char> = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    if compact.is_empty() {
        return Err("double pinyin expects ascii letters".into());
    }

    let mut out = Vec::new();
    let mut i = 0usize;
    while i < compact.len() {
        let remain = compact.len() - i;
        if remain == 1 {
            return Ok((out, Some(compact[i].to_string())));
        }
        let a = compact[i];
        let b = compact[i + 1];
        let Some(syllable) = decode_double_pinyin(a, b) else {
            return Err(format!("unknown double-pinyin code: {}{}", a, b));
        };
        if !syllable_set.contains(&syllable) {
            return Err(format!(
                "double-pinyin resolved to invalid syllable: {}",
                syllable
            ));
        }
        out.push(syllable);
        i += 2;
    }
    Ok((out, None))
}

fn decode_double_pinyin(initial_key: char, final_key: char) -> Option<String> {
    let initial = decode_double_initial(initial_key)?;
    let final_part = decode_double_final(final_key)?;
    let syllable = combine_initial_final(initial, final_part)?;
    Some(syllable)
}

fn decode_double_initial(key: char) -> Option<&'static str> {
    Some(match key {
        'o' => "",
        'b' => "b",
        'p' => "p",
        'm' => "m",
        'f' => "f",
        'd' => "d",
        't' => "t",
        'n' => "n",
        'l' => "l",
        'g' => "g",
        'k' => "k",
        'h' => "h",
        'j' => "j",
        'q' => "q",
        'x' => "x",
        'r' => "r",
        'z' => "z",
        'c' => "c",
        's' => "s",
        'y' => "y",
        'w' => "w",
        'v' => "zh",
        'i' => "ch",
        'u' => "sh",
        _ => return None,
    })
}

fn decode_double_final(key: char) -> Option<&'static str> {
    Some(match key {
        'a' => "a",
        'b' => "in",
        'c' => "ao",
        'd' => "ai",
        'e' => "e",
        'f' => "en",
        'g' => "eng",
        'h' => "ang",
        'i' => "i",
        'j' => "an",
        'k' => "ing",
        'l' => "iang",
        'm' => "ian",
        'n' => "iao",
        'o' => "uo",
        'p' => "ie",
        'q' => "iu",
        'r' => "uan",
        's' => "ong",
        't' => "ue",
        'w' => "ei",
        'x' => "ia",
        'y' => "un",
        'z' => "ou",
        _ => return None,
    })
}

fn combine_initial_final(initial: &str, final_part: &str) -> Option<String> {
    if initial.is_empty() {
        return Some(match final_part {
            "uo" => "wo".to_string(),
            "ue" => "yue".to_string(),
            "iu" => "you".to_string(),
            "un" => "wen".to_string(),
            "ia" => "ya".to_string(),
            "iang" => "yang".to_string(),
            "ian" => "yan".to_string(),
            "iao" => "yao".to_string(),
            "ie" => "ye".to_string(),
            "ing" => "ying".to_string(),
            "in" => "yin".to_string(),
            "ong" => "yong".to_string(),
            other => other.to_string(),
        });
    }

    let final_part = match (initial, final_part) {
        ("j" | "q" | "x", "ue") => "ue",
        ("j" | "q" | "x" | "y", "un") => "un",
        ("j" | "q" | "x" | "y", "ia") => "u",
        _ => final_part,
    };

    let mut out = String::from(initial);
    out.push_str(final_part);
    Some(out)
}

fn expand_fuzzy_variants(
    variants: Vec<ParseVariant>,
    syllable_set: &HashSet<String>,
    pairs: crate::fuzzy_prefs::FuzzyPairs,
    max_variants: usize,
) -> Vec<ParseVariant> {
    let mut out = Vec::new();
    for variant in variants {
        if variant.tail.is_some() {
            out.push(variant);
            continue;
        }

        let mut acc: Vec<(Vec<String>, bool)> = vec![(Vec::new(), false)];
        for syl in &variant.syllables {
            let choices = fuzzy_alternatives(syl, syllable_set, pairs);
            let mut next = Vec::new();
            for (prefix, used_fuzzy) in &acc {
                for choice in &choices {
                    let mut v = prefix.clone();
                    v.push(choice.clone());
                    next.push((v, *used_fuzzy || choice != syl));
                    if next.len() >= max_variants {
                        break;
                    }
                }
                if next.len() >= max_variants {
                    break;
                }
            }
            acc = next;
            if acc.is_empty() {
                break;
            }
        }
        for (seq, used_fuzzy) in acc {
            let source = if variant.source == ParseVariantSource::KeyboardTypo {
                ParseVariantSource::KeyboardTypo
            } else if variant.source == ParseVariantSource::SyllableConfusion {
                ParseVariantSource::SyllableConfusion
            } else if variant.source == ParseVariantSource::PhoneticTypo {
                ParseVariantSource::PhoneticTypo
            } else if used_fuzzy {
                ParseVariantSource::Fuzzy
            } else {
                variant.source
            };
            out.push(ParseVariant {
                syllables: seq,
                tail: None,
                source,
                corrected_input: variant.corrected_input.clone(),
            });
            if out.len() >= max_variants {
                break;
            }
        }
        if out.len() >= max_variants {
            break;
        }
    }
    dedup_variants(&mut out);
    out
}

fn fuzzy_alternatives(
    syllable: &str,
    syllable_set: &HashSet<String>,
    pairs: crate::fuzzy_prefs::FuzzyPairs,
) -> Vec<String> {
    let mut set = BTreeSet::new();
    set.insert(syllable.to_string());

    for alt in fuzzy_transforms(syllable, pairs) {
        if syllable_set.contains(&alt) {
            set.insert(alt);
        }
    }
    set.into_iter().collect()
}

fn fuzzy_transforms(syllable: &str, pairs: crate::fuzzy_prefs::FuzzyPairs) -> Vec<String> {
    let mut out = Vec::new();

    let mut initial_swaps = Vec::new();
    if pairs.zh_z {
        initial_swaps.extend([("zh", "z"), ("z", "zh")]);
    }
    if pairs.ch_c {
        initial_swaps.extend([("ch", "c"), ("c", "ch")]);
    }
    if pairs.sh_s {
        initial_swaps.extend([("sh", "s"), ("s", "sh")]);
    }
    if pairs.n_l {
        initial_swaps.extend([("l", "n"), ("n", "l")]);
    }
    if pairs.f_h {
        initial_swaps.extend([("f", "h"), ("h", "f")]);
    }
    for (from, to) in initial_swaps {
        if let Some(rest) = syllable.strip_prefix(from) {
            out.push(format!("{}{}", to, rest));
        }
    }

    let mut final_swaps = Vec::new();
    if pairs.an_ang {
        final_swaps.extend([("ang", "an"), ("an", "ang")]);
    }
    if pairs.en_eng {
        final_swaps.extend([("eng", "en"), ("en", "eng")]);
    }
    if pairs.in_ing {
        final_swaps.extend([("ing", "in"), ("in", "ing")]);
    }
    for (from, to) in final_swaps {
        if let Some(rest) = syllable.strip_suffix(from) {
            out.push(format!("{}{}", rest, to));
        }
    }

    out
}

/// 纠错档位驱动的扩展音节级音近变换。
/// 与 fuzzy 音近分开，避免在 fuzzy 关闭时丢失这些常见拼音纠错能力。
fn correction_phonetic_transforms(syllable: &str, syllable_set: &HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();

    let initial_swaps: &[(&str, &str)] = &[("r", "l"), ("l", "r")];
    for (from, to) in initial_swaps {
        if let Some(rest) = syllable.strip_prefix(from) {
            let alt = format!("{}{}", to, rest);
            if syllable_set.contains(&alt) {
                out.push(alt);
            }
        }
    }

    // 注意顺序：先匹配较长的 iang/uang，再匹配较短的 ian/uan，避免重复。
    let final_swaps: &[(&str, &str)] = &[
        ("ian", "iang"),
        ("iang", "ian"),
        ("uan", "uang"),
        ("uang", "uan"),
        ("un", "ong"),
        ("ong", "un"),
    ];
    for (from, to) in final_swaps {
        if let Some(rest) = syllable.strip_suffix(from) {
            let alt = format!("{}{}", rest, to);
            if syllable_set.contains(&alt) {
                out.push(alt);
            }
        }
    }

    out
}

fn dedup_variants(variants: &mut Vec<ParseVariant>) {
    let mut seen = BTreeSet::new();
    variants.retain(|item| seen.insert((item.syllables.clone(), item.tail.clone())));
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MixedPinyinSegment {
    Full(String),
    Prefix(String),
    Initial(char),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MixedPinyinKeyExpansion {
    pub key: String,
    pub syllables: Vec<String>,
    pub segment_count: usize,
    pub full_segments: usize,
    pub prefix_segments: usize,
    pub initial_segments: usize,
    pub leading_full: bool,
    pub substantive_full_segments: usize,
    pub path_score: f64,
}

const MIXED_PINYIN_MAX_SEGMENTS: usize = 10;
const MIXED_PINYIN_INITIAL_CHOICES: usize = 32;
const MIXED_PINYIN_INTERNAL_BEAM: usize = 16;
const MIXED_PINYIN_LEADING_INITIAL_BEAM: usize = 32;
const MIXED_PINYIN_PATTERN_LIMIT: usize = 12;
const MIXED_PINYIN_SEGMENT_SLACK: usize = 1;

#[derive(Clone, Debug)]
struct IncrementalMixedPath {
    segments: Vec<MixedPinyinSegment>,
    full_count: usize,
    prefix_count: usize,
    initial_count: usize,
}

/// Retains mixed-pinyin segmentation states across adjacent edits.
///
/// Typing normally extends or shortens the previous ASCII reading by one
/// character. States before the common prefix remain valid, so only the new
/// suffix needs to be segmented.
#[derive(Debug)]
pub struct MixedPinyinIncrementalCache {
    compact: String,
    states: Vec<Vec<IncrementalMixedPath>>,
}

impl Default for MixedPinyinIncrementalCache {
    fn default() -> Self {
        Self {
            compact: String::new(),
            states: vec![vec![IncrementalMixedPath {
                segments: Vec::new(),
                full_count: 0,
                prefix_count: 0,
                initial_count: 0,
            }]],
        }
    }
}

impl MixedPinyinIncrementalCache {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn expand_by_score<F>(
        &mut self,
        input: &str,
        syllable_set: &HashSet<String>,
        max_keys: usize,
        score_fn: F,
    ) -> Vec<MixedPinyinKeyExpansion>
    where
        F: FnMut(&str, &str) -> f64,
    {
        self.expand_by_score_until(input, syllable_set, max_keys, score_fn, || false)
            .0
    }

    /// Budget-aware mixed-pinyin expansion. The boolean result reports
    /// whether all patterns completed; callers must not cache partial output.
    pub fn expand_by_score_until<F, S>(
        &mut self,
        input: &str,
        syllable_set: &HashSet<String>,
        max_keys: usize,
        mut score_fn: F,
        mut should_stop: S,
    ) -> (Vec<MixedPinyinKeyExpansion>, bool)
    where
        F: FnMut(&str, &str) -> f64,
        S: FnMut() -> bool,
    {
        if max_keys == 0 {
            return (Vec::new(), true);
        }
        let compact = normalize_token(input);
        if compact.is_empty() {
            return (Vec::new(), true);
        }
        if !compact.is_ascii() {
            let expanded = expand_mixed_pinyin_exact_key_details_by_score(
                &compact,
                syllable_set,
                max_keys,
                score_fn,
            );
            return (expanded, !should_stop());
        }

        self.retain_common_prefix(&compact);
        self.extend_to(&compact, syllable_set);
        if should_stop() {
            return (Vec::new(), false);
        }

        let Some(paths) = self.states.get(compact.len()) else {
            return (Vec::new(), true);
        };
        let Some(min_segments) = paths
            .iter()
            .filter(|path| path.full_count > 0 && path.initial_count + path.prefix_count > 0)
            .map(|path| path.segments.len())
            .min()
        else {
            return (Vec::new(), true);
        };

        let mut patterns = paths
            .iter()
            .filter(|path| {
                path.full_count > 0
                    && path.initial_count + path.prefix_count > 0
                    && path.segments.len() <= min_segments + MIXED_PINYIN_SEGMENT_SLACK
            })
            .map(|path| path.segments.clone())
            .collect::<Vec<_>>();
        patterns.sort_by(|left, right| compare_mixed_patterns(left, right));
        patterns.truncate(MIXED_PINYIN_PATTERN_LIMIT);

        let mut expanded = Vec::new();
        for pattern in patterns {
            if should_stop() {
                return (finalize_mixed_expansions(expanded, max_keys), false);
            }
            let mut pattern_expanded = Vec::new();
            let completed = expand_mixed_pattern_keys_until(
                &pattern,
                syllable_set,
                max_keys,
                &mut pattern_expanded,
                &mut score_fn,
                &mut should_stop,
            );
            expanded.extend(pattern_expanded);
            if !completed {
                return (finalize_mixed_expansions(expanded, max_keys), false);
            }
        }
        (finalize_mixed_expansions(expanded, max_keys), true)
    }

    fn retain_common_prefix(&mut self, next: &str) {
        let common = self
            .compact
            .as_bytes()
            .iter()
            .zip(next.as_bytes())
            .take_while(|(left, right)| left == right)
            .count();
        self.compact.truncate(common);
        self.states.truncate(common.saturating_add(1));
        if self.states.is_empty() {
            *self = Self::default();
        }
    }

    fn extend_to(&mut self, next: &str, syllable_set: &HashSet<String>) {
        for end in (self.compact.len() + 1)..=next.len() {
            let mut paths = Vec::new();
            let mut seen = BTreeSet::new();

            for len in (1..=6).rev() {
                if len > end {
                    continue;
                }
                let start = end - len;
                let piece = &next[start..end];
                if !syllable_set.contains(piece) {
                    continue;
                }
                for base in self.states.get(start).into_iter().flatten() {
                    if base.segments.len() >= MIXED_PINYIN_MAX_SEGMENTS {
                        continue;
                    }
                    let mut path = base.clone();
                    path.segments
                        .push(MixedPinyinSegment::Full(piece.to_string()));
                    path.full_count += 1;
                    if seen.insert(path.segments.clone()) {
                        paths.push(path);
                    }
                }
            }

            for len in (2..=5).rev() {
                if len > end {
                    continue;
                }
                let start = end - len;
                let piece = &next[start..end];
                if !is_strict_syllable_prefix(piece, syllable_set) {
                    continue;
                }
                for base in self.states.get(start).into_iter().flatten() {
                    if base.segments.len() >= MIXED_PINYIN_MAX_SEGMENTS {
                        continue;
                    }
                    let mut path = base.clone();
                    path.segments
                        .push(MixedPinyinSegment::Prefix(piece.to_string()));
                    path.prefix_count += 1;
                    if seen.insert(path.segments.clone()) {
                        paths.push(path);
                    }
                }
            }

            let start = end - 1;
            let ch = next.as_bytes()[start] as char;
            let prefix = ch.to_string();
            if ch.is_ascii_lowercase() && prefix_is_valid_syllable(&prefix, syllable_set) {
                for base in self.states.get(start).into_iter().flatten() {
                    if base.segments.len() >= MIXED_PINYIN_MAX_SEGMENTS {
                        continue;
                    }
                    let mut path = base.clone();
                    path.segments.push(MixedPinyinSegment::Initial(ch));
                    path.initial_count += 1;
                    if seen.insert(path.segments.clone()) {
                        paths.push(path);
                    }
                }
            }

            self.states.push(paths);
        }
        self.compact.clear();
        self.compact.push_str(next);
    }
}

fn compare_mixed_patterns(
    left: &[MixedPinyinSegment],
    right: &[MixedPinyinSegment],
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    for (left_segment, right_segment) in left.iter().zip(right) {
        let ordering = match (left_segment, right_segment) {
            (MixedPinyinSegment::Full(left), MixedPinyinSegment::Full(right)) => {
                right.len().cmp(&left.len()).then_with(|| left.cmp(right))
            }
            (MixedPinyinSegment::Full(_), MixedPinyinSegment::Initial(_)) => Ordering::Less,
            (MixedPinyinSegment::Initial(_), MixedPinyinSegment::Full(_)) => Ordering::Greater,
            (MixedPinyinSegment::Full(_), MixedPinyinSegment::Prefix(_)) => Ordering::Less,
            (MixedPinyinSegment::Prefix(_), MixedPinyinSegment::Full(_)) => Ordering::Greater,
            (MixedPinyinSegment::Prefix(left), MixedPinyinSegment::Prefix(right)) => {
                right.len().cmp(&left.len()).then_with(|| left.cmp(right))
            }
            (MixedPinyinSegment::Prefix(_), MixedPinyinSegment::Initial(_)) => Ordering::Less,
            (MixedPinyinSegment::Initial(_), MixedPinyinSegment::Prefix(_)) => Ordering::Greater,
            (MixedPinyinSegment::Initial(left), MixedPinyinSegment::Initial(right)) => {
                left.cmp(right)
            }
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

pub fn expand_mixed_pinyin_exact_keys(
    input: &str,
    syllable_set: &HashSet<String>,
    max_keys: usize,
) -> Vec<(String, usize)> {
    expand_mixed_pinyin_exact_key_details(input, syllable_set, max_keys)
        .into_iter()
        .map(|item| (item.key, item.segment_count))
        .collect()
}

pub fn expand_mixed_pinyin_exact_key_details(
    input: &str,
    syllable_set: &HashSet<String>,
    max_keys: usize,
) -> Vec<MixedPinyinKeyExpansion> {
    expand_mixed_pinyin_exact_key_details_by_score(input, syllable_set, max_keys, |_, _| 0.0)
}

pub fn expand_mixed_pinyin_exact_key_details_by_score<F>(
    input: &str,
    syllable_set: &HashSet<String>,
    max_keys: usize,
    mut score_fn: F,
) -> Vec<MixedPinyinKeyExpansion>
where
    F: FnMut(&str, &str) -> f64,
{
    if max_keys == 0 {
        return Vec::new();
    }

    let compact = normalize_token(input);
    if compact.is_empty() {
        return Vec::new();
    }

    let mut patterns = Vec::new();
    let mut current = Vec::new();
    collect_mixed_pinyin_patterns(
        &compact,
        syllable_set,
        0,
        0,
        0,
        0,
        &mut current,
        &mut patterns,
    );
    if patterns.is_empty() {
        return Vec::new();
    }

    let Some(min_segments) = patterns.iter().map(Vec::len).min() else {
        return Vec::new();
    };

    let mut patterns = patterns
        .into_iter()
        .filter(|pattern| pattern.len() <= min_segments + MIXED_PINYIN_SEGMENT_SLACK)
        .collect::<Vec<_>>();
    patterns.sort_by(|left, right| compare_mixed_patterns(left, right));
    patterns.truncate(MIXED_PINYIN_PATTERN_LIMIT);

    let mut expanded = Vec::new();
    for pattern in patterns {
        let mut pattern_expanded = Vec::new();
        expand_mixed_pattern_keys(
            &pattern,
            syllable_set,
            max_keys,
            &mut pattern_expanded,
            &mut score_fn,
        );
        expanded.extend(pattern_expanded);
    }

    finalize_mixed_expansions(expanded, max_keys)
}

fn finalize_mixed_expansions(
    mut expanded: Vec<MixedPinyinKeyExpansion>,
    max_keys: usize,
) -> Vec<MixedPinyinKeyExpansion> {
    expanded.sort_by(|left, right| {
        right
            .path_score
            .total_cmp(&left.path_score)
            // The same expanded key can come from `suan` or `su` + `an`.
            // Keep the more compact syllable path so downstream word-graph
            // decoding sees the intended word boundaries.
            .then_with(|| {
                let left_compact_segments = (left.key.len() >= 10)
                    .then_some(left.segment_count)
                    .unwrap_or(0);
                let right_compact_segments = (right.key.len() >= 10)
                    .then_some(right.segment_count)
                    .unwrap_or(0);
                left_compact_segments.cmp(&right_compact_segments)
            })
            .then_with(|| {
                right
                    .substantive_full_segments
                    .cmp(&left.substantive_full_segments)
            })
            .then_with(|| left.initial_segments.cmp(&right.initial_segments))
            .then_with(|| left.prefix_segments.cmp(&right.prefix_segments))
            .then_with(|| left.key.cmp(&right.key))
    });
    let mut seen = BTreeSet::new();
    expanded.retain(|item| seen.insert(item.key.clone()));
    expanded.truncate(max_keys);
    expanded
}

fn collect_mixed_pinyin_patterns(
    compact: &str,
    syllable_set: &HashSet<String>,
    offset: usize,
    full_count: usize,
    prefix_count: usize,
    initial_count: usize,
    current: &mut Vec<MixedPinyinSegment>,
    out: &mut Vec<Vec<MixedPinyinSegment>>,
) {
    if current.len() > MIXED_PINYIN_MAX_SEGMENTS {
        return;
    }
    if offset == compact.len() {
        if full_count > 0 && initial_count + prefix_count > 0 {
            out.push(current.clone());
        }
        return;
    }

    for len in (1..=6).rev() {
        if offset + len > compact.len() {
            continue;
        }
        let piece = &compact[offset..offset + len];
        if !syllable_set.contains(piece) {
            continue;
        }
        current.push(MixedPinyinSegment::Full(piece.to_string()));
        collect_mixed_pinyin_patterns(
            compact,
            syllable_set,
            offset + len,
            full_count + 1,
            prefix_count,
            initial_count,
            current,
            out,
        );
        current.pop();
    }

    for len in (2..=5).rev() {
        if offset + len > compact.len() {
            continue;
        }
        let piece = &compact[offset..offset + len];
        if !is_strict_syllable_prefix(piece, syllable_set) {
            continue;
        }
        current.push(MixedPinyinSegment::Prefix(piece.to_string()));
        collect_mixed_pinyin_patterns(
            compact,
            syllable_set,
            offset + len,
            full_count,
            prefix_count + 1,
            initial_count,
            current,
            out,
        );
        current.pop();
    }

    let Some(ch) = compact[offset..].chars().next() else {
        return;
    };
    if !ch.is_ascii_lowercase() {
        return;
    }

    let prefix = ch.to_string();
    if !prefix_is_valid_syllable(&prefix, syllable_set) {
        return;
    }

    current.push(MixedPinyinSegment::Initial(ch));
    collect_mixed_pinyin_patterns(
        compact,
        syllable_set,
        offset + ch.len_utf8(),
        full_count,
        prefix_count,
        initial_count + 1,
        current,
        out,
    );
    current.pop();
}

fn is_strict_syllable_prefix(prefix: &str, syllable_set: &HashSet<String>) -> bool {
    prefix.len() >= 2
        && syllable_set
            .iter()
            .any(|syllable| syllable.len() > prefix.len() && syllable.starts_with(prefix))
}

fn expand_mixed_pattern_keys(
    pattern: &[MixedPinyinSegment],
    syllable_set: &HashSet<String>,
    max_keys: usize,
    out: &mut Vec<MixedPinyinKeyExpansion>,
    score_fn: &mut impl FnMut(&str, &str) -> f64,
) {
    let _ = expand_mixed_pattern_keys_until(
        pattern,
        syllable_set,
        max_keys,
        out,
        score_fn,
        &mut || false,
    );
}

fn expand_mixed_pattern_keys_until(
    pattern: &[MixedPinyinSegment],
    syllable_set: &HashSet<String>,
    max_keys: usize,
    out: &mut Vec<MixedPinyinKeyExpansion>,
    score_fn: &mut impl FnMut(&str, &str) -> f64,
    should_stop: &mut impl FnMut() -> bool,
) -> bool {
    let full_segments = pattern
        .iter()
        .filter(|segment| matches!(segment, MixedPinyinSegment::Full(_)))
        .count();
    let prefix_segments = pattern
        .iter()
        .filter(|segment| matches!(segment, MixedPinyinSegment::Prefix(_)))
        .count();
    let initial_segments = pattern
        .iter()
        .filter(|segment| matches!(segment, MixedPinyinSegment::Initial(_)))
        .count();
    let leading_full = matches!(
        pattern.first(),
        Some(MixedPinyinSegment::Full(syllable)) if syllable.len() > 1
    );
    let substantive_full_segments = pattern
        .iter()
        .filter(|segment| {
            matches!(
                segment,
                MixedPinyinSegment::Full(syllable) if syllable.len() > 1
            )
        })
        .count();
    let mut partial = vec![MixedPinyinPartial {
        key: String::new(),
        syllables: Vec::new(),
        score: 0.0,
    }];
    let beam_width = max_keys.min(MIXED_PINYIN_INTERNAL_BEAM).max(1);
    for segment in pattern {
        if should_stop() {
            return false;
        }
        match segment {
            MixedPinyinSegment::Full(syllable) => {
                for item in &mut partial {
                    item.score = score_fn(&item.key, syllable);
                    item.key.push_str(syllable);
                    item.syllables.push(syllable.clone());
                }
            }
            MixedPinyinSegment::Initial(ch) => {
                let prefix = ch.to_string();
                if !expand_mixed_partial_beam_until(
                    &mut partial,
                    &prefix,
                    syllable_set,
                    beam_width,
                    score_fn,
                    should_stop,
                ) {
                    return false;
                }
                if partial.is_empty() {
                    return true;
                }
            }
            MixedPinyinSegment::Prefix(prefix) => {
                if !expand_mixed_partial_beam_until(
                    &mut partial,
                    prefix,
                    syllable_set,
                    beam_width,
                    score_fn,
                    should_stop,
                ) {
                    return false;
                }
                if partial.is_empty() {
                    return true;
                }
            }
        }
    }

    partial.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.key.cmp(&right.key))
    });
    for item in partial {
        if out.len() >= max_keys {
            break;
        }
        if out.iter().any(|seen| seen.key == item.key) {
            continue;
        }
        out.push(MixedPinyinKeyExpansion {
            key: item.key,
            syllables: item.syllables,
            segment_count: pattern.len(),
            full_segments,
            prefix_segments,
            initial_segments,
            leading_full,
            substantive_full_segments,
            path_score: item.score,
        });
    }
    true
}

fn expand_mixed_partial_beam_until(
    partial: &mut Vec<MixedPinyinPartial>,
    prefix: &str,
    syllable_set: &HashSet<String>,
    beam_width: usize,
    score_fn: &mut impl FnMut(&str, &str) -> f64,
    should_stop: &mut impl FnMut() -> bool,
) -> bool {
    let stage_beam = if prefix.len() == 1
        && partial.len() == 1
        && partial.first().is_some_and(|item| item.key.is_empty())
    {
        beam_width.max(MIXED_PINYIN_LEADING_INITIAL_BEAM)
    } else {
        beam_width
    };
    let mut next = Vec::new();
    for base in partial.iter() {
        if should_stop() {
            return false;
        }
        let completions = predict_syllable_completions_by_score(
            prefix,
            syllable_set,
            MIXED_PINYIN_INITIAL_CHOICES,
            |syllable| score_fn(&base.key, syllable),
        );
        if completions.is_empty() {
            continue;
        }
        for completion in &completions {
            if should_stop() {
                return false;
            }
            let mut candidate = base.clone();
            candidate.score = score_fn(&base.key, completion);
            candidate.key.push_str(completion);
            candidate.syllables.push(completion.clone());
            next.push(candidate);
        }
    }
    next.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.key.cmp(&right.key))
    });
    let mut seen = BTreeSet::new();
    next.retain(|item| seen.insert(item.key.clone()));
    next.truncate(stage_beam);
    *partial = next;
    true
}

#[derive(Clone, Debug)]
struct MixedPinyinPartial {
    key: String,
    syllables: Vec<String>,
    score: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_nue_lue_aliases_to_v_form() {
        assert_eq!(normalize_token("nue"), "nve");
        assert_eq!(normalize_token("lue"), "lve");
        assert_eq!(normalize_token("NUE5"), "nve");
        assert_eq!(normalize_token("LÜE"), "lve");
    }

    fn tiny_set() -> HashSet<String> {
        [
            "hao", "hai", "han", "hang", "wo", "ai", "bei", "jing", "ha", "shi", "zhong", "an",
            "ying", "yao",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    #[test]
    fn h_prefix_continuous() {
        let set = tiny_set();
        let (s, t) = split_syllables_with_tail("h", &set).unwrap();
        assert!(s.is_empty());
        assert_eq!(t.as_deref(), Some("h"));
        let pred = predict_syllable_completions("h", &set, 20);
        assert!(pred.contains(&"hao".into()));
        assert!(pred.contains(&"hai".into()));
    }

    #[test]
    fn wo_then_h() {
        let set = tiny_set();
        let (s, t) = split_syllables_with_tail("woh", &set).unwrap();
        assert_eq!(s, vec!["wo".to_string()]);
        assert_eq!(t.as_deref(), Some("h"));
    }

    #[test]
    fn complete_no_tail() {
        let set = tiny_set();
        let (s, t) = split_syllables_with_tail("hao", &set).unwrap();
        assert_eq!(s, vec!["hao".to_string()]);
        assert!(t.is_none());
    }

    #[test]
    fn fuzzy_variants_include_swaps() {
        let set = tiny_set();
        let vars = parse_input_variants(
            "han",
            &set,
            ParseOptions {
                fuzzy_enabled: true,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars.iter().any(|(s, _)| s == &vec!["hang".to_string()]));
    }

    #[test]
    fn fuzzy_variants_respect_pair_switches() {
        let mut set = tiny_set();
        set.insert("zong".to_string());
        set.insert("fa".to_string());

        let pairs = crate::fuzzy_prefs::FuzzyPairs {
            f_h: false,
            ..Default::default()
        };
        let vars = parse_input_variants(
            "fa",
            &set,
            ParseOptions {
                fuzzy_enabled: true,
                fuzzy_pairs: pairs,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(!vars.iter().any(|(s, _)| s == &vec!["ha".to_string()]));

        let vars = parse_input_variants(
            "fa",
            &set,
            ParseOptions {
                fuzzy_enabled: true,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars.iter().any(|(s, _)| s == &vec!["ha".to_string()]));
    }

    #[test]
    fn syllable_confusion_emits_hang_without_fuzzy_mode() {
        let set = tiny_set();
        let vars = parse_input_variants_detailed(
            "han",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars.iter().any(|v| {
            v.source == ParseVariantSource::SyllableConfusion
                && v.syllables == ["hang".to_string()]
                && v.tail.is_none()
                && v.corrected_input.as_deref() == Some("hang")
        }));
    }

    #[test]
    fn double_pinyin_basic() {
        let set = tiny_set();
        let (s, t) = split_syllables_with_tail_mode(
            "ui",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: true,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert_eq!(s, vec!["shi".to_string()]);
        assert!(t.is_none());
    }

    #[test]
    fn apostrophe_syllable_boundary() {
        let mut set = std::collections::HashSet::new();
        for syl in ["xi", "an", "hao"] {
            set.insert(syl.to_string());
        }
        let (s, t) = split_syllables_with_tail("xi'an", &set).unwrap();
        assert_eq!(s, vec!["xi".to_string(), "an".to_string()]);
        assert!(t.is_none());
    }

    #[test]
    fn hyphen_splits_like_apostrophe() {
        let mut set = std::collections::HashSet::new();
        for syl in ["xi", "an"] {
            set.insert(syl.to_string());
        }
        let (s, t) = split_syllables_with_tail("xi-an", &set).unwrap();
        assert_eq!(s, vec!["xi".to_string(), "an".to_string()]);
        assert!(t.is_none());
    }

    #[test]
    fn xian_emits_xi_an_alternate() {
        let mut set = tiny_set();
        for syl in ["xi", "an", "xian"] {
            set.insert(syl.to_string());
        }
        let vars = parse_input_variants(
            "xian",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars
            .iter()
            .any(|(v, t)| { t.is_none() && v == &vec!["xian".to_string()] }));
        assert!(vars
            .iter()
            .any(|(v, t)| { t.is_none() && v == &vec!["xi".to_string(), "an".to_string()] }));
    }

    #[test]
    fn greedy_failed_continuous_input_keeps_single_valid_alternate_split() {
        let mut set = tiny_set();
        for syl in ["ang", "gui"] {
            set.insert(syl.to_string());
        }
        let vars = parse_input_variants_detailed(
            "angui",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars.iter().any(|variant| {
            variant.source == ParseVariantSource::AlternateSplit
                && variant.tail.is_none()
                && variant.syllables == ["an".to_string(), "gui".to_string()]
        }));
    }

    #[test]
    fn fuzzy_input_typo_variants_include_neighbor_fix() {
        let mut set = tiny_set();
        set.insert("jing".to_string());
        let vars = parse_input_variants(
            "jimg",
            &set,
            ParseOptions {
                fuzzy_enabled: true,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars
            .iter()
            .any(|(v, t)| t.is_none() && v == &vec!["jing".to_string()]));
    }

    #[test]
    fn keyboard_typo_variants_work_even_without_fuzzy_mode() {
        let mut set = tiny_set();
        set.insert("jing".to_string());
        let vars = parse_input_variants(
            "jimg",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars
            .iter()
            .any(|(v, t)| t.is_none() && v == &vec!["jing".to_string()]));
    }

    #[test]
    fn keyboard_typo_variants_include_missing_letter_fix() {
        let mut set = tiny_set();
        set.insert("jing".to_string());
        let vars = parse_input_variants(
            "jng",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars
            .iter()
            .any(|(v, t)| t.is_none() && v == &vec!["jing".to_string()]));
    }

    #[test]
    fn keyboard_typo_variants_include_extra_letter_fix() {
        let mut set = tiny_set();
        set.insert("jing".to_string());
        let vars = parse_input_variants(
            "jiing",
            &set,
            ParseOptions {
                fuzzy_enabled: false,
                double_pinyin_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars
            .iter()
            .any(|(v, t)| t.is_none() && v == &vec!["jing".to_string()]));
    }

    #[test]
    fn phonetic_typo_enabled_emits_lan_for_ran() {
        let mut set = HashSet::new();
        for syl in ["ran", "lan"] {
            set.insert(syl.to_string());
        }
        let vars = parse_input_variants_detailed(
            "ran",
            &set,
            ParseOptions {
                correction_enabled: true,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(vars.iter().any(|v| {
            v.source == ParseVariantSource::PhoneticTypo
                && v.syllables == ["lan".to_string()]
                && v.tail.is_none()
                && v.corrected_input.as_deref() == Some("lan")
        }));
    }

    #[test]
    fn phonetic_typo_disabled_skips_lan_for_ran() {
        let mut set = HashSet::new();
        for syl in ["ran", "lan"] {
            set.insert(syl.to_string());
        }
        let vars = parse_input_variants_detailed(
            "ran",
            &set,
            ParseOptions {
                correction_enabled: false,
                ..ParseOptions::default()
            },
        )
        .unwrap();
        assert!(!vars.iter().any(|v| {
            v.source == ParseVariantSource::PhoneticTypo && v.syllables == ["lan".to_string()]
        }));
    }

    #[test]
    fn mixed_pinyin_keys_expand_initial_and_full_syllables() {
        let mut set = tiny_set();
        for syl in ["shu", "ru", "fa", "su"] {
            set.insert(syl.to_string());
        }

        let shur = expand_mixed_pinyin_exact_keys("shur", &set, 32);
        assert!(shur
            .iter()
            .any(|(key, segments)| key == "shuru" && *segments == 2));

        let sru = expand_mixed_pinyin_exact_keys("sru", &set, 32);
        assert!(sru
            .iter()
            .any(|(key, segments)| key == "shuru" && *segments == 2));
        assert!(sru.iter().all(|(_, segments)| *segments == 2));

        set.insert("chong".to_string());
        set.insert("qing".to_string());
        let cqing = expand_mixed_pinyin_exact_keys("cqing", &set, 128);
        assert!(cqing
            .iter()
            .any(|(key, segments)| key == "chongqing" && *segments == 2));
    }

    #[test]
    fn mixed_pinyin_key_details_track_path_and_rank_completion() {
        let mut set = tiny_set();
        for syl in ["shu", "ru", "ren"] {
            set.insert(syl.to_string());
        }

        let ranked =
            expand_mixed_pinyin_exact_key_details_by_score("shur", &set, 8, |leading, syllable| {
                match (leading, syllable) {
                    ("shu", "ru") => 10.0,
                    _ => 0.0,
                }
            });

        let first = ranked.first().expect("mixed expansion");
        assert_eq!(first.key, "shuru");
        assert_eq!(first.segment_count, 2);
        assert_eq!(first.full_segments, 1);
        assert_eq!(first.initial_segments, 1);
    }

    #[test]
    fn mixed_pinyin_supports_non_initial_syllable_prefixes() {
        let mut set = tiny_set();
        for syllable in ["shu", "ru", "fa", "fang"] {
            set.insert(syllable.to_string());
        }

        let ranked = expand_mixed_pinyin_exact_key_details_by_score(
            "shurfa",
            &set,
            32,
            |leading, syllable| {
                let key = format!("{leading}{syllable}");
                if "shurufang".starts_with(&key) {
                    20.0
                } else {
                    0.0
                }
            },
        );
        let target = ranked
            .iter()
            .find(|item| item.key == "shurufang")
            .expect("mixed prefix expansion");
        assert_eq!(target.segment_count, 3);
        assert_eq!(target.full_segments, 1);
        assert_eq!(target.prefix_segments, 1);
        assert_eq!(target.initial_segments, 1);
    }

    #[test]
    fn mixed_pinyin_beam_keeps_globally_scored_four_segment_path() {
        let mut set = tiny_set();
        for syllable in [
            "shi", "shou", "shui", "su", "si", "qi", "qiu", "qing", "qu", "sheng",
        ] {
            set.insert(syllable.to_string());
        }

        let target = "shishiqiushi";
        let ranked = expand_mixed_pinyin_exact_key_details_by_score(
            "shisqs",
            &set,
            24,
            |leading, syllable| {
                let key = format!("{leading}{syllable}");
                if target.starts_with(&key) {
                    100.0
                } else {
                    0.0
                }
            },
        );
        assert!(
            ranked.iter().any(|item| item.key == target),
            "ranked keys: {:?}",
            ranked
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn incremental_mixed_pinyin_matches_full_expansion_across_adjacent_edits() {
        let mut set = tiny_set();
        for syl in [
            "shu", "ru", "fa", "su", "ren", "ran", "chong", "qing", "qi", "qu",
        ] {
            set.insert(syl.to_string());
        }
        let mut cache = MixedPinyinIncrementalCache::default();
        for input in [
            "s", "sh", "shu", "shur", "shuru", "shuruf", "shurufa", "shuruf", "shuruq", "shuruqi",
            "cqing",
        ] {
            let expected = expand_mixed_pinyin_exact_key_details_by_score(
                input,
                &set,
                64,
                |leading, syllable| (leading.len() * 10 + syllable.len()) as f64,
            );
            let actual = cache.expand_by_score(input, &set, 64, |leading, syllable| {
                (leading.len() * 10 + syllable.len()) as f64
            });
            assert_eq!(
                actual, expected,
                "incremental expansion differs for {input}"
            );
        }
    }

    #[test]
    fn incremental_mixed_pinyin_budget_abort_does_not_break_full_retry() {
        let mut set = tiny_set();
        for syllable in ["ji", "suan", "wang", "luo", "li", "wu"] {
            set.insert(syllable.to_string());
        }
        let mut cache = MixedPinyinIncrementalCache::default();
        let mut checks = 0usize;
        let (_, completed) = cache.expand_by_score_until(
            "jisuanjwl",
            &set,
            16,
            |leading, syllable| (leading.len() + syllable.len()) as f64,
            || {
                checks += 1;
                checks >= 2
            },
        );
        assert!(!completed);

        let retried = cache.expand_by_score("jisuanjwl", &set, 16, |leading, syllable| {
            (leading.len() + syllable.len()) as f64
        });
        assert!(
            !retried.is_empty(),
            "budget-free retry must finish expansion"
        );
    }

    #[test]
    fn predict_syllable_completions_can_rank_by_score() {
        let set = tiny_set();
        let ranked = predict_syllable_completions_by_score("h", &set, 4, |syl| match syl {
            "hao" => 8.0,
            "han" => 6.0,
            "hai" => 4.0,
            _ => 1.0,
        });
        assert_eq!(ranked[0], "hao");
        assert_eq!(ranked[1], "han");
        assert_eq!(ranked[2], "hai");
    }
}
