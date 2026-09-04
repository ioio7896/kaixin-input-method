//! Native text lexicon loader and prebaked `lexicon.bin` support.

use crate::text_encoding::read_text_file;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Packaged lexicon without a source-path digest.
const PREBAKED_MAGIC: &[u8; 8] = b"SRFLX002";
const PREBAKED_SCHEMA_VERSION: u32 = 9;
const PREBAKED_MIN_SUPPORTED_SCHEMA_VERSION: u32 = 5;

const PREBAKED_MAX_STRING_BYTES: usize = 16_384;
const PREBAKED_MAX_MAP_KEYS: usize = 12_000_000;
const PREBAKED_MAX_ENTRIES_PER_KEY: usize = 65_536;
const PREBAKED_MAX_DELETION_REFERENCES: usize = 24_000_000;
const PREFIX_INDEX_TOP_K: usize = 48;
const PREFIX_SCAN_MAX_KEYS: usize = 4096;
const PREBAKED_PREFIX_MIN_KEYS: usize = 16;
const DELETION_INDEX_MAX_KEY_LEN: usize = 40;
const DELETION_INDEX_MAX_KEYS_TO_BUILD: usize = 250_000;
const MAX_HETERONYM_KEYS_PER_PHRASE: usize = 64;
const MAX_MIXED_KEYS_PER_PHRASE: usize = 128;
const MAX_MIXED_KEY_PHRASE_CHARS: usize = 4;
/// All curated text lexicons use this normalized frequency scale.
pub(crate) const MAX_LEXICON_FREQ: u64 = 10_000;
/// Base contains both common words and a sizeable tail. Keep the direct mixed
/// index focused on entries that can plausibly affect the interactive head;
/// Core remains an explicit, always-indexed override layer.
const MIXED_HOT_BASE_MIN_FREQ: u64 = MAX_LEXICON_FREQ * 35 / 100;
/// Long phrases stay out of the mixed-initial index, but the most common
/// explicit 5–7 character entries get a small full-pinyin/prefix hot index.
const LONG_PHRASE_HOT_MIN_CHARS: usize = 5;
const LONG_PHRASE_HOT_MAX_CHARS: usize = 7;
const LONG_PHRASE_HOT_MIN_FREQ: u64 = MAX_LEXICON_FREQ * 60 / 100;
const LONG_PHRASE_HOT_TOP_K: usize = 16;
const PREBAKED_ENTRIES_PER_KEY_TOP_K: usize = 96;
const STANDARD_LARGE_ENTRY_MAX_CHARS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LexiconBuildProfile {
    Hot,
    Standard,
    Full,
}

impl LexiconBuildProfile {
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "hot" | "fast" | "core" => Self::Hot,
            "full" | "all" | "large" => Self::Full,
            _ => Self::Standard,
        }
    }

    fn includes_path(self, path: &Path) -> bool {
        match self {
            Self::Hot => matches!(
                LexiconLayer::from_path(path),
                LexiconLayer::Core | LexiconLayer::Base | LexiconLayer::Ext
            ),
            Self::Full => true,
            // Standard scans `large`, but entry filtering keeps only short
            // phrases so common 3-char words are not missing from runtime.
            Self::Standard => true,
        }
    }

    fn includes_entry(self, path: &Path, phrase: &str) -> bool {
        match self {
            Self::Standard if LexiconLayer::from_path(path) == LexiconLayer::Large => {
                lexicon_phrase_char_count(phrase) <= STANDARD_LARGE_ENTRY_MAX_CHARS
            }
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LexiconLoadOptions {
    pub entries_per_key_limit: usize,
}

impl Default for LexiconLoadOptions {
    fn default() -> Self {
        Self {
            entries_per_key_limit: PREBAKED_ENTRIES_PER_KEY_TOP_K,
        }
    }
}

#[inline]
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThuoclEntry {
    pub phrase: String,
    pub freq: u64,
    pub code: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LexiconLayer {
    #[default]
    Unknown,
    Core,
    Base,
    Ext,
    Large,
    En,
}

impl LexiconLayer {
    pub fn from_path(path: &Path) -> Self {
        if path_component_eq(path, "en") {
            Self::En
        } else if path_component_eq(path, "zh-ext") {
            // Directory ownership is authoritative: optional/long-tail
            // dictionaries under zh-ext must never inherit Core/Base status
            // merely because they retain a legacy file name.
            Self::Ext
        } else if path_component_eq(path, "core") {
            Self::Core
        } else if path_component_eq(path, "base") {
            Self::Base
        } else if path_component_eq(path, "ext") {
            Self::Ext
        } else if path_component_eq(path, "large") {
            Self::Large
        } else if path_component_eq(path, "zh") {
            Self::from_flat_zh_file(path)
        } else {
            Self::Unknown
        }
    }

    fn from_flat_zh_file(path: &Path) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "kaixin_explicit.txt"
                | "kaixin_polyphone.txt"
                | "kaixin_pronunciation_aliases.txt"
                | "lfie-common-3char.txt"
        ) {
            Self::Core
        } else if name.contains("_tail_") || name.starts_with("large_") {
            Self::Large
        } else {
            // The zh directory is the curated hot/base layer. New files added
            // there should receive Base treatment without another hard-coded
            // filename allow-list. Optional dictionaries belong in zh-ext.
            Self::Base
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Core => "core",
            Self::Base => "base",
            Self::Ext => "ext",
            Self::Large => "large",
            Self::En => "en",
        }
    }

    fn sort_order(self) -> u8 {
        match self {
            Self::Core => 0,
            Self::Base => 1,
            Self::Ext => 2,
            Self::Large => 3,
            Self::En => 4,
            Self::Unknown => u8::MAX,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Core => 1,
            Self::Base => 2,
            Self::Ext => 3,
            Self::Large => 4,
            Self::En => 5,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Core,
            2 => Self::Base,
            3 => Self::Ext,
            4 => Self::Large,
            5 => Self::En,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompactEntry {
    phrase_id: u32,
    freq: u64,
}

pub fn abbrev_for_phrase(phrase: &str) -> Option<String> {
    abbrev_keys_for_phrase(phrase).into_iter().next()
}

pub fn abbrev_keys_for_phrase(phrase: &str) -> Vec<String> {
    phrase_pinyin_keys(phrase, true)
}

pub fn pinyin_key_for_phrase(phrase: &str) -> Option<String> {
    pinyin_keys_for_phrase(phrase).into_iter().next()
}

pub fn pinyin_keys_for_phrase(phrase: &str) -> Vec<String> {
    phrase_pinyin_keys(phrase, false)
}

/// Return one syllable per character for evaluation tooling.  Keeping the
/// boundaries here avoids re-segmenting a compact key with a greedy splitter
/// (for example `jia na da` must not become `jian a da`).
pub fn primary_pinyin_syllables_for_phrase(phrase: &str) -> Option<Vec<String>> {
    phrase
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .map(|ch| pinyin_options_for_char(ch, false).into_iter().next())
        .collect()
}

fn normalize_pinyin_code(code: &str) -> Option<String> {
    let compact: String = code
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '\'' && *ch != '-' && *ch != '_')
        .flat_map(char::to_lowercase)
        .collect();
    (!compact.is_empty()).then_some(compact)
}

fn abbrev_key_for_pinyin_code(code: &str) -> Option<String> {
    let mut out = String::new();
    for syllable in code.split_whitespace() {
        if let Some(ch) = syllable.chars().find(|ch| ch.is_ascii_alphabetic()) {
            out.push(ch.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        normalize_pinyin_code(code).and_then(|key| key.chars().next().map(|ch| ch.to_string()))
    } else {
        Some(out)
    }
}

pub fn mixed_pinyin_keys_for_phrase(phrase: &str) -> Vec<String> {
    let mut syllable_options = Vec::new();
    for ch in phrase.chars().filter(|ch| !ch.is_whitespace()) {
        let options = pinyin_options_for_char(ch, false);
        if options.is_empty() {
            return Vec::new();
        }
        syllable_options.push(options);
    }

    mixed_pinyin_keys_from_options(syllable_options)
}

/// Generate partial-full/partial-initial keys from an explicit syllable code.
///
/// Unlike [`mixed_pinyin_keys_for_phrase`], this never guesses character
/// readings. It is therefore suitable for the prebaked direct-hit index, where
/// a false reading would become an exact match.
pub fn mixed_pinyin_keys_for_code(code: &str) -> Vec<String> {
    let syllable_options = explicit_pinyin_syllables(code)
        .into_iter()
        .map(|syllable| vec![syllable])
        .collect();
    mixed_pinyin_keys_from_options(syllable_options)
}

fn explicit_pinyin_syllables(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in code
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '-' | '_'))
        .filter(|part| !part.is_empty())
    {
        if !part.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return Vec::new();
        }
        out.push(part.to_ascii_lowercase());
    }
    out
}

/// Explicit readings are authoritative only when they can describe every
/// Chinese character in the phrase.  A malformed third-party row must not
/// poison its full-pinyin and abbreviation indexes; callers then fall back to
/// the built-in per-character reading table.
fn has_matching_explicit_pinyin(phrase: &str, code: &str) -> bool {
    let phrase_len = lexicon_phrase_char_count(phrase);
    phrase_len > 0 && explicit_pinyin_syllables(code).len() == phrase_len
}

fn mixed_pinyin_keys_from_options(syllable_options: Vec<Vec<String>>) -> Vec<String> {
    let len = syllable_options.len();
    if !(2..=MAX_MIXED_KEY_PHRASE_CHARS).contains(&len) {
        return Vec::new();
    }

    let mut full_keys: Vec<Vec<String>> = vec![Vec::new()];
    'outer: for options in syllable_options {
        let mut next = Vec::new();
        for prefix in &full_keys {
            for option in &options {
                let mut key = prefix.clone();
                key.push(option.clone());
                if !next.iter().any(|seen| seen == &key) {
                    next.push(key);
                    if next.len() >= MAX_HETERONYM_KEYS_PER_PHRASE {
                        break 'outer;
                    }
                }
            }
        }
        full_keys = next;
    }

    let mut out = Vec::new();
    for syllables in full_keys {
        if syllables.len() != len {
            continue;
        }
        let all_initial_mask = 0usize;
        let all_full_mask = (1usize << len) - 1;
        for mask in 1usize..all_full_mask {
            if mask == all_initial_mask || mask == all_full_mask {
                continue;
            }
            let mut key = String::new();
            let mut valid = true;
            for (idx, syllable) in syllables.iter().enumerate() {
                if (mask & (1usize << idx)) != 0 {
                    key.push_str(syllable);
                } else if let Some(initial) = syllable.chars().next() {
                    key.push(initial);
                } else {
                    valid = false;
                    break;
                }
            }
            if valid && !out.iter().any(|seen| seen == &key) {
                out.push(key);
                if out.len() >= MAX_MIXED_KEYS_PER_PHRASE {
                    return out;
                }
            }
        }
    }
    out
}

fn phrase_pinyin_keys(phrase: &str, first_letter_only: bool) -> Vec<String> {
    let mut keys = vec![String::new()];
    for ch in phrase.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let options = pinyin_options_for_char(ch, first_letter_only);
        if options.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        'outer: for prefix in &keys {
            for option in &options {
                let mut key = prefix.clone();
                key.push_str(option);
                if !next.iter().any(|seen| seen == &key) {
                    next.push(key);
                    if next.len() >= MAX_HETERONYM_KEYS_PER_PHRASE {
                        break 'outer;
                    }
                }
            }
        }
        keys = next;
        if keys.is_empty() {
            return Vec::new();
        }
    }
    keys.retain(|key| !key.is_empty());
    keys
}

