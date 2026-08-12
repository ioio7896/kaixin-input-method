use crate::runtime_log::{self, RuntimeLogLevel};
use crate::user_dict_io::{
    read_user_dict_sqlite_export_text, read_user_dict_sqlite_text, user_dict_reset_stamp,
    write_plain_user_dict_sqlite, write_user_dict_sqlite_snapshot_checked,
    write_user_dict_sqlite_snapshot_unlocked, write_user_dict_sqlite_snapshot_with_reset,
    UserDictExclusiveGuard, UserDictResetStamp, UserDictSharedGuard,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LearnedStats {
    pub freq: u64,
    pub last_used: u64,
    /// Q16 fixed-point counters. They are decayed before every update, so a
    /// single recent use cannot revive the whole historical `freq` value.
    pub short_count_q16: u64,
    pub long_count_q16: u64,
    pub last_decay_tick: u64,
}

impl LearnedStats {
    const COUNT_SCALE: f64 = 65_536.0;

    pub fn effective_counts(
        &self,
        now: u64,
        short_half_life: f64,
        long_half_life: f64,
    ) -> (f64, f64) {
        let (short_q16, long_q16, decay_tick) = if self.last_decay_tick == 0 {
            let legacy = self.freq.saturating_mul(65_536);
            (legacy, legacy, self.last_used)
        } else {
            (
                self.short_count_q16,
                self.long_count_q16,
                self.last_decay_tick,
            )
        };
        let age = now.saturating_sub(decay_tick) as f64;
        let short =
            short_q16 as f64 / Self::COUNT_SCALE * (0.5_f64).powf(age / short_half_life.max(1.0));
        let long =
            long_q16 as f64 / Self::COUNT_SCALE * (0.5_f64).powf(age / long_half_life.max(1.0));
        (short, long)
    }

    fn add_decayed(&mut self, now: u64, delta: u64, short_half_life: f64, long_half_life: f64) {
        let (short, long) = self.effective_counts(now, short_half_life, long_half_life);
        let delta_q16 = delta.saturating_mul(65_536);
        self.short_count_q16 =
            ((short * Self::COUNT_SCALE).round() as u64).saturating_add(delta_q16);
        self.long_count_q16 = ((long * Self::COUNT_SCALE).round() as u64).saturating_add(delta_q16);
        self.last_decay_tick = now;
        self.freq = self.freq.saturating_add(delta);
        self.last_used = now;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum SuggestionConfidence {
    #[default]
    Remembered = 1,
    Confirmed = 2,
    Established = 3,
}

impl SuggestionConfidence {
    fn from_u64(value: u64) -> Self {
        match value {
            3.. => Self::Established,
            2 => Self::Confirmed,
            _ => Self::Remembered,
        }
    }

    pub fn score_scale(self) -> f64 {
        match self {
            Self::Remembered => 0.68,
            Self::Confirmed => 0.98,
            Self::Established => 1.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LearnedEntry {
    pub phrase: String,
    pub freq: u64,
    pub last_used: u64,
    pub pinned: bool,
    pub short_count_q16: u64,
    pub long_count_q16: u64,
    pub last_decay_tick: u64,
    pub confidence: SuggestionConfidence,
    pub observation_count: u64,
    pub selection_count: u64,
}

impl LearnedEntry {
    pub fn stats(&self) -> LearnedStats {
        LearnedStats {
            freq: self.freq,
            last_used: self.last_used,
            short_count_q16: self.short_count_q16,
            long_count_q16: self.long_count_q16,
            last_decay_tick: self.last_decay_tick,
        }
    }

    pub fn is_pinned_exact_input(&self) -> bool {
        self.pinned
    }

    pub fn suggestion_score_scale(&self) -> f64 {
        if self.pinned {
            1.0
        } else {
            self.confidence.score_scale()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitSource {
    Direct,
    Weak,
    Composed,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentCommit {
    key: String,
    phrase: String,
    tick: u64,
    source: CommitSource,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionFeedbackEntry {
    pub phrase: String,
    /// 正数为正反馈（选中/置顶），负数为负反馈（删除/忽略）。
    pub score: i64,
    pub last_used: u64,
}

impl SelectionFeedbackEntry {
    pub fn stats(&self) -> LearnedStats {
        LearnedStats {
            freq: self.score.unsigned_abs(),
            last_used: self.last_used,
            ..LearnedStats::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedUserPhrase {
    pub key: String,
    pub phrase: String,
    pub freq: u64,
    pub last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedNovelPhrase {
    pub key: String,
    pub phrase: String,
    pub freq: u64,
    pub last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedBlockedPhrase {
    pub phrase: String,
    pub last_used: u64,
}

#[derive(Debug)]
pub struct UserLexicon {
    path: PathBuf,
    clock: u64,
    exact_input: BTreeMap<String, Vec<LearnedEntry>>,
    mixed_input: BTreeMap<String, Vec<LearnedEntry>>,
    observed_input: BTreeMap<String, Vec<LearnedEntry>>,
    /// 个人纠错记忆：raw_input -> [(corrected_pinyin, freq, last_used)]。
    correction_pairs: BTreeMap<String, Vec<LearnedEntry>>,
    /// 自动发现的生词：reading -> [(phrase, freq, last_used)]。
    novel_phrases: BTreeMap<String, Vec<LearnedEntry>>,
    /// 永远不学：phrase -> last_used。
    blocked_phrases: BTreeMap<String, u64>,
    /// 候选交互反馈：reading -> [(phrase, score, last_used)]。
    /// score > 0 表示用户曾选择/置顶该候选，score < 0 表示删除/忽略。
    selection_feedback: BTreeMap<String, Vec<SelectionFeedbackEntry>>,
    /// 上下文候选反馈：(prev_phrase, reading) -> [(phrase, score, last_used)]。
    context_selection_feedback: BTreeMap<(String, String), Vec<SelectionFeedbackEntry>>,
    /// 可见页未选候选的弱观察：reading -> [(phrase, negative_count, last_used)]。
    weak_unselected_feedback: BTreeMap<String, Vec<SelectionFeedbackEntry>>,
    /// 上下文可见页未选弱观察：(prev_phrase, reading) -> [(phrase, negative_count, last_used)]。
    context_weak_unselected_feedback: BTreeMap<(String, String), Vec<SelectionFeedbackEntry>>,
    approx_index: HashMap<String, Vec<String>>,
    phrase_stats: BTreeMap<String, LearnedStats>,
    context_links: BTreeMap<String, Vec<LearnedEntry>>,
    context_trigrams: BTreeMap<(String, String), Vec<LearnedEntry>>,
    recent_commits: Vec<RecentCommit>,
    recent_word_tokens: Vec<String>,
    persistence: Option<PersistenceWorker>,
    learns_since_snapshot: usize,
    reset_stamp: SharedResetStamp,
}

/// 最多解析行数（超长文件尾部忽略）。
const MAX_USER_DICT_LINES: usize = 2_000_000;
/// 单行最大字节（超长行跳过）。
const MAX_USER_DICT_LINE_BYTES: usize = 65_536;
/// SQLite 增量写达到这个学习次数后，后台顺手做一次完整快照，压平历史更新。
const SNAPSHOT_EVERY_LEARNS: usize = 48;
/// 后台写线程的攒批窗口，减少高频 commit 的磁盘抖动。
const PERSIST_BATCH_WINDOW_MS: u64 = 160;
const APPROX_INDEX_MAX_KEY_LEN: usize = 40;
const MAX_USER_ENTRIES_PER_KEY: usize = 64;
const MAX_CONTEXT_ENTRIES_PER_PREV: usize = 32;
const MAX_TOTAL_EXACT_ENTRIES: usize = 100_000;
const MAX_TOTAL_CONTEXT_LINKS: usize = 100_000;
const MAX_PHRASE_STATS: usize = 100_000;
const MAX_BLOCKED_PHRASES: usize = 10_000;
const MAX_USER_KEY_CHARS: usize = 80;
const MAX_USER_PHRASE_CHARS: usize = 128;
const LEGACY_PINNED_EXACT_FREQ_DELTA: u64 = 100_000;
const MAX_RECENT_COMMITS: usize = 4;
const MAX_RECENT_WORD_TOKENS: usize = 4;
const ADJACENT_NOVEL_MIN_CHARS: usize = 4;
const ADJACENT_NOVEL_MAX_CHARS: usize = 8;
const ADJACENT_NOVEL_MAX_EVENT_GAP: u64 = 64;
const ADJACENT_NOVEL_PROMOTE_FREQ: u64 = 2;
const NEGATIVE_FEEDBACK_DEMOTE_THRESHOLD: i64 = -4;
const SHORT_COUNT_HALF_LIFE_TICKS: f64 = 24.0;
const LONG_COUNT_HALF_LIFE_TICKS: f64 = 4000.0;

type SharedResetStamp = Arc<Mutex<UserDictResetStamp>>;

#[derive(Debug)]
struct PersistenceWorker {
    tx: mpsc::Sender<PersistenceCommand>,
    join: Option<thread::JoinHandle<()>>,
}

#[derive(Debug)]
enum PersistenceCommand {
    Append(Vec<String>),
    Snapshot(String),
    Flush(mpsc::Sender<Result<(), String>>),
    Shutdown,
}

impl Default for UserLexicon {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            clock: 0,
            exact_input: BTreeMap::new(),
            mixed_input: BTreeMap::new(),
            observed_input: BTreeMap::new(),
            correction_pairs: BTreeMap::new(),
            novel_phrases: BTreeMap::new(),
            blocked_phrases: BTreeMap::new(),
            selection_feedback: BTreeMap::new(),
            context_selection_feedback: BTreeMap::new(),
            weak_unselected_feedback: BTreeMap::new(),
            context_weak_unselected_feedback: BTreeMap::new(),
            approx_index: HashMap::new(),
            phrase_stats: BTreeMap::new(),
            context_links: BTreeMap::new(),
            context_trigrams: BTreeMap::new(),
            recent_commits: Vec::new(),
            recent_word_tokens: Vec::new(),
            persistence: None,
            learns_since_snapshot: 0,
            reset_stamp: Arc::new(Mutex::new(UserDictResetStamp::default())),
        }
    }
}

fn reset_stamp_registry() -> &'static Mutex<HashMap<PathBuf, SharedResetStamp>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, SharedResetStamp>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reset_stamp_path_key(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn shared_reset_stamp_for_path(path: &Path) -> SharedResetStamp {
    let key = reset_stamp_path_key(path);
    let Ok(mut registry) = reset_stamp_registry().lock() else {
        return Arc::new(Mutex::new(user_dict_reset_stamp(path)));
    };
    registry
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(user_dict_reset_stamp(path))))
        .clone()
}

fn current_reset_stamp(stamp: &SharedResetStamp) -> UserDictResetStamp {
    stamp.lock().map(|guard| *guard).unwrap_or_default()
}

fn set_reset_stamp(stamp: &SharedResetStamp, value: UserDictResetStamp) {
    if let Ok(mut guard) = stamp.lock() {
        *guard = value;
    }
}

fn refresh_reset_stamp(path: &Path, stamp: &SharedResetStamp) -> UserDictResetStamp {
    let value = user_dict_reset_stamp(path);
    set_reset_stamp(stamp, value);
    value
}

fn update_shared_reset_stamp_for_path(path: &Path, value: UserDictResetStamp) {
    let stamp = shared_reset_stamp_for_path(path);
    set_reset_stamp(&stamp, value);
}

fn log_user_dict_error(event: &str, message: impl AsRef<str>) {
    runtime_log::log_engine(RuntimeLogLevel::Error, event, message);
}

impl UserLexicon {
    fn empty_for_path(path: PathBuf) -> Self {
        let reset_stamp = shared_reset_stamp_for_path(&path);
        Self {
            path,
            clock: 0,
            exact_input: BTreeMap::new(),
            mixed_input: BTreeMap::new(),
            observed_input: BTreeMap::new(),
            correction_pairs: BTreeMap::new(),
            novel_phrases: BTreeMap::new(),
            blocked_phrases: BTreeMap::new(),
            selection_feedback: BTreeMap::new(),
            context_selection_feedback: BTreeMap::new(),
            weak_unselected_feedback: BTreeMap::new(),
            context_weak_unselected_feedback: BTreeMap::new(),
            approx_index: HashMap::new(),
            phrase_stats: BTreeMap::new(),
            context_links: BTreeMap::new(),
            context_trigrams: BTreeMap::new(),
            recent_commits: Vec::new(),
            recent_word_tokens: Vec::new(),
            persistence: None,
            learns_since_snapshot: 0,
            reset_stamp,
        }
    }

    pub fn load_default() -> Self {
        let path = default_user_dict_path();
        Self::load_from_path(path.clone()).unwrap_or_else(|err| {
            log_user_dict_error(
                "srf_user_dict_load_failed",
                format!("path={} error={err}", path.display()),
            );
            Self::default()
        })
    }

    fn parse_text_lines(&mut self, text: &str) {
        let mut n_lines = 0usize;
        for line in text.lines() {
            n_lines += 1;
            if n_lines > MAX_USER_DICT_LINES {
                break;
            }
            if line.len() > MAX_USER_DICT_LINE_BYTES {
                continue;
            }
            self.parse_line(line);
        }
    }

    pub fn load_from_path(path: PathBuf) -> io::Result<Self> {
        let mut lex = Self::empty_for_path(path);
        {
            let _lock = UserDictSharedGuard::new(&lex.path)?;
            set_reset_stamp(&lex.reset_stamp, user_dict_reset_stamp(&lex.path));

            match read_user_dict_sqlite_text(&lex.path) {
                Ok(Some(text)) => lex.parse_text_lines(&text),
                Ok(None) => {}
                Err(err) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("read user dictionary sqlite: {err}"),
                    ));
                }
            }
        }
        lex.prune_for_memory();

        let needs_encryption_rewrite = crate::user_dict_io::user_dict_encryption_enabled()
            && lex.path.is_file()
            && !crate::user_dict_io::user_dict_file_is_encrypted(&lex.path).unwrap_or(true);
        if needs_encryption_rewrite && !lex.path.as_os_str().is_empty() {
            match write_user_dict_sqlite_snapshot_with_reset(
                &lex.path,
                lex.serialize_snapshot().as_bytes(),
            ) {
                Ok(stamp) => update_shared_reset_stamp_for_path(&lex.path, stamp),
                Err(err) => log_user_dict_error(
                    "srf_user_dict_sqlite_migration_write_failed",
                    format!("path={} error={err}", lex.path.display()),
                ),
            }
        }
        if !lex.path.as_os_str().is_empty() {
            lex.persistence = Some(PersistenceWorker::spawn(
                lex.path.clone(),
                lex.reset_stamp.clone(),
            ));
        }
        Ok(lex)
    }

    pub fn current_clock(&self) -> u64 {
        self.clock
    }

    pub fn advance_clock_for_eval(&mut self, ticks: u64) {
        self.clock = self.clock.saturating_add(ticks);
        self.recent_commits.clear();
        self.recent_word_tokens.clear();
    }

    pub fn last_committed(&self) -> Option<&str> {
        self.recent_commits
            .first()
            .map(|commit| commit.phrase.as_str())
    }

    /// 用于上下文加分：最近上屏词，新→旧，最多 3 个。
    pub fn preceding_commits_for_context(&self) -> Vec<&str> {
        self.recent_commits
            .iter()
            .take(3)
            .map(|commit| commit.phrase.as_str())
            .collect()
    }

    pub fn preceding_word_tokens_for_context(&self) -> Vec<&str> {
        self.recent_word_tokens
            .iter()
            .take(3)
            .map(String::as_str)
            .collect()
    }

    pub fn clear_context_history(&mut self) {
        self.recent_commits.clear();
        self.recent_word_tokens.clear();
    }

    pub fn lookup_input(&self, key: &str) -> Option<&[LearnedEntry]> {
        self.exact_input.get(key).map(|v| v.as_slice())
    }

    pub fn lookup_mixed_input(&self, key: &str) -> Option<&[LearnedEntry]> {
        self.mixed_input.get(key).map(|v| v.as_slice())
    }

    pub fn lookup_observed_input(&self, key: &str) -> Option<&[LearnedEntry]> {
        self.observed_input.get(key).map(|v| v.as_slice())
    }

    pub fn lookup_correction_pairs(&self, raw: &str) -> Option<&[LearnedEntry]> {
        self.correction_pairs.get(raw).map(|v| v.as_slice())
    }

    pub fn learn_correction(&mut self, raw: &str, corrected: &str) -> io::Result<()> {
        let raw = raw.trim().to_ascii_lowercase();
        let corrected = corrected.trim().to_ascii_lowercase();
        if raw.is_empty() || corrected.is_empty() || raw == corrected {
            return Ok(());
        }
        if raw.chars().count() > MAX_USER_KEY_CHARS
            || corrected.chars().count() > MAX_USER_KEY_CHARS
        {
            return Ok(());
        }
        if !raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' || ch == ' ')
            || !corrected
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' || ch == ' ')
        {
            return Ok(());
        }

        let now = self.bump_clock();
        let entries = self.correction_pairs.entry(raw.clone()).or_default();
        upsert_entry(entries, corrected.to_string(), now, 1);

        let updates = entries
            .iter()
            .find(|entry| entry.phrase == corrected)
            .map(|entry| {
                vec![format!(
                    "R\t{}\t{}\t{}\t{}",
                    raw, entry.phrase, entry.freq, entry.last_used
                )]
            })
            .unwrap_or_default();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn selection_signal(&self, reading: &str) -> Option<&[SelectionFeedbackEntry]> {
        self.selection_feedback.get(reading).map(|v| v.as_slice())
    }

    pub fn context_selection_signal(
        &self,
        prev: &str,
        reading: &str,
    ) -> Option<&[SelectionFeedbackEntry]> {
        self.context_selection_feedback
            .get(&(prev.to_string(), reading.to_string()))
            .map(|v| v.as_slice())
    }

    pub fn weak_unselected_signal(&self, reading: &str) -> Option<&[SelectionFeedbackEntry]> {
        self.weak_unselected_feedback
            .get(reading)
            .map(|v| v.as_slice())
    }

    pub fn context_weak_unselected_signal(
        &self,
        prev: &str,
        reading: &str,
    ) -> Option<&[SelectionFeedbackEntry]> {
        self.context_weak_unselected_feedback
            .get(&(prev.to_string(), reading.to_string()))
            .map(|v| v.as_slice())
    }

    pub fn phrase_is_blocked(&self, phrase: &str) -> bool {
        normalize_selection_feedback_phrase(phrase)
            .is_some_and(|phrase| self.blocked_phrases.contains_key(&phrase))
    }

    pub fn block_phrase(&mut self, phrase: &str) -> io::Result<bool> {
        let Some(phrase) = normalize_selection_feedback_phrase(phrase) else {
            return Ok(false);
        };

        let now = self.bump_clock();
        self.blocked_phrases.insert(phrase.clone(), now);
        remove_phrase_everywhere(self, &phrase);
        if self.blocked_phrases.len() > MAX_BLOCKED_PHRASES {
            prune_blocked_phrases(&mut self.blocked_phrases, MAX_BLOCKED_PHRASES);
        }
        self.persist_updates(vec![format!("B\t{}\t{}", phrase, now)])
            .map(|_| true)
    }

    pub fn unblock_phrase(&mut self, phrase: &str) -> io::Result<bool> {
        let Some(phrase) = normalize_selection_feedback_phrase(phrase) else {
            return Ok(false);
        };
        let removed = self.blocked_phrases.remove(&phrase).is_some();
        if removed {
            self.persist_updates(vec![format!("B\t{}\t0", phrase)])?;
        }
        Ok(removed)
    }

    pub fn record_selection(&mut self, reading: &str, phrase: &str, delta: i64) -> io::Result<()> {
        self.record_selection_feedback(reading, [(phrase, delta)])
    }

    pub fn record_context_selection_feedback<'a, I>(
        &mut self,
        prev: &str,
        reading: &str,
        adjustments: I,
    ) -> io::Result<()>
    where
        I: IntoIterator<Item = (&'a str, i64)>,
    {
        let Some(prev) = normalize_selection_feedback_phrase(prev) else {
            return Ok(());
        };
        let Some(reading) = normalize_selection_feedback_reading(reading) else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&prev) {
            return Ok(());
        }

        let deltas = self.normalized_selection_deltas(adjustments);
        if deltas.is_empty() {
            return Ok(());
        }

        let now = self.bump_clock();
        let key = (prev, reading);
        let entries = self
            .context_selection_feedback
            .entry(key.clone())
            .or_default();
        apply_selection_deltas(entries, &deltas, now);
        let changed_phrases: HashSet<String> = deltas.keys().cloned().collect();
        let updates = entries
            .iter()
            .filter(|entry| changed_phrases.contains(&entry.phrase))
            .map(|entry| {
                format!(
                    "X\t{}\t{}\t{}\t{}\t{}",
                    key.0, key.1, entry.phrase, entry.score, entry.last_used
                )
            })
            .collect::<Vec<_>>();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn record_weak_unselected_feedback<'a, I>(
        &mut self,
        reading: &str,
        adjustments: I,
    ) -> io::Result<()>
    where
        I: IntoIterator<Item = (&'a str, i64)>,
    {
        let Some(reading) = normalize_selection_feedback_reading(reading) else {
            return Ok(());
        };

        let deltas = self.normalized_selection_deltas(adjustments);
        if deltas.is_empty() {
            return Ok(());
        }

        let now = self.bump_clock();
        let entries = self
            .weak_unselected_feedback
            .entry(reading.clone())
            .or_default();
        apply_selection_deltas(entries, &deltas, now);
        let changed_phrases: HashSet<String> = deltas.keys().cloned().collect();
        let updates = entries
            .iter()
            .filter(|entry| changed_phrases.contains(&entry.phrase))
            .map(|entry| {
                format!(
                    "U\t{}\t{}\t{}\t{}",
                    reading, entry.phrase, entry.score, entry.last_used
                )
            })
            .collect::<Vec<_>>();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn record_context_weak_unselected_feedback<'a, I>(
        &mut self,
        prev: &str,
        reading: &str,
        adjustments: I,
    ) -> io::Result<()>
    where
        I: IntoIterator<Item = (&'a str, i64)>,
    {
        let Some(prev) = normalize_selection_feedback_phrase(prev) else {
            return Ok(());
        };
        let Some(reading) = normalize_selection_feedback_reading(reading) else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&prev) {
            return Ok(());
        }

        let deltas = self.normalized_selection_deltas(adjustments);
        if deltas.is_empty() {
            return Ok(());
        }

        let now = self.bump_clock();
        let key = (prev, reading);
        let entries = self
            .context_weak_unselected_feedback
            .entry(key.clone())
            .or_default();
        apply_selection_deltas(entries, &deltas, now);
        let changed_phrases: HashSet<String> = deltas.keys().cloned().collect();
        let updates = entries
            .iter()
            .filter(|entry| changed_phrases.contains(&entry.phrase))
            .map(|entry| {
                format!(
                    "Y\t{}\t{}\t{}\t{}\t{}",
                    key.0, key.1, entry.phrase, entry.score, entry.last_used
                )
            })
            .collect::<Vec<_>>();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    fn normalized_selection_deltas<'a, I>(&self, adjustments: I) -> BTreeMap<String, i64>
    where
        I: IntoIterator<Item = (&'a str, i64)>,
    {
        let mut deltas: BTreeMap<String, i64> = BTreeMap::new();
        for (phrase, delta) in adjustments {
            if delta == 0 {
                continue;
            }
            let Some(phrase) = normalize_selection_feedback_phrase(phrase) else {
                continue;
            };
            if self.blocked_phrases.contains_key(&phrase) {
                continue;
            }
            let entry = deltas.entry(phrase).or_insert(0);
            *entry = entry.saturating_add(delta);
        }
        deltas.retain(|_, delta| *delta != 0);
        deltas
    }

    pub fn record_selection_feedback<'a, I>(
        &mut self,
        reading: &str,
        adjustments: I,
    ) -> io::Result<()>
    where
        I: IntoIterator<Item = (&'a str, i64)>,
    {
        let Some(reading) = normalize_selection_feedback_reading(reading) else {
            return Ok(());
        };

        let deltas = self.normalized_selection_deltas(adjustments);
        if deltas.is_empty() {
            return Ok(());
        }

        let now = self.bump_clock();
        let entries = self.selection_feedback.entry(reading.clone()).or_default();
        apply_selection_deltas(entries, &deltas, now);

        let changed_phrases: HashSet<String> = deltas.keys().cloned().collect();
        let mut updates = entries
            .iter()
            .filter(|entry| changed_phrases.contains(&entry.phrase))
            .map(|entry| {
                format!(
                    "S\t{}\t{}\t{}\t{}",
                    reading, entry.phrase, entry.score, entry.last_used
                )
            })
            .collect::<Vec<_>>();
        if let Some(exact_entries) = self.exact_input.get_mut(&reading) {
            for (phrase, delta) in &deltas {
                if *delta <= 0 {
                    continue;
                }
                if let Some(entry) = exact_entries
                    .iter_mut()
                    .find(|entry| entry.phrase == *phrase)
                {
                    entry.selection_count = entry.selection_count.saturating_add(1);
                    refresh_suggestion_confidence(entry);
                    updates.push(format!(
                        "I\t{}\t{}\t{}\t{}\t{}",
                        reading,
                        entry.phrase,
                        entry.freq,
                        entry.last_used,
                        u8::from(entry.pinned)
                    ));
                }
            }
            sort_entries(exact_entries);
        }
        let demoted = self.demote_negatively_reinforced_exact_entries(
            &reading,
            &changed_phrases,
            NEGATIVE_FEEDBACK_DEMOTE_THRESHOLD,
            now,
        );

        if self.prune_for_memory() || demoted {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    fn demote_negatively_reinforced_exact_entries(
        &mut self,
        reading: &str,
        changed_phrases: &HashSet<String>,
        threshold: i64,
        now: u64,
    ) -> bool {
        let Some(feedback) = self.selection_feedback.get(reading) else {
            return false;
        };
        let demote: HashSet<String> = feedback
            .iter()
            .filter(|entry| changed_phrases.contains(&entry.phrase) && entry.score <= threshold)
            .map(|entry| entry.phrase.clone())
            .collect();
        if demote.is_empty() {
            return false;
        }

        let mut demoted = Vec::new();
        let mut remove_key = false;
        if let Some(entries) = self.exact_input.get_mut(reading) {
            let mut kept = Vec::with_capacity(entries.len());
            for entry in entries.drain(..) {
                if demote.contains(&entry.phrase) && !entry.pinned {
                    demoted.push(entry);
                } else {
                    kept.push(entry);
                }
            }
            *entries = kept;
            remove_key = entries.is_empty();
        }
        if remove_key {
            self.exact_input.remove(reading);
        }
        if demoted.is_empty() {
            return false;
        }

        rebuild_approx_index(self);
        for entry in demoted {
            let observed_entries = self.observed_input.entry(reading.to_string()).or_default();
            upsert_entry(observed_entries, entry.phrase.clone(), now, 1);
            if !self
                .exact_input
                .values()
                .flat_map(|entries| entries.iter())
                .any(|exact| exact.phrase == entry.phrase)
            {
                self.phrase_stats.remove(&entry.phrase);
            }
            self.recent_commits
                .retain(|commit| commit.phrase != entry.phrase);
            self.recent_word_tokens
                .retain(|token| token != &entry.phrase);
        }
        true
    }

    pub fn approx_lookup_input(
        &self,
        key: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<LearnedEntry> {
        self.approx_lookup_input_scored(key, max_distance, limit)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }

    pub fn approx_lookup_input_scored(
        &self,
        key: &str,
        max_distance: usize,
        limit: usize,
    ) -> Vec<(LearnedEntry, usize)> {
        approximate_lookup_scored(
            &self.exact_input,
            &self.approx_index,
            key,
            max_distance,
            limit,
        )
    }

    pub fn phrase_signal(&self, phrase: &str) -> LearnedStats {
        self.phrase_stats.get(phrase).copied().unwrap_or_default()
    }

    pub fn context_signal(&self, prev: &str, phrase: &str) -> LearnedStats {
        self.context_links
            .get(prev)
            .and_then(|entries| entries.iter().find(|entry| entry.phrase == phrase))
            .map(LearnedEntry::stats)
            .unwrap_or_default()
    }

    pub fn context_trigram_signal(&self, prev2: &str, prev1: &str, phrase: &str) -> LearnedStats {
        self.context_trigrams
            .get(&(prev2.to_string(), prev1.to_string()))
            .and_then(|entries| entries.iter().find(|entry| entry.phrase == phrase))
            .map(LearnedEntry::stats)
            .unwrap_or_default()
    }

    pub fn context_outgoing_stats(&self, prev: &str) -> (u64, usize) {
        if let Some(entries) = self.context_links.get(prev) {
            let total = entries.iter().map(|entry| entry.freq).sum();
            (total, entries.len())
        } else {
            (0, 0)
        }
    }

    pub fn context_trigram_outgoing_stats(&self, prev2: &str, prev1: &str) -> (u64, usize) {
        if let Some(entries) = self
            .context_trigrams
            .get(&(prev2.to_string(), prev1.to_string()))
        {
            let total = entries.iter().map(|entry| entry.freq).sum();
            (total, entries.len())
        } else {
            (0, 0)
        }
    }

    pub fn phrase_vocabulary_size(&self) -> usize {
        self.phrase_stats.len()
    }

    pub fn learn(&mut self, key: &str, phrase: &str) -> io::Result<()> {
        self.learn_with_delta(key, phrase, 1)
    }

    pub fn learn_with_delta(&mut self, key: &str, phrase: &str, delta: u64) -> io::Result<()> {
        self.learn_with_source_and_delta(key, phrase, delta, CommitSource::Direct)
    }

    pub fn learn_with_source_and_delta(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
        source: CommitSource,
    ) -> io::Result<()> {
        self.learn_with_source_delta_and_tokens(key, phrase, delta, source, &[phrase.to_string()])
    }

    pub fn learn_with_source_delta_and_tokens(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
        source: CommitSource,
        word_tokens: &[String],
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }
        let delta = delta.max(1);

        let now = self.bump_clock();
        let mut updates = Vec::with_capacity(8);
        let adjacent_candidates = self.adjacent_novel_candidates(&key, &phrase, now, source);

        let exact_entries = self.exact_input.entry(key.clone()).or_default();
        upsert_entry(exact_entries, phrase.to_string(), now, delta);
        if let Some(entry) = exact_entries
            .iter_mut()
            .find(|entry| entry.phrase == phrase)
        {
            record_exact_evidence(entry, source);
        }
        register_approx_key(&mut self.approx_index, &key);
        if let Some(entry) = exact_entries.iter().find(|entry| entry.phrase == phrase) {
            updates.push(format!(
                "I\t{}\t{}\t{}\t{}\t{}",
                key,
                entry.phrase,
                entry.freq,
                entry.last_used,
                u8::from(entry.pinned)
            ));
        }

        upsert_stats(&mut self.phrase_stats, phrase.to_string(), now, delta);
        if let Some(stats) = self.phrase_stats.get(&phrase).copied() {
            updates.push(format!(
                "P\t{}\t{}\t{}",
                phrase, stats.freq, stats.last_used
            ));
        }

        self.record_word_ngram_updates(&phrase, word_tokens, now, delta, &mut updates);

        self.record_adjacent_novel_candidates(adjacent_candidates, now, &mut updates);
        self.push_recent_commit(key, phrase, now, source);
        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    /// Promote a composed phrase from the observation bucket into the exact
    /// user dictionary. The observation count is real user evidence: carry it
    /// into both frequency and confidence instead of restarting the entry at
    /// frequency one/Remembered.
    pub fn promote_observed_input_with_source_and_delta(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
        source: CommitSource,
        observed: LearnedStats,
    ) -> io::Result<()> {
        self.promote_observed_input_with_source_delta_and_tokens(
            key,
            phrase,
            delta,
            source,
            observed,
            &[phrase.to_string()],
        )
    }

    pub fn promote_observed_input_with_source_delta_and_tokens(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
        source: CommitSource,
        observed: LearnedStats,
        word_tokens: &[String],
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }

        // The exact entry replaces the provisional observation. A snapshot is
        // queued below so the removal is durable as well as visible in memory.
        remove_observed_input_phrase(self, &key, &phrase);
        self.learn_with_source_delta_and_tokens(
            &key,
            &phrase,
            delta.max(observed.freq).max(1),
            source,
            word_tokens,
        )?;

        if let Some(entry) = self
            .exact_input
            .get_mut(&key)
            .and_then(|entries| entries.iter_mut().find(|entry| entry.phrase == phrase))
        {
            entry.observation_count = entry.observation_count.max(observed.freq);
            refresh_suggestion_confidence(entry);
        }
        self.persist_snapshot_now()
    }

    pub fn record_word_sequence(
        &mut self,
        committed_phrase: &str,
        word_tokens: &[String],
        delta: u64,
    ) -> io::Result<()> {
        let committed_phrase = committed_phrase.trim();
        if committed_phrase.is_empty() || self.blocked_phrases.contains_key(committed_phrase) {
            return Ok(());
        }
        let now = if self.clock == 0 {
            self.bump_clock()
        } else {
            self.clock
        };
        let mut updates = Vec::new();
        self.record_word_ngram_updates(
            committed_phrase,
            word_tokens,
            now,
            delta.max(1),
            &mut updates,
        );
        if updates.is_empty() {
            return Ok(());
        }
        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    fn record_word_ngram_updates(
        &mut self,
        committed_phrase: &str,
        word_tokens: &[String],
        now: u64,
        delta: u64,
        updates: &mut Vec<String>,
    ) {
        let mut tokens = word_tokens
            .iter()
            .filter_map(|token| normalize_selection_feedback_phrase(token))
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            if let Some(phrase) = normalize_selection_feedback_phrase(committed_phrase) {
                tokens.push(phrase);
            }
        }
        if tokens.is_empty() {
            return;
        }

        let mut prev1 = self.recent_word_tokens.first().cloned();
        let mut prev2 = self.recent_word_tokens.get(1).cloned();
        for token in &tokens {
            if tokens.len() > 1 || token != committed_phrase {
                upsert_stats(&mut self.phrase_stats, token.clone(), now, delta);
                if let Some(stats) = self.phrase_stats.get(token).copied() {
                    updates.push(format!("P\t{}\t{}\t{}", token, stats.freq, stats.last_used));
                }
            }

            if let Some(previous) = prev1.as_deref().filter(|previous| *previous != token) {
                let transitions = self.context_links.entry(previous.to_string()).or_default();
                upsert_entry(transitions, token.clone(), now, delta);
                if let Some(entry) = transitions.iter().find(|entry| entry.phrase == *token) {
                    updates.push(format!(
                        "C\t{}\t{}\t{}\t{}",
                        previous, entry.phrase, entry.freq, entry.last_used
                    ));
                }

                if let Some(previous2) = prev2.as_deref().filter(|previous2| *previous2 != token) {
                    let trigram_key = (previous2.to_string(), previous.to_string());
                    let trigrams = self.context_trigrams.entry(trigram_key).or_default();
                    upsert_entry(trigrams, token.clone(), now, delta);
                    if let Some(entry) = trigrams.iter().find(|entry| entry.phrase == *token) {
                        updates.push(format!(
                            "T\t{}\t{}\t{}\t{}\t{}",
                            previous2, previous, entry.phrase, entry.freq, entry.last_used
                        ));
                    }
                }
            }

            prev2 = prev1;
            prev1 = Some(token.clone());
        }

        for token in tokens {
            if self
                .recent_word_tokens
                .first()
                .is_some_and(|recent| recent == &token)
            {
                continue;
            }
            self.recent_word_tokens.insert(0, token);
            self.recent_word_tokens.truncate(MAX_RECENT_WORD_TOKENS);
        }
    }

    pub fn record_commit_observation(
        &mut self,
        key: &str,
        phrase: &str,
        source: CommitSource,
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }
        let now = self.bump_clock();
        let adjacent_candidates = self.adjacent_novel_candidates(&key, &phrase, now, source);
        let mut updates = Vec::new();
        self.record_adjacent_novel_candidates(adjacent_candidates, now, &mut updates);
        self.push_recent_commit(key, phrase, now, source);
        if updates.is_empty() {
            Ok(())
        } else if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    fn adjacent_novel_candidates(
        &self,
        key: &str,
        phrase: &str,
        now: u64,
        source: CommitSource,
    ) -> Vec<(String, String)> {
        if matches!(source, CommitSource::Weak) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut prior: Vec<&RecentCommit> = Vec::new();
        for commit in self.recent_commits.iter().take(2) {
            if now.saturating_sub(commit.tick) > ADJACENT_NOVEL_MAX_EVENT_GAP {
                break;
            }
            if matches!(commit.source, CommitSource::Weak) {
                break;
            }
            prior.push(commit);
            let mut combined_key = String::new();
            let mut combined_phrase = String::new();
            for item in prior.iter().rev() {
                combined_key.push_str(&item.key);
                combined_phrase.push_str(&item.phrase);
            }
            combined_key.push_str(key);
            combined_phrase.push_str(phrase);
            if is_adjacent_novel_candidate(&combined_key, &combined_phrase)
                && !self.blocked_phrases.contains_key(&combined_phrase)
                && !out.iter().any(|(seen_key, seen_phrase)| {
                    seen_key == &combined_key && seen_phrase == &combined_phrase
                })
            {
                out.push((combined_key, combined_phrase));
            }
        }
        out
    }

    fn record_adjacent_novel_candidates(
        &mut self,
        candidates: Vec<(String, String)>,
        now: u64,
        updates: &mut Vec<String>,
    ) {
        for (key, phrase) in candidates {
            let observed_entries = self.observed_input.entry(key.clone()).or_default();
            upsert_entry(observed_entries, phrase.clone(), now, 1);
            let observed = observed_entries
                .iter()
                .find(|entry| entry.phrase == phrase)
                .map(LearnedEntry::stats)
                .unwrap_or_default();
            if observed.freq > 0 {
                updates.push(format!(
                    "O\t{}\t{}\t{}\t{}",
                    key, phrase, observed.freq, observed.last_used
                ));
            }

            let novel_entries = self.novel_phrases.entry(key.clone()).or_default();
            upsert_entry(novel_entries, phrase.clone(), now, 1);
            if let Some(entry) = novel_entries.iter().find(|entry| entry.phrase == phrase) {
                updates.push(format!(
                    "N\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                ));
            }

            if observed.freq >= ADJACENT_NOVEL_PROMOTE_FREQ {
                let exact_entries = self.exact_input.entry(key.clone()).or_default();
                upsert_entry(exact_entries, phrase.clone(), now, 1);
                if let Some(entry) = exact_entries
                    .iter_mut()
                    .find(|entry| entry.phrase == phrase)
                {
                    record_exact_evidence(entry, CommitSource::Composed);
                }
                register_approx_key(&mut self.approx_index, &key);
                if let Some(entry) = exact_entries.iter().find(|entry| entry.phrase == phrase) {
                    updates.push(format!(
                        "I\t{}\t{}\t{}\t{}\t{}",
                        key,
                        entry.phrase,
                        entry.freq,
                        entry.last_used,
                        u8::from(entry.pinned)
                    ));
                }
                upsert_stats(&mut self.phrase_stats, phrase.clone(), now, 1);
                if let Some(stats) = self.phrase_stats.get(&phrase).copied() {
                    updates.push(format!(
                        "P\t{}\t{}\t{}",
                        phrase, stats.freq, stats.last_used
                    ));
                }
            }
        }
    }

    fn push_recent_commit(&mut self, key: String, phrase: String, tick: u64, source: CommitSource) {
        if phrase.is_empty() {
            return;
        }
        if self
            .recent_commits
            .first()
            .is_some_and(|commit| commit.key == key && commit.phrase == phrase)
        {
            if let Some(commit) = self.recent_commits.first_mut() {
                commit.tick = tick;
                commit.source = source;
            }
            return;
        }
        self.recent_commits.insert(
            0,
            RecentCommit {
                key,
                phrase,
                tick,
                source,
            },
        );
        self.recent_commits.truncate(MAX_RECENT_COMMITS);
    }

    pub fn learn_input_only_with_delta(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }
        let delta = delta.max(1);

        let now = self.bump_clock();
        let mut updates = Vec::with_capacity(2);

        let exact_entries = self.exact_input.entry(key.clone()).or_default();
        upsert_entry(exact_entries, phrase.to_string(), now, delta);
        if let Some(entry) = exact_entries
            .iter_mut()
            .find(|entry| entry.phrase == phrase)
        {
            record_exact_evidence(entry, CommitSource::Weak);
        }
        register_approx_key(&mut self.approx_index, &key);
        if let Some(entry) = exact_entries.iter().find(|entry| entry.phrase == phrase) {
            updates.push(format!(
                "I\t{}\t{}\t{}\t{}\t{}",
                key,
                entry.phrase,
                entry.freq,
                entry.last_used,
                u8::from(entry.pinned)
            ));
        }

        upsert_stats(&mut self.phrase_stats, phrase.to_string(), now, delta);
        if let Some(stats) = self.phrase_stats.get(&phrase).copied() {
            updates.push(format!(
                "P\t{}\t{}\t{}",
                phrase, stats.freq, stats.last_used
            ));
        }

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn learn_exact_input_without_phrase_signal(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }
        let delta = delta.max(1);

        let now = self.bump_clock();
        let exact_entries = self.exact_input.entry(key.clone()).or_default();
        upsert_entry(exact_entries, phrase.to_string(), now, delta);
        if let Some(entry) = exact_entries
            .iter_mut()
            .find(|entry| entry.phrase == phrase)
        {
            record_exact_evidence(entry, CommitSource::Direct);
        }
        let updates = exact_entries
            .iter()
            .find(|entry| entry.phrase == phrase)
            .map(|entry| {
                vec![format!(
                    "I\t{}\t{}\t{}\t{}\t{}",
                    key,
                    entry.phrase,
                    entry.freq,
                    entry.last_used,
                    u8::from(entry.pinned)
                )]
            })
            .unwrap_or_default();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn record_novel_phrase(&mut self, key: &str, phrase: &str) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }

        let now = self.bump_clock();
        let entries = self.novel_phrases.entry(key.clone()).or_default();
        upsert_entry(entries, phrase.to_string(), now, 1);
        let updates = entries
            .iter()
            .find(|entry| entry.phrase == phrase)
            .map(|entry| {
                vec![format!(
                    "N\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                )]
            })
            .unwrap_or_default();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn learn_mixed_input(&mut self, key: &str, phrase: &str) -> io::Result<()> {
        self.learn_mixed_input_with_delta(key, phrase, 1)
    }

    pub fn learn_mixed_input_with_delta(
        &mut self,
        key: &str,
        phrase: &str,
        delta: u64,
    ) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(());
        }
        let delta = delta.max(1);

        let now = self.bump_clock();
        let mixed_entries = self.mixed_input.entry(key.clone()).or_default();
        upsert_entry(mixed_entries, phrase.to_string(), now, delta);

        let updates = mixed_entries
            .iter()
            .find(|entry| entry.phrase == phrase)
            .map(|entry| {
                vec![format!(
                    "M\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                )]
            })
            .unwrap_or_default();

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn observe_input(&mut self, key: &str, phrase: &str) -> io::Result<LearnedStats> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(LearnedStats::default());
        };
        if self.blocked_phrases.contains_key(&phrase) {
            return Ok(LearnedStats::default());
        }

        let now = self.bump_clock();
        let observed_entries = self.observed_input.entry(key.clone()).or_default();
        upsert_entry(observed_entries, phrase.to_string(), now, 1);

        let stats = observed_entries
            .iter()
            .find(|entry| entry.phrase == phrase)
            .map(LearnedEntry::stats)
            .unwrap_or_default();
        let updates = vec![format!(
            "O\t{}\t{}\t{}\t{}",
            key, phrase, stats.freq, stats.last_used
        )];

        if self.prune_for_memory() {
            self.persist_snapshot_now()?;
        } else {
            self.persist_updates(updates)?;
        }
        Ok(stats)
    }

    pub fn pin_exact_input(&mut self, key: &str, phrase: &str) -> io::Result<()> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(());
        };

        let now = self.bump_clock();
        let exact_entries = self.exact_input.entry(key.clone()).or_default();
        upsert_entry(exact_entries, phrase.to_string(), now, 1);
        if let Some(entry) = exact_entries
            .iter_mut()
            .find(|entry| entry.phrase == phrase)
        {
            entry.pinned = true;
            entry.confidence = SuggestionConfidence::Established;
        }
        // Re-sort so the newly pinned entry moves above unpinned ones.
        sort_entries(exact_entries);
        register_approx_key(&mut self.approx_index, &key);
        upsert_stats(&mut self.phrase_stats, phrase.to_string(), now, 1);

        let mut updates = exact_entries
            .iter()
            .find(|entry| entry.phrase == phrase)
            .map(|entry| {
                vec![format!(
                    "I\t{}\t{}\t{}\t{}\t{}",
                    key,
                    entry.phrase,
                    entry.freq,
                    entry.last_used,
                    u8::from(entry.pinned)
                )]
            })
            .unwrap_or_default();
        if let Some(stats) = self.phrase_stats.get(&phrase).copied() {
            updates.push(format!(
                "P\t{}\t{}\t{}",
                phrase, stats.freq, stats.last_used
            ));
        }

        if self.prune_for_memory() {
            self.persist_snapshot_now()
        } else {
            self.persist_updates(updates)
        }
    }

    pub fn unpin_exact_input(&mut self, key: &str, phrase: &str) -> io::Result<bool> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(false);
        };

        let now = self.bump_clock();
        let mut changed = false;
        let mut update = None;
        if let Some(entries) = self.exact_input.get_mut(&key) {
            if let Some(entry) = entries.iter_mut().find(|entry| entry.phrase == phrase) {
                if entry.pinned {
                    entry.pinned = false;
                    entry.last_used = now;
                    changed = true;
                    update = Some(format!(
                        "I\t{}\t{}\t{}\t{}\t{}",
                        key,
                        entry.phrase,
                        entry.freq,
                        entry.last_used,
                        u8::from(entry.pinned)
                    ));
                    // Re-sort so the unpinned entry moves below any remaining
                    // pinned entries for this key.
                    sort_entries(entries);
                }
            }
        }
        if changed {
            self.persist_updates(update.into_iter().collect())?;
        }
        Ok(changed)
    }

    pub fn remove_exact_input(&mut self, key: &str, phrase: &str) -> io::Result<bool> {
        let Some((key, phrase)) = validate_user_phrase_parts(key, phrase)? else {
            return Ok(false);
        };

        let mut removed = false;
        let mut remove_key = false;
        if let Some(entries) = self.exact_input.get_mut(&key) {
            let before = entries.len();
            entries.retain(|entry| entry.phrase != phrase);
            removed = entries.len() != before;
            remove_key = entries.is_empty();
        }
        if remove_key {
            self.exact_input.remove(&key);
            rebuild_approx_index(self);
        }
        let mixed_removed = remove_mixed_input_phrase(self, &key, &phrase);
        let observed_removed = remove_observed_input_phrase(self, &key, &phrase);
        let novel_removed = remove_novel_phrase(self, &key, &phrase);
        if removed || mixed_removed || observed_removed || novel_removed {
            prune_phrase_related_stats(self, &phrase);
            self.write_snapshot_sync()?;
        }
        Ok(removed || mixed_removed || observed_removed || novel_removed)
    }

    fn bump_clock(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1).max(1);
        self.clock
    }

    fn parse_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return;
        }

        let mut parts = line.split('\t');
        let kind = parts.next().unwrap_or_default();
        match kind {
            "I" => {
                let key = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let raw_freq = parse_u64(parts.next());
                let mut freq = raw_freq;
                let last_used = parse_last_used(parts.next(), freq);
                let mut pinned = parse_bool_flag(parts.next());
                if !pinned && raw_freq >= LEGACY_PINNED_EXACT_FREQ_DELTA {
                    pinned = true;
                    freq = raw_freq
                        .saturating_sub(LEGACY_PINNED_EXACT_FREQ_DELTA)
                        .max(1);
                }
                if !key.is_empty()
                    && !phrase.is_empty()
                    && freq > 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.exact_input.entry(key.clone()).or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, pinned);
                    register_approx_key(&mut self.approx_index, &key);
                    self.clock = self.clock.max(last_used);
                }
            }
            "M" => {
                let key = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !key.is_empty()
                    && !phrase.is_empty()
                    && freq > 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.mixed_input.entry(key).or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "O" => {
                let key = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !key.is_empty()
                    && !phrase.is_empty()
                    && freq > 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.observed_input.entry(key).or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "R" => {
                let raw = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let corrected = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !raw.is_empty() && !corrected.is_empty() && freq > 0 {
                    let entries = self.correction_pairs.entry(raw).or_default();
                    merge_loaded_entry(entries, corrected.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "N" => {
                let key = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !key.is_empty()
                    && !phrase.is_empty()
                    && freq > 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.novel_phrases.entry(key).or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "B" => {
                let phrase = parts.next().unwrap_or_default().trim();
                let last_used = parse_u64(parts.next());
                if normalize_selection_feedback_phrase(phrase).is_some() {
                    let phrase = phrase.to_string();
                    if last_used == 0 {
                        self.blocked_phrases.remove(&phrase);
                    } else {
                        let current = self
                            .blocked_phrases
                            .entry(phrase.clone())
                            .or_insert(last_used);
                        *current = (*current).max(last_used);
                        self.clock = self.clock.max(last_used);
                        remove_phrase_everywhere(self, &phrase);
                    }
                }
            }
            "S" => {
                let reading = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let score = parse_i64(parts.next());
                let last_used = parse_u64(parts.next()).max(1);
                if !reading.is_empty()
                    && !phrase.is_empty()
                    && score != 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.selection_feedback.entry(reading).or_default();
                    merge_loaded_selection_entry(entries, phrase.to_string(), score, last_used);
                    self.clock = self.clock.max(last_used);
                }
            }
            "U" => {
                let reading = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let score = parse_i64(parts.next());
                let last_used = parse_u64(parts.next()).max(1);
                if !reading.is_empty()
                    && !phrase.is_empty()
                    && score != 0
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.weak_unselected_feedback.entry(reading).or_default();
                    merge_loaded_selection_entry(entries, phrase.to_string(), score, last_used);
                    self.clock = self.clock.max(last_used);
                }
            }
            "X" => {
                let prev = parts.next().unwrap_or_default().trim();
                let reading = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let score = parse_i64(parts.next());
                let last_used = parse_u64(parts.next()).max(1);
                if !prev.is_empty()
                    && !reading.is_empty()
                    && !phrase.is_empty()
                    && score != 0
                    && !self.blocked_phrases.contains_key(prev)
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self
                        .context_selection_feedback
                        .entry((prev.to_string(), reading))
                        .or_default();
                    merge_loaded_selection_entry(entries, phrase.to_string(), score, last_used);
                    self.clock = self.clock.max(last_used);
                }
            }
            "Y" => {
                let prev = parts.next().unwrap_or_default().trim();
                let reading = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
                let phrase = parts.next().unwrap_or_default().trim();
                let score = parse_i64(parts.next());
                let last_used = parse_u64(parts.next()).max(1);
                if !prev.is_empty()
                    && !reading.is_empty()
                    && !phrase.is_empty()
                    && score != 0
                    && !self.blocked_phrases.contains_key(prev)
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self
                        .context_weak_unselected_feedback
                        .entry((prev.to_string(), reading))
                        .or_default();
                    merge_loaded_selection_entry(entries, phrase.to_string(), score, last_used);
                    self.clock = self.clock.max(last_used);
                }
            }
            "P" => {
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !phrase.is_empty() && freq > 0 && !self.blocked_phrases.contains_key(phrase) {
                    merge_loaded_stats(
                        &mut self.phrase_stats,
                        phrase.to_string(),
                        LearnedStats {
                            freq,
                            last_used,
                            short_count_q16: freq.saturating_mul(65_536),
                            long_count_q16: freq.saturating_mul(65_536),
                            last_decay_tick: last_used,
                        },
                    );
                    self.clock = self.clock.max(last_used);
                }
            }
            "C" => {
                let prev = parts.next().unwrap_or_default().trim();
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !prev.is_empty()
                    && !phrase.is_empty()
                    && freq > 0
                    && !self.blocked_phrases.contains_key(prev)
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self.context_links.entry(prev.to_string()).or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "T" => {
                let prev2 = parts.next().unwrap_or_default().trim();
                let prev1 = parts.next().unwrap_or_default().trim();
                let phrase = parts.next().unwrap_or_default().trim();
                let freq = parse_u64(parts.next());
                let last_used = parse_last_used(parts.next(), freq);
                if !prev2.is_empty()
                    && !prev1.is_empty()
                    && !phrase.is_empty()
                    && prev1 != phrase
                    && prev2 != phrase
                    && freq > 0
                    && !self.blocked_phrases.contains_key(prev2)
                    && !self.blocked_phrases.contains_key(prev1)
                    && !self.blocked_phrases.contains_key(phrase)
                {
                    let entries = self
                        .context_trigrams
                        .entry((prev2.to_string(), prev1.to_string()))
                        .or_default();
                    merge_loaded_entry(entries, phrase.to_string(), freq, last_used, false);
                    self.clock = self.clock.max(last_used);
                }
            }
            "D" => {
                let scope = parts.next().unwrap_or_default();
                let key1 = parts.next().unwrap_or_default();
                let key2 = parts.next().unwrap_or_default();
                let phrase = parts.next().unwrap_or_default();
                let short_count_q16 = parse_u64(parts.next());
                let long_count_q16 = parse_u64(parts.next());
                let last_decay_tick = parse_u64(parts.next());
                if last_decay_tick > 0 {
                    self.apply_decay_metadata(
                        scope,
                        key1,
                        key2,
                        phrase,
                        short_count_q16,
                        long_count_q16,
                        last_decay_tick,
                    );
                    self.clock = self.clock.max(last_decay_tick);
                }
            }
            "E" => {
                let key = parts.next().unwrap_or_default();
                let phrase = parts.next().unwrap_or_default();
                let confidence = SuggestionConfidence::from_u64(parse_u64(parts.next()));
                let observation_count = parse_u64(parts.next());
                let selection_count = parse_u64(parts.next());
                if let Some(entry) = self
                    .exact_input
                    .get_mut(key)
                    .and_then(|entries| entries.iter_mut().find(|entry| entry.phrase == phrase))
                {
                    entry.confidence = confidence;
                    entry.observation_count = observation_count;
                    entry.selection_count = selection_count;
                    if entry.pinned {
                        entry.confidence = SuggestionConfidence::Established;
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_decay_metadata(
        &mut self,
        scope: &str,
        key1: &str,
        key2: &str,
        phrase: &str,
        short_count_q16: u64,
        long_count_q16: u64,
        last_decay_tick: u64,
    ) {
        let entry = match scope {
            "I" => self.exact_input.get_mut(key1),
            "M" => self.mixed_input.get_mut(key1),
            "O" => self.observed_input.get_mut(key1),
            "R" => self.correction_pairs.get_mut(key1),
            "N" => self.novel_phrases.get_mut(key1),
            "C" => self.context_links.get_mut(key1),
            "T" => self
                .context_trigrams
                .get_mut(&(key1.to_string(), key2.to_string())),
            _ => None,
        }
        .and_then(|entries| entries.iter_mut().find(|entry| entry.phrase == phrase));
        if let Some(entry) = entry {
            if last_decay_tick >= entry.last_decay_tick {
                entry.short_count_q16 = short_count_q16;
                entry.long_count_q16 = long_count_q16;
                entry.last_decay_tick = last_decay_tick;
            }
            return;
        }
        if scope == "P" {
            if let Some(stats) = self.phrase_stats.get_mut(phrase) {
                if last_decay_tick >= stats.last_decay_tick {
                    stats.short_count_q16 = short_count_q16;
                    stats.long_count_q16 = long_count_q16;
                    stats.last_decay_tick = last_decay_tick;
                }
            }
        }
    }

    fn decay_metadata_line(
        scope: &str,
        key1: &str,
        key2: &str,
        phrase: &str,
        stats: LearnedStats,
    ) -> String {
        format!(
            "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            scope,
            key1,
            key2,
            phrase,
            stats.short_count_q16,
            stats.long_count_q16,
            stats.last_decay_tick
        )
    }

    fn evidence_metadata_line(key: &str, entry: &LearnedEntry) -> String {
        format!(
            "E\t{}\t{}\t{}\t{}\t{}",
            key,
            entry.phrase,
            entry.confidence as u8,
            entry.observation_count,
            entry.selection_count
        )
    }

    fn metadata_updates_for_lines(&self, lines: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for line in lines {
            let mut parts = line.split('\t');
            let kind = parts.next().unwrap_or_default();
            let (scope, key1, key2, phrase) = match kind {
                "I" | "M" | "O" | "R" | "N" => (
                    kind,
                    parts.next().unwrap_or_default(),
                    "",
                    parts.next().unwrap_or_default(),
                ),
                "P" => ("P", "", "", parts.next().unwrap_or_default()),
                "C" => (
                    "C",
                    parts.next().unwrap_or_default(),
                    "",
                    parts.next().unwrap_or_default(),
                ),
                "T" => (
                    "T",
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                ),
                _ => continue,
            };
            let entry = match scope {
                "I" => self.exact_input.get(key1),
                "M" => self.mixed_input.get(key1),
                "O" => self.observed_input.get(key1),
                "R" => self.correction_pairs.get(key1),
                "N" => self.novel_phrases.get(key1),
                "C" => self.context_links.get(key1),
                "T" => self
                    .context_trigrams
                    .get(&(key1.to_string(), key2.to_string())),
                _ => None,
            }
            .and_then(|entries| entries.iter().find(|entry| entry.phrase == phrase));
            if let Some(entry) = entry {
                out.push(Self::decay_metadata_line(
                    scope,
                    key1,
                    key2,
                    phrase,
                    entry.stats(),
                ));
                if scope == "I" {
                    out.push(Self::evidence_metadata_line(key1, entry));
                }
            } else if scope == "P" {
                if let Some(stats) = self.phrase_stats.get(phrase).copied() {
                    out.push(Self::decay_metadata_line(scope, key1, key2, phrase, stats));
                }
            }
        }
        out
    }

    pub fn flush_pending(&mut self) -> io::Result<()> {
        if let Some(persistence) = &self.persistence {
            persistence.flush()
        } else {
            self.write_snapshot_sync()
        }
    }

    fn serialize_snapshot(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for (key, entries) in &self.exact_input {
            for entry in entries {
                lines.push(format!(
                    "I\t{}\t{}\t{}\t{}\t{}",
                    key,
                    entry.phrase,
                    entry.freq,
                    entry.last_used,
                    u8::from(entry.pinned)
                ));
            }
        }
        for (key, entries) in &self.mixed_input {
            for entry in entries {
                lines.push(format!(
                    "M\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for (key, entries) in &self.observed_input {
            for entry in entries {
                lines.push(format!(
                    "O\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for (raw, entries) in &self.correction_pairs {
            for entry in entries {
                lines.push(format!(
                    "R\t{}\t{}\t{}\t{}",
                    raw, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for (key, entries) in &self.novel_phrases {
            for entry in entries {
                lines.push(format!(
                    "N\t{}\t{}\t{}\t{}",
                    key, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for (phrase, last_used) in &self.blocked_phrases {
            lines.push(format!("B\t{}\t{}", phrase, last_used));
        }
        for (reading, entries) in &self.selection_feedback {
            for entry in entries {
                lines.push(format!(
                    "S\t{}\t{}\t{}\t{}",
                    reading, entry.phrase, entry.score, entry.last_used
                ));
            }
        }
        for (reading, entries) in &self.weak_unselected_feedback {
            for entry in entries {
                lines.push(format!(
                    "U\t{}\t{}\t{}\t{}",
                    reading, entry.phrase, entry.score, entry.last_used
                ));
            }
        }
        for ((prev, reading), entries) in &self.context_selection_feedback {
            for entry in entries {
                lines.push(format!(
                    "X\t{}\t{}\t{}\t{}\t{}",
                    prev, reading, entry.phrase, entry.score, entry.last_used
                ));
            }
        }
        for ((prev, reading), entries) in &self.context_weak_unselected_feedback {
            for entry in entries {
                lines.push(format!(
                    "Y\t{}\t{}\t{}\t{}\t{}",
                    prev, reading, entry.phrase, entry.score, entry.last_used
                ));
            }
        }
        for (phrase, stats) in &self.phrase_stats {
            lines.push(format!(
                "P\t{}\t{}\t{}",
                phrase, stats.freq, stats.last_used
            ));
        }
        for (prev, entries) in &self.context_links {
            for entry in entries {
                lines.push(format!(
                    "C\t{}\t{}\t{}\t{}",
                    prev, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for ((prev2, prev1), entries) in &self.context_trigrams {
            for entry in entries {
                lines.push(format!(
                    "T\t{}\t{}\t{}\t{}\t{}",
                    prev2, prev1, entry.phrase, entry.freq, entry.last_used
                ));
            }
        }
        for (key, entries) in &self.exact_input {
            for entry in entries {
                lines.push(Self::decay_metadata_line(
                    "I",
                    key,
                    "",
                    &entry.phrase,
                    entry.stats(),
                ));
                lines.push(Self::evidence_metadata_line(key, entry));
            }
        }
        for (scope, map) in [
            ("M", &self.mixed_input),
            ("O", &self.observed_input),
            ("R", &self.correction_pairs),
            ("N", &self.novel_phrases),
            ("C", &self.context_links),
        ] {
            for (key, entries) in map {
                for entry in entries {
                    lines.push(Self::decay_metadata_line(
                        scope,
                        key,
                        "",
                        &entry.phrase,
                        entry.stats(),
                    ));
                }
            }
        }
        for (phrase, stats) in &self.phrase_stats {
            lines.push(Self::decay_metadata_line("P", "", "", phrase, *stats));
        }
        for ((prev2, prev1), entries) in &self.context_trigrams {
            for entry in entries {
                lines.push(Self::decay_metadata_line(
                    "T",
                    prev2,
                    prev1,
                    &entry.phrase,
                    entry.stats(),
                ));
            }
        }
        lines.join("\n")
    }

    fn write_snapshot_sync(&self) -> io::Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let wrote = write_user_dict_sqlite_snapshot_checked(
            &self.path,
            current_reset_stamp(&self.reset_stamp),
            self.serialize_snapshot().as_bytes(),
        )?;
        if !wrote {
            refresh_reset_stamp(&self.path, &self.reset_stamp);
        }
        Ok(())
    }

    fn persist_snapshot_now(&mut self) -> io::Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let snapshot = self.serialize_snapshot();
        if let Some(persistence) = &self.persistence {
            if persistence.snapshot(snapshot).is_ok() {
                self.learns_since_snapshot = 0;
                return Ok(());
            }
        }
        self.learns_since_snapshot = 0;
        self.write_snapshot_sync()
    }

    fn persist_updates(&mut self, updates: Vec<String>) -> io::Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }

        let mut updates = updates;
        let metadata_updates = self.metadata_updates_for_lines(&updates);
        updates.extend(metadata_updates);

        let Some(persistence) = &self.persistence else {
            return self.write_snapshot_sync();
        };

        if persistence.append(updates).is_err() {
            return self.write_snapshot_sync();
        }

        self.learns_since_snapshot = self.learns_since_snapshot.saturating_add(1);
        if self.learns_since_snapshot >= SNAPSHOT_EVERY_LEARNS {
            self.learns_since_snapshot = 0;
            if persistence.snapshot(self.serialize_snapshot()).is_err() {
                return self.write_snapshot_sync();
            }
        }
        Ok(())
    }

    fn prune_for_memory(&mut self) -> bool {
        let mut changed = false;
        let mut exact_index_changed = false;

        self.exact_input.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
                exact_index_changed = true;
            }
            !entries.is_empty()
        });
        self.mixed_input.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });
        self.observed_input.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.correction_pairs.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.novel_phrases.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.selection_feedback.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.weak_unselected_feedback.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.context_selection_feedback.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.context_weak_unselected_feedback.retain(|_, entries| {
            if entries.len() > MAX_USER_ENTRIES_PER_KEY {
                entries.truncate(MAX_USER_ENTRIES_PER_KEY);
                changed = true;
            }
            !entries.is_empty()
        });

        self.context_links.retain(|_, entries| {
            if entries.len() > MAX_CONTEXT_ENTRIES_PER_PREV {
                entries.truncate(MAX_CONTEXT_ENTRIES_PER_PREV);
                changed = true;
            }
            !entries.is_empty()
        });

        self.context_trigrams.retain(|_, entries| {
            if entries.len() > MAX_CONTEXT_ENTRIES_PER_PREV {
                entries.truncate(MAX_CONTEXT_ENTRIES_PER_PREV);
                changed = true;
            }
            !entries.is_empty()
        });

        if prune_total_exact_entries(&mut self.exact_input, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
            exact_index_changed = true;
        }
        if prune_total_exact_entries(&mut self.mixed_input, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
        }
        if prune_total_exact_entries(&mut self.observed_input, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
        }
        if prune_total_exact_entries(&mut self.correction_pairs, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
        }
        if prune_total_exact_entries(&mut self.novel_phrases, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
        }
        if prune_total_selection_feedback(&mut self.selection_feedback, MAX_TOTAL_EXACT_ENTRIES) {
            changed = true;
        }
        if prune_total_selection_feedback(
            &mut self.weak_unselected_feedback,
            MAX_TOTAL_EXACT_ENTRIES,
        ) {
            changed = true;
        }
        if prune_total_context_selection_feedback(
            &mut self.context_selection_feedback,
            MAX_TOTAL_CONTEXT_LINKS,
        ) {
            changed = true;
        }
        if prune_total_context_selection_feedback(
            &mut self.context_weak_unselected_feedback,
            MAX_TOTAL_CONTEXT_LINKS,
        ) {
            changed = true;
        }
        if prune_total_context_links(&mut self.context_links, MAX_TOTAL_CONTEXT_LINKS) {
            changed = true;
        }
        if prune_total_context_trigrams(&mut self.context_trigrams, MAX_TOTAL_CONTEXT_LINKS) {
            changed = true;
        }
        if prune_phrase_stats(&mut self.phrase_stats, MAX_PHRASE_STATS) {
            changed = true;
        }
        if prune_blocked_phrases(&mut self.blocked_phrases, MAX_BLOCKED_PHRASES) {
            changed = true;
        }
        if exact_index_changed {
            rebuild_approx_index(self);
        }
        changed
    }
}

pub fn default_user_dict_path() -> PathBuf {
    if let Ok(path) = std::env::var("SRF_USER_DICT_PATH") {
        return PathBuf::from(path);
    }
    if let Some(base) = crate::app_paths::local_data_dir() {
        return base.join("user_dict.sqlite");
    }
    PathBuf::from("user_dict.sqlite")
}

pub fn list_user_phrases() -> io::Result<Vec<ManagedUserPhrase>> {
    let lex = UserLexicon::load_default();
    let mut out = Vec::new();
    for (key, entries) in &lex.exact_input {
        for entry in entries {
            out.push(ManagedUserPhrase {
                key: key.clone(),
                phrase: entry.phrase.clone(),
                freq: entry.freq,
                last_used: entry.last_used,
            });
        }
    }
    out.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| b.freq.cmp(&a.freq))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
    Ok(out)
}

pub fn list_recent_novel_phrases(limit: usize) -> io::Result<Vec<ManagedNovelPhrase>> {
    let lex = UserLexicon::load_default();
    let mut out = Vec::new();
    for (key, entries) in &lex.novel_phrases {
        for entry in entries {
            out.push(ManagedNovelPhrase {
                key: key.clone(),
                phrase: entry.phrase.clone(),
                freq: entry.freq,
                last_used: entry.last_used,
            });
        }
    }
    out.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| b.freq.cmp(&a.freq))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
    if limit > 0 && out.len() > limit {
        out.truncate(limit);
    }
    Ok(out)
}

pub fn list_blocked_phrases(limit: usize) -> io::Result<Vec<ManagedBlockedPhrase>> {
    let lex = UserLexicon::load_default();
    let mut out: Vec<_> = lex
        .blocked_phrases
        .iter()
        .map(|(phrase, last_used)| ManagedBlockedPhrase {
            phrase: phrase.clone(),
            last_used: *last_used,
        })
        .collect();
    out.sort_by(|a, b| {
        b.last_used
            .cmp(&a.last_used)
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
    if limit > 0 && out.len() > limit {
        out.truncate(limit);
    }
    Ok(out)
}

pub fn block_user_phrase(phrase: &str) -> io::Result<bool> {
    let mut lex = UserLexicon::load_default();
    lex.block_phrase(phrase)
}

pub fn unblock_user_phrase(phrase: &str) -> io::Result<bool> {
    let mut lex = UserLexicon::load_default();
    lex.unblock_phrase(phrase)
}

pub fn add_user_phrase(key: &str, phrase: &str) -> io::Result<()> {
    let mut lex = UserLexicon::load_default();
    let _ = lex.unblock_phrase(phrase)?;
    lex.learn(key, phrase)?;
    lex.flush_pending()
}

pub fn pin_user_phrase(key: &str, phrase: &str) -> io::Result<()> {
    let mut lex = UserLexicon::load_default();
    let _ = lex.unblock_phrase(phrase)?;
    lex.pin_exact_input(key, phrase)?;
    lex.flush_pending()
}

pub fn remove_user_phrase(key: &str, phrase: &str) -> io::Result<bool> {
    let mut lex = UserLexicon::load_default();
    lex.remove_exact_input(key, phrase)
}

pub fn remove_user_phrase_everywhere(phrase: &str) -> io::Result<bool> {
    let Some(phrase) = normalize_selection_feedback_phrase(phrase) else {
        return Ok(false);
    };
    let mut lex = UserLexicon::load_default();
    let removed = remove_phrase_everywhere(&mut lex, &phrase);
    if removed {
        lex.write_snapshot_sync()?;
    }
    Ok(removed)
}

pub fn clear_user_dict() -> io::Result<()> {
    let path = default_user_dict_path();
    clear_user_dict_at_path(&path)
}

fn clear_user_dict_at_path(path: &Path) -> io::Result<()> {
    write_user_dict_sqlite_snapshot_with_reset(path, b"").map(|stamp| {
        update_shared_reset_stamp_for_path(path, stamp);
    })
}

pub fn export_user_dict(path: &Path) -> io::Result<()> {
    let lex = UserLexicon::load_from_path(default_user_dict_path())?;
    write_plain_user_dict_sqlite(path, &lex.serialize_snapshot())
}

pub fn export_decrypted_user_dict(path: &Path) -> io::Result<()> {
    export_user_dict(path)
}

pub fn import_user_dict(path: &Path) -> io::Result<()> {
    let contents = read_user_dict_sqlite_export_text(path)?;
    let target = default_user_dict_path();
    write_user_dict_sqlite_snapshot_with_reset(&target, contents.as_bytes()).map(|stamp| {
        update_shared_reset_stamp_for_path(&target, stamp);
    })
}

fn prune_phrase_related_stats(lex: &mut UserLexicon, phrase: &str) {
    let still_used = lex
        .exact_input
        .values()
        .flat_map(|entries| entries.iter())
        .any(|entry| entry.phrase == phrase);
    if !still_used {
        lex.phrase_stats.remove(phrase);
    }
    lex.context_links.retain(|prev, entries| {
        if prev == phrase {
            return false;
        }
        entries.retain(|entry| entry.phrase != phrase);
        !entries.is_empty()
    });
    lex.context_trigrams.retain(|(prev2, prev1), entries| {
        if prev2 == phrase || prev1 == phrase {
            return false;
        }
        entries.retain(|entry| entry.phrase != phrase);
        !entries.is_empty()
    });
    lex.selection_feedback.retain(|_, entries| {
        entries.retain(|entry| entry.phrase != phrase);
        !entries.is_empty()
    });
    lex.weak_unselected_feedback.retain(|_, entries| {
        entries.retain(|entry| entry.phrase != phrase);
        !entries.is_empty()
    });
    lex.context_selection_feedback.retain(|(prev, _), entries| {
        if prev == phrase {
            return false;
        }
        entries.retain(|entry| entry.phrase != phrase);
        !entries.is_empty()
    });
    lex.context_weak_unselected_feedback
        .retain(|(prev, _), entries| {
            if prev == phrase {
                return false;
            }
            entries.retain(|entry| entry.phrase != phrase);
            !entries.is_empty()
        });
    lex.recent_commits
        .retain(|commit| commit.phrase.as_str() != phrase);
    lex.recent_word_tokens.retain(|token| token != phrase);
}

fn remove_phrase_everywhere(lex: &mut UserLexicon, phrase: &str) -> bool {
    let exact_removed = remove_phrase_from_learned_map(&mut lex.exact_input, phrase);
    if exact_removed {
        rebuild_approx_index(lex);
    }
    let mut removed = exact_removed;
    removed |= remove_phrase_from_learned_map(&mut lex.mixed_input, phrase);
    removed |= remove_phrase_from_learned_map(&mut lex.observed_input, phrase);
    removed |= remove_phrase_from_learned_map(&mut lex.novel_phrases, phrase);
    removed |= remove_phrase_from_selection_feedback(&mut lex.selection_feedback, phrase);
    removed |= remove_phrase_from_selection_feedback(&mut lex.weak_unselected_feedback, phrase);
    removed |=
        remove_phrase_from_context_selection_feedback(&mut lex.context_selection_feedback, phrase);
    removed |= remove_phrase_from_context_selection_feedback(
        &mut lex.context_weak_unselected_feedback,
        phrase,
    );
    removed |= lex.phrase_stats.remove(phrase).is_some();
    prune_phrase_related_stats(lex, phrase);
    removed
}

fn remove_phrase_from_learned_map(
    map: &mut BTreeMap<String, Vec<LearnedEntry>>,
    phrase: &str,
) -> bool {
    let mut removed = false;
    map.retain(|_, entries| {
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed |= entries.len() != before;
        !entries.is_empty()
    });
    removed
}

fn remove_phrase_from_selection_feedback(
    map: &mut BTreeMap<String, Vec<SelectionFeedbackEntry>>,
    phrase: &str,
) -> bool {
    let mut removed = false;
    map.retain(|_, entries| {
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed |= entries.len() != before;
        !entries.is_empty()
    });
    removed
}

fn remove_phrase_from_context_selection_feedback(
    map: &mut BTreeMap<(String, String), Vec<SelectionFeedbackEntry>>,
    phrase: &str,
) -> bool {
    let mut removed = false;
    map.retain(|(prev, _), entries| {
        if prev == phrase {
            removed = true;
            return false;
        }
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed |= entries.len() != before;
        !entries.is_empty()
    });
    removed
}

fn remove_mixed_input_phrase(lex: &mut UserLexicon, key: &str, phrase: &str) -> bool {
    let mut remove_key = false;
    let mut removed = false;
    if let Some(entries) = lex.mixed_input.get_mut(key) {
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed = entries.len() != before;
        remove_key = entries.is_empty();
    }
    if remove_key {
        lex.mixed_input.remove(key);
    }
    removed
}

fn remove_observed_input_phrase(lex: &mut UserLexicon, key: &str, phrase: &str) -> bool {
    let mut remove_key = false;
    let mut removed = false;
    if let Some(entries) = lex.observed_input.get_mut(key) {
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed = entries.len() != before;
        remove_key = entries.is_empty();
    }
    if remove_key {
        lex.observed_input.remove(key);
    }
    removed
}

fn remove_novel_phrase(lex: &mut UserLexicon, key: &str, phrase: &str) -> bool {
    let mut remove_key = false;
    let mut removed = false;
    if let Some(entries) = lex.novel_phrases.get_mut(key) {
        let before = entries.len();
        entries.retain(|entry| entry.phrase != phrase);
        removed = entries.len() != before;
        remove_key = entries.is_empty();
    }
    if remove_key {
        lex.novel_phrases.remove(key);
    }
    removed
}

fn rebuild_approx_index(lex: &mut UserLexicon) {
    lex.approx_index.clear();
    for key in lex.exact_input.keys() {
        register_approx_key(&mut lex.approx_index, key);
    }
}

fn validate_user_phrase_parts(key: &str, phrase: &str) -> io::Result<Option<(String, String)>> {
    if key.chars().any(|ch| ch == '\t' || ch == '\n' || ch == '\r')
        || phrase
            .chars()
            .any(|ch| ch == '\t' || ch == '\n' || ch == '\r')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "user phrase key and text cannot contain tabs or line breaks",
        ));
    }

    let key = key.trim().to_ascii_lowercase();
    let phrase = phrase.trim().to_string();
    if key.is_empty() || phrase.is_empty() {
        return Ok(None);
    }
    if key.chars().count() > MAX_USER_KEY_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("user phrase key is too long (max {MAX_USER_KEY_CHARS} chars)"),
        ));
    }
    if phrase.chars().count() > MAX_USER_PHRASE_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("user phrase is too long (max {MAX_USER_PHRASE_CHARS} chars)"),
        ));
    }
    if !key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' || ch == ' ')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "user phrase key can only contain letters, digits, spaces, apostrophes, or hyphens",
        ));
    }
    Ok(Some((key, phrase)))
}

impl Drop for UserLexicon {
    fn drop(&mut self) {
        if let Some(mut persistence) = self.persistence.take() {
            let _ = persistence.shutdown();
        }
    }
}

impl PersistenceWorker {
    fn spawn(path: PathBuf, reset_stamp: SharedResetStamp) -> Self {
        let (tx, rx) = mpsc::channel();
        let join = thread::spawn(move || persistence_worker_loop(path, reset_stamp, rx));
        Self {
            tx,
            join: Some(join),
        }
    }

    fn append(&self, lines: Vec<String>) -> io::Result<()> {
        self.tx
            .send(PersistenceCommand::Append(lines))
            .map_err(persistence_send_error)
    }

    fn snapshot(&self, snapshot: String) -> io::Result<()> {
        self.tx
            .send(PersistenceCommand::Snapshot(snapshot))
            .map_err(persistence_send_error)
    }

    fn flush(&self) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PersistenceCommand::Flush(tx))
            .map_err(persistence_send_error)?;
        match rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(msg)) => Err(io::Error::other(msg)),
            Err(err) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("wait user dict flush: {err}"),
            )),
        }
    }

    fn shutdown(&mut self) -> io::Result<()> {
        let _ = self.flush();
        let _ = self.tx.send(PersistenceCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        Ok(())
    }
}

fn persistence_send_error<T>(err: mpsc::SendError<T>) -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("queue user dict persistence: {err}"),
    )
}

fn persistence_worker_loop(
    path: PathBuf,
    reset_stamp: SharedResetStamp,
    rx: mpsc::Receiver<PersistenceCommand>,
) {
    let mut pending_lines: Vec<String> = Vec::new();
    let mut pending_snapshot: Option<String> = None;

    loop {
        let mut should_stop = false;
        let mut flush_waiters = Vec::new();

        match rx.recv_timeout(Duration::from_millis(PERSIST_BATCH_WINDOW_MS)) {
            Ok(cmd) => handle_persistence_command(
                cmd,
                &mut pending_lines,
                &mut pending_snapshot,
                &mut flush_waiters,
                &mut should_stop,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if pending_lines.is_empty() && pending_snapshot.is_none() {
                    continue;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                should_stop = true;
            }
        }

        while let Ok(cmd) = rx.try_recv() {
            handle_persistence_command(
                cmd,
                &mut pending_lines,
                &mut pending_snapshot,
                &mut flush_waiters,
                &mut should_stop,
            );
        }

        let result = flush_persistence_batch(
            &path,
            &reset_stamp,
            &mut pending_lines,
            &mut pending_snapshot,
        );
        let status = match &result {
            Ok(()) => Ok(()),
            Err(err) => Err(err.to_string()),
        };
        for waiter in flush_waiters {
            let _ = waiter.send(status.clone());
        }

        if should_stop {
            break;
        }
    }
}

fn handle_persistence_command(
    cmd: PersistenceCommand,
    pending_lines: &mut Vec<String>,
    pending_snapshot: &mut Option<String>,
    flush_waiters: &mut Vec<mpsc::Sender<Result<(), String>>>,
    should_stop: &mut bool,
) {
    match cmd {
        PersistenceCommand::Append(lines) => pending_lines.extend(lines),
        PersistenceCommand::Snapshot(snapshot) => {
            pending_lines.clear();
            *pending_snapshot = Some(snapshot);
        }
        PersistenceCommand::Flush(waiter) => flush_waiters.push(waiter),
        PersistenceCommand::Shutdown => *should_stop = true,
    }
}

fn flush_persistence_batch(
    path: &Path,
    reset_stamp: &SharedResetStamp,
    pending_lines: &mut Vec<String>,
    pending_snapshot: &mut Option<String>,
) -> io::Result<()> {
    if let Some(snapshot) = pending_snapshot.take() {
        pending_lines.clear();
        let wrote = write_user_dict_sqlite_snapshot_checked(
            path,
            current_reset_stamp(reset_stamp),
            snapshot.as_bytes(),
        )?;
        if !wrote {
            refresh_reset_stamp(path, reset_stamp);
        }
        return Ok(());
    }
    if pending_lines.is_empty() {
        return Ok(());
    }
    let lines = std::mem::take(pending_lines);
    let wrote =
        apply_user_dict_sqlite_updates_checked(path, current_reset_stamp(reset_stamp), &lines)?;
    if !wrote {
        let refreshed = refresh_reset_stamp(path, reset_stamp);
        let wrote = apply_user_dict_sqlite_updates_checked(path, refreshed, &lines)?;
        if !wrote {
            refresh_reset_stamp(path, reset_stamp);
        }
    }
    Ok(())
}

fn apply_user_dict_sqlite_updates_checked(
    path: &Path,
    expected_reset: UserDictResetStamp,
    lines: &[String],
) -> io::Result<bool> {
    if lines.is_empty() {
        return Ok(true);
    }
    let _lock = UserDictExclusiveGuard::new(path)?;
    if user_dict_reset_stamp(path) != expected_reset {
        return Ok(false);
    }
    let mut lex = UserLexicon::empty_for_path(PathBuf::new());
    if let Some(text) = read_user_dict_sqlite_text(path)? {
        lex.parse_text_lines(&text);
    }
    for line in lines {
        lex.parse_line(line);
    }
    lex.prune_for_memory();
    write_user_dict_sqlite_snapshot_unlocked(path, lex.serialize_snapshot().as_bytes())?;
    Ok(true)
}

fn parse_u64(part: Option<&str>) -> u64 {
    part.unwrap_or_default()
        .trim()
        .parse::<u64>()
        .ok()
        .unwrap_or(0)
}

fn parse_last_used(part: Option<&str>, fallback: u64) -> u64 {
    let value = parse_u64(part);
    if value == 0 {
        fallback.max(1)
    } else {
        value
    }
}

fn parse_i64(part: Option<&str>) -> i64 {
    part.unwrap_or_default()
        .trim()
        .parse::<i64>()
        .ok()
        .unwrap_or(0)
}

fn parse_bool_flag(part: Option<&str>) -> bool {
    matches!(
        part.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("pinned")
    )
}

fn normalize_selection_feedback_reading(reading: &str) -> Option<String> {
    let reading = reading.trim().to_ascii_lowercase();
    if reading.is_empty() || reading.chars().count() > MAX_USER_KEY_CHARS {
        return None;
    }
    if !reading
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' || ch == ' ')
    {
        return None;
    }
    Some(reading)
}

fn normalize_selection_feedback_phrase(phrase: &str) -> Option<String> {
    let phrase = phrase.trim();
    if phrase.is_empty() || phrase.chars().count() > MAX_USER_PHRASE_CHARS {
        return None;
    }
    if phrase
        .chars()
        .any(|ch| ch == '\t' || ch == '\r' || ch == '\n' || ch.is_control())
    {
        return None;
    }
    let mut has_printable = false;
    let mut all_ascii_punctuation = true;
    let mut all_ascii_digits = true;
    for ch in phrase.chars() {
        if !ch.is_whitespace() {
            has_printable = true;
        }
        if !ch.is_ascii_punctuation() && !ch.is_whitespace() {
            all_ascii_punctuation = false;
        }
        if !ch.is_ascii_digit() && !ch.is_whitespace() {
            all_ascii_digits = false;
        }
    }
    if !has_printable || all_ascii_punctuation || all_ascii_digits {
        return None;
    }
    Some(phrase.to_string())
}

fn is_adjacent_novel_candidate(key: &str, phrase: &str) -> bool {
    if key.is_empty() || phrase.is_empty() {
        return false;
    }
    if key.chars().count() > MAX_USER_KEY_CHARS {
        return false;
    }
    let chars = phrase.chars().count();
    if !(ADJACENT_NOVEL_MIN_CHARS..=ADJACENT_NOVEL_MAX_CHARS).contains(&chars) {
        return false;
    }
    if phrase
        .chars()
        .any(|ch| ch == '\t' || ch == '\r' || ch == '\n' || ch.is_control())
    {
        return false;
    }
    key.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-' || ch == ' ')
}

fn upsert_entry(entries: &mut Vec<LearnedEntry>, phrase: String, now: u64, delta: u64) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.phrase == phrase) {
        add_decayed_entry_counts(entry, now, delta);
    } else {
        entries.push(LearnedEntry {
            phrase,
            freq: delta,
            last_used: now,
            pinned: false,
            short_count_q16: delta.saturating_mul(65_536),
            long_count_q16: delta.saturating_mul(65_536),
            last_decay_tick: now,
            confidence: SuggestionConfidence::Remembered,
            observation_count: 0,
            selection_count: 0,
        });
    }
    sort_entries(entries);
}

fn add_decayed_entry_counts(entry: &mut LearnedEntry, now: u64, delta: u64) {
    let mut stats = entry.stats();
    stats.add_decayed(
        now,
        delta,
        SHORT_COUNT_HALF_LIFE_TICKS,
        LONG_COUNT_HALF_LIFE_TICKS,
    );
    entry.freq = stats.freq;
    entry.last_used = stats.last_used;
    entry.short_count_q16 = stats.short_count_q16;
    entry.long_count_q16 = stats.long_count_q16;
    entry.last_decay_tick = stats.last_decay_tick;
}

fn refresh_suggestion_confidence(entry: &mut LearnedEntry) {
    if entry.pinned {
        entry.confidence = SuggestionConfidence::Established;
        return;
    }
    let evidence = entry
        .observation_count
        .saturating_add(entry.selection_count.saturating_mul(2));
    entry.confidence = if evidence >= 6 {
        SuggestionConfidence::Established
    } else if evidence >= 3 {
        SuggestionConfidence::Confirmed
    } else {
        SuggestionConfidence::Remembered
    };
}

fn record_exact_evidence(entry: &mut LearnedEntry, source: CommitSource) {
    entry.observation_count = entry.observation_count.saturating_add(1);
    // A phrase explicitly assembled candidate-by-candidate is as deliberate as
    // a direct candidate selection. Count that intent so standard learning is
    // at least Confirmed immediately.
    if matches!(source, CommitSource::Direct | CommitSource::Composed) {
        entry.selection_count = entry.selection_count.saturating_add(1);
    }
    refresh_suggestion_confidence(entry);
}

fn merge_loaded_entry(
    entries: &mut Vec<LearnedEntry>,
    phrase: String,
    freq: u64,
    last_used: u64,
    pinned: bool,
) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.phrase == phrase) {
        entry.freq = entry.freq.max(freq);
        entry.last_used = entry.last_used.max(last_used);
        entry.pinned |= pinned;
    } else {
        entries.push(LearnedEntry {
            phrase,
            freq,
            last_used,
            pinned,
            short_count_q16: freq.saturating_mul(65_536),
            long_count_q16: freq.saturating_mul(65_536),
            last_decay_tick: last_used,
            confidence: if pinned || freq >= 4 {
                SuggestionConfidence::Established
            } else if freq >= 2 {
                SuggestionConfidence::Confirmed
            } else {
                SuggestionConfidence::Remembered
            },
            observation_count: freq,
            selection_count: 0,
        });
    }
    sort_entries(entries);
}

fn merge_loaded_selection_entry(
    entries: &mut Vec<SelectionFeedbackEntry>,
    phrase: String,
    score: i64,
    last_used: u64,
) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.phrase == phrase) {
        if last_used > entry.last_used
            || (last_used == entry.last_used && score.abs() >= entry.score.abs())
        {
            entry.score = score;
        }
        entry.last_used = entry.last_used.max(last_used);
    } else {
        entries.push(SelectionFeedbackEntry {
            phrase,
            score,
            last_used,
        });
    }
    entries.sort_by(|a, b| {
        b.score
            .abs()
            .cmp(&a.score.abs())
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
}

fn apply_selection_deltas(
    entries: &mut Vec<SelectionFeedbackEntry>,
    deltas: &BTreeMap<String, i64>,
    now: u64,
) {
    for (phrase, delta) in deltas {
        if let Some(entry) = entries.iter_mut().find(|entry| entry.phrase == *phrase) {
            entry.score = entry.score.saturating_add(*delta);
            entry.last_used = now;
        } else {
            entries.push(SelectionFeedbackEntry {
                phrase: phrase.clone(),
                score: *delta,
                last_used: now,
            });
        }
    }
    entries.sort_by(|a, b| {
        b.score
            .abs()
            .cmp(&a.score.abs())
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
}

fn sort_entries(entries: &mut [LearnedEntry]) {
    entries.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.freq.cmp(&a.freq))
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
}

fn prune_total_exact_entries(
    map: &mut BTreeMap<String, Vec<LearnedEntry>>,
    max_entries: usize,
) -> bool {
    let total: usize = map.values().map(Vec::len).sum();
    if total <= max_entries {
        return false;
    }
    let remove_count = total - max_entries;
    let mut ranked = Vec::with_capacity(total);
    for (key, entries) in map.iter() {
        for entry in entries {
            ranked.push((
                entry.last_used,
                entry.freq,
                key.clone(),
                entry.phrase.clone(),
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, key, phrase)| joined_prune_key(&key, &phrase))
        .collect();
    map.retain(|key, entries| {
        entries.retain(|entry| !contains_joined_prune_key(&remove, key, &entry.phrase));
        !entries.is_empty()
    });
    true
}

fn prune_total_context_links(
    map: &mut BTreeMap<String, Vec<LearnedEntry>>,
    max_entries: usize,
) -> bool {
    let total: usize = map.values().map(Vec::len).sum();
    if total <= max_entries {
        return false;
    }
    let remove_count = total - max_entries;
    let mut ranked = Vec::with_capacity(total);
    for (prev, entries) in map.iter() {
        for entry in entries {
            ranked.push((
                entry.last_used,
                entry.freq,
                prev.clone(),
                entry.phrase.clone(),
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, prev, phrase)| joined_prune_key(&prev, &phrase))
        .collect();
    map.retain(|prev, entries| {
        entries.retain(|entry| !contains_joined_prune_key(&remove, prev, &entry.phrase));
        !entries.is_empty()
    });
    true
}

fn prune_total_selection_feedback(
    map: &mut BTreeMap<String, Vec<SelectionFeedbackEntry>>,
    max_entries: usize,
) -> bool {
    let total: usize = map.values().map(Vec::len).sum();
    if total <= max_entries {
        return false;
    }
    let remove_count = total - max_entries;
    let mut ranked = Vec::with_capacity(total);
    for (reading, entries) in map.iter() {
        for entry in entries {
            ranked.push((
                entry.last_used,
                entry.score.unsigned_abs(),
                reading.clone(),
                entry.phrase.clone(),
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, reading, phrase)| joined_prune_key(&reading, &phrase))
        .collect();
    map.retain(|reading, entries| {
        entries.retain(|entry| !contains_joined_prune_key(&remove, reading, &entry.phrase));
        !entries.is_empty()
    });
    true
}

fn prune_total_context_selection_feedback(
    map: &mut BTreeMap<(String, String), Vec<SelectionFeedbackEntry>>,
    max_entries: usize,
) -> bool {
    let total: usize = map.values().map(Vec::len).sum();
    if total <= max_entries {
        return false;
    }
    let remove_count = total - max_entries;
    let mut ranked = Vec::with_capacity(total);
    for ((prev, reading), entries) in map.iter() {
        for entry in entries {
            ranked.push((
                entry.last_used,
                entry.score.unsigned_abs(),
                prev.clone(),
                reading.clone(),
                entry.phrase.clone(),
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, prev, reading, phrase)| joined_triple_prune_key(&prev, &reading, &phrase))
        .collect();
    map.retain(|(prev, reading), entries| {
        entries.retain(|entry| !contains_triple_prune_key(&remove, prev, reading, &entry.phrase));
        !entries.is_empty()
    });
    true
}

fn joined_prune_key(left: &str, right: &str) -> String {
    let mut out = String::with_capacity(left.len() + right.len() + 1);
    out.push_str(left);
    out.push('\0');
    out.push_str(right);
    out
}

fn joined_triple_prune_key(left: &str, middle: &str, right: &str) -> String {
    let mut out = String::with_capacity(left.len() + middle.len() + right.len() + 2);
    out.push_str(left);
    out.push('\0');
    out.push_str(middle);
    out.push('\0');
    out.push_str(right);
    out
}

fn contains_joined_prune_key(remove: &HashSet<String>, left: &str, right: &str) -> bool {
    let mut key = String::with_capacity(left.len() + right.len() + 1);
    key.push_str(left);
    key.push('\0');
    key.push_str(right);
    remove.contains(key.as_str())
}

fn contains_triple_prune_key(
    remove: &HashSet<String>,
    left: &str,
    middle: &str,
    right: &str,
) -> bool {
    let mut key = String::with_capacity(left.len() + middle.len() + right.len() + 2);
    key.push_str(left);
    key.push('\0');
    key.push_str(middle);
    key.push('\0');
    key.push_str(right);
    remove.contains(&key)
}

fn prune_total_context_trigrams(
    map: &mut BTreeMap<(String, String), Vec<LearnedEntry>>,
    max_entries: usize,
) -> bool {
    let total: usize = map.values().map(Vec::len).sum();
    if total <= max_entries {
        return false;
    }
    let remove_count = total - max_entries;
    let mut ranked = Vec::with_capacity(total);
    for ((prev2, prev1), entries) in map.iter() {
        for entry in entries {
            ranked.push((
                entry.last_used,
                entry.freq,
                prev2.clone(),
                prev1.clone(),
                entry.phrase.clone(),
            ));
        }
    }
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<(String, String, String)> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, prev2, prev1, phrase)| (prev2, prev1, phrase))
        .collect();
    map.retain(|(prev2, prev1), entries| {
        entries.retain(|entry| {
            !remove.contains(&(prev2.clone(), prev1.clone(), entry.phrase.clone()))
        });
        !entries.is_empty()
    });
    true
}

fn prune_phrase_stats(map: &mut BTreeMap<String, LearnedStats>, max_entries: usize) -> bool {
    if map.len() <= max_entries {
        return false;
    }
    let remove_count = map.len() - max_entries;
    let mut ranked: Vec<_> = map
        .iter()
        .map(|(phrase, stats)| (stats.last_used, stats.freq, phrase.clone()))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, _, phrase)| phrase)
        .collect();
    map.retain(|phrase, _| !remove.contains(phrase));
    true
}

fn prune_blocked_phrases(map: &mut BTreeMap<String, u64>, max_entries: usize) -> bool {
    if map.len() <= max_entries {
        return false;
    }
    let remove_count = map.len() - max_entries;
    let mut ranked: Vec<_> = map
        .iter()
        .map(|(phrase, last_used)| (*last_used, phrase.clone()))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let remove: HashSet<String> = ranked
        .into_iter()
        .take(remove_count)
        .map(|(_, phrase)| phrase)
        .collect();
    map.retain(|phrase, _| !remove.contains(phrase));
    true
}

fn upsert_stats(map: &mut BTreeMap<String, LearnedStats>, phrase: String, now: u64, delta: u64) {
    let stats = map.entry(phrase).or_default();
    stats.add_decayed(
        now,
        delta,
        SHORT_COUNT_HALF_LIFE_TICKS,
        LONG_COUNT_HALF_LIFE_TICKS,
    );
}

fn merge_loaded_stats(
    map: &mut BTreeMap<String, LearnedStats>,
    phrase: String,
    stats: LearnedStats,
) {
    let current = map.entry(phrase).or_default();
    if stats.last_used > current.last_used
        || (stats.last_used == current.last_used && stats.freq >= current.freq)
    {
        *current = stats;
    }
}

fn approximate_lookup_scored(
    map: &BTreeMap<String, Vec<LearnedEntry>>,
    approx_index: &HashMap<String, Vec<String>>,
    key: &str,
    max_distance: usize,
    limit: usize,
) -> Vec<(LearnedEntry, usize)> {
    if key.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut ranked: Vec<(usize, usize, LearnedEntry)> = Vec::new();
    let candidates = if max_distance == 1 && key.len() <= APPROX_INDEX_MAX_KEY_LEN {
        candidate_keys_from_index(approx_index, key)
    } else {
        Vec::new()
    };
    if !candidates.is_empty() {
        for candidate_key in candidates {
            let Some(entries) = map.get(&candidate_key) else {
                continue;
            };
            if candidate_key == key {
                continue;
            }
            let Some(distance) = bounded_edit_distance(&candidate_key, key, max_distance) else {
                continue;
            };
            for (rank, entry) in entries.iter().take(3).enumerate() {
                if learned_phrase_char_count(&entry.phrase) == 1 {
                    continue;
                }
                ranked.push((distance, rank, entry.clone()));
            }
        }
    } else {
        for (candidate_key, entries) in map {
            if candidate_key == key {
                continue;
            }
            let Some(distance) = bounded_edit_distance(candidate_key, key, max_distance) else {
                continue;
            };
            for (rank, entry) in entries.iter().take(3).enumerate() {
                if learned_phrase_char_count(&entry.phrase) == 1 {
                    continue;
                }
                ranked.push((distance, rank, entry.clone()));
            }
        }
    }

    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| b.2.freq.cmp(&a.2.freq))
            .then_with(|| b.2.last_used.cmp(&a.2.last_used))
            .then_with(|| a.2.phrase.cmp(&b.2.phrase))
    });

    let mut out: Vec<(LearnedEntry, usize)> = Vec::new();
    for (dist, _, entry) in ranked {
        if out.iter().any(|(e, _)| e.phrase == entry.phrase) {
            continue;
        }
        out.push((entry, dist));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn register_approx_key(index: &mut HashMap<String, Vec<String>>, key: &str) {
    if key.is_empty() || key.len() > APPROX_INDEX_MAX_KEY_LEN {
        return;
    }
    for deleted in deletion_forms(key) {
        let bucket = index.entry(deleted).or_default();
        if !bucket.iter().any(|seen| seen == key) {
            bucket.push(key.to_string());
        }
    }
}

fn learned_phrase_char_count(phrase: &str) -> usize {
    phrase.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn candidate_keys_from_index(index: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for deleted in deletion_forms(key) {
        let Some(bucket) = index.get(&deleted) else {
            continue;
        };
        for candidate in bucket {
            if seen.insert(candidate.clone()) {
                out.push(candidate.clone());
            }
        }
    }
    out
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
