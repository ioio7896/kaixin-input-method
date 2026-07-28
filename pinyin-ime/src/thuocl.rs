//! Native text lexicon loader and prebaked `lexicon.bin` support.

use crate::text_encoding::read_text_file;
use pinyin::ToPinyinMulti;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Packaged lexicon without a source-path digest.
const PREBAKED_MAGIC: &[u8; 8] = b"SRFLX002";
const PREBAKED_SCHEMA_VERSION: u32 = 7;
const PREBAKED_MIN_SUPPORTED_SCHEMA_VERSION: u32 = 5;

const PREBAKED_MAX_STRING_BYTES: usize = 16_384;
const PREBAKED_MAX_MAP_KEYS: usize = 12_000_000;
const PREBAKED_MAX_ENTRIES_PER_KEY: usize = 65_536;
const PREBAKED_MAX_DELETION_REFERENCES: usize = 24_000_000;
const PREFIX_INDEX_TOP_K: usize = 48;
const PREFIX_SCAN_MAX_KEYS: usize = 4096;
const PREBAKED_PREFIX_MIN_KEYS: usize = 32;
const DELETION_INDEX_MAX_KEY_LEN: usize = 40;
const DELETION_INDEX_MAX_KEYS_TO_BUILD: usize = 250_000;
const MAX_HETERONYM_KEYS_PER_PHRASE: usize = 64;
const MAX_MIXED_KEYS_PER_PHRASE: usize = 128;
const MAX_MIXED_KEY_PHRASE_CHARS: usize = 4;
const PREBAKED_ENTRIES_PER_KEY_TOP_K: usize = 64;
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
            Self::Hot => path_component_eq(path, "core") || path_component_eq(path, "base"),
            Self::Full => true,
            // Standard scans `large`, but entry filtering keeps only short
            // phrases so common 3-char words are not missing from runtime.
            Self::Standard => true,
        }
    }

    fn includes_entry(self, path: &Path, phrase: &str) -> bool {
        match self {
            Self::Standard if path_component_eq(path, "large") => {
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
        if path_component_eq(path, "core") {
            Self::Core
        } else if path_component_eq(path, "base") {
            Self::Base
        } else if path_component_eq(path, "ext") {
            Self::Ext
        } else if path_component_eq(path, "large") {
            Self::Large
        } else if path_component_eq(path, "en") {
            Self::En
        } else {
            Self::Unknown
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
    let Some(multi) = ch.to_pinyin_multi() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for py in multi {
        let plain = py.plain().to_ascii_lowercase();
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

#[derive(Debug, Clone, Default)]
struct RuntimeIndexes {
    abbrev_delete: LazyDeletionIndex,
    pinyin_delete: LazyDeletionIndex,
    abbrev_exact: HashMap<String, Arc<[ThuoclEntry]>>,
    pinyin_exact: HashMap<String, Arc<[ThuoclEntry]>>,
    hot_abbrev_exact: HashMap<String, Arc<[ThuoclEntry]>>,
    hot_pinyin_exact: HashMap<String, Arc<[ThuoclEntry]>>,
}

const HOT_EXACT_TOP_K: usize = 24;
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
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(chars.len() + 1);
    if seen.insert(key.to_string()) {
        out.push(key.to_string());
    }
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
    out
}

#[derive(Debug)]
pub struct AbbrevLexicon {
    inner: BTreeMap<String, Vec<CompactEntry>>,
    pinyin_inner: BTreeMap<String, Vec<CompactEntry>>,
    prefix_inner: HashMap<String, Vec<CompactEntry>>,
    pinyin_prefix_inner: HashMap<String, Vec<CompactEntry>>,
    phrases: Vec<Arc<str>>,
    phrase_freq: Vec<u64>,
    phrase_layer: Vec<LexiconLayer>,
    phrase_ids: Option<HashMap<Arc<str>, u32>>,
    runtime: RefCell<RuntimeIndexes>,
}

impl Default for AbbrevLexicon {
    fn default() -> Self {
        Self {
            inner: BTreeMap::new(),
            pinyin_inner: BTreeMap::new(),
            prefix_inner: HashMap::new(),
            pinyin_prefix_inner: HashMap::new(),
            phrases: Vec::new(),
            phrase_freq: Vec::new(),
            phrase_layer: Vec::new(),
            phrase_ids: Some(HashMap::new()),
            runtime: RefCell::new(RuntimeIndexes::default()),
        }
    }
}

impl Clone for AbbrevLexicon {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            pinyin_inner: self.pinyin_inner.clone(),
            prefix_inner: self.prefix_inner.clone(),
            pinyin_prefix_inner: self.pinyin_prefix_inner.clone(),
            phrases: self.phrases.clone(),
            phrase_freq: self.phrase_freq.clone(),
            phrase_layer: self.phrase_layer.clone(),
            phrase_ids: self.phrase_ids.clone(),
            runtime: RefCell::new(RuntimeIndexes::default()),
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
                .any(|entries| entries.len() > limit);
        trim_compact_map_entries(&mut self.inner, options.entries_per_key_limit);
        trim_compact_map_entries(&mut self.pinyin_inner, options.entries_per_key_limit);
        if entries_changed {
            self.mark_runtime_dirty();
        }
    }

    #[cfg(test)]
    pub(crate) fn has_prebaked_prefix_index(&self, pinyin: bool, prefix: &str) -> bool {
        if pinyin {
            self.pinyin_prefix_inner.contains_key(prefix)
        } else {
            self.prefix_inner.contains_key(prefix)
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
        let abbrev_keys = entry
            .code
            .as_deref()
            .and_then(abbrev_key_for_pinyin_code)
            .map(|key| vec![key])
            .unwrap_or_else(|| abbrev_keys_for_phrase(&entry.phrase));
        for key in abbrev_keys {
            let v = self.inner.entry(key).or_default();
            merge_compact_entry(v, phrase_id, entry.freq);
            trim_compact_entries(v, PREBAKED_ENTRIES_PER_KEY_TOP_K);
            inserted = true;
        }
        let pinyin_keys = entry
            .code
            .as_deref()
            .and_then(normalize_pinyin_code)
            .map(|key| vec![key])
            .unwrap_or_else(|| pinyin_keys_for_phrase(&entry.phrase));
        for key in pinyin_keys {
            let v = self.pinyin_inner.entry(key).or_default();
            merge_compact_entry(v, phrase_id, entry.freq);
            trim_compact_entries(v, PREBAKED_ENTRIES_PER_KEY_TOP_K);
            inserted = true;
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
        collect_prefix_entries(self, &self.pinyin_inner, prefix_lower)
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
        }
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
                    self.resolve_phrase(entry.phrase_id)
                        .map(|phrase| ThuoclEntry {
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

        let map = if pinyin {
            &self.pinyin_inner
        } else {
            &self.inner
        };
        let materialized = self.materialize_entries(map.get(key)?);
        let mut runtime = self.runtime.borrow_mut();
        let cache = if pinyin {
            &mut runtime.pinyin_exact
        } else {
            &mut runtime.abbrev_exact
        };
        if cache.len() >= EXACT_CACHE_MAX_KEYS {
            cache.clear();
        }
        cache.insert(key.to_string(), Arc::clone(&materialized));
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

        let map = if pinyin {
            &self.pinyin_inner
        } else {
            &self.inner
        };
        let compact = map.get(key)?;
        let materialized = self.materialize_entries(&compact[..compact.len().min(HOT_EXACT_TOP_K)]);
        let mut runtime = self.runtime.borrow_mut();
        let cache = if pinyin {
            &mut runtime.hot_pinyin_exact
        } else {
            &mut runtime.hot_abbrev_exact
        };
        if cache.len() >= HOT_EXACT_CACHE_MAX_KEYS {
            cache.clear();
        }
        cache.insert(key.to_string(), Arc::clone(&materialized));
        Some(materialized)
    }
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
    if prefix_lower.is_empty() {
        return empty_arc_entries();
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

    if top.is_empty() {
        empty_arc_entries()
    } else {
        lexicon.materialize_entries(&top)
    }
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
    let pinyin_prefix_inner = build_prebaked_prefix_index(&lexicon.pinyin_inner);
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
    let (prefix_inner, pinyin_prefix_inner) = if version >= 6 {
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

    Ok(AbbrevLexicon {
        inner,
        pinyin_inner,
        prefix_inner,
        pinyin_prefix_inner,
        phrase_ids: Some(build_phrase_id_index(&phrases)),
        phrases,
        phrase_freq,
        phrase_layer,
        runtime: RefCell::new(runtime),
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
            index_entry(
                &mut lex,
                &mut report,
                profile,
                path,
                calibrators.get(path),
                line_no + 1,
                line,
                parse_lexicon_entry_line(line),
            );
        }
    }

    Ok((lex, report))
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
        let rank = lower + upper.saturating_sub(lower).saturating_sub(1) / 2;
        let index = if source.len() <= 1 || reference.len() <= 1 {
            0
        } else {
            rank.saturating_mul(reference.len() - 1) / (source.len() - 1)
        };
        reference[index].max(1)
    }
}

fn is_frequency_reference_source(path: &Path) -> bool {
    path_component_eq(path, "base")
}

fn is_absolute_priority_source(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("kaixin_polyphone.txt"))
}

fn should_calibrate_source_frequency(path: &Path) -> bool {
    !is_frequency_reference_source(path) && !is_absolute_priority_source(path)
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

#[derive(Debug, Default)]
pub struct LoadReport {
    pub files_read: usize,
    pub files_failed: Vec<(PathBuf, String)>,
    pub lines_total: usize,
    pub entries_indexed: usize,
    pub lines_skipped_no_abbrev: usize,
    pub sample_skipped_lines: Vec<(PathBuf, usize, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_line_tab_freq() {
        let e = parse_line("信鸽\t220963").unwrap();
        assert_eq!(e.phrase, "信鸽");
        assert_eq!(e.freq, 220963);
    }

    #[test]
    fn auxiliary_source_frequencies_are_normalized_to_base_distribution() {
        let root = std::env::temp_dir().join(format!(
            "pinyin-ime-frequency-calibration-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("base")).unwrap();
        fs::create_dir_all(root.join("core")).unwrap();
        fs::create_dir_all(root.join("ext")).unwrap();
        fs::write(
            root.join("base/base.txt"),
            "低频\tdi pin\t100\n中频\tzhong pin\t1000\n高频\tgao pin\t10000\n",
        )
        .unwrap();
        fs::write(
            root.join("ext/auxiliary.txt"),
            "甲乙\tjia yi\t120000\n丙丁\tbing ding\t180000\n戊己\twu ji\t240000\n",
        )
        .unwrap();
        fs::write(
            root.join("core/kaixin_polyphone.txt"),
            "优先\tyou xian\t120000\n",
        )
        .unwrap();

        let (lexicon, _) = load_dir_txt(&root).unwrap();
        assert_eq!(lexicon.phrase_frequency("甲乙"), 100);
        assert_eq!(lexicon.phrase_frequency("丙丁"), 1000);
        assert_eq!(lexicon.phrase_frequency("戊己"), 10000);
        assert_eq!(lexicon.phrase_frequency("高频"), 10000);
        assert_eq!(lexicon.phrase_frequency("优先"), 120000);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parse_fallback_native_phrase_line() {
        let e = parse_fallback_tabbed_line("北京\tbei jing\t999").unwrap();
        assert_eq!(e.phrase, "北京");
        assert_eq!(e.freq, 999);
    }

    #[test]
    fn parse_unified_lexicon_entry_line_variants() {
        let tab_freq = parse_lexicon_entry_line("\u{4fe1}\u{9e3d}\t220963").unwrap();
        assert_eq!(tab_freq.phrase, "\u{4fe1}\u{9e3d}");
        assert_eq!(tab_freq.freq, 220963);
        assert_eq!(tab_freq.code, None);

        let explicit = parse_lexicon_entry_line("\u{5317}\u{4eac}\tbei jing\t999").unwrap();
        assert_eq!(explicit.phrase, "\u{5317}\u{4eac}");
        assert_eq!(explicit.freq, 999);
        assert_eq!(explicit.code.as_deref(), Some("bei jing"));

        let spaced = parse_lexicon_entry_line("\u{4e0a}\u{6d77} shang hai 888").unwrap();
        assert_eq!(spaced.phrase, "\u{4e0a}\u{6d77}");
        assert_eq!(spaced.freq, 888);
        assert_eq!(spaced.code.as_deref(), Some("shang hai"));

        assert!(parse_lexicon_entry_line("name: base").is_none());
        assert!(parse_lexicon_entry_line("README file for dictionaries").is_none());
    }

    #[test]
    fn explicit_code_lve_indexes_phrase_pinyin_key() {
        let mut lex = AbbrevLexicon::default();
        let entry = parse_fallback_tabbed_line("\u{7b80}\u{7565}\tjian lve\t17915").unwrap();
        assert!(lex.insert_parsed(entry));
        assert!(lex.lookup_pinyin("jianlve").is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.phrase == "\u{7b80}\u{7565}")
        }));
    }

    #[test]
    fn load_dir_clamps_zero_frequency() {
        let dir = std::env::temp_dir().join("pinyin_ime_zero_frequency_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut f = fs::File::create(dir.join("THUOCL_animal.txt")).unwrap();
        writeln!(f, "\u{5c0f}\u{5c97}\t0").unwrap();

        let (lex, rep) = load_dir_txt(&dir).unwrap();
        assert_eq!(rep.entries_indexed, 1);
        assert_eq!(lex.phrase_frequency("\u{5c0f}\u{5c97}"), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn standard_profile_keeps_short_large_entries_only() {
        let dir = std::env::temp_dir().join("pinyin_ime_standard_large_short_test");
        let large_dir = dir.join("large");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&large_dir).unwrap();
        let mut f = fs::File::create(large_dir.join("external.txt")).unwrap();
        writeln!(f, "\u{9b54}\u{529b}\u{9e1f}\t100").unwrap();
        writeln!(f, "\u{83ab}\u{91cc}\u{5c3c}\u{5965}\t100").unwrap();

        let (lex, rep) =
            load_dir_txt_with_profile_default_enabled(&dir, LexiconBuildProfile::Standard).unwrap();
        assert_eq!(rep.files_read, 1);
        assert!(lex.lookup_pinyin("moliniao").is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.phrase == "\u{9b54}\u{529b}\u{9e1f}")));
        assert!(lex.lookup("mln").is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.phrase == "\u{9b54}\u{529b}\u{9e1f}")));
        assert!(!lex.lookup_pinyin("moliniao").is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.phrase == "\u{83ab}\u{91cc}\u{5c3c}\u{5965}")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn abbrev_xin_ge() {
        let k = abbrev_for_phrase("信鸽").unwrap();
        assert_eq!(k, "xg");
    }

    #[test]
    fn abbrev_tian_wai_you_tian() {
        let k = abbrev_for_phrase("天外有天").unwrap();
        assert_eq!(k, "twyt");
    }

    #[test]
    fn pinyin_key_tian_wai_you_tian() {
        let k = pinyin_key_for_phrase("天外有天").unwrap();
        assert_eq!(k, "tianwaiyoutian");
    }

    #[test]
    fn lexicon_indexes_heteronym_phrase_readings() {
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "重庆".into(),
            freq: 88,
            code: None,
        }));

        assert!(lex
            .lookup_pinyin("chongqing")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == "重庆")));
        assert!(lex
            .lookup("cq")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == "重庆")));
    }

    #[test]
    fn lexicon_sort_by_freq_same_key() {
        let mut lex = AbbrevLexicon::default();
        assert_eq!(
            abbrev_for_phrase("兰州").unwrap(),
            abbrev_for_phrase("兰舟").unwrap()
        );
        lex.insert_parsed(ThuoclEntry {
            phrase: "兰舟".into(),
            freq: 500,
            code: None,
        });
        lex.insert_parsed(ThuoclEntry {
            phrase: "兰州".into(),
            freq: 100,
            code: None,
        });
        let key = abbrev_for_phrase("兰州").unwrap();
        let list = lex.lookup(&key).unwrap();
        assert_eq!(list[0].phrase, "兰舟");
        assert_eq!(list[0].freq, 500);
        assert_eq!(list[1].phrase, "兰州");
    }

    #[test]
    fn approximate_indexes_can_be_warmed_before_lookup() {
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 100_000,
            code: Some("bei jing".into()),
        }));
        assert!(matches!(
            lex.runtime.borrow().abbrev_delete,
            LazyDeletionIndex::Uninitialized
        ));
        assert!(matches!(
            lex.runtime.borrow().pinyin_delete,
            LazyDeletionIndex::Uninitialized
        ));

        lex.warmup_approx_indexes();

        let runtime = lex.runtime.borrow();
        assert!(matches!(
            runtime.abbrev_delete,
            LazyDeletionIndex::Ready(_) | LazyDeletionIndex::Disabled
        ));
        assert!(matches!(
            runtime.pinyin_delete,
            LazyDeletionIndex::Ready(_) | LazyDeletionIndex::Disabled
        ));
    }

    #[test]
    fn hot_exact_index_keeps_frequency_order_and_refreshes_after_mutation() {
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [("甲乙", 10), ("丙丁", 30), ("戊己", 20)] {
            lex.insert_parsed(ThuoclEntry {
                phrase: phrase.to_string(),
                freq,
                code: Some("jia yi".to_string()),
            });
        }

        let first = lex.lookup_pinyin_hot("jiayi").unwrap();
        assert_eq!(first[0].phrase, "丙丁");
        assert_eq!(lex.runtime.borrow().hot_pinyin_exact.len(), 1);

        lex.insert_parsed(ThuoclEntry {
            phrase: "新增".to_string(),
            freq: 40,
            code: Some("jia yi".to_string()),
        });
        let refreshed = lex.lookup_pinyin_hot("jiayi").unwrap();
        assert_eq!(refreshed[0].phrase, "新增");
    }

    #[test]
    fn exact_lookup_reuses_materialized_entries_and_refreshes_after_mutation() {
        let mut lex = AbbrevLexicon::default();
        lex.insert_parsed(ThuoclEntry {
            phrase: "输入法".to_string(),
            freq: 30,
            code: Some("shu ru fa".to_string()),
        });

        let abbrev_first = lex.lookup("srf").unwrap();
        let abbrev_second = lex.lookup("srf").unwrap();
        assert!(Arc::ptr_eq(&abbrev_first, &abbrev_second));

        let pinyin_first = lex.lookup_pinyin("shurufa").unwrap();
        let pinyin_second = lex.lookup_pinyin("shurufa").unwrap();
        assert!(Arc::ptr_eq(&pinyin_first, &pinyin_second));

        lex.insert_parsed(ThuoclEntry {
            phrase: "输入方式".to_string(),
            freq: 40,
            code: Some("shu ru fa".to_string()),
        });
        let refreshed = lex.lookup_pinyin("shurufa").unwrap();
        assert!(!Arc::ptr_eq(&pinyin_first, &refreshed));
        assert_eq!(refreshed[0].phrase, "输入方式");
    }

    #[test]
    fn lexicon_merge_same_phrase_keeps_max_freq() {
        let mut lex = AbbrevLexicon::default();
        lex.insert_parsed(ThuoclEntry {
            phrase: "信鸽".into(),
            freq: 10,
            code: None,
        });
        lex.insert_parsed(ThuoclEntry {
            phrase: "信鸽".into(),
            freq: 99,
            code: None,
        });
        let list = lex.lookup("xg").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].freq, 99);
    }

    #[test]
    fn load_dir_smoke() {
        let dir = std::env::temp_dir().join("pinyin_ime_thuocl_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut f = fs::File::create(dir.join("a.txt")).unwrap();
        writeln!(f, "信鸽\t10").unwrap();
        writeln!(f, "黄蜂\t20").unwrap();

        let (lex, rep) = load_dir_txt(&dir).unwrap();
        assert_eq!(rep.files_read, 1);
        assert!(rep.entries_indexed >= 2);
        assert!(lex.lookup("xg").is_some());
        assert!(lex.lookup_pinyin("xinge").is_some());
        let pre = lex.prefix_flat("x");
        assert!(!pre.is_empty());
        let py = lex.prefix_pinyin_flat("xin");
        assert!(!py.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_native_txt_from_nested_dir() {
        let dir = std::env::temp_dir().join("pinyin_ime_native_nested_test");
        let nested = dir.join("base");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&nested).unwrap();
        let mut f = fs::File::create(nested.join("base.txt")).unwrap();
        writeln!(f, "北京\tbei jing\t88").unwrap();

        let (lex, rep) = load_dir_txt(&dir).unwrap();
        assert_eq!(rep.files_read, 1);
        assert!(rep.entries_indexed >= 1);
        assert!(lex.lookup("bj").is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_dir_uses_native_txt_parser_and_skips_source_documents() {
        let dir = std::env::temp_dir().join("pinyin_ime_unified_lexicon_parser_test");
        let core_dir = dir.join("core");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&core_dir).unwrap();

        let mut core = fs::File::create(core_dir.join("base.txt")).unwrap();
        writeln!(core, "\u{660e}\u{5929}\tming tian\t888").unwrap();

        let mut txt = fs::File::create(dir.join("custom.txt")).unwrap();
        writeln!(txt, "\u{8f93}\u{5165}\u{6cd5}\tshu ru fa\t777").unwrap();

        let mut readme = fs::File::create(dir.join("README_noise.txt")).unwrap();
        writeln!(readme, "README file for dictionaries").unwrap();

        let mut source_list = fs::File::create(dir.join("stations_纯名单.txt")).unwrap();
        writeln!(source_list, "\u{6e90}\u{7d20}\u{6750}").unwrap();

        let (lex, rep) = load_dir_txt(&dir).unwrap();
        assert_eq!(rep.files_read, 2);
        assert!(lex.lookup_pinyin("mingtian").is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.phrase == "\u{660e}\u{5929}")));
        assert!(lex.lookup_pinyin("shurufa").is_some_and(|entries| entries
            .iter()
            .any(|entry| entry.phrase == "\u{8f93}\u{5165}\u{6cd5}")));
        assert!(
            !lex.lookup_pinyin("yuansucai").is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{6e90}\u{7d20}\u{6750}"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_utf16le_txt_with_bom() {
        let dir = std::env::temp_dir().join("pinyin_ime_utf16_dict_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let utf16le = [
            0xFF, 0xFE, 0xE1, 0x4F, 0x3D, 0x9E, 0x09, 0x00, 0x31, 0x00, 0x30, 0x00, 0x0D, 0x00,
            0x0A, 0x00,
        ];
        fs::write(dir.join("utf16.txt"), utf16le).unwrap();

        let (lex, rep) = load_dir_txt(&dir).unwrap();
        assert_eq!(rep.files_read, 1);
        assert!(rep.entries_indexed >= 1);
        assert_eq!(lex.lookup("xg").unwrap()[0].phrase, "信鸽");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prebaked_roundtrip_preserves_lookup() {
        let dir = std::env::temp_dir().join("pinyin_ime_prebaked_test");
        let bin = dir.join("lex.bin");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 88,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "背景".into(),
            freq: 12,
            code: None,
        }));

        save_prebaked_lexicon(&bin, &lex).unwrap();
        let loaded = load_prebaked_lexicon(&bin).unwrap();

        assert_eq!(loaded.lookup("bj").unwrap()[0].phrase, "北京");
        assert_eq!(loaded.lookup_pinyin("beijing").unwrap()[0].phrase, "北京");
        assert_eq!(loaded.lookup("bj").unwrap()[0].freq, 88);
        assert!(loaded.has_phrase_id_index());
        assert!(matches!(
            loaded.runtime.borrow().pinyin_delete,
            LazyDeletionIndex::Ready(_)
        ));
        assert!(loaded
            .approx_lookup_pinyin_scored("beijinh", 1, 4)
            .iter()
            .any(|(entry, distance)| entry.phrase == "北京" && *distance == 1));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn approximate_lookup_honors_an_internal_stop_before_lazy_index_build() {
        let mut lex = AbbrevLexicon::default();
        lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 88,
            code: Some("bei jing".into()),
        });
        let mut checks = 0usize;
        let result = lex.approx_lookup_pinyin_scored_until("beijinh", 1, 4, || {
            checks += 1;
            true
        });
        assert!(result.is_empty());
        assert!(checks > 0);
        assert!(matches!(
            lex.runtime.borrow().pinyin_delete,
            LazyDeletionIndex::Uninitialized
        ));
    }

    #[test]
    fn prebaked_roundtrip_preserves_dense_prefix_index_and_invalidates_on_mutation() {
        let dir = std::env::temp_dir().join("pinyin_ime_prebaked_prefix_test");
        let bin = dir.join("lex.bin");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut lex = AbbrevLexicon::default();
        for idx in 0..PREBAKED_PREFIX_MIN_KEYS {
            lex.insert_parsed(ThuoclEntry {
                phrase: format!("词{idx}"),
                freq: 10_000 - idx as u64,
                code: Some(format!("sharedprefix{idx:02}")),
            });
        }
        let expected = lex.prefix_pinyin_flat("sharedprefix");
        save_prebaked_lexicon(&bin, &lex).unwrap();
        let mut loaded = load_prebaked_lexicon(&bin).unwrap();
        assert!(loaded.pinyin_prefix_inner.contains_key("sharedprefix"));
        assert_eq!(loaded.prefix_pinyin_flat("sharedprefix"), expected);

        loaded.insert_parsed(ThuoclEntry {
            phrase: "新增".into(),
            freq: 99_999,
            code: Some("sharedprefix99".into()),
        });
        assert!(loaded.pinyin_prefix_inner.is_empty());
        assert_eq!(loaded.prefix_pinyin_flat("sharedprefix")[0].phrase, "新增");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prebaked_roundtrip_preserves_source_layer() {
        let dir = std::env::temp_dir().join("pinyin_ime_prebaked_layer_test");
        let bin = dir.join("lex.bin");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "\u{5317}\u{4eac}".into(),
                freq: 88,
                code: Some("bei jing".into()),
            },
            LexiconLayer::Ext,
        ));

        save_prebaked_lexicon(&bin, &lex).unwrap();
        let loaded = load_prebaked_lexicon(&bin).unwrap();

        assert_eq!(loaded.phrase_layer("\u{5317}\u{4eac}"), LexiconLayer::Ext);
        let _ = fs::remove_dir_all(&dir);
    }
}