fn pinyin_options_for_char(ch: char, first_letter_only: bool) -> Vec<String> {
    let mut out = Vec::new();
    for plain in crate::dict::pinyin_plain_options(ch) {
        let option = if first_letter_only {
            let Some(first) = plain.chars().next() else {
                continue;
            };
            if !first.is_ascii_alphabetic() {
                continue;
            }
            first.to_string()
        } else {
            if plain.is_empty() {
                continue;
            }
            plain
        };
        if out.iter().any(|seen| seen == &option) {
            continue;
        }
        out.push(option);
    }
    out
}

pub fn parse_line(line: &str) -> Option<ThuoclEntry> {
    let line = strip_bom(line.trim());
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut it = line.splitn(2, '\t');
    let phrase = it.next()?.trim();
    let freq_part = it.next()?.trim();
    if phrase.is_empty() {
        return None;
    }
    let freq = freq_part.parse().ok()?;
    Some(ThuoclEntry {
        phrase: phrase.to_string(),
        freq,
        code: None,
    })
}

pub fn parse_lexicon_entry_line(line: &str) -> Option<ThuoclEntry> {
    let line = strip_bom(line.trim());
    if line.is_empty()
        || line.starts_with('#')
        || line.starts_with("---")
        || line == "..."
        || line.starts_with("- ")
    {
        return None;
    }

    let tab_fields: Vec<&str> = line
        .split('\t')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect();
    let whitespace_fields;
    let (fields, from_whitespace) = if tab_fields.len() <= 1 {
        whitespace_fields = line
            .split_whitespace()
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        let can_use_whitespace = whitespace_fields.len() >= 2
            && (phrase_contains_cjk(whitespace_fields[0])
                || whitespace_fields
                    .last()
                    .is_some_and(|field| field.parse::<u64>().is_ok()));
        if can_use_whitespace {
            (whitespace_fields.as_slice(), true)
        } else {
            (tab_fields.as_slice(), false)
        }
    } else {
        (tab_fields.as_slice(), false)
    };
    if fields.is_empty() {
        return None;
    }

    let phrase = fields[0];
    if fields.len() == 1 && (phrase.contains(':') || !phrase_contains_cjk(phrase)) {
        return None;
    }

    let freq_index = fields
        .iter()
        .rposition(|field| field.parse::<u64>().is_ok());
    let freq = freq_index
        .and_then(|idx| fields[idx].parse::<u64>().ok())
        .unwrap_or(1);
    let code = if from_whitespace {
        let end = freq_index.unwrap_or(fields.len());
        (end > 1).then(|| fields[1..end].join(" "))
    } else {
        (fields.len() >= 2
            && freq_index != Some(1)
            && fields[1].chars().any(|ch| ch.is_ascii_alphabetic()))
        .then(|| fields[1].to_string())
    };

    Some(ThuoclEntry {
        phrase: phrase.to_string(),
        freq,
        code,
    })
}

pub fn parse_fallback_tabbed_line(line: &str) -> Option<ThuoclEntry> {
    parse_lexicon_entry_line(line)
}

fn path_component_eq(path: &Path, expected: &str) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(expected)
    })
}

fn lexicon_phrase_char_count(phrase: &str) -> usize {
    phrase.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn phrase_contains_cjk(phrase: &str) -> bool {
    phrase.chars().any(|ch| {
        matches!(
            ch as u32,
            0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
                | 0x20000..=0x2A6DF
                | 0x2A700..=0x2B73F
                | 0x2B740..=0x2B81F
                | 0x2B820..=0x2CEAF
                | 0x30000..=0x3134F
        )
    })
}

fn merge_compact_entry(vec: &mut Vec<CompactEntry>, phrase_id: u32, freq: u64) {
    if let Some(i) = vec.iter().position(|e| e.phrase_id == phrase_id) {
        if freq > vec[i].freq {
            vec[i].freq = freq;
        }
    } else {
        vec.push(CompactEntry { phrase_id, freq });
    }
    vec.sort_by_key(|entry| Reverse(entry.freq));
}

fn trim_compact_entries(entries: &mut Vec<CompactEntry>, limit: usize) {
    if limit > 0 && entries.len() > limit {
        entries.truncate(limit);
    }
}

fn trim_compact_map_entries(map: &mut BTreeMap<String, Vec<CompactEntry>>, limit: usize) {
    if limit == 0 {
        map.clear();
        return;
    }
    for entries in map.values_mut() {
        trim_compact_entries(entries, limit);
    }
}

#[derive(Debug, Default, Clone)]
struct DeletionIndex {
    keys: Vec<String>,
    deletes: HashMap<String, Vec<u32>>,
}

#[derive(Debug, Clone, Default)]
enum LazyDeletionIndex {
    #[default]
    Uninitialized,
    Disabled,
    Ready(DeletionIndex),
}

/// A bounded HashMap with FIFO eviction: when full, the oldest 25% of entries
/// are removed rather than the entire cache being cleared at once, avoiding
/// periodic cache-hit cliffs during sustained typing.
#[derive(Debug, Clone, Default)]
struct FifoCache<V> {
    map: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V: Clone> FifoCache<V> {
    fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key)
    }

    fn insert(&mut self, key: String, value: V, max_keys: usize) {
        if self.map.len() >= max_keys {
            let evict_count = (max_keys / 4).max(1);
            for _ in 0..evict_count {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
        // Remove old position if already present (re-insert = move to back)
        if self.map.contains_key(&key) {
            self.order.retain(|k| k != &key);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

#[derive(Debug, Clone, Default)]
struct RuntimeIndexes {
    abbrev_delete: LazyDeletionIndex,
    pinyin_delete: LazyDeletionIndex,
    abbrev_exact: FifoCache<Arc<[ThuoclEntry]>>,
    pinyin_exact: FifoCache<Arc<[ThuoclEntry]>>,
    hot_abbrev_exact: FifoCache<Arc<[ThuoclEntry]>>,
    hot_pinyin_exact: FifoCache<Arc<[ThuoclEntry]>>,
    mixed_hot_exact: FifoCache<Arc<[ThuoclEntry]>>,
}

// Keep a wider exact-abbreviation head for short, high-collision keys. The
// interactive first page still truncates aggressively, but materializing a
// wider head prevents everyday two-character phrases from disappearing
// behind unrelated candidates (for example `sj`/`dt`).
const HOT_EXACT_TOP_K: usize = 96;
const HOT_EXACT_CACHE_MAX_KEYS: usize = 1024;
const EXACT_CACHE_MAX_KEYS: usize = 2048;

impl DeletionIndex {
    fn build_from(map: &BTreeMap<String, Vec<CompactEntry>>) -> Self {
        Self::build_from_until(map, || false).unwrap_or_default()
    }

    fn build_from_until(
        map: &BTreeMap<String, Vec<CompactEntry>>,
        mut should_stop: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut keys = Vec::with_capacity(map.len());
        let mut deletes: HashMap<String, Vec<u32>> = HashMap::new();
        for (iteration, key) in map.keys().enumerate() {
            if iteration % 256 == 0 && should_stop() {
                return None;
            }
            if key.is_empty() || key.len() > DELETION_INDEX_MAX_KEY_LEN {
                continue;
            }
            let idx = u32::try_from(keys.len()).unwrap_or(u32::MAX);
            if idx == u32::MAX {
                break;
            }
            keys.push(key.clone());
            for deleted in deletion_forms(key) {
                deletes.entry(deleted).or_default().push(idx);
            }
        }
        Some(Self { keys, deletes })
    }

    fn candidate_keys(&self, key: &str, max_distance: usize) -> Option<Vec<String>> {
        if key.is_empty() || max_distance != 1 || key.len() > DELETION_INDEX_MAX_KEY_LEN {
            return None;
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for deleted in deletion_forms(key) {
            let Some(indices) = self.deletes.get(&deleted) else {
                continue;
            };
            for &idx in indices {
                if !seen.insert(idx) {
                    continue;
                }
                if let Some(candidate) = self.keys.get(idx as usize) {
                    out.push(candidate.clone());
                }
            }
        }
        Some(out)
    }
}

fn write_deletion_index<W: Write>(
    writer: &mut W,
    map: &BTreeMap<String, Vec<CompactEntry>>,
) -> io::Result<()> {
    if map.len() > DELETION_INDEX_MAX_KEYS_TO_BUILD {
        write_u8(writer, 0)?;
        return Ok(());
    }
    write_u8(writer, 1)?;
    let index = DeletionIndex::build_from(map);
    write_count(writer, index.keys.len())?;
    for key in &index.keys {
        write_string(writer, key)?;
    }
    write_count(writer, index.deletes.len())?;
    let mut deletes = index.deletes.iter().collect::<Vec<_>>();
    deletes.sort_unstable_by(|left, right| left.0.cmp(right.0));
    for (deleted, indices) in deletes {
        write_string(writer, deleted)?;
        write_count(writer, indices.len())?;
        for &index in indices {
            write_u32(writer, index)?;
        }
    }
    Ok(())
}

fn read_deletion_index<R: Read>(reader: &mut R) -> io::Result<LazyDeletionIndex> {
    if read_u8(reader)? == 0 {
        return Ok(LazyDeletionIndex::Disabled);
    }
    let key_count = read_u32(reader)? as usize;
    if key_count > DELETION_INDEX_MAX_KEYS_TO_BUILD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked deletion key count exceeds max",
        ));
    }
    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        keys.push(read_string(reader)?);
    }
    let delete_count = read_u32(reader)? as usize;
    if delete_count > PREBAKED_MAX_MAP_KEYS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked deletion map exceeds max",
        ));
    }
    let mut references = 0usize;
    let mut deletes = HashMap::with_capacity(delete_count);
    for _ in 0..delete_count {
        let deleted = read_string(reader)?;
        let count = read_u32(reader)? as usize;
        references = references.saturating_add(count);
        if references > PREBAKED_MAX_DELETION_REFERENCES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prebaked deletion references exceed max",
            ));
        }
        let mut indices = Vec::with_capacity(count);
        for _ in 0..count {
            let index = read_u32(reader)?;
            if index as usize >= keys.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prebaked deletion key index out of range",
                ));
            }
            indices.push(index);
        }
        deletes.insert(deleted, indices);
    }
    Ok(LazyDeletionIndex::Ready(DeletionIndex { keys, deletes }))
}

fn merge_prefix_top(top: &mut Vec<CompactEntry>, entries: &[CompactEntry]) {
    for entry in entries.iter().take(PREFIX_INDEX_TOP_K) {
        if let Some(existing) = top
            .iter_mut()
            .find(|seen| seen.phrase_id == entry.phrase_id)
        {
            existing.freq = existing.freq.max(entry.freq);
        } else {
            top.push(*entry);
        }
    }
    top.sort_by(|a, b| {
        b.freq
            .cmp(&a.freq)
            .then_with(|| a.phrase_id.cmp(&b.phrase_id))
    });
    if top.len() > PREFIX_INDEX_TOP_K {
        top.truncate(PREFIX_INDEX_TOP_K);
    }
}

fn deletion_forms(key: &str) -> Vec<String> {
    let chars: Vec<char> = key.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    // Allocate for: original + N deletions + (N-1) adjacent swaps
    let capacity = 1 + chars.len() + chars.len().saturating_sub(1);
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(capacity);
    if seen.insert(key.to_string()) {
        out.push(key.to_string());
    }
    // Single-character deletions (edit distance 1)
    for skip in 0..chars.len() {
        let mut deleted = String::with_capacity(key.len());
        for (idx, ch) in chars.iter().enumerate() {
            if idx != skip {
                deleted.push(*ch);
            }
        }
        if seen.insert(deleted.clone()) {
            out.push(deleted);
        }
    }
    // Adjacent character swaps (Damerau-Levenshtein distance 1)
    for pos in 0..chars.len().saturating_sub(1) {
        let mut swapped = String::with_capacity(key.len());
        for (idx, ch) in chars.iter().enumerate() {
            if idx == pos {
                swapped.push(chars[pos + 1]);
            } else if idx == pos + 1 {
                swapped.push(chars[pos]);
            } else {
                swapped.push(*ch);
            }
        }
        if seen.insert(swapped.clone()) {
            out.push(swapped);
        }
    }
    out
}

#[derive(Debug)]
pub struct AbbrevLexicon {
    inner: BTreeMap<String, Vec<CompactEntry>>,
    pinyin_inner: BTreeMap<String, Vec<CompactEntry>>,
    long_pinyin_hot_inner: BTreeMap<String, Vec<CompactEntry>>,
    mixed_hot_inner: BTreeMap<String, Vec<CompactEntry>>,
    prefix_inner: HashMap<String, Vec<CompactEntry>>,
    pinyin_prefix_inner: HashMap<String, Vec<CompactEntry>>,
    phrases: Vec<Arc<str>>,
    phrase_freq: Vec<u64>,
    phrase_layer: Vec<LexiconLayer>,
    phrase_ids: Option<HashMap<Arc<str>, u32>>,
    runtime: RefCell<RuntimeIndexes>,
    /// Build-time counter: number of Base entries skipped from mixed-hot index
    /// because their frequency was below MIXED_HOT_BASE_MIN_FREQ.
    pub mixed_hot_base_skipped: usize,
}

impl Default for AbbrevLexicon {
    fn default() -> Self {
        Self {
            inner: BTreeMap::new(),
            pinyin_inner: BTreeMap::new(),
            long_pinyin_hot_inner: BTreeMap::new(),
            mixed_hot_inner: BTreeMap::new(),
            prefix_inner: HashMap::new(),
            pinyin_prefix_inner: HashMap::new(),
            phrases: Vec::new(),
            phrase_freq: Vec::new(),
            phrase_layer: Vec::new(),
            phrase_ids: Some(HashMap::new()),
            runtime: RefCell::new(RuntimeIndexes::default()),
            mixed_hot_base_skipped: 0,
        }
    }
}

impl Clone for AbbrevLexicon {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            pinyin_inner: self.pinyin_inner.clone(),
            long_pinyin_hot_inner: self.long_pinyin_hot_inner.clone(),
            mixed_hot_inner: self.mixed_hot_inner.clone(),
            prefix_inner: self.prefix_inner.clone(),
            pinyin_prefix_inner: self.pinyin_prefix_inner.clone(),
            phrases: self.phrases.clone(),
            phrase_freq: self.phrase_freq.clone(),
            phrase_layer: self.phrase_layer.clone(),
            phrase_ids: self.phrase_ids.clone(),
            runtime: RefCell::new(RuntimeIndexes::default()),
            mixed_hot_base_skipped: self.mixed_hot_base_skipped,
        }
    }
}

impl AbbrevLexicon {
    pub fn with_load_options(mut self, options: LexiconLoadOptions) -> Self {
        self.apply_load_options(options);
        self
    }

    pub fn apply_load_options(&mut self, options: LexiconLoadOptions) {
        let limit = options.entries_per_key_limit;
        let entries_changed = limit == 0
            || self.inner.values().any(|entries| entries.len() > limit)
            || self
                .pinyin_inner
                .values()
                .any(|entries| entries.len() > limit)
            || self
                .long_pinyin_hot_inner
                .values()
                .any(|entries| entries.len() > limit.min(LONG_PHRASE_HOT_TOP_K))
            || self
                .mixed_hot_inner
                .values()
                .any(|entries| entries.len() > limit.min(HOT_EXACT_TOP_K));
        trim_compact_map_entries(&mut self.inner, options.entries_per_key_limit);
        trim_compact_map_entries(&mut self.pinyin_inner, options.entries_per_key_limit);
        trim_compact_map_entries(
            &mut self.long_pinyin_hot_inner,
            options.entries_per_key_limit.min(LONG_PHRASE_HOT_TOP_K),
        );
        trim_compact_map_entries(
            &mut self.mixed_hot_inner,
            options.entries_per_key_limit.min(HOT_EXACT_TOP_K),
        );
        if entries_changed {
            self.mark_runtime_dirty();
        }
    }

    pub fn len_keys(&self) -> usize {
        self.inner.len()
    }

    pub fn len_pinyin_keys(&self) -> usize {
        self.pinyin_inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty() && self.pinyin_inner.is_empty()
    }

    /// Returns high-frequency exact-pinyin samples for offline ranking audits.
    pub fn evaluation_samples_by_phrase_len(
        &self,
        phrase_len: usize,
        limit: usize,
        per_key_limit: usize,
    ) -> Vec<(String, String, u64, LexiconLayer)> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (key, entries) in &self.pinyin_inner {
            for entry in entries.iter().take(per_key_limit.max(1)) {
                let Some(phrase) = self.phrases.get(entry.phrase_id as usize) else {
                    continue;
                };
                if lexicon_phrase_char_count(phrase) != phrase_len
                    || !seen.insert((key.clone(), entry.phrase_id))
                {
                    continue;
                }
                out.push((
                    key.clone(),
                    phrase.to_string(),
                    entry.freq,
                    self.phrase_layer
                        .get(entry.phrase_id as usize)
                        .copied()
                        .unwrap_or(LexiconLayer::Unknown),
                ));
            }
        }
        out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        out.truncate(limit);
        out
    }

    pub fn insert_parsed(&mut self, entry: ThuoclEntry) -> bool {
        self.insert_parsed_with_layer(entry, LexiconLayer::Unknown)
    }

    pub fn insert_parsed_with_layer(
        &mut self,
        entry: ThuoclEntry,
        source_layer: LexiconLayer,
    ) -> bool {
        let mut inserted = false;
        let phrase_id = self.intern_phrase(&entry.phrase, entry.freq, source_layer);
        let explicit_code = entry
            .code
            .as_deref()
            .filter(|code| has_matching_explicit_pinyin(&entry.phrase, code));
        let abbrev_keys = explicit_code
            .and_then(abbrev_key_for_pinyin_code)
            .map(|key| vec![key])
            .unwrap_or_else(|| abbrev_keys_for_phrase(&entry.phrase));
        for key in abbrev_keys {
            let v = self.inner.entry(key).or_default();
            merge_compact_entry(v, phrase_id, entry.freq);
            trim_compact_entries(v, PREBAKED_ENTRIES_PER_KEY_TOP_K);
            inserted = true;
        }
        let pinyin_keys = explicit_code
            .and_then(normalize_pinyin_code)
            .map(|key| vec![key])
            .unwrap_or_else(|| pinyin_keys_for_phrase(&entry.phrase));
        for key in &pinyin_keys {
            let v = self.pinyin_inner.entry(key.clone()).or_default();
            merge_compact_entry(v, phrase_id, entry.freq);
            trim_compact_entries(v, PREBAKED_ENTRIES_PER_KEY_TOP_K);
            inserted = true;
        }
        if should_index_long_phrase_hot(
            source_layer,
            entry.freq,
            &entry.phrase,
            entry.code.as_deref(),
        ) {
            for key in pinyin_keys {
                let v = self.long_pinyin_hot_inner.entry(key).or_default();
                merge_compact_entry(v, phrase_id, entry.freq);
                trim_compact_entries(v, LONG_PHRASE_HOT_TOP_K);
            }
        }
        if should_index_mixed_hot(source_layer, entry.freq) {
            if let Some(code) = explicit_code {
                let phrase_char_count = lexicon_phrase_char_count(&entry.phrase);
                let syllable_count = explicit_pinyin_syllables(code).len();
                if (2..=MAX_MIXED_KEY_PHRASE_CHARS).contains(&phrase_char_count)
                    && syllable_count == phrase_char_count
                {
                    for key in mixed_pinyin_keys_for_code(code) {
                        let v = self.mixed_hot_inner.entry(key).or_default();
                        merge_compact_entry(v, phrase_id, entry.freq);
                        trim_compact_entries(v, HOT_EXACT_TOP_K);
                    }
                }
            }
        } else if source_layer == LexiconLayer::Base
            && entry.code.is_some()
            && (2..=MAX_MIXED_KEY_PHRASE_CHARS).contains(&lexicon_phrase_char_count(&entry.phrase))
        {
            // Track how many Base entries are skipped from mixed-hot due to
            // frequency, so build logs can guide MIXED_HOT_BASE_MIN_FREQ tuning.
            self.mixed_hot_base_skipped += 1;
        }
        self.mark_runtime_dirty();
        inserted
    }

    pub fn lookup(&self, abbrev_lower: &str) -> Option<Arc<[ThuoclEntry]>> {
        self.lookup_exact(abbrev_lower, false)
    }

    /// Materializes only the high-value head used by the interactive first
    /// page and retains it in a compact runtime index. Normal `lookup` remains
    /// available to offline/full-tail callers that need every stored entry.
    pub(crate) fn lookup_hot(&self, abbrev_lower: &str) -> Option<Arc<[ThuoclEntry]>> {
        self.lookup_hot_exact(abbrev_lower, false)
    }

    pub fn lookup_pinyin(&self, pinyin_lower: &str) -> Option<Arc<[ThuoclEntry]>> {
        self.lookup_exact(pinyin_lower, true)
    }

    pub(crate) fn lookup_pinyin_hot(&self, pinyin_lower: &str) -> Option<Arc<[ThuoclEntry]>> {
        self.lookup_hot_exact(pinyin_lower, true)
    }

    /// Looks up an exact partial-full/partial-initial key for a curated hot
    /// system phrase. The index is built only from explicit 2–4-syllable codes
    /// in Core and sufficiently frequent Base entries.
    pub(crate) fn lookup_mixed_hot(&self, mixed_lower: &str) -> Option<Arc<[ThuoclEntry]>> {
        {
            let runtime = self.runtime.borrow();
            if let Some(entries) = runtime.mixed_hot_exact.get(mixed_lower) {
                return Some(Arc::clone(entries));
            }
        }

        let compact = self.mixed_hot_inner.get(mixed_lower)?;
        let materialized = self.materialize_entries(&compact[..compact.len().min(HOT_EXACT_TOP_K)]);
        let mut runtime = self.runtime.borrow_mut();
        runtime.mixed_hot_exact.insert(
            mixed_lower.to_string(),
            Arc::clone(&materialized),
            HOT_EXACT_CACHE_MAX_KEYS,
        );
        Some(materialized)
    }

    pub fn prefix_flat(&self, prefix_lower: &str) -> Arc<[ThuoclEntry]> {
        if let Some(entries) = self.prefix_inner.get(prefix_lower) {
            return self.materialize_entries(entries);
        }
        collect_prefix_entries(self, &self.inner, prefix_lower)
    }

    pub fn prefix_pinyin_flat(&self, prefix_lower: &str) -> Arc<[ThuoclEntry]> {
        if let Some(entries) = self.pinyin_prefix_inner.get(prefix_lower) {
            return self.materialize_entries(entries);
        }
        collect_prefix_entries_with_extra(
            self,
            &self.pinyin_inner,
            &self.long_pinyin_hot_inner,
            prefix_lower,
        )
    }

    /// Scores a prebaked pinyin-prefix head without materializing phrase strings.
    /// Mixed-pinyin beam search calls this many times for one cold lookup, so
    /// avoiding repeated `ThuoclEntry` allocation keeps the first query bounded.
    pub(crate) fn pinyin_prefix_probability_score_until(
        &self,
        prefix_lower: &str,
        mut should_stop: impl FnMut() -> bool,
    ) -> Option<f64> {
        const DYNAMIC_PREFIX_KEY_SCAN_LIMIT: usize = 16;
        let mut dynamic = Vec::new();
        let entries = if let Some(entries) = self.pinyin_prefix_inner.get(prefix_lower) {
            entries.as_slice()
        } else {
            for (scan_index, (key, entries)) in self
                .pinyin_inner
                .range(prefix_lower.to_string()..)
                .take(DYNAMIC_PREFIX_KEY_SCAN_LIMIT)
                .enumerate()
            {
                if scan_index % 4 == 0 && should_stop() {
                    return None;
                }
                if !key.starts_with(prefix_lower) {
                    break;
                }
                merge_prefix_top(&mut dynamic, entries);
            }
            for (scan_index, (key, entries)) in self
                .long_pinyin_hot_inner
                .range(prefix_lower.to_string()..)
                .take(DYNAMIC_PREFIX_KEY_SCAN_LIMIT)
                .enumerate()
            {
                if scan_index % 4 == 0 && should_stop() {
                    return None;
                }
                if !key.starts_with(prefix_lower) {
                    break;
                }
                merge_prefix_top(&mut dynamic, entries);
            }
            dynamic.as_slice()
        };
        if should_stop() {
            return None;
        }
        Some(
            entries
                .iter()
                .take(10)
                .enumerate()
                .map(|(rank, entry)| {
                    let phrase_len = self
                        .resolve_phrase(entry.phrase_id)
                        .map(lexicon_phrase_char_count)
                        .unwrap_or(0)
                        .min(8);
                    (entry.freq as f64 + 1.0).ln() * 0.75 + phrase_len as f64 * 0.35
                        - rank as f64 * 0.18
                })
                .sum(),
        )
    }

    pub fn phrase_frequency(&self, phrase: &str) -> u64 {
        self.phrase_ids
            .as_ref()
            .and_then(|ids| ids.get(phrase))
            .and_then(|id| self.phrase_freq.get(*id as usize))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn phrase_id(&self, phrase: &str) -> Option<u32> {
        self.phrase_ids
            .as_ref()
            .and_then(|ids| ids.get(phrase))
            .copied()
    }

    pub fn phrase_layer(&self, phrase: &str) -> LexiconLayer {
        self.phrase_ids
            .as_ref()
            .and_then(|ids| ids.get(phrase))
            .and_then(|id| self.phrase_layer.get(*id as usize))
            .copied()
            .unwrap_or(LexiconLayer::Unknown)
    }

    fn phrase_ids_mut(&mut self) -> &mut HashMap<Arc<str>, u32> {
        if self.phrase_ids.is_none() {
            let mut ids = HashMap::with_capacity(self.phrases.len());
            for (idx, phrase) in self.phrases.iter().enumerate() {
                if let Ok(id) = u32::try_from(idx) {
                    ids.insert(Arc::clone(phrase), id);
                }
            }
            self.phrase_ids = Some(ids);
        }
        self.phrase_ids.as_mut().expect("phrase id index exists")
    }

    pub fn drop_phrase_id_index(&mut self) {
        self.phrase_ids = None;
    }

    pub fn has_phrase_id_index(&self) -> bool {
        self.phrase_ids.is_some()
    }

    pub fn approx_lookup(
        &self,
        abbrev_lower: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<ThuoclEntry> {
        self.approx_lookup_scored(abbrev_lower, max_distance, limit)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }

    /// Like `approx_lookup`, but also returns edit distance.
    pub fn approx_lookup_scored(
        &self,
        abbrev_lower: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<(ThuoclEntry, usize)> {
        self.approx_lookup_scored_until(abbrev_lower, max_distance, limit, || false)
    }

    pub(crate) fn approx_lookup_scored_until(
        &self,
        abbrev_lower: &str,
        max_distance: usize,
        limit: usize,
        mut should_stop: impl FnMut() -> bool,
    ) -> Vec<(ThuoclEntry, usize)> {
        let candidate_keys = self.candidate_keys_for_approx_until(
            false,
            abbrev_lower,
            max_distance,
            &mut should_stop,
        );
        approximate_lookup_scored_until(
            self,
            &self.inner,
            candidate_keys.as_deref(),
            abbrev_lower,
            max_distance,
            limit,
            &mut should_stop,
        )
    }

    pub fn approx_lookup_pinyin(
        &self,
        pinyin_lower: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<ThuoclEntry> {
        self.approx_lookup_pinyin_scored(pinyin_lower, max_distance, limit)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }

    pub fn approx_lookup_pinyin_scored(
        &self,
        pinyin_lower: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<(ThuoclEntry, usize)> {
        self.approx_lookup_pinyin_scored_until(pinyin_lower, max_distance, limit, || false)
    }

    pub(crate) fn approx_lookup_pinyin_scored_until(
        &self,
        pinyin_lower: &str,
        max_distance: usize,
        limit: usize,
        mut should_stop: impl FnMut() -> bool,
    ) -> Vec<(ThuoclEntry, usize)> {
        let candidate_keys = self.candidate_keys_for_approx_until(
            true,
            pinyin_lower,
            max_distance,
            &mut should_stop,
        );
        approximate_lookup_scored_until(
            self,
            &self.pinyin_inner,
            candidate_keys.as_deref(),
            pinyin_lower,
            max_distance,
            limit,
            &mut should_stop,
        )
    }

    /// Build (or explicitly disable for oversized maps) the approximate-match
    /// indexes before the first interactive typo lookup reaches this lexicon.
    pub fn warmup_approx_indexes(&self) {
        let _ = self.candidate_keys_for_approx_until(false, "a", 1, &mut || false);
        let _ = self.candidate_keys_for_approx_until(true, "a", 1, &mut || false);
    }

    pub fn merge_from(&mut self, other: &AbbrevLexicon) {
        let mut remapped_phrase_ids = Vec::with_capacity(other.phrases.len());
        for (idx, (phrase, freq)) in other
            .phrases
            .iter()
            .zip(other.phrase_freq.iter())
            .enumerate()
        {
            let source_layer = other.phrase_layer.get(idx).copied().unwrap_or_default();
            let _ = self.insert_parsed_with_layer(
                ThuoclEntry {
                    phrase: phrase.to_string(),
                    freq: *freq,
                    code: None,
                },
                source_layer,
            );
            remapped_phrase_ids.push(self.phrase_id(phrase));
        }
        for (key, entries) in &other.mixed_hot_inner {
            let target = self.mixed_hot_inner.entry(key.clone()).or_default();
            for entry in entries {
                let Some(Some(phrase_id)) = remapped_phrase_ids.get(entry.phrase_id as usize)
                else {
                    continue;
                };
                merge_compact_entry(target, *phrase_id, entry.freq);
            }
            trim_compact_entries(target, HOT_EXACT_TOP_K);
        }
        for (key, entries) in &other.long_pinyin_hot_inner {
            let target = self.long_pinyin_hot_inner.entry(key.clone()).or_default();
            for entry in entries {
                let Some(Some(phrase_id)) = remapped_phrase_ids.get(entry.phrase_id as usize)
                else {
                    continue;
                };
                merge_compact_entry(target, *phrase_id, entry.freq);
            }
            trim_compact_entries(target, LONG_PHRASE_HOT_TOP_K);
        }
        self.mark_runtime_dirty();
    }

    fn mark_runtime_dirty(&mut self) {
        self.prefix_inner.clear();
        self.pinyin_prefix_inner.clear();
        *self.runtime.borrow_mut() = RuntimeIndexes::default();
    }

    fn candidate_keys_for_approx_until(
        &self,
        pinyin: bool,
        key: &str,
        max_distance: usize,
        should_stop: &mut impl FnMut() -> bool,
    ) -> Option<Vec<String>> {
        if key.is_empty() || max_distance != 1 || key.len() > DELETION_INDEX_MAX_KEY_LEN {
            return None;
        }

        {
            let runtime = self.runtime.borrow();
            let slot = if pinyin {
                &runtime.pinyin_delete
            } else {
                &runtime.abbrev_delete
            };
            match slot {
                LazyDeletionIndex::Ready(index) => {
                    return index.candidate_keys(key, max_distance);
                }
                LazyDeletionIndex::Disabled => {
                    return Some(Vec::new());
                }
                LazyDeletionIndex::Uninitialized => {}
            }
        }

        let map = if pinyin {
            &self.pinyin_inner
        } else {
            &self.inner
        };
        if map.len() > DELETION_INDEX_MAX_KEYS_TO_BUILD {
            let mut runtime = self.runtime.borrow_mut();
            let slot = if pinyin {
                &mut runtime.pinyin_delete
            } else {
                &mut runtime.abbrev_delete
            };
            *slot = LazyDeletionIndex::Disabled;
            return Some(Vec::new());
        }

        let built = DeletionIndex::build_from_until(map, should_stop)?;
        let out = built.candidate_keys(key, max_distance);
        let mut runtime = self.runtime.borrow_mut();
        let slot = if pinyin {
            &mut runtime.pinyin_delete
        } else {
            &mut runtime.abbrev_delete
        };
        *slot = LazyDeletionIndex::Ready(built);
        out
    }

    fn intern_phrase(&mut self, phrase: &str, freq: u64, source_layer: LexiconLayer) -> u32 {
        if let Some(&id) = self.phrase_ids_mut().get(phrase) {
            if let Some(stored) = self.phrase_freq.get_mut(id as usize) {
                if freq > *stored {
                    *stored = freq;
                    if let Some(layer) = self.phrase_layer.get_mut(id as usize) {
                        if should_replace_phrase_layer(*layer, source_layer) {
                            *layer = source_layer;
                        }
                    }
                } else if freq == *stored {
                    if let Some(layer) = self.phrase_layer.get_mut(id as usize) {
                        if should_replace_phrase_layer(*layer, source_layer) {
                            *layer = source_layer;
                        }
                    }
                }
            }
            return id;
        }

        let phrase_arc: Arc<str> = Arc::from(phrase);
        let id = u32::try_from(self.phrases.len()).unwrap_or(u32::MAX);
        if id == u32::MAX {
            return id;
        }
        self.phrases.push(Arc::clone(&phrase_arc));
        self.phrase_freq.push(freq);
        self.phrase_layer.push(source_layer);
        self.phrase_ids_mut().insert(phrase_arc, id);
        id
    }

    fn resolve_phrase(&self, phrase_id: u32) -> Option<&str> {
        self.phrases
            .get(phrase_id as usize)
            .map(|phrase| phrase.as_ref())
    }

    fn materialize_entries(&self, entries: &[CompactEntry]) -> Arc<[ThuoclEntry]> {
        Arc::from(
            entries
                .iter()
                .filter_map(|entry| {
                    let phrase_id = entry.phrase_id as usize;
                    self.phrases.get(phrase_id).map(|phrase| ThuoclEntry {
                        phrase: phrase.to_string(),
                        freq: entry.freq,
                        code: None,
                    })
                })
                .collect::<Vec<_>>(),
        )
    }

    fn lookup_exact(&self, key: &str, pinyin: bool) -> Option<Arc<[ThuoclEntry]>> {
        {
            let runtime = self.runtime.borrow();
            let cache = if pinyin {
                &runtime.pinyin_exact
            } else {
                &runtime.abbrev_exact
            };
            if let Some(entries) = cache.get(key) {
                return Some(Arc::clone(entries));
            }
        }

        let materialized = if pinyin {
            let compact = self.merged_pinyin_entries(key, PREBAKED_ENTRIES_PER_KEY_TOP_K)?;
            self.materialize_entries(&compact)
        } else {
            let compact = self.inner.get(key)?;
            self.materialize_entries(compact)
        };
        let mut runtime = self.runtime.borrow_mut();
        let cache = if pinyin {
            &mut runtime.pinyin_exact
        } else {
            &mut runtime.abbrev_exact
        };
        cache.insert(
            key.to_string(),
            Arc::clone(&materialized),
            EXACT_CACHE_MAX_KEYS,
        );
        Some(materialized)
    }

    fn lookup_hot_exact(&self, key: &str, pinyin: bool) -> Option<Arc<[ThuoclEntry]>> {
        {
            let runtime = self.runtime.borrow();
            let cache = if pinyin {
                &runtime.hot_pinyin_exact
            } else {
                &runtime.hot_abbrev_exact
            };
            if let Some(entries) = cache.get(key) {
                return Some(Arc::clone(entries));
            }
        }

        let materialized = if pinyin {
            let compact = self.merged_pinyin_entries(key, HOT_EXACT_TOP_K)?;
            self.materialize_entries(&compact)
        } else {
            let compact = self.inner.get(key)?;
            self.materialize_entries(&compact[..compact.len().min(HOT_EXACT_TOP_K)])
        };
        let mut runtime = self.runtime.borrow_mut();
        let cache = if pinyin {
            &mut runtime.hot_pinyin_exact
        } else {
            &mut runtime.hot_abbrev_exact
        };
        cache.insert(
            key.to_string(),
            Arc::clone(&materialized),
            HOT_EXACT_CACHE_MAX_KEYS,
        );
        Some(materialized)
    }

    fn merged_pinyin_entries(&self, key: &str, limit: usize) -> Option<Vec<CompactEntry>> {
        let regular = self.pinyin_inner.get(key);
        let long_hot = self.long_pinyin_hot_inner.get(key);
        if regular.is_none() && long_hot.is_none() {
            return None;
        }

        let mut merged = Vec::with_capacity(limit.min(PREBAKED_ENTRIES_PER_KEY_TOP_K));
        let regular_limit = if long_hot.is_some() {
            limit.saturating_sub(LONG_PHRASE_HOT_TOP_K)
        } else {
            limit
        };
        if let Some(entries) = regular {
            for entry in entries.iter().take(regular_limit) {
                merge_compact_entry(&mut merged, entry.phrase_id, entry.freq);
            }
        }
        if let Some(entries) = long_hot {
            for entry in entries.iter().take(LONG_PHRASE_HOT_TOP_K.min(limit)) {
                merge_compact_entry(&mut merged, entry.phrase_id, entry.freq);
            }
        }
        trim_compact_entries(&mut merged, limit);
        Some(merged)
    }
}

fn should_index_mixed_hot(source_layer: LexiconLayer, freq: u64) -> bool {
    source_layer == LexiconLayer::Core
        || (source_layer == LexiconLayer::Base && freq >= MIXED_HOT_BASE_MIN_FREQ)
}

fn should_index_long_phrase_hot(
    source_layer: LexiconLayer,
    freq: u64,
    phrase: &str,
    code: Option<&str>,
) -> bool {
    let Some(code) = code else {
        return false;
    };
    if !matches!(source_layer, LexiconLayer::Core | LexiconLayer::Base)
        || !(LONG_PHRASE_HOT_MIN_CHARS..=LONG_PHRASE_HOT_MAX_CHARS)
            .contains(&lexicon_phrase_char_count(phrase))
        || explicit_pinyin_syllables(code).len() != lexicon_phrase_char_count(phrase)
    {
        return false;
    }
    source_layer == LexiconLayer::Core || freq >= LONG_PHRASE_HOT_MIN_FREQ
}

fn should_replace_phrase_layer(current: LexiconLayer, incoming: LexiconLayer) -> bool {
    incoming != LexiconLayer::Unknown
        && (current == LexiconLayer::Unknown || incoming.sort_order() < current.sort_order())
}

fn empty_arc_entries() -> Arc<[ThuoclEntry]> {
    Arc::from(Vec::<ThuoclEntry>::new())
}

fn collect_prefix_entries(
    lexicon: &AbbrevLexicon,
    map: &BTreeMap<String, Vec<CompactEntry>>,
    prefix_lower: &str,
) -> Arc<[ThuoclEntry]> {
    let top = collect_prefix_compact_entries(map, prefix_lower);
    if top.is_empty() {
        empty_arc_entries()
    } else {
        lexicon.materialize_entries(&top)
    }
}

fn collect_prefix_entries_with_extra(
    lexicon: &AbbrevLexicon,
    map: &BTreeMap<String, Vec<CompactEntry>>,
    extra: &BTreeMap<String, Vec<CompactEntry>>,
    prefix_lower: &str,
) -> Arc<[ThuoclEntry]> {
    let mut top = collect_prefix_compact_entries(map, prefix_lower);
    if prefix_lower.is_empty() {
        return empty_arc_entries();
    }
    let start = prefix_lower.to_string();
    let mut scanned_keys = 0usize;
    for (key, entries) in extra.range(start..) {
        if !key.starts_with(prefix_lower) {
            break;
        }
        merge_prefix_top(&mut top, entries);
        scanned_keys += 1;
        if scanned_keys >= PREFIX_SCAN_MAX_KEYS {
            break;
        }
    }
    if top.is_empty() {
        empty_arc_entries()
    } else {
        lexicon.materialize_entries(&top)
    }
}

fn collect_prefix_compact_entries(
    map: &BTreeMap<String, Vec<CompactEntry>>,
    prefix_lower: &str,
) -> Vec<CompactEntry> {
    if prefix_lower.is_empty() {
        return Vec::new();
    }

    let start = prefix_lower.to_string();
    let mut top = Vec::new();
    let mut scanned_keys = 0usize;
    for (key, entries) in map.range(start..) {
        if !key.starts_with(prefix_lower) {
            break;
        }
        merge_prefix_top(&mut top, entries);
        scanned_keys += 1;
        if scanned_keys >= PREFIX_SCAN_MAX_KEYS {
            break;
        }
    }

    top
}

fn approximate_lookup_scored_until(
    lexicon: &AbbrevLexicon,
    map: &BTreeMap<String, Vec<CompactEntry>>,
    candidate_keys: Option<&[String]>,
    key: &str,
    max_distance: usize,
    limit: usize,
    should_stop: &mut impl FnMut() -> bool,
) -> Vec<(ThuoclEntry, usize)> {
    if key.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut ranked: Vec<(usize, usize, CompactEntry)> = Vec::new();
    if let Some(candidate_keys) = candidate_keys {
        for (iteration, candidate_key) in candidate_keys.iter().enumerate() {
            if iteration % 64 == 0 && should_stop() {
                break;
            }
            let Some(entries) = map.get(candidate_key) else {
                continue;
            };
            if candidate_key == key {
                continue;
            }
            let Some(distance) = bounded_edit_distance(candidate_key, key, max_distance) else {
                continue;
            };
            for (rank, entry) in entries.iter().take(3).enumerate() {
                ranked.push((distance, rank, *entry));
            }
        }
    } else {
        for (iteration, (candidate_key, entries)) in map.iter().enumerate() {
            if iteration % 64 == 0 && should_stop() {
                break;
            }
            if candidate_key == key {
                continue;
            }
            let Some(distance) = bounded_edit_distance(candidate_key, key, max_distance) else {
                continue;
            };
            for (rank, entry) in entries.iter().take(3).enumerate() {
                ranked.push((distance, rank, *entry));
            }
        }
    }

    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.2.freq.cmp(&a.2.freq))
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.phrase_id.cmp(&b.2.phrase_id))
    });

    let mut out: Vec<(ThuoclEntry, usize)> = Vec::new();
    let mut seen = HashSet::new();
    for (dist, _, entry) in ranked {
        if !seen.insert(entry.phrase_id) {
            continue;
        }
        let Some(phrase) = lexicon.resolve_phrase(entry.phrase_id) else {
            continue;
        };
        out.push((
            ThuoclEntry {
                phrase: phrase.to_string(),
                freq: entry.freq,
                code: None,
            },
            dist,
        ));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn bounded_edit_distance(left: &str, right: &str, max_distance: usize) -> Option<usize> {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    if a.len().abs_diff(b.len()) > max_distance {
        return None;
    }

    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        let mut row_min = curr[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let value = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
            curr[j + 1] = value;
            row_min = row_min.min(value);
        }
        if row_min > max_distance {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let distance = prev[b.len()];
    (distance <= max_distance).then_some(distance)
}

fn should_skip_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| matches!(name, "lua" | "opencc" | "en_dicts" | ".git"))
        .unwrap_or(false)
}

fn should_include_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let is_txt = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("txt"))
        .unwrap_or(false);
    if !is_txt {
        return false;
    }
    let normalized = file_name.to_ascii_lowercase();
    !normalized.starts_with("readme") && !normalized.ends_with("_纯名单.txt")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LexiconSourceGroup {
    Prebaked,
    Core,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconInventoryEntry {
    pub group: LexiconSourceGroup,
    pub path: PathBuf,
    pub thuocl_tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconInventory {
    pub primary_dir: PathBuf,
    pub full_dir: Option<PathBuf>,
    pub entries: Vec<LexiconInventoryEntry>,
}

fn collect_supported_files_with_profile(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    profile: LexiconBuildProfile,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_supported_files_with_profile(&path, out, profile)?;
            continue;
        }

        if should_include_file(&path) && profile.includes_path(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_supported_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    collect_supported_files_with_profile(dir, out, LexiconBuildProfile::Full)
}

fn lexicon_layer_sort_key(path: &Path) -> (u8, String) {
    (
        LexiconLayer::from_path(path).sort_order(),
        path.to_string_lossy().to_ascii_lowercase(),
    )
}

fn collect_inventory_entries(
    dir: &Path,
    group: LexiconSourceGroup,
    out: &mut Vec<LexiconInventoryEntry>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_inventory_entries(&path, group, out)?;
            continue;
        }

        if !should_include_file(&path) {
            continue;
        }

        out.push(LexiconInventoryEntry {
            group,
            thuocl_tag: crate::lexicon_prefs::optional_lexicon_path_tag(&path),
            path,
        });
    }
    Ok(())
}

/// List lexicon sources in the same order as the runtime loader.
pub fn discover_lexicon_inventory(dir: &Path) -> io::Result<LexiconInventory> {
    let mut entries = Vec::new();
    let primary_dir = dir.to_path_buf();
    let mut prebaked = prebaked_lexicon_candidate_paths(dir)
        .into_iter()
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    prebaked.sort();
    prebaked.dedup();
    for path in prebaked {
        entries.push(LexiconInventoryEntry {
            group: LexiconSourceGroup::Prebaked,
            thuocl_tag: None,
            path,
        });
    }

    collect_inventory_entries(dir, LexiconSourceGroup::Core, &mut entries)?;

    entries.sort_by(|lhs, rhs| {
        lhs.group
            .cmp(&rhs.group)
            .then_with(|| lexicon_layer_sort_key(&lhs.path).cmp(&lexicon_layer_sort_key(&rhs.path)))
    });
    Ok(LexiconInventory {
        primary_dir,
        full_dir: None,
        entries,
    })
}

fn read_u32<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u8<R: Read>(reader: &mut R) -> io::Result<u8> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

fn read_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_u32<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u8<W: Write>(writer: &mut W, value: u8) -> io::Result<()> {
    writer.write_all(&[value])
}

fn write_u64<W: Write>(writer: &mut W, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_count<W: Write>(writer: &mut W, count: usize) -> io::Result<()> {
    let value = u32::try_from(count)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "count does not fit into u32"))?;
    write_u32(writer, value)
}

fn read_string<R: Read>(reader: &mut R) -> io::Result<String> {
    let len = read_u32(reader)? as usize;
    if len > PREBAKED_MAX_STRING_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked string exceeds max length",
        ));
    }
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("invalid utf-8: {err}")))
}

fn write_string<W: Write>(writer: &mut W, value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    write_count(writer, bytes.len())?;
    writer.write_all(bytes)
}

fn build_prebaked_prefix_index(
    map: &BTreeMap<String, Vec<CompactEntry>>,
) -> HashMap<String, Vec<CompactEntry>> {
    if map.len() < PREBAKED_PREFIX_MIN_KEYS {
        return HashMap::new();
    }
    let keys = map.keys().map(String::as_str).collect::<Vec<_>>();
    let mut qualifying = HashSet::new();
    for start in 0..=keys.len() - PREBAKED_PREFIX_MIN_KEYS {
        let left = keys[start].as_bytes();
        let right = keys[start + PREBAKED_PREFIX_MIN_KEYS - 1].as_bytes();
        let lcp = left
            .iter()
            .zip(right.iter())
            .take_while(|(a, b)| a == b)
            .count();
        for len in 1..=lcp {
            if keys[start].is_char_boundary(len) {
                qualifying.insert(keys[start][..len].to_string());
            }
        }
    }

    let mut out = HashMap::with_capacity(qualifying.len());
    for prefix in qualifying {
        let mut top = Vec::new();
        for (_key, entries) in map
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .take(PREFIX_SCAN_MAX_KEYS)
        {
            merge_prefix_top(&mut top, entries);
        }
        if !top.is_empty() {
            out.insert(prefix, top);
        }
    }
    out
}

fn extend_prefix_index_with_long_hot(
    index: &mut HashMap<String, Vec<CompactEntry>>,
    long_hot: &BTreeMap<String, Vec<CompactEntry>>,
) {
    for (key, entries) in long_hot {
        for (start, ch) in key.char_indices() {
            let end = start + ch.len_utf8();
            let prefix = key[..end].to_string();
            merge_prefix_top(index.entry(prefix).or_default(), entries);
        }
    }
}

fn write_prefix_map<W: Write>(
    writer: &mut W,
    map: &HashMap<String, Vec<CompactEntry>>,
) -> io::Result<()> {
    write_count(writer, map.len())?;
    let mut keys = map.keys().collect::<Vec<_>>();
    keys.sort_unstable();
    for key in keys {
        write_string(writer, key)?;
        let entries = &map[key];
        write_count(writer, entries.len())?;
        for entry in entries {
            write_u32(writer, entry.phrase_id)?;
        }
    }
    Ok(())
}

fn read_prefix_map<R: Read>(
    reader: &mut R,
    phrases: &[Arc<str>],
    phrase_freq: &[u64],
) -> io::Result<HashMap<String, Vec<CompactEntry>>> {
    let count = read_u32(reader)? as usize;
    if count > PREBAKED_MAX_MAP_KEYS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked prefix map exceeds max",
        ));
    }
    let mut map = HashMap::with_capacity(count);
    for _ in 0..count {
        let key = read_string(reader)?;
        let entry_count = read_u32(reader)? as usize;
        if entry_count > PREFIX_INDEX_TOP_K {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prebaked prefix list exceeds top-k",
            ));
        }
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let id = read_u32(reader)? as usize;
            let Some(&freq) = phrase_freq.get(id) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prebaked prefix phrase id out of range",
                ));
            };
            if phrases.get(id).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prebaked prefix phrase missing",
                ));
            }
            entries.push(CompactEntry {
                phrase_id: id as u32,
                freq,
            });
        }
        map.insert(key, entries);
    }
    Ok(map)
}

fn write_compact_prebaked<W: Write>(writer: &mut W, lexicon: &AbbrevLexicon) -> io::Result<()> {
    write_compact_prebaked_v7_fields(writer, lexicon)?;
    write_compact_map(writer, &lexicon.mixed_hot_inner)?;
    write_compact_map(writer, &lexicon.long_pinyin_hot_inner)?;
    Ok(())
}

/// Fields shared with schema v7. Kept separate so compatibility tests can
/// prove that current readers still accept already-packaged v7 dictionaries.
fn write_compact_prebaked_v7_fields<W: Write>(
    writer: &mut W,
    lexicon: &AbbrevLexicon,
) -> io::Result<()> {
    write_count(writer, lexicon.phrases.len())?;
    for (idx, (phrase, freq)) in lexicon
        .phrases
        .iter()
        .zip(lexicon.phrase_freq.iter())
        .enumerate()
    {
        write_string(writer, phrase)?;
        write_u64(writer, *freq)?;
        write_u8(
            writer,
            lexicon
                .phrase_layer
                .get(idx)
                .copied()
                .unwrap_or_default()
                .to_u8(),
        )?;
    }

    write_compact_map(writer, &lexicon.inner)?;
    write_compact_map(writer, &lexicon.pinyin_inner)?;
    let prefix_inner = build_prebaked_prefix_index(&lexicon.inner);
    let mut pinyin_prefix_inner = build_prebaked_prefix_index(&lexicon.pinyin_inner);
    extend_prefix_index_with_long_hot(&mut pinyin_prefix_inner, &lexicon.long_pinyin_hot_inner);
    write_prefix_map(writer, &prefix_inner)?;
    write_prefix_map(writer, &pinyin_prefix_inner)?;
    write_deletion_index(writer, &lexicon.inner)?;
    write_deletion_index(writer, &lexicon.pinyin_inner)?;
    Ok(())
}

fn write_compact_map<W: Write>(
    writer: &mut W,
    map: &BTreeMap<String, Vec<CompactEntry>>,
) -> io::Result<()> {
    write_count(writer, map.len())?;
    for (key, entries) in map {
        write_string(writer, key)?;
        let kept = entries.len().min(PREBAKED_ENTRIES_PER_KEY_TOP_K);
        write_count(writer, kept)?;
        for entry in entries.iter().take(kept) {
            write_u32(writer, entry.phrase_id)?;
        }
    }
    Ok(())
}

fn read_compact_prebaked<R: Read>(reader: &mut R, version: u32) -> io::Result<AbbrevLexicon> {
    let phrase_count = read_u32(reader)? as usize;
    if phrase_count > PREBAKED_MAX_MAP_KEYS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked phrase table count exceeds max",
        ));
    }

    let mut phrases: Vec<Arc<str>> = Vec::with_capacity(phrase_count);
    let mut phrase_freq = Vec::with_capacity(phrase_count);
    let mut phrase_layer = Vec::with_capacity(phrase_count);
    for _ in 0..phrase_count {
        let phrase: Arc<str> = Arc::from(read_string(reader)?);
        let freq = read_u64(reader)?;
        let source_layer = if version >= 5 {
            LexiconLayer::from_u8(read_u8(reader)?)
        } else {
            LexiconLayer::Unknown
        };
        let _phrase_id = u32::try_from(phrases.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "prebaked phrase table index exceeds u32",
            )
        })?;
        phrases.push(phrase);
        phrase_freq.push(freq);
        phrase_layer.push(source_layer);
    }

    let inner = read_compact_map(reader, &phrases, &phrase_freq)?;
    let pinyin_inner = read_compact_map(reader, &phrases, &phrase_freq)?;
    let (prefix_inner, mut pinyin_prefix_inner) = if version >= 6 {
        (
            read_prefix_map(reader, &phrases, &phrase_freq)?,
            read_prefix_map(reader, &phrases, &phrase_freq)?,
        )
    } else {
        (HashMap::new(), HashMap::new())
    };

    let runtime = if version >= 7 {
        RuntimeIndexes {
            abbrev_delete: read_deletion_index(reader)?,
            pinyin_delete: read_deletion_index(reader)?,
            ..RuntimeIndexes::default()
        }
    } else {
        RuntimeIndexes::default()
    };
    let mixed_hot_inner = if version >= 8 {
        read_compact_map(reader, &phrases, &phrase_freq)?
    } else {
        BTreeMap::new()
    };
    let long_pinyin_hot_inner = if version >= 9 {
        read_compact_map(reader, &phrases, &phrase_freq)?
    } else {
        BTreeMap::new()
    };
    extend_prefix_index_with_long_hot(&mut pinyin_prefix_inner, &long_pinyin_hot_inner);

    Ok(AbbrevLexicon {
        inner,
        pinyin_inner,
        long_pinyin_hot_inner,
        mixed_hot_inner,
        prefix_inner,
        pinyin_prefix_inner,
        phrase_ids: Some(build_phrase_id_index(&phrases)),
        phrases,
        phrase_freq,
        phrase_layer,
        runtime: RefCell::new(runtime),
        mixed_hot_base_skipped: 0,
    })
}

fn build_phrase_id_index(phrases: &[Arc<str>]) -> HashMap<Arc<str>, u32> {
    let mut ids = HashMap::with_capacity(phrases.len());
    for (idx, phrase) in phrases.iter().enumerate() {
        if let Ok(id) = u32::try_from(idx) {
            ids.insert(Arc::clone(phrase), id);
        }
    }
    ids
}

fn read_compact_map<R: Read>(
    reader: &mut R,
    phrases: &[Arc<str>],
    phrase_freq: &[u64],
) -> io::Result<BTreeMap<String, Vec<CompactEntry>>> {
    let count = read_u32(reader)? as usize;
    if count > PREBAKED_MAX_MAP_KEYS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prebaked compact map key count exceeds max",
        ));
    }
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let key = read_string(reader)?;
        let entry_count = read_u32(reader)? as usize;
        if entry_count > PREBAKED_MAX_ENTRIES_PER_KEY {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prebaked compact entry list exceeds max length",
            ));
        }
        let keep_count = entry_count.min(PREBAKED_ENTRIES_PER_KEY_TOP_K);
        let mut entries = Vec::with_capacity(keep_count);
        for idx in 0..entry_count {
            let id = read_u32(reader)? as usize;
            let Some(_phrase) = phrases.get(id) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prebaked compact phrase id out of range",
                ));
            };
            let Some(&freq) = phrase_freq.get(id) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "prebaked compact phrase freq out of range",
                ));
            };
            if idx < keep_count {
                entries.push(CompactEntry {
                    phrase_id: id as u32,
                    freq,
                });
            }
        }
        map.insert(key, entries);
    }
    Ok(map)
}

/// Write a merged `lexicon.bin` (SRFLX002) for fast runtime loading.
pub fn save_prebaked_lexicon(path: &Path, lexicon: &AbbrevLexicon) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_file = path.with_extension("tmp");
    {
        let file = fs::File::create(&temp_file)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(PREBAKED_MAGIC)?;
        write_u32(&mut writer, PREBAKED_SCHEMA_VERSION)?;
        write_compact_prebaked(&mut writer, lexicon)?;
        writer.flush()?;
    }
    let _ = fs::remove_file(path);
    fs::rename(temp_file, path)?;
    Ok(())
}

pub fn load_prebaked_lexicon(path: &Path) -> io::Result<AbbrevLexicon> {
    let file = fs::File::open(path)?;
    // Mapping keeps the 300MB prebaked file in the OS page cache instead of
    // staging a separate user-space read buffer during cold start.
    if let Ok(mmap) = unsafe { memmap2::Mmap::map(&file) } {
        let mut reader = Cursor::new(&mmap[..]);
        return read_prebaked_lexicon(&mut reader);
    }
    let mut reader = BufReader::new(file);
    read_prebaked_lexicon(&mut reader)
}

fn read_prebaked_lexicon<R: Read>(reader: &mut R) -> io::Result<AbbrevLexicon> {
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != PREBAKED_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected SRFLX002 prebaked lexicon",
        ));
    }
    let version = read_u32(reader)?;
    if !(PREBAKED_MIN_SUPPORTED_SCHEMA_VERSION..=PREBAKED_SCHEMA_VERSION).contains(&version) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported SRFLX002 schema version",
        ));
    }
    read_compact_prebaked(reader, version)
}

fn prebaked_lexicon_candidate_paths(dir: &Path) -> Vec<PathBuf> {
    prebaked_lexicon_candidate_paths_for(dir, "lexicon.bin")
}

fn prebaked_lexicon_candidate_paths_for(dir: &Path, file_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    out.push(dir.join(file_name));
    if let Some(parent) = dir.parent() {
        out.push(parent.join(file_name));
    }
    out
}

/// Find a nearby `lexicon.bin` (SRFLX002).
pub fn try_load_prebaked_near(dir: &Path) -> io::Result<Option<AbbrevLexicon>> {
    try_load_prebaked_named_near(dir, "lexicon.bin")
}

/// Find a nearby `hot_lexicon.bin` (SRFLX002).
pub fn try_load_hot_prebaked_near(dir: &Path) -> io::Result<Option<AbbrevLexicon>> {
    try_load_prebaked_named_near(dir, "hot_lexicon.bin")
}

fn try_load_prebaked_named_near(dir: &Path, file_name: &str) -> io::Result<Option<AbbrevLexicon>> {
    for path in prebaked_lexicon_candidate_paths_for(dir, file_name) {
        if !path.is_file() {
            continue;
        }
        match load_prebaked_lexicon(&path) {
            Ok(lex) => return Ok(Some(lex)),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

fn load_paths(
    paths: &[PathBuf],
    profile: LexiconBuildProfile,
) -> io::Result<(AbbrevLexicon, LoadReport)> {
    let mut lex = AbbrevLexicon::default();
    let mut report = LoadReport::default();
    let calibrators = build_frequency_calibrators(paths, profile);
    // Track the first file where each phrase was seen: phrase -> (path, code, freq)
    let mut phrase_first_seen: HashMap<String, (PathBuf, Option<String>, u64)> = HashMap::new();

    for path in paths {
        let text = match read_text_file(path) {
            Ok(t) => t,
            Err(e) => {
                report.files_failed.push((path.clone(), e.to_string()));
                continue;
            }
        };
        report.files_read += 1;
        for (line_no, line) in text.lines().enumerate() {
            report.lines_total += 1;
            let parsed = parse_lexicon_entry_line(line);
            // Detect duplicates across different files before indexing
            if let Some(ref entry) = parsed {
                if let Some((prev_path, prev_code, prev_freq)) =
                    phrase_first_seen.get(&entry.phrase)
                {
                    if prev_path != path {
                        let kind = classify_duplicate(
                            prev_code.as_deref(),
                            *prev_freq,
                            entry.code.as_deref(),
                            entry.freq,
                            LexiconLayer::from_path(prev_path),
                            LexiconLayer::from_path(path),
                        );
                        report.duplicates.push(DuplicateEntryReport {
                            phrase: entry.phrase.clone(),
                            file1: prev_path.clone(),
                            pinyin1: prev_code.clone(),
                            freq1: *prev_freq,
                            file2: path.clone(),
                            pinyin2: entry.code.clone(),
                            freq2: entry.freq,
                            conflict_kind: kind,
                        });
                    }
                } else {
                    phrase_first_seen.insert(
                        entry.phrase.clone(),
                        (path.clone(), entry.code.clone(), entry.freq),
                    );
                }
            }
            index_entry(
                &mut lex,
                &mut report,
                profile,
                path,
                calibrators.get(path),
                line_no + 1,
                line,
                parsed,
            );
        }
    }

    // TXT loading is the fallback used when optional ext preferences are
    // customized. Build the same bounded prefix heads that are embedded in a
    // prebaked lexicon, so the first lookup after a reload does not scan the
    // full BTreeMap for every newly typed prefix.
    lex.prefix_inner = build_prebaked_prefix_index(&lex.inner);
    lex.pinyin_prefix_inner = build_prebaked_prefix_index(&lex.pinyin_inner);
    extend_prefix_index_with_long_hot(&mut lex.pinyin_prefix_inner, &lex.long_pinyin_hot_inner);

    report.mixed_hot_base_skipped = lex.mixed_hot_base_skipped;
    Ok((lex, report))
}

/// Classify the severity of a duplicate entry found in two different source files.
fn classify_duplicate(
    code1: Option<&str>,
    freq1: u64,
    code2: Option<&str>,
    freq2: u64,
    layer1: LexiconLayer,
    layer2: LexiconLayer,
) -> DuplicateConflictKind {
    // Pinyin mismatch: both entries have explicit codes that differ
    if let (Some(c1), Some(c2)) = (code1, code2) {
        let norm1 = normalize_pinyin_code(c1);
        let norm2 = normalize_pinyin_code(c2);
        if norm1.is_some() && norm2.is_some() && norm1 != norm2 {
            return DuplicateConflictKind::PinyinMismatch;
        }
    }
    // Weight divergence: one weight is >10x the other
    let ratio = if freq1 >= freq2 {
        freq1 as f64 / freq2.max(1) as f64
    } else {
        freq2 as f64 / freq1.max(1) as f64
    };
    if ratio > 10.0 {
        return DuplicateConflictKind::WeightDivergent;
    }
    // Layer conflict: different priority layers disagree on the same phrase
    if layer1.sort_order() != layer2.sort_order()
        && layer1 != LexiconLayer::Unknown
        && layer2 != LexiconLayer::Unknown
    {
        return DuplicateConflictKind::LayerConflict;
    }
    DuplicateConflictKind::LayerConflict // benign: same layer, same code, similar weight
}

/// Normalizes auxiliary-dictionary frequencies to the distribution used by the
/// base dictionary for the same phrase length.  Several bundled dictionaries
/// use hand-authored weights rather than corpus counts; comparing those raw
/// values directly would otherwise make their source-specific offsets rank
/// above the base corpus.
#[derive(Debug)]
struct SourceFrequencyCalibrator {
    source_by_len: HashMap<usize, Vec<u64>>,
    reference_by_len: HashMap<usize, Vec<u64>>,
}

impl SourceFrequencyCalibrator {
    fn normalize(&self, phrase: &str, raw: u64) -> u64 {
        let len = lexicon_phrase_char_count(phrase);
        let Some(source) = self.source_by_len.get(&len) else {
            return raw.max(1);
        };
        let Some(reference) = self.reference_by_len.get(&len) else {
            return raw.max(1);
        };
        if source.is_empty() || reference.is_empty() {
            return raw.max(1);
        }

        // Use the middle rank for tied hand-authored weights.  This prevents a
        // large flat dictionary from being treated as either all rare or all
        // extremely common entries.
        let lower = source.partition_point(|&weight| weight < raw);
        let upper = source.partition_point(|&weight| weight <= raw);
        let rank = (lower + upper.saturating_sub(lower).saturating_sub(1) / 2)
            .min(source.len().saturating_sub(1));
        let index = if source.len() <= 1 || reference.len() <= 1 {
            0
        } else {
            rank.saturating_mul(reference.len() - 1) / (source.len() - 1)
        };
        reference[index.min(reference.len().saturating_sub(1))].max(1)
    }
}

fn is_frequency_reference_source(path: &Path) -> bool {
    LexiconLayer::from_path(path) == LexiconLayer::Base
}

fn is_absolute_priority_source(path: &Path) -> bool {
    LexiconLayer::from_path(path) == LexiconLayer::Core
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("kaixin_polyphone.txt")
                    || name.eq_ignore_ascii_case("kaixin_pronunciation_aliases.txt")
            })
}

fn is_unscaled_source(path: &Path) -> bool {
    // Names and local places are deliberately low-priority recall sources.
    // Their lists use flat low weights, so percentile calibration would
    // promote them into the same range as everyday words.
    is_absolute_priority_source(path)
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("people_names.txt")
                    || name.eq_ignore_ascii_case("hangzhou_local.txt")
            })
}

fn should_calibrate_source_frequency(path: &Path) -> bool {
    !is_frequency_reference_source(path) && !is_unscaled_source(path)
}

fn collect_frequency_values(
    path: &Path,
    profile: LexiconBuildProfile,
    values: &mut HashMap<usize, Vec<u64>>,
) {
    let Ok(text) = read_text_file(path) else {
        return;
    };
    for line in text.lines() {
        let Some(entry) = parse_lexicon_entry_line(line) else {
            continue;
        };
        if !profile.includes_entry(path, &entry.phrase) || !phrase_contains_cjk(&entry.phrase) {
            continue;
        }
        values
            .entry(lexicon_phrase_char_count(&entry.phrase))
            .or_default()
            .push(entry.freq.max(1));
    }
}

fn build_frequency_calibrators(
    paths: &[PathBuf],
    profile: LexiconBuildProfile,
) -> HashMap<PathBuf, SourceFrequencyCalibrator> {
    let mut reference_by_len = HashMap::new();
    for path in paths
        .iter()
        .filter(|path| is_frequency_reference_source(path))
    {
        collect_frequency_values(path, profile, &mut reference_by_len);
    }
    for values in reference_by_len.values_mut() {
        values.sort_unstable();
    }

    let mut out = HashMap::new();
    for path in paths
        .iter()
        .filter(|path| should_calibrate_source_frequency(path))
    {
        let mut source_by_len = HashMap::new();
        collect_frequency_values(path, profile, &mut source_by_len);
        if source_by_len.is_empty() {
            continue;
        }
        for values in source_by_len.values_mut() {
            values.sort_unstable();
        }
        out.insert(
            path.clone(),
            SourceFrequencyCalibrator {
                source_by_len,
                reference_by_len: reference_by_len.clone(),
            },
        );
    }
    out
}

fn index_entry(
    lex: &mut AbbrevLexicon,
    report: &mut LoadReport,
    profile: LexiconBuildProfile,
    path: &Path,
    calibrator: Option<&SourceFrequencyCalibrator>,
    line_no: usize,
    line: &str,
    entry: Option<ThuoclEntry>,
) {
    let Some(mut entry) = entry else {
        return;
    };
    if !profile.includes_entry(path, &entry.phrase) {
        return;
    }
    entry.freq = calibrator
        .map(|calibrator| calibrator.normalize(&entry.phrase, entry.freq.max(1)))
        .unwrap_or_else(|| entry.freq.max(1));
    if lex.insert_parsed_with_layer(entry, LexiconLayer::from_path(path)) {
        report.entries_indexed += 1;
    } else {
        report.lines_skipped_no_abbrev += 1;
        if report.sample_skipped_lines.len() < 5 {
            report.sample_skipped_lines.push((
                path.to_path_buf(),
                line_no,
                line.chars().take(80).collect(),
            ));
        }
    }
}

pub fn load_dir_txt(dir: &Path) -> io::Result<(AbbrevLexicon, LoadReport)> {
    let mut paths = Vec::new();
    collect_supported_files(dir, &mut paths)?;
    paths.sort_by_key(|path| lexicon_layer_sort_key(path));
    load_paths(&paths, LexiconBuildProfile::Full)
}

pub fn load_dir_txt_with_profile(
    dir: &Path,
    profile: LexiconBuildProfile,
) -> io::Result<(AbbrevLexicon, LoadReport)> {
    let mut paths = Vec::new();
    collect_supported_files_with_profile(dir, &mut paths, profile)?;
    paths.sort_by_key(|path| lexicon_layer_sort_key(path));
    load_paths(&paths, profile)
}

pub fn load_dir_txt_with_profile_default_enabled(
    dir: &Path,
    profile: LexiconBuildProfile,
) -> io::Result<(AbbrevLexicon, LoadReport)> {
    let mut paths = Vec::new();
    collect_supported_files_with_profile(dir, &mut paths, profile)?;
    crate::lexicon_prefs::filter_optional_lexicon_paths_by_default(&mut paths);
    paths.sort_by_key(|path| lexicon_layer_sort_key(path));
    load_paths(&paths, profile)
}

pub fn load_dir_txt_with_profile_filtered_by_prefs(
    dir: &Path,
    profile: LexiconBuildProfile,
) -> io::Result<(AbbrevLexicon, LoadReport)> {
    let mut paths = Vec::new();
    collect_supported_files_with_profile(dir, &mut paths, profile)?;
    crate::lexicon_prefs::filter_optional_lexicon_paths_by_prefs(&mut paths);
    paths.sort_by_key(|path| lexicon_layer_sort_key(path));
    load_paths(&paths, profile)
}

impl LoadReport {
    /// Write duplicate entries as CSV to `path`. Returns the number of rows written.
    pub fn write_duplicate_csv(&self, path: &Path) -> io::Result<usize> {
        let mut w = BufWriter::new(fs::File::create(path)?);
        writeln!(
            w,
            "phrase,file1,pinyin1,freq1,file2,pinyin2,freq2,conflict_kind"
        )?;
        for d in &self.duplicates {
            writeln!(
                w,
                "{},{},{},{},{},{},{},{}",
                csv_escape(&d.phrase),
                csv_escape(&d.file1.to_string_lossy()),
                csv_escape(d.pinyin1.as_deref().unwrap_or("")),
                d.freq1,
                csv_escape(&d.file2.to_string_lossy()),
                csv_escape(d.pinyin2.as_deref().unwrap_or("")),
                d.freq2,
                conflict_kind_label(d.conflict_kind),
            )?;
        }
        Ok(self.duplicates.len())
    }

    /// Returns true if any duplicate has a pinyin mismatch — this should fail CI.
    pub fn has_pinyin_conflicts(&self) -> bool {
        self.duplicates
            .iter()
            .any(|d| d.conflict_kind == DuplicateConflictKind::PinyinMismatch)
    }

    pub fn pinyin_conflict_count(&self) -> usize {
        self.duplicates
            .iter()
            .filter(|d| d.conflict_kind == DuplicateConflictKind::PinyinMismatch)
            .count()
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn conflict_kind_label(kind: DuplicateConflictKind) -> &'static str {
    match kind {
        DuplicateConflictKind::PinyinMismatch => "拼音不一致",
        DuplicateConflictKind::WeightDivergent => "权重相差>10x",
        DuplicateConflictKind::LayerConflict => "层级冲突",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateConflictKind {
    PinyinMismatch,
    WeightDivergent,
    LayerConflict,
}

#[derive(Debug, Clone)]
pub struct DuplicateEntryReport {
    pub phrase: String,
    pub file1: PathBuf,
    pub pinyin1: Option<String>,
    pub freq1: u64,
    pub file2: PathBuf,
    pub pinyin2: Option<String>,
    pub freq2: u64,
    pub conflict_kind: DuplicateConflictKind,
}

#[derive(Debug, Default)]
pub struct LoadReport {
    pub files_read: usize,
    pub files_failed: Vec<(PathBuf, String)>,
    pub lines_total: usize,
    pub entries_indexed: usize,
    pub lines_skipped_no_abbrev: usize,
    pub sample_skipped_lines: Vec<(PathBuf, usize, String)>,
    pub duplicates: Vec<DuplicateEntryReport>,
    pub mixed_hot_base_skipped: usize,
}

/// Verify that every phrase in the `hot` lexicon exists in the `full` lexicon
/// with the same frequency and a compatible source layer.  Returns the list of
/// discrepancies (empty = valid).
pub fn validate_hot_subset(hot: &AbbrevLexicon, full: &AbbrevLexicon) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (idx, phrase) in hot.phrases.iter().enumerate() {
        let hot_freq = hot.phrase_freq.get(idx).copied().unwrap_or(0);
        let hot_layer = hot.phrase_layer.get(idx).copied().unwrap_or_default();
        let full_freq = full.phrase_frequency(phrase);
        let full_layer = full.phrase_layer(phrase);
        if full_freq == 0 {
            mismatches.push(format!(
                "hot phrase '{}' (freq={}, layer={:?}) not in full lexicon",
                phrase, hot_freq, hot_layer,
            ));
        } else if full_freq != hot_freq {
            mismatches.push(format!(
                "hot phrase '{}' freq mismatch: hot={}, full={}",
                phrase, hot_freq, full_freq,
            ));
        }
        if full_layer.sort_order() > hot_layer.sort_order()
            && hot_layer != LexiconLayer::Unknown
            && full_layer != LexiconLayer::Unknown
        {
            mismatches.push(format!(
                "hot phrase '{}' layer downgrade: hot={:?}, full={:?}",
                phrase, hot_layer, full_layer,
            ));
        }
    }
    mismatches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatched_explicit_pinyin_falls_back_to_character_readings() {
        let mut lexicon = AbbrevLexicon::default();
        lexicon.insert_parsed(ThuoclEntry {
            phrase: "布洛芬".to_string(),
            // A shifted neighbouring row: four syllables for a three-char word.
            code: Some("jia xiao zuo pian".to_string()),
            freq: 5_000,
        });

        assert!(lexicon
            .lookup_pinyin("buluofen")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == "布洛芬")));
        assert!(lexicon.lookup_pinyin("jiaxiaozuopian").is_none());
    }
}
