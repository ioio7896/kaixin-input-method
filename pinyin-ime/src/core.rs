use crate::candidate_prefs;
use crate::compiled_data::shared_engine_model;
use crate::correction_prefs::{CorrectionLevel, CorrectionPrefs};
use crate::engine::{decode, decode_until, IncrementalDecodeCache};
use crate::lm::Lm;
use crate::rerank_prefs;
use crate::runtime_log::{self, RuntimeLogLevel};
use crate::segment::{
    expand_mixed_pinyin_exact_key_details_by_score, expand_mixed_pinyin_exact_keys,
    normalize_compact_pinyin_aliases, parse_input_variants, parse_input_variants_detailed,
    predict_syllable_completions_by_score, split_syllables_with_tail_mode, IncrementalParseCache,
    MixedPinyinIncrementalCache, MixedPinyinKeyExpansion, ParseOptions, ParseVariant,
    ParseVariantSource,
};
use crate::text_norm::normalize_pinyin_line;
use crate::thuocl::{
    load_dir_txt_with_profile_default_enabled, load_dir_txt_with_profile_filtered_by_prefs,
    try_load_hot_prebaked_near, try_load_prebaked_near, AbbrevLexicon, LexiconBuildProfile,
    LexiconLayer, ThuoclEntry,
};
use crate::user_dict::{
    default_user_dict_path, CommitSource, LearnedEntry, LearnedStats, UserLexicon,
};
use crate::user_dict_io::user_dict_reset_stamp;
use crate::user_hotword_prefs;
use lru::LruCache;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime};
mod cache;
mod learning;
mod lookup;
mod postprocess;
mod ranking;

use cache::*;
pub(crate) use cache::{
    lookup_request_superseded, next_lookup_request_generation, publish_latest_lookup_request_id,
    LookupCancellationScope, LookupCancellationToken,
};
pub(crate) use cache::{parse_engine_tuning_ini, EngineTuning};
use learning::*;
pub use lookup::default_phrase_lexicon_dir;
pub(crate) use lookup::validate_trusted_phrase_dir;
use lookup::*;
use postprocess::*;
use ranking::*;

pub const TSF_PAGE_SIZE: usize = 9;
pub const TSF_MAX_CANDIDATES: usize = 128;

pub const MODE_FUZZY_PINYIN: u32 = 0x0001;
pub const MODE_DOUBLE_PINYIN: u32 = 0x0002;
pub const MODE_JIANPIN: u32 = 0x0004;
pub const MODE_V_ASSIST: u32 = 0x0008;
pub const MODE_U_MODE: u32 = 0x0010;
pub const MODE_DATE_AUTO_FORMAT: u32 = 0x0020;
pub const MODE_CLIPBOARD_BACKGROUND: u32 = 0x0040;
pub const MODE_MIXED_PINYIN: u32 = 0x0080;
pub const MODE_MIXED_PINYIN_AGGRESSIVE: u32 = 0x0100;
pub const MODE_TRADITIONAL_OUTPUT: u32 = 0x0200;
pub const MODE_ENGLISH_WORD_INPUT: u32 = 0x0400;
pub const MODE_CLIPBOARD_DISABLED: u32 = 0x0800;
pub const MODE_USER_PHRASE_COMPOSE_ACTIVE: u32 = 0x1000;
pub const MODE_SYMBOL_TOOLBOX: u32 = 0x2000;
pub const MODE_EMOJI_INPUT: u32 = 0x4000;
pub const MODE_LEARNING_AGGRESSIVE: u32 = 0x8000;
pub const MODE_LEARNING_CONSERVATIVE: u32 = 0x1_0000;
pub const LEARN_FLAG_WEAK: u32 = 1 << 0;
pub const LEARN_FLAG_COMPOSED_PHRASE: u32 = 1 << 1;
/// TSF 端已标记为 no_learn 的候选，Rust 端作为兜底二次校验。
pub const LEARN_FLAG_NO_LEARN: u32 = 1 << 2;

const DEFAULT_MODE_FLAGS: u32 = MODE_JIANPIN | MODE_MIXED_PINYIN;

static RADICAL_PINYIN_CACHE: OnceLock<Vec<(String, String)>> = OnceLock::new();
static NEXT_SHARED_DATA_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_shared_data_generation() -> u64 {
    NEXT_SHARED_DATA_GENERATION
        .fetch_add(1, Ordering::Relaxed)
        .max(1)
}

const BASE_DECODE_BEAM: usize = 48;
const BASE_DECODE_LIMIT: usize = 96;
/// 缩写模式使用较小的 beam，降低计算量，同时对缩写准确度影响甚微。

const ABBREV_DECODE_BEAM: usize = 32;
const PREDICT_DECODE_TOP: usize = 18;
const PREDICT_WORD_GRAPH_TOP: usize = 6;
const MAX_PARSE_VARIANTS: usize = 16;
const WORD_GRAPH_BEAM: usize = 40;
const WORD_GRAPH_MAX_SPAN: usize = 6;
const WORD_GRAPH_PHRASE_LIMIT: usize = 6;
const WORD_GRAPH_CHAR_LIMIT: usize = 12;
const WORD_GRAPH_SENTENCE_BONUS: f64 = 18.0;
const WORD_GRAPH_PHRASE_EDGE_BONUS: f64 = 7.5;
const WORD_GRAPH_USER_EDGE_BONUS: f64 = 11.0;
const WORD_GRAPH_TOKEN_PENALTY: f64 = 0.45;
const WORD_GRAPH_SINGLE_TOKEN_PENALTY: f64 = 3.0;
const WORD_GRAPH_CONSECUTIVE_SINGLE_PENALTY: f64 = 6.0;
const PHRASE_EXACT_PINYIN_BONUS: f64 = 220.0;
const PHRASE_EXACT_MIXED_PINYIN_BONUS: f64 = 300.0;
const PHRASE_DIRECT_PINYIN_LOOKUP_BONUS: f64 = 325.0;
const PHRASE_PREFIX_PINYIN_BONUS: f64 = 185.0;
const PHRASE_EXACT_ABBREV_BONUS: f64 = 245.0;
const PHRASE_PREFIX_ABBREV_BONUS: f64 = 205.0;
const USER_EXACT_INPUT_BONUS: f64 = 280.0;
const USER_MIXED_INPUT_BONUS: f64 = 168.0;
const USER_OBSERVED_INPUT_BONUS: f64 = 82.0;
const USER_MIXED_INPUT_MIN_FREQ: u64 = 2;
const USER_EXACT_INPUT_SCALE: f64 = 24.0;
const USER_PHRASE_SCORE_SCALE: f64 = 8.0;
const USER_RECENCY_SCALE: f64 = 15.0;
const USER_LOW_LEXICON_FREQ_THRESHOLD: u64 = 80_000;
const USER_LOW_LEXICON_FREQ_DISCOUNT: f64 = 0.42;
const USER_LEXICON_LOW_TRUST_MULTIPLIER: f64 = 0.30;
const USER_PHRASE_COMPOSE_SINGLE_BOOST: f64 = 42.0;
const LOOKUP_STABILITY_BONUS: f64 = 9.0;
const LOOKUP_STABILITY_TOP_N: usize = 5;
const FIRST_LEARNED_USER_SOFT_CAP: f64 = 34.0;
const CONTEXT_FREQ_SCALE: f64 = 10.5;
const CONTEXT_RECENCY_SCALE: f64 = 9.0;
const CONTEXT_SUM_CAP: f64 = 28.0;
const CONTEXT_TRIGRAM_FREQ_SCALE: f64 = 18.0;
const CONTEXT_TRIGRAM_RECENCY_SCALE: f64 = 13.5;
const CONTEXT_TRIGRAM_CAP: f64 = 22.0;
const CONTEXT_HEAD_MAX_LEN: usize = 6;
const CONTEXT_TOKEN_DECAY: f64 = 0.72;
const CONTEXT_SUM_DECAY: f64 = 0.92;
/// 频率衰减半衰期：每经过 500 次学习事件，未使用词的有效频率减半。

const SHORT_TERM_FREQ_DECAY_HALF_LIFE: f64 = 24.0;
const LONG_TERM_FREQ_DECAY_HALF_LIFE: f64 = 4000.0;
const SHORT_TERM_SIGNAL_WEIGHT: f64 = 0.72;
const LONG_TERM_SIGNAL_WEIGHT: f64 = 0.34;
const RECENCY_SIGNAL_WEIGHT: f64 = 0.80;
const CONTEXT_HEAD_DISCOUNT: f64 = 0.65;
const WORD_TRANSITION_SCALE: f64 = 6.8;
const WORD_TRANSITION_RECENCY_SCALE: f64 = 3.0;
const WORD_TRANSITION_CAP: f64 = 18.0;
const SYLLABLE_PREDICT_USER_HIT_SCALE: f64 = 3.2;
const SYLLABLE_PREDICT_LEX_FREQ_SCALE: f64 = 1.25;
const SYLLABLE_PREDICT_LEX_PREFIX_SCALE: f64 = 0.9;
const SYLLABLE_PREDICT_LEX_SUFFIX_SCALE: f64 = 1.4;
const SYLLABLE_PREDICT_LEX_PARTITION_SCALE: f64 = 1.8;
const SYLLABLE_PREDICT_DICT_PRIOR_SCALE: f64 = 1.15;
const VARIANT_PENALTY: f64 = 0.25;
const PREDICTION_PENALTY: f64 = 0.05;
const TYPO_CORRECTION_PENALTY: f64 = 16.0;
const TYPO_DISTANCE2_EXTRA_PENALTY: f64 = 22.0;
const TYPO_LOOKUP_LIMIT: usize = 16;
const TYPO_RESULT_LIMIT: usize = 3;
// Long-input correction is a fallback, not part of the first-page critical
// path. Keep its index probes bounded so an exact/near-exact reading does not
// spend tens of milliseconds exploring low-value edit variants.
const LONG_CORRECTION_INPUT_CHARS: usize = 8;
const LONG_CORRECTION_LOOKUP_LIMIT: usize = 8;
const LONG_MISSING_LETTER_VARIANT_LIMIT: usize = 32;
/// 用户对该输入的 exact_input 累计频率达到此阈值后，不再触发字符级/缺字母纠错，
/// 避免把用户常用简拼、缩写或人名拼音强行纠成别的词。
const USER_EXACT_SUPPRESS_CORRECTION_THRESHOLD: u64 = 5;
const SELECTION_POSITIVE_FEEDBACK_BONUS: f64 = 12.0;
const SELECTION_NEGATIVE_FEEDBACK_PENALTY: f64 = -18.0;
const SELECTION_FINAL_NEGATIVE_FEEDBACK_SCALE: f64 = 0.65;
const SELECTION_FEEDBACK_SCORE_CAP: i64 = 10;
const SELECTION_FEEDBACK_HALF_LIFE_TICKS: f64 = 1200.0;
const SELECTION_SELECTED_BASE_DELTA: i64 = 2;
const FOUR_CHAR_SELECTION_EXTRA_DELTA: i64 = 1;
const REMOVED_PHRASE_FEEDBACK_DELTA: i64 = -SELECTION_FEEDBACK_SCORE_CAP;
const SELECTION_PAGE_SELECTED_DELTA: i64 = 4;
const SELECTION_FRONT_PENALTY: i64 = -2;
const SELECTION_SKIPPED_PENALTY: i64 = -1;
const SELECTION_PREVIOUS_PAGE_EXTRA_PENALTY: i64 = -1;
const SELECTION_CONTEXT_FEEDBACK_SCALE: f64 = 0.72;
const WEAK_UNSELECTED_FEEDBACK_SCORE_CAP: i64 = 12;
const WEAK_UNSELECTED_FEEDBACK_APPLY_THRESHOLD: i64 = 3;
const WEAK_UNSELECTED_FEEDBACK_PENALTY: f64 = -2.6;
const WEAK_UNSELECTED_FEEDBACK_HALF_LIFE_TICKS: f64 = 240.0;
const TOP1_STABILITY_CONFIDENCE_MARGIN: f64 = 10.0;
const TOP1_STABILITY_RECHECK_LIMIT: usize = 5;
const AUTO_NOVEL_MIN_CHARS: usize = 4;
const AUTO_NOVEL_MAX_CHARS: usize = 8;
const SHORT_HOTWORD_MAX_CHARS: usize = 4;
// Known multi-character words should outweigh small character-bigram quirks in
// sentence composition (for example 世界 over the less frequent 使节 in
// nihaoshijie). A stronger logarithmic scale keeps the effect smooth.
const SENTENCE_WORD_FREQ_SCALE: f64 = 3.0;
const SENTENCE_WORD_LEN_SCALE: f64 = 1.55;
const SENTENCE_SINGLE_CHAR_PENALTY: f64 = 0.55;
const MAX_PHRASE_SPAN_CHARS: usize = 8;

// Rerank weights are loaded from INI `[rank]` via `rerank_prefs`.
/// 前缀查询缓存（LRU）：prefix_flat / prefix_pinyin_flat 会扫描 BTreeMap range 并重新聚合排序，代价较高。
/// 连续输入（w -> wo -> woa / 回退）命中率高，用较大容量缓存可显著减轻开销。

const DEFAULT_PREFIX_CACHE_CAPACITY: usize = 384;
/// 单个前缀缓存最多保留多少条短语（防止极短前缀如 `w` 导致缓存爆大）。

const PREFIX_CACHE_MAX_ITEMS: usize = 48;
const RERANK_INPUT_PRUNE_TRIGGER: usize = TSF_MAX_CANDIDATES * 4;
const RERANK_INPUT_SOFT_CAP: usize = TSF_MAX_CANDIDATES * 3;
const LONG_INPUT_COMPACT_CHAR_LIMIT: usize = 20;
const LONG_INPUT_RERANK_SOFT_CAP: usize = TSF_PAGE_SIZE * 2;
const DEFAULT_FINAL_LOOKUP_CACHE_CAPACITY: usize = 128;
const DEFAULT_SHORT_LOOKUP_CACHE_CAPACITY: usize = 192;
const SHORT_LOOKUP_CACHE_MAX_INPUT_CHARS: usize = 4;
// Interactive lookups should produce a visible first page before doing the
// expensive correction/rerank tail.  Four letters is the point where mixed
// parsing and alternate phrase expansion start to become noticeably wide;
// shorter inputs stay on the fast complete path and avoid an extra retry.
const LONG_LOOKUP_SOFT_BUDGET_INPUT_LEN: usize = 3;
const DEFAULT_LONG_LOOKUP_SOFT_BUDGET_MS: u64 = 4;
const DEFAULT_LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES: usize = 6;
const MAX_PENDING_FULL_RETRY_KEYS: usize = 16;
const MIXED_PINYIN_EXPANSION_CACHE_CAPACITY: usize = 32;
const MIXED_COMPLETION_SCORE_CACHE_CAPACITY: usize = 1024;
const RERANK_SCORE_CACHE_CAPACITY: usize = 2048;
const MIXED_PINYIN_KEY_LIMIT: usize = 128;
const MIXED_PINYIN_MEDIUM_KEY_LIMIT: usize = 64;
const MIXED_PINYIN_LONG_KEY_LIMIT: usize = 32;
const MIXED_PINYIN_SHORT_KEY_LIMIT: usize = 12;
const MIXED_PINYIN_FOUR_CHAR_KEY_LIMIT: usize = 24;
const MIXED_PINYIN_INITIAL_SEGMENT_PENALTY: f64 = 80.0;
const MIXED_PINYIN_LONG_INITIAL_SEGMENT_PENALTY: f64 = 24.0;
const MIXED_PINYIN_PREFIX_SEGMENT_PENALTY: f64 = 10.0;
const MIXED_PINYIN_HIGH_CONFIDENCE_BONUS: f64 = 18.0;
const MIXED_PINYIN_LOW_CONFIDENCE_PENALTY: f64 = 45.0;
const MIXED_PINYIN_WORD_GRAPH_LIMIT: usize = 10;
const MIXED_PINYIN_COMPOSITION_GRAPH_LIMIT: usize = 2;
const MIXED_PINYIN_KNOWN_COMPOSITION_ADJUSTMENT_FLOOR: f64 = 32.0;
const MIXED_PINYIN_KNOWN_BOUNDARY_MATCH_BONUS: f64 = 48.0;
const MIXED_PINYIN_KNOWN_BOUNDARY_MISMATCH_PENALTY: f64 = 42.0;
const MIXED_PINYIN_PARTITION_SCORE_GAP_SCALE: f64 = 10.0;
const MIXED_PINYIN_PARTITION_SCORE_GAP_CAP: f64 = 72.0;
const MIXED_PINYIN_COMPOSITION_GRAPH_SCORE_GAP: f64 = 1.0;
const MIXED_PINYIN_STRONG_COMPOSITION_VISIBLE_GAP: f64 = 120.0;
const MIXED_PINYIN_WORD_GRAPH_PENALTY: f64 = 36.0;
const MIXED_PINYIN_DECODE_PENALTY: f64 = 46.0;
const MIXED_PREFIX_CORE_PHRASE_PROMOTE_MARGIN: f64 = 24.0;
const FULL_PINYIN_EXACT_LENGTH_BONUS: f64 = 185.0;
const FULL_PINYIN_EXT_GATE_PENALTY: f64 = 64.0;
const PRIMARY_LEXICON_LAYER_BONUS: f64 = 18.0;
const PRIMARY_SHORT_WORD_FREQ_BONUS: f64 = 34.0;
const PRIMARY_SINGLE_CHAR_FREQ_BONUS: f64 = 28.0;
const SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_BASE: f64 = 12.0;
const SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_SCALE: f64 = 14.0;
const SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_CAP: f64 = 108.0;
const EXT_LEXICON_LAYER_PENALTY: f64 = 18.0;
const EXT_SHORT_WORD_GATE_PENALTY: f64 = 24.0;
const LARGE_LEXICON_LAYER_PENALTY: f64 = 8.0;
const TOP1_CONFIDENCE_GAP: f64 = 24.0;

/// 超长输入兜底：当输入过长且常规候选不足时，按分段前缀/近似前缀继续给候选，
/// 避免 TSF 侧只剩下预编辑下划线却没有候选栏。

const ABBREV_FALLBACK_CHUNK: usize = 7;
const ABBREV_FALLBACK_MAX_CHUNKS: usize = 16;
const ABBREV_FALLBACK_MIN_CHUNK: usize = 1;
const ABBREV_FALLBACK_APPROX_MIN_CHUNK: usize = 2;
const ABBREV_FALLBACK_BASE_BONUS: f64 = 140.0;
const ABBREV_FALLBACK_CHUNK_PENALTY: f64 = 12.0;
const ABBREV_FALLBACK_SHORTER_CHUNK_PENALTY: f64 = 2.5;
const ABBREV_FALLBACK_PINYIN_PENALTY: f64 = 6.0;
const ABBREV_FALLBACK_APPROX_PENALTY: f64 = 18.0;
const SMALL_INPUT_EXPANSION_LIMIT: usize = 2;
const SMALL_INPUT_THREE_CHAR_EXPANSION_LIMIT: usize = 1;
const SHORT_INTENT_TOTAL_EXPANSION_LIMIT: usize = 2;
const SHORT_INTENT_THREE_CHAR_EXPANSION_LIMIT: usize = 1;
const SMALL_INPUT_SINGLE_CHAR_FRONT_SLOTS: usize = 2;
const THREE_CHAR_INTENT_EXACT_BOOST: f64 = 24.0;
const FOUR_CHAR_INTENT_EXACT_BOOST: f64 = 18.0;
const TYPO_CANDIDATE_META: &str = "纠错";
const TYPO_CANDIDATE_META_MARKED: &str = "纠错\ttypo=1\tcorrection=lookup";
const USER_CANDIDATE_META: &str = "用户词";
const MIXED_USER_CANDIDATE_META: &str = "mixed_pinyin_user";
const OBSERVED_USER_CANDIDATE_META: &str = "observed_user";
const ABBREV_CANDIDATE_META: &str = "简拼";
const MIXED_HIGH_CANDIDATE_META: &str = "混拼:高可信";
const MIXED_LOW_CANDIDATE_META: &str = "混拼:宽松";
const PINYIN_CANDIDATE_META: &str = "全拼";
const ASCII_DIRECT_META: &str = "no_learn=1\t直输";
const ASCII_COMPLETION_META: &str = "no_learn=1\t英文";
const SHORT_COMPACT_EXACT_BOOST: f64 = 56.0;
const SHORT_ABBREV_EXACT_LENGTH_RERANK_BONUS: f64 = 42.0;
const SHORT_ABBREV_TOP1_FREQ_BONUS_SCALE: f64 = 5.5;
const SHORT_ABBREV_TOP1_FREQ_BONUS_OFFSET: f64 = -42.0;
const SHORT_ABBREV_TOP1_FREQ_BONUS_CAP: f64 = 26.0;
const SHORT_ASCII_MULTI_CHAR_RERANK_DISCOUNT: f64 = 8.0;
const EXPLICIT_SEPARATOR_PHRASE_BONUS: f64 = 72.0;
const SHORT_COMPACT_BACKOFF_STEP_PENALTY: f64 = 26.0;
const HIGH_PRIORITY_TWO_CHAR_FREQ_MIN: u64 = 98_000;
const DAILY_SHORT_FREQ_MAX: u64 = 100_000;
const COMMON_SHORT_ABBREV_LAYER_FREQ_MIN: u64 = 40_000;
/// 简拼短词优先级平滑分数阈值（范围 [0, 1]）。
/// 基于词频相对排名，避免硬档边界附近 1% 词频差导致 bonus 跳变。
const HIGH_PRIORITY_SCORE_THRESHOLD: f64 = 1.0;
const SHORT_ABBREV_FREQ_BONUS_SCALE: f64 = 10.5;
const SHORT_ABBREV_FREQ_BONUS_OFFSET: f64 = -80.0;
const SHORT_ABBREV_FREQ_BONUS_CAP_TWO: f64 = 128.0;
const SHORT_ABBREV_FREQ_BONUS_CAP_THREE: f64 = 96.0;
const SHORT_ABBREV_FREQ_BONUS_CAP_FOUR: f64 = 72.0;
const USER_SHORT_ABBREV_FREQ_SCALE: f64 = 24.0;
const USER_SHORT_ABBREV_BONUS_CAP: f64 = 92.0;
const USER_PINNED_SHORT_ABBREV_BONUS: f64 = 180.0;
const USER_PINNED_SHORT_ABBREV_FREQ_MIN: u64 = 100_000;
const SHORT_FALLBACK_FREQ_RELIEF_CAP: f64 = 16.0;
const SHORT_ABBREV_SINGLE_CONTEXT_SCALE: f64 = 0.45;
/// 多音节整词头部上限（实际条数还受分数断崖与词频门槛约束，见 `apply_multi_syllable_phrase_head_then_first_syllable_singles`）。

const MULTI_SYLL_PHRASE_HEAD_MAX: usize = 3;
/// 分数断崖：相邻两条整词候选分差超过该值且已保留最少条数时，停止继续收整词。

const MULTI_SYLL_PHRASE_SCORE_GAP: f64 = 22.0;
/// 断崖截断前至少保留的整词条数（在有多条候选时生效）。

const MULTI_SYLL_PHRASE_HEAD_MIN: usize = 2;
const MULTI_SYLL_PHRASE_HEAD_FREQ_MIN: u64 = 80_000;
const FULL_PINYIN_PREFIX_SUPPORT_LIMIT: usize = 3;
const SHORT_INPUT_EXACT_FRONT_FREQ_MIN: u64 = 50_000;
const SHORT_INPUT_EXACT_FRONT_LIMIT_TWO: usize = 4;
const SHORT_INPUT_EXACT_FRONT_LIMIT_THREE: usize = 3;
const SHORT_INPUT_EXACT_FRONT_LIMIT_FOUR: usize = 3;
const SHORT_INPUT_LOW_FREQ_EXACT_FRONT_LIMIT: usize = 1;
const TWO_CHAR_INTENT_SINGLE_RESERVE_PAGES: usize = 3;
const SHORT_PHRASE_SINGLE_RERANK_EXTRA: f64 = 20.0;
const SHORT_PHRASE_TWO_CHAR_RERANK_BONUS: f64 = 22.0;
const SHORT_PHRASE_THREE_CHAR_RERANK_BONUS: f64 = 18.0;
const MULTI_CHAR_PHRASE_RERANK_BONUS: f64 = 8.0;
const MULTI_CHAR_PHRASE_RERANK_STEP: f64 = 1.0;
const MULTI_CHAR_PHRASE_RERANK_CAP: f64 = 11.0;
const ABBREV_CONTEXT_EXTRA_SCALE: f64 = 0.72;
const FOUR_CHAR_ABBREV_CONTEXT_EXTRA_SCALE: f64 = 1.05;
const ABBREV_STABILITY_EXTRA_BONUS: f64 = 7.0;
const ABBREV_OVERLONG_BASE_PENALTY: f64 = 18.0;
const ABBREV_OVERLONG_STEP_PENALTY: f64 = 7.0;

fn common_missing_letter_phrase_bonus(compact_key: &str, corrected_key: &str, phrase: &str) -> f64 {
    match (compact_key, corrected_key, phrase) {
        ("bejing", "beijing", "\u{5317}\u{4eac}") => 120.0,
        ("migtian", "mingtian", "\u{660e}\u{5929}") => 120.0,
        _ => 0.0,
    }
}

#[inline]
fn decode_candidate_limit(syllable_count: usize) -> usize {
    if syllable_count <= 1 {
        TSF_MAX_CANDIDATES
    } else {
        BASE_DECODE_LIMIT
    }
}

pub struct PinyinEngine {
    dict: Arc<HashMap<String, Vec<char>>>,
    syllables: Arc<HashSet<String>>,
    lm: Arc<Lm>,
    pub(crate) phrase_lexicon: Option<AbbrevLexicon>,
    user_lexicon: UserLexicon,
    user_lexicon_stamp: UserLexiconDiskStamp,
    user_lexicon_generation: u64,
    mode_flags: u32,
    pub char_count: usize,
    prefix_cache_abbrev: Mutex<PrefixLruCache>,
    prefix_cache_pinyin: Mutex<PrefixLruCache>,
    mixed_expansion_cache: Mutex<MixedExpansionLruCache>,
    mixed_incremental_cache: Mutex<MixedPinyinIncrementalCache>,
    mixed_completion_score_cache: Mutex<MixedCompletionScoreCache>,
    incremental_parse_cache: RefCell<IncrementalParseCache>,
    incremental_decode_cache: RefCell<IncrementalDecodeCache>,
    incremental_word_graph_cache: RefCell<IncrementalWordGraphCache>,
    rerank_score_cache: Mutex<RerankScoreCache>,
    final_lookup_cache: FinalLookupCache,
    short_lookup_cache: ShortLookupCache,
    pending_lookup_continuation: Option<PendingLookupContinuation>,
    // A partial first batch must eventually receive one budget-free pass.
    // Keep several keys because the shared engine serves interleaved clients;
    // a single continuation slot can be overwritten by another application.
    pending_full_retry_keys: VecDeque<(String, u32)>,
    cache_epoch: u64,
    /// Runtime INI generation used to build the composition-local caches.
    /// Correction/fuzzy/rerank preferences are not represented by mode flags,
    /// so a hot-reloaded config must explicitly invalidate these caches.
    runtime_config_generation: u64,
    // 热路径复用：避免每次 lookup_full 分配新容器。
    merged_buf: MergedCandidateMap,
    ranked_buf: Vec<RankedCandidate>,
    last_lookup: Option<LookupStabilityState>,
    /// Generation of shared lexicon/user data. Lookup sessions compare their
    /// local generation before being attached so a learning event cannot
    /// leave another client's final cache stale.
    shared_data_generation: u64,
    active_session_generation: u64,
}

/// Mutable state owned by one TSF composition client.
///
/// The compiled model, system lexicon and user dictionary stay on
/// `PinyinEngine`; everything whose value depends on the current reading or
/// mode travels with this object. The IPC service keeps one instance per pipe
/// connection, which prevents interleaved applications from replacing each
/// other's mode, continuation and incremental decoder state.
pub(crate) struct LookupSession {
    cancellation: LookupCancellationToken,
    mode_flags: u32,
    prefix_cache_abbrev: Mutex<PrefixLruCache>,
    prefix_cache_pinyin: Mutex<PrefixLruCache>,
    mixed_expansion_cache: Mutex<MixedExpansionLruCache>,
    mixed_incremental_cache: Mutex<MixedPinyinIncrementalCache>,
    mixed_completion_score_cache: Mutex<MixedCompletionScoreCache>,
    incremental_parse_cache: RefCell<IncrementalParseCache>,
    incremental_decode_cache: RefCell<IncrementalDecodeCache>,
    incremental_word_graph_cache: RefCell<IncrementalWordGraphCache>,
    rerank_score_cache: Mutex<RerankScoreCache>,
    final_lookup_cache: FinalLookupCache,
    short_lookup_cache: ShortLookupCache,
    pending_lookup_continuation: Option<PendingLookupContinuation>,
    pending_full_retry_keys: VecDeque<(String, u32)>,
    cache_epoch: u64,
    runtime_config_generation: u64,
    merged_buf: MergedCandidateMap,
    ranked_buf: Vec<RankedCandidate>,
    last_lookup: Option<LookupStabilityState>,
    shared_data_generation: u64,
}

impl Default for LookupSession {
    fn default() -> Self {
        Self {
            cancellation: LookupCancellationToken::default(),
            mode_flags: DEFAULT_MODE_FLAGS,
            prefix_cache_abbrev: Mutex::new(PrefixLruCache::default()),
            prefix_cache_pinyin: Mutex::new(PrefixLruCache::default()),
            mixed_expansion_cache: Mutex::new(MixedExpansionLruCache::default()),
            mixed_incremental_cache: Mutex::new(MixedPinyinIncrementalCache::default()),
            mixed_completion_score_cache: Mutex::new(MixedCompletionScoreCache::default()),
            incremental_parse_cache: RefCell::new(IncrementalParseCache::default()),
            incremental_decode_cache: RefCell::new(IncrementalDecodeCache::default()),
            incremental_word_graph_cache: RefCell::new(IncrementalWordGraphCache::default()),
            rerank_score_cache: Mutex::new(RerankScoreCache::default()),
            final_lookup_cache: FinalLookupCache::default(),
            short_lookup_cache: ShortLookupCache::default(),
            pending_lookup_continuation: None,
            pending_full_retry_keys: VecDeque::new(),
            cache_epoch: 0,
            runtime_config_generation: 0,
            merged_buf: MergedCandidateMap::with_capacity(256),
            ranked_buf: Vec::with_capacity(128),
            last_lookup: None,
            shared_data_generation: 0,
        }
    }
}

impl LookupSession {
    pub(crate) fn next_lookup_generation(&self) -> u64 {
        self.cancellation.next_generation()
    }

    pub(crate) fn cancellation_token(&self) -> LookupCancellationToken {
        self.cancellation.clone()
    }
}

#[derive(Clone, Debug, Default)]
struct MergedCandidate {
    phrase: String,
    score: f64,
    meta: CandidateMeta,
}

#[derive(Default)]
struct MergedCandidateMap {
    dynamic: HashMap<String, MergedCandidate>,
    system: HashMap<u32, MergedCandidate>,
    system_phrase_ids: HashMap<String, u32>,
}

impl MergedCandidateMap {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            dynamic: HashMap::with_capacity(capacity / 2),
            system: HashMap::with_capacity(capacity / 2),
            system_phrase_ids: HashMap::with_capacity(capacity / 2),
        }
    }

    fn len(&self) -> usize {
        self.dynamic.len() + self.system.len()
    }

    fn clear(&mut self) {
        self.dynamic.clear();
        self.system.clear();
        self.system_phrase_ids.clear();
    }

    fn phrases(&self) -> impl Iterator<Item = &String> {
        self.dynamic
            .keys()
            .chain(self.system.values().map(|candidate| &candidate.phrase))
    }

    fn candidates(&self) -> impl Iterator<Item = (&str, &MergedCandidate)> {
        self.dynamic
            .iter()
            .map(|(phrase, candidate)| (phrase.as_str(), candidate))
            .chain(
                self.system
                    .values()
                    .map(|candidate| (candidate.phrase.as_str(), candidate)),
            )
    }

    fn contains_phrase(&self, phrase: &str) -> bool {
        self.dynamic.contains_key(phrase) || self.system_phrase_ids.contains_key(phrase)
    }

    #[cfg(test)]
    fn contains_key(&self, phrase: &str) -> bool {
        self.contains_phrase(phrase)
    }

    fn drain_candidates(&mut self) -> Vec<(String, MergedCandidate)> {
        let mut out = Vec::with_capacity(self.len());
        out.extend(self.dynamic.drain());
        out.extend(
            self.system
                .drain()
                .map(|(_, candidate)| (candidate.phrase.clone(), candidate)),
        );
        self.system_phrase_ids.clear();
        out
    }

    fn retain_phrases(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.dynamic.retain(|phrase, _| keep(phrase));
        self.system.retain(|_, candidate| keep(&candidate.phrase));
        self.system_phrase_ids
            .retain(|_, phrase_id| self.system.contains_key(phrase_id));
    }
}

#[derive(Clone, Debug)]
struct PendingLookupContinuation {
    raw: String,
    mode_flags: u32,
    cache_epoch: u64,
    user_clock: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct RankedCandidate {
    pub phrase: String,
    pub score: f64,
    pub meta: CandidateMeta,
}

struct CandidatePostprocessContext<'a> {
    raw: &'a str,
    compact_key: &'a str,
    correction_prefs: CorrectionPrefs,
    suppress_correction: bool,
    short_phrase_intent: Option<usize>,
    final_short_phrase_intent: Option<usize>,
    input_intent: InputIntent,
    exact_guard_syllables: Option<usize>,
    overlong_pinned_exact_guard_syllables: Option<usize>,
    skip_final_short_density: bool,
    exact_single_syllable_input: bool,
    applied_multi_syllable_phrase_layout: bool,
    direct_input_shortcut: bool,
    user_hotword_front_limit: usize,
    preserved_direct_phrases: &'a [String],
    preserved_short_phrases: &'a [String],
    preserved_daily_short_two_char_phrases: &'a [String],
    preserved_daily_short_three_char_phrases: &'a [String],
    preserved_exact_lexicon_phrases: &'a [String],
    preserved_exact_user_phrases: &'a [String],
    preserved_mixed_user_phrases: &'a [String],
    preserved_pinned_user_phrases: &'a [String],
    preserved_high_priority_two_char_phrases: &'a [String],
    preserved_chat_priority_two_char_phrases: &'a [String],
    preserved_separator_phrases: &'a [String],
    final_preferred_phrases: &'a [String],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputIntent {
    #[default]
    Unknown,
    Direct,
    English,
    SingleSyllable,
    FullPinyin,
    ShortAbbrev,
    MixedPrefix,
}

impl InputIntent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Direct => "direct",
            Self::English => "english",
            Self::SingleSyllable => "single_syllable",
            Self::FullPinyin => "full_pinyin",
            Self::ShortAbbrev => "short_abbrev",
            Self::MixedPrefix => "mixed_prefix",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CandidateMeta {
    pub source: CandidateSource,
    pub source_layer: LexiconLayer,
    pub match_kind: CandidateMatchKind,
    pub pinned: bool,
    pub blocked: bool,
    pub partial: bool,
    pub correction_target: Option<String>,
    display: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateSource {
    #[default]
    System,
    Direct,
    Shortcut,
    User,
    Correction,
    Abbrev,
    Mixed,
    English,
    Pinyin,
    UMode,
    DateTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CandidateMatchKind {
    #[default]
    Unknown,
    Direct,
    Shortcut,
    User,
    Correction,
    ShortAbbrev,
    MixedPrefix,
    English,
    FullPinyin,
    SingleSyllable,
    DateTime,
}

impl CandidateMatchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Direct => "direct",
            Self::Shortcut => "shortcut",
            Self::User => "user",
            Self::Correction => "correction",
            Self::ShortAbbrev => "short_abbrev",
            Self::MixedPrefix => "mixed_prefix",
            Self::English => "english",
            Self::FullPinyin => "full_pinyin",
            Self::SingleSyllable => "single_syllable",
            Self::DateTime => "date_time",
        }
    }
}

impl CandidateSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Direct => "direct",
            Self::Shortcut => "shortcut",
            Self::User => "user",
            Self::Correction => "correction",
            Self::Abbrev => "abbrev",
            Self::Mixed => "mixed",
            Self::English => "english",
            Self::Pinyin => "pinyin",
            Self::UMode => "u_mode",
            Self::DateTime => "date_time",
        }
    }
}

fn match_kind_for_source(source: CandidateSource) -> CandidateMatchKind {
    match source {
        CandidateSource::Direct => CandidateMatchKind::Direct,
        CandidateSource::Shortcut | CandidateSource::UMode => CandidateMatchKind::Shortcut,
        CandidateSource::User => CandidateMatchKind::User,
        CandidateSource::Correction => CandidateMatchKind::Correction,
        CandidateSource::Abbrev => CandidateMatchKind::ShortAbbrev,
        CandidateSource::Mixed => CandidateMatchKind::MixedPrefix,
        CandidateSource::English => CandidateMatchKind::English,
        CandidateSource::Pinyin => CandidateMatchKind::FullPinyin,
        CandidateSource::DateTime => CandidateMatchKind::DateTime,
        CandidateSource::System => CandidateMatchKind::Unknown,
    }
}

impl CandidateMeta {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn legacy(text: impl Into<String>) -> Self {
        let text = text.into();
        let lower = text.to_ascii_lowercase();
        let source = if lower.contains("no_learn=1") {
            CandidateSource::Direct
        } else if lower.contains("custom") || text.contains("自定义") {
            CandidateSource::Shortcut
        } else if lower.contains("user") || lower.contains("observed") || text.contains("用户") {
            CandidateSource::User
        } else if lower.contains("typo") || text.contains("纠错") {
            CandidateSource::Correction
        } else if lower.contains("abbrev") || text.contains("简拼") {
            CandidateSource::Abbrev
        } else if lower.contains("mixed") || text.contains("混拼") {
            CandidateSource::Mixed
        } else if lower.contains("english") || text.contains("英文") {
            CandidateSource::English
        } else if text.contains("全拼") {
            CandidateSource::Pinyin
        } else if text.contains("U 拆字") {
            CandidateSource::UMode
        } else if text.contains("日期") || text.contains("时间") {
            CandidateSource::DateTime
        } else {
            CandidateSource::System
        };
        let tokens = text.split('\t').map(str::trim).collect::<Vec<_>>();
        Self {
            source,
            source_layer: LexiconLayer::Unknown,
            match_kind: match_kind_for_source(source),
            pinned: tokens
                .iter()
                .any(|token| *token == "pinned=1" || *token == "pinned"),
            blocked: lower.contains("blocked") || text.contains("屏蔽"),
            partial: tokens.contains(&"partial=1"),
            correction_target: correction_target_from_legacy_meta(&text),
            display: Some(text),
        }
    }

    pub fn user(pinned: bool) -> Self {
        Self::user_with_freq(pinned, 2)
    }

    pub fn user_with_freq(pinned: bool, freq: u64) -> Self {
        let source = if freq <= 1 {
            "source=user_fresh"
        } else {
            "source=user_common"
        };
        let display = if pinned {
            format!("{source}\tpinned=1")
        } else {
            source.to_string()
        };
        Self {
            source: CandidateSource::User,
            source_layer: LexiconLayer::Unknown,
            match_kind: CandidateMatchKind::User,
            pinned,
            display: Some(display),
            ..Self::default()
        }
    }

    pub fn with_partial(mut self) -> Self {
        self.partial = true;
        self
    }

    pub fn correction(kind: CandidateSource, label: String, target: Option<String>) -> Self {
        Self {
            source: kind,
            source_layer: LexiconLayer::Unknown,
            match_kind: CandidateMatchKind::Correction,
            correction_target: target,
            display: Some(label),
            ..Self::default()
        }
    }

    pub fn with_source_layer(mut self, source_layer: LexiconLayer) -> Self {
        self.source_layer = source_layer;
        self
    }

    pub fn with_match_kind(mut self, match_kind: CandidateMatchKind) -> Self {
        self.match_kind = match_kind;
        self
    }

    pub fn display_text(&self) -> Option<&str> {
        self.display.as_deref()
    }
}
fn candidate_meta_from_legacy(meta: Option<&str>) -> CandidateMeta {
    meta.filter(|text| !text.trim().is_empty())
        .map(CandidateMeta::legacy)
        .unwrap_or_default()
}

fn correction_target_from_legacy_meta(meta: &str) -> Option<String> {
    if !(meta.contains("纠错") || meta.to_ascii_lowercase().contains("typo")) {
        return None;
    }
    if let Some(target) = meta
        .split('\t')
        .find_map(|token| token.trim().strip_prefix("corrected_reading="))
    {
        let target = target.trim();
        if !target.is_empty() {
            return Some(target.to_string());
        }
    }
    let (_, rhs) = meta.split_once("->")?;
    let corrected = rhs
        .split(|ch: char| ch.is_whitespace() || ch == '\t' || ch == ',' || ch == ';')
        .next()
        .unwrap_or("")
        .trim();
    (!corrected.is_empty()).then(|| corrected.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate_prefs::set_effective_page_size_for_test;
    use crate::thuocl::{save_prebaked_lexicon, ThuoclEntry};
    use crate::user_dict::SuggestionConfidence;
    use crate::user_hotword_prefs::{set_user_hotword_level_for_test, UserHotwordBoostLevel};

    fn ranked_candidate(phrase: &str, score: f64) -> RankedCandidate {
        RankedCandidate {
            phrase: phrase.to_string(),
            score,
            meta: CandidateMeta::default(),
        }
    }

    #[test]
    fn compact_key_drops_spaces() {
        assert_eq!(compact_lookup_key("bei jing"), "beijing");
        assert_eq!(compact_lookup_key("ui"), "ui");
    }

    #[test]
    fn strip_leading_syllable_continues_safe_multi_syllable_input() {
        let engine = PinyinEngine::with_phrase_dir(None);

        assert_eq!(
            engine.strip_leading_syllable_from_reading("nihao"),
            Some("hao".to_string())
        );
        assert_eq!(
            engine.strip_leading_syllable_from_reading("bei jing"),
            Some("jing".to_string())
        );
    }

    #[test]
    fn strip_leading_syllable_rejects_mode_and_non_alpha_prefixes() {
        let engine = PinyinEngine::with_phrase_dir(None);

        for input in [
            "vv url test",
            "vabc",
            "u4e00",
            "uheng",
            "1hao",
            "/hao",
            "-nihao",
        ] {
            assert_eq!(
                engine.strip_leading_syllable_from_reading(input),
                None,
                "input should not auto-advance: {input}"
            );
        }
    }

    #[test]
    fn lookup_sessions_isolate_mode_reading_and_incremental_state() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let engine_mode = engine.mode_flags;
        let mut first = LookupSession::default();
        let mut second = LookupSession::default();

        let (first_candidates, first_error) = engine.lookup_with_session(&mut first, 0, "nihao", 0);
        let (second_candidates, second_error) = engine.lookup_with_session(
            &mut second,
            MODE_FUZZY_PINYIN | MODE_MIXED_PINYIN,
            "touchui",
            0,
        );

        assert!(first_error.is_none() && !first_candidates.is_empty());
        assert!(second_error.is_none() && !second_candidates.is_empty());
        assert_eq!(first.mode_flags, 0);
        assert_eq!(second.mode_flags, MODE_FUZZY_PINYIN | MODE_MIXED_PINYIN);
        assert_eq!(
            first.last_lookup.as_ref().map(|state| state.key.as_str()),
            Some("nihao")
        );
        assert_eq!(
            second.last_lookup.as_ref().map(|state| state.key.as_str()),
            Some("touchui")
        );
        assert_eq!(engine.mode_flags, engine_mode, "session swap leaked mode");
        assert!(engine.last_lookup.is_none(), "session swap leaked reading");
    }

    #[test]
    fn lookup_cancellation_is_scoped_to_one_session() {
        let first = LookupSession::default();
        let second = LookupSession::default();
        let first_generation = first.next_lookup_generation();
        let second_generation = second.next_lookup_generation();
        let _scope = LookupCancellationScope::enter(first.cancellation_token(), first_generation);

        second.cancellation_token().publish(second_generation + 10);
        assert!(
            !lookup_request_superseded(first_generation),
            "another session must not cancel this lookup"
        );

        first.cancellation_token().publish(first_generation + 1);
        assert!(lookup_request_superseded(first_generation));
    }

    #[test]
    fn parse_snapshots_restore_previous_prefix_after_backspace() {
        let engine = PinyinEngine::with_phrase_dir(None);
        let options = engine.parse_options();
        let before = engine.parse_variants_cached("nihao").unwrap();
        let _ = engine.parse_variants_cached("nihaos").unwrap();
        assert!(engine
            .incremental_parse_cache
            .borrow()
            .contains("nihao", options));
        let restored = engine.parse_variants_cached("nihao").unwrap();
        assert_eq!(restored, before);
    }

    #[test]
    fn incremental_word_graph_matches_cold_decode_and_restores_backspace() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let now = engine.user_lexicon.current_clock();
        let prefix = vec!["ni".to_string()];
        let intermediate = vec!["ni".to_string(), "hao".to_string()];
        let extended = vec![
            "ni".to_string(),
            "hao".to_string(),
            "shi".to_string(),
            "jie".to_string(),
        ];

        let prefix_result = engine.decode_word_graph(&prefix, now);
        let intermediate_result = engine.decode_word_graph(&intermediate, now);
        let incremental = engine.decode_word_graph(&extended, now);
        let restored_intermediate = engine.decode_word_graph(&intermediate, now);
        let restored = engine.decode_word_graph(&prefix, now);
        assert_eq!(restored_intermediate, intermediate_result);
        assert_eq!(restored, prefix_result);

        engine.incremental_word_graph_cache.borrow_mut().clear();
        let cold = engine.decode_word_graph(&extended, now);
        assert_eq!(incremental, cold);
    }

    #[test]
    fn complete_full_pinyin_first_page_uses_exact_route() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let (ranked, error) = engine.lookup_full_detailed("touchui");
        assert!(error.is_none());
        assert!(ranked.iter().any(|candidate| candidate.phrase == "头锤"));
        assert!(ranked.iter().take(TSF_PAGE_SIZE).all(|candidate| {
            !matches!(
                candidate.meta.match_kind,
                CandidateMatchKind::ShortAbbrev | CandidateMatchKind::MixedPrefix
            )
        }));
    }

    #[test]
    fn engine_tuning_ini_overrides_cache_and_budget_defaults() {
        let tuning = parse_engine_tuning_ini(
            "[engine]\n\
             prefix_cache_capacity=80\n\
             final_lookup_cache_capacity=40\n\
             short_lookup_cache_capacity=120\n\
             long_lookup_soft_budget_ms=8\n\
             long_lookup_min_first_batch_candidates=12\n",
        );

        assert_eq!(tuning.prefix_cache_capacity, 80);
        assert_eq!(tuning.final_lookup_cache_capacity, 40);
        assert_eq!(tuning.short_lookup_cache_capacity, 120);
        assert_eq!(tuning.long_lookup_soft_budget, Duration::from_millis(8));
        assert_eq!(tuning.long_lookup_min_first_batch_candidates, 12);
    }

    #[test]
    fn runtime_loader_preserves_prebaked_prefix_index() {
        let dir = std::env::temp_dir().join(format!(
            "pinyin_ime_runtime_prefix_loader_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut lexicon = AbbrevLexicon::default();
        for idx in 0..32 {
            lexicon.insert_parsed(ThuoclEntry {
                phrase: format!("测试词{idx}"),
                freq: 10_000 - idx,
                code: Some(format!("sharedprefix{idx:02}")),
            });
        }
        save_prebaked_lexicon(&dir.join("lexicon.bin"), &lexicon).unwrap();

        let loaded = lookup::load_phrase_lexicon_core_with_prefs(&dir, false).unwrap();
        assert!(loaded.has_prebaked_prefix_index(true, "sharedprefix"));
        assert_eq!(
            loaded.prefix_pinyin_flat("sharedprefix")[0].phrase,
            "测试词0"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signal_bonus_prefers_recent_entries() {
        let stale = LearnedStats {
            freq: 4,
            last_used: 1,
            ..LearnedStats::default()
        };
        let fresh = LearnedStats {
            freq: 4,
            last_used: 9,
            ..LearnedStats::default()
        };
        assert!(signal_bonus(fresh, 10, 3.0, 8.0) > signal_bonus(stale, 10, 3.0, 8.0));
    }

    #[test]
    fn signal_bonus_keeps_short_term_and_long_term_layers() {
        let now = 1_000;
        let stale_once = LearnedStats {
            freq: 1,
            last_used: 1,
            ..LearnedStats::default()
        };
        let fresh_once = LearnedStats {
            freq: 1,
            last_used: now,
            ..LearnedStats::default()
        };
        let stale_repeated = LearnedStats {
            freq: 80,
            last_used: 1,
            ..LearnedStats::default()
        };

        let fresh_bonus = signal_bonus(fresh_once, now, 8.0, 15.0);
        let stale_once_bonus = signal_bonus(stale_once, now, 8.0, 15.0);
        let stale_repeated_bonus = signal_bonus(stale_repeated, now, 8.0, 15.0);

        assert!(fresh_bonus > stale_once_bonus);
        assert!(stale_repeated_bonus > stale_once_bonus);
    }

    #[test]
    fn common_three_char_short_abbrev_sorts_before_ext_fixed_freq() {
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "\u{8f93}\u{5165}\u{6cd5}".into(),
                freq: COMMON_SHORT_ABBREV_LAYER_FREQ_MIN,
                code: None,
            },
            LexiconLayer::Base,
        ));
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "\u{4e09}\u{65e5}\u{4efd}".into(),
                freq: 120_000,
                code: None,
            },
            LexiconLayer::Ext,
        ));
        let mut phrases = vec![
            "\u{4e09}\u{65e5}\u{4efd}".to_string(),
            "\u{8f93}\u{5165}\u{6cd5}".to_string(),
        ];

        sort_preserved_phrases_by_lexicon_frequency(&mut phrases, Some(&lex));

        assert_eq!(phrases[0], "\u{8f93}\u{5165}\u{6cd5}");
    }

    #[test]
    fn phrase_path_rewards_known_multi_char_words() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: 200,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);

        let known = engine.best_phrase_path_score("\u{5317}\u{4eac}", 0);
        let unknown = engine.best_phrase_path_score("\u{5317}\u{7532}", 0);
        assert!(known > unknown);
    }

    #[test]
    fn phrase_path_score_is_cached() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: 200,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        let key = rerank_score_cache_key(
            RerankScoreKind::PhrasePath,
            "\u{5317}\u{4eac}",
            0,
            engine.cache_epoch,
            Vec::new(),
        );

        assert!(engine
            .rerank_score_cache
            .lock()
            .unwrap()
            .get(&key)
            .is_none());
        let score = engine.best_phrase_path_score("\u{5317}\u{4eac}", 0);
        assert_eq!(
            engine.rerank_score_cache.lock().unwrap().get(&key),
            Some(score)
        );
    }

    #[test]
    fn single_char_channel_prefers_common_chars() {
        let vocab = HashSet::from(['中', '龥']);
        let mut u_count = HashMap::new();
        u_count.insert('中', 100usize);
        u_count.insert('龥', 1usize);

        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.user_lexicon = UserLexicon::default();
        engine.lm = Arc::new(Lm::from_counts(vocab, u_count, HashMap::new()));

        let common = engine.single_char_channel_boost("\u{4e2d}", 3.35);
        let rare = engine.single_char_channel_boost("\u{9fa5}", 3.35);
        assert!(common > rare);
    }

    #[test]
    fn exact_single_syllable_promotes_common_single_char_by_lexicon_frequency() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.dict = Arc::new(HashMap::from([(
            "yi".to_string(),
            vec!['\u{4ea6}', '\u{4e00}'],
        )]));
        let vocab = HashSet::from(['\u{4ea6}', '\u{4e00}']);
        let mut u_count = HashMap::new();
        u_count.insert('\u{4ea6}', 10usize);
        u_count.insert('\u{4e00}', 1usize);
        engine.lm = Arc::new(Lm::from_counts(vocab, u_count, HashMap::new()));

        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "\u{4e00}".into(),
                freq: 35_000_000,
                code: Some("zz".into()),
            },
            LexiconLayer::Core,
        ));
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "\u{4ea6}".into(),
                freq: 80_000,
                code: Some("zz".into()),
            },
            LexiconLayer::Core,
        ));
        engine.install_phrase_lexicon(Some(lex));

        let common_prior =
            engine.single_syllable_common_char_prior("\u{4e00}", InputIntent::SingleSyllable);
        let uncommon_prior =
            engine.single_syllable_common_char_prior("\u{4ea6}", InputIntent::SingleSyllable);
        assert!(common_prior > uncommon_prior + 60.0);
        assert_eq!(
            engine.single_syllable_common_char_prior("\u{4e00}", InputIntent::FullPinyin),
            0.0
        );

        let (ranked, err) = engine.lookup_full("yi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4e00}")
        );
    }

    #[test]
    fn direct_datetime_shortcuts_prefer_live_date_and_time() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (rq_ranked, rq_err) = engine.lookup_full("rq");
        assert!(rq_err.is_none());
        assert!(!rq_ranked.is_empty());
        assert_eq!(rq_ranked[0].0.len(), 10);
        assert_eq!(&rq_ranked[0].0[4..5], "-");
        assert_eq!(&rq_ranked[0].0[7..8], "-");

        let (sj_ranked, sj_err) = engine.lookup_full("sj");
        assert!(sj_err.is_none());
        assert!(!sj_ranked.is_empty());
        assert_eq!(sj_ranked[0].0.len(), 8);
        assert_eq!(&sj_ranked[0].0[2..3], ":");
        assert_eq!(&sj_ranked[0].0[5..6], ":");
    }

    #[test]
    fn direct_datetime_shortcuts_expose_live_date_and_beijing_time_meta() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (rq_ranked, rq_err) = engine.lookup_full_detailed("rq");
        assert!(rq_err.is_none());
        assert!(!rq_ranked.is_empty());
        assert_eq!(rq_ranked[0].meta.display_text(), Some("实时日期"));

        let (sj_ranked, sj_err) = engine.lookup_full_detailed("sj");
        assert!(sj_err.is_none());
        assert!(!sj_ranked.is_empty());
        assert_eq!(sj_ranked[0].meta.display_text(), Some("北京时间"));
        assert_eq!(sj_ranked[0].phrase.len(), 8);
        assert_eq!(&sj_ranked[0].phrase[2..3], ":");
        assert_eq!(&sj_ranked[0].phrase[5..6], ":");
    }

    #[test]
    fn pinyin_datetime_alias_does_not_hide_zhou_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.set_mode_flags(MODE_DATE_AUTO_FORMAT | MODE_JIANPIN | MODE_MIXED_PINYIN);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("zhou");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert_ne!(ranked[0].meta.match_kind, CandidateMatchKind::DateTime);
        assert!(
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .any(|item| matches!(item.phrase.as_str(), "\u{5468}" | "\u{5dde}" | "\u{6d32}")),
            "ranked: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ascii_url_passthrough_is_promoted_and_not_learned() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let input = "https://example.com/path?q=1";
        let (ranked, err) = engine.lookup_full_detailed(input);
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(ranked.first().map(|item| item.phrase.as_str()), Some(input));
        assert!(ranked
            .first()
            .and_then(|item| item.meta.display_text())
            .is_some_and(|meta| meta.contains("no_learn=1")));
    }

    #[test]
    fn code_like_ascii_token_passthrough_is_promoted() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("my_varName");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("my_varName")
        );
    }

    #[test]
    fn single_leading_uppercase_pinyin_is_not_code_case() {
        assert!(!should_preserve_ascii_input("Xguang"));
        assert!(should_preserve_ascii_input("fooBar"));
        assert!(should_preserve_ascii_input("UserID"));
    }

    #[test]
    fn mixed_ascii_cjk_phrase_beats_ascii_passthrough() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "X\u{5149}".into(),
            freq: 10_000,
            code: Some("Xguang".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full_detailed("Xguang");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("X\u{5149}")
        );
    }

    #[test]
    fn invalid_english_prefix_gets_local_completions() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.set_mode_flags(MODE_ENGLISH_WORD_INPUT);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("compu");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            ranked.iter().any(|item| item.phrase == "compu"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            ranked.iter().any(|item| item.phrase == "computer"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            ranked
                .iter()
                .all(|item| item.meta.source != CandidateSource::Mixed),
            "English lexicon intent should bypass mixed expansion: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| (&item.phrase, item.meta.source))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn english_word_input_disabled_by_default() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("compu");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            !ranked
                .iter()
                .any(|item| item.phrase == "compu" || item.phrase == "computer"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn english_lexicon_layer_obeys_english_word_input_toggle() {
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "rust".into(),
                freq: 1_000_000,
                code: Some("zzqx".into()),
            },
            LexiconLayer::En,
        ));

        let mut disabled = PinyinEngine::with_phrase_dir(None);
        disabled.clear_user_lexicon_for_test();
        disabled.install_phrase_lexicon(Some(lex.clone()));

        let (ranked, err) = disabled.lookup_full_detailed("zzqx");
        assert!(err.is_none() || err.as_deref() == Some("no candidates"));
        assert!(
            !ranked.iter().any(|item| item.phrase == "rust"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );

        let mut enabled = PinyinEngine::with_phrase_dir(None);
        enabled.set_mode_flags(MODE_ENGLISH_WORD_INPUT);
        enabled.clear_user_lexicon_for_test();
        enabled.install_phrase_lexicon(Some(lex));

        let (ranked, err) = enabled.lookup_full_detailed("zzqx");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            ranked.iter().any(|item| {
                item.phrase == "rust" && item.meta.source_layer == LexiconLayer::En
            }),
            "ranked: {:?}",
            ranked
                .iter()
                .take(12)
                .map(|item| (item.phrase.as_str(), item.meta.source_layer))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn non_english_intent_keeps_at_most_two_english_candidates_at_tail() {
        let mut ranked = vec![
            ranked_candidate("中文一", 100.0),
            RankedCandidate {
                phrase: "computer".into(),
                score: 99.0,
                meta: CandidateMeta::legacy("english").with_source_layer(LexiconLayer::En),
            },
            RankedCandidate {
                phrase: "compare".into(),
                score: 98.0,
                meta: CandidateMeta::legacy("english").with_source_layer(LexiconLayer::En),
            },
            ranked_candidate("中文二", 97.0),
            RankedCandidate {
                phrase: "company".into(),
                score: 96.0,
                meta: CandidateMeta::legacy("english").with_source_layer(LexiconLayer::En),
            },
        ];
        let mut english_intent_ranked = ranked.clone();

        limit_english_candidates_for_non_english_intent(&mut ranked, InputIntent::FullPinyin);

        assert_eq!(
            ranked
                .iter()
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>(),
            vec!["中文一", "中文二", "computer", "compare"]
        );

        limit_english_candidates_for_non_english_intent(
            &mut english_intent_ranked,
            InputIntent::English,
        );
        assert_eq!(english_intent_ranked.len(), 5);
    }

    #[test]
    fn direct_datetime_shortcuts_ignore_stale_learned_values() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.user_lexicon.learn("rq", "9999-99-99").unwrap();
        engine.user_lexicon.learn("sj", "99:99:99").unwrap();

        let (rq_ranked, rq_err) = engine.lookup_full("rq");
        assert!(rq_err.is_none());
        assert!(!rq_ranked.iter().any(|(phrase, _)| phrase == "9999-99-99"));

        let (sj_ranked, sj_err) = engine.lookup_full("sj");
        assert!(sj_err.is_none());
        assert!(!sj_ranked.iter().any(|(phrase, _)| phrase == "99:99:99"));
    }

    #[test]
    fn learn_commit_skips_direct_datetime_shortcuts() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine.learn_commit("rq", "9999-99-99").unwrap();
        engine.learn_commit("sj", "99:99:99").unwrap();

        assert!(engine.user_lexicon.lookup_input("rq").is_none());
        assert!(engine.user_lexicon.lookup_input("sj").is_none());
    }

    #[test]
    fn traditional_mode_skips_user_learning_and_pinning() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(MODE_TRADITIONAL_OUTPUT);

        engine.learn_commit("ce shi ci", "测试词甲").unwrap();
        engine
            .learn_commit_with_flags(
                "kai xin shu ru fa",
                "开心输入法",
                LEARN_FLAG_COMPOSED_PHRASE,
            )
            .unwrap();
        engine.set_candidate_pin("bj", "保健", true).unwrap();

        assert!(engine.user_lexicon.lookup_input("ceshici").is_none());
        assert!(engine.user_lexicon.lookup_input("kaixinshurufa").is_none());
        assert!(engine
            .user_lexicon
            .lookup_observed_input("kaixinshurufa")
            .is_none());
        assert!(engine.user_lexicon.lookup_input("bj").is_none());
    }

    #[test]
    fn traditional_output_converts_safe_direct_text_but_keeps_literals() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(MODE_TRADITIONAL_OUTPUT | MODE_V_ASSIST | MODE_DATE_AUTO_FORMAT);

        let (sj_ranked, sj_err) = engine.lookup_full_detailed("sj");
        assert!(sj_err.is_none(), "unexpected error: {sj_err:?}");
        assert!(
            sj_ranked
                .iter()
                .any(|item| item.phrase.contains("北京時間")),
            "sj candidates: {:?}",
            sj_ranked
                .iter()
                .take(8)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );

        let (email_ranked, email_err) = engine.lookup_full_detailed("vv mail 测试");
        assert!(email_err.is_none(), "unexpected error: {email_err:?}");
        assert_eq!(
            email_ranked.first().map(|item| item.phrase.as_str()),
            Some("测试@qq.com")
        );

        let (url_ranked, url_err) = engine.lookup_full_detailed("vv url 测试");
        assert!(url_err.is_none(), "unexpected error: {url_err:?}");
        assert_eq!(
            url_ranked.first().map(|item| item.phrase.as_str()),
            Some("https://测试.com")
        );

        let (money_ranked, money_err) = engine.lookup_full_detailed("vv dx 123.45");
        assert!(money_err.is_none(), "unexpected error: {money_err:?}");
        assert_eq!(
            money_ranked.first().map(|item| item.phrase.as_str()),
            Some("壹佰貳拾叄元肆角伍分")
        );
    }

    #[test]
    fn clipboard_disabled_mode_suppresses_v_mode_clipboard_lookup_only() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(MODE_V_ASSIST | MODE_CLIPBOARD_DISABLED | MODE_DATE_AUTO_FORMAT);

        let (clipboard_ranked, clipboard_err) = engine.lookup_full_detailed("vv cb");
        assert!(
            clipboard_err.is_none(),
            "unexpected error: {clipboard_err:?}"
        );
        assert!(clipboard_ranked.is_empty());

        let (date_ranked, date_err) = engine.lookup_full_detailed("vv rq");
        assert!(date_err.is_none(), "unexpected error: {date_err:?}");
        assert!(!date_ranked.is_empty());
    }

    #[test]
    fn v_mode_symbol_and_emoji_flags_gate_commands() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(MODE_V_ASSIST);

        let (symbol_off, symbol_off_err) = engine.lookup_full_detailed("vv sym");
        assert!(
            symbol_off_err.is_none(),
            "unexpected error: {symbol_off_err:?}"
        );
        assert!(symbol_off.is_empty());

        engine.set_mode_flags(MODE_V_ASSIST | MODE_SYMBOL_TOOLBOX);
        let (symbol_on, symbol_on_err) = engine.lookup_full_detailed("vv sym");
        assert!(
            symbol_on_err.is_none(),
            "unexpected error: {symbol_on_err:?}"
        );
        assert!(!symbol_on.is_empty());

        let (emoji_off, emoji_off_err) = engine.lookup_full_detailed("vv emoji");
        assert!(
            emoji_off_err.is_none(),
            "unexpected error: {emoji_off_err:?}"
        );
        assert!(emoji_off.is_empty());

        engine.set_mode_flags(MODE_V_ASSIST | MODE_EMOJI_INPUT);
        let (emoji_on, emoji_on_err) = engine.lookup_full_detailed("vv emoji");
        assert!(emoji_on_err.is_none(), "unexpected error: {emoji_on_err:?}");
        assert!(!emoji_on.is_empty());
    }

    #[test]
    fn learned_user_phrase_is_promoted_to_first_page_on_next_lookup() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: 1_000_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);

        let learned = "\u{5317}\u{7532}";
        engine.learn_commit("bei jing", learned).unwrap();

        let first_page = engine.lookup_first_page("beijing");
        assert_eq!(first_page.first().map(String::as_str), Some(learned));
    }

    #[test]
    fn composed_phrase_is_exact_and_confirmed_on_first_standard_learning() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";

        engine
            .learn_commit_with_flags("kai xin shu ru fa", phrase, LEARN_FLAG_COMPOSED_PHRASE)
            .expect("first composed learn");
        let exact = engine
            .user_lexicon
            .lookup_input("kaixinshurufa")
            .expect("first composed commit should be exact");
        assert_eq!(exact[0].confidence, SuggestionConfidence::Confirmed);
        assert!(engine
            .user_lexicon
            .lookup_observed_input("kaixinshurufa")
            .is_none());
        assert!(engine.user_lexicon.lookup_input("kxsrf").is_some());
        assert!(engine
            .user_lexicon
            .lookup_mixed_input("kaixinsrf")
            .is_some());
    }

    #[test]
    fn standard_composed_two_and_three_char_phrases_skip_observation_wait() {
        for (reading, key, phrase) in [
            ("jia yi", "jiayi", "\u{7532}\u{4e59}"),
            ("jia yi bing", "jiayibing", "\u{7532}\u{4e59}\u{4e19}"),
        ] {
            let mut engine = PinyinEngine::with_phrase_dir(None);
            engine.clear_user_lexicon_for_test();
            engine
                .learn_commit_with_flags(reading, phrase, LEARN_FLAG_COMPOSED_PHRASE)
                .expect("first explicit composition should learn");

            let entry = engine
                .user_lexicon
                .lookup_input(key)
                .and_then(|entries| entries.iter().find(|entry| entry.phrase == phrase))
                .expect("composed phrase should immediately enter exact user dictionary");
            assert_eq!(entry.confidence, SuggestionConfidence::Confirmed);
            assert!(engine.user_lexicon.lookup_observed_input(key).is_none());
        }
    }

    #[test]
    fn learning_sensitivity_controls_composed_phrase_promotion() {
        let phrase = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";

        let mut aggressive = PinyinEngine::with_phrase_dir(None);
        aggressive.clear_user_lexicon_for_test();
        aggressive.set_mode_flags(MODE_LEARNING_AGGRESSIVE);
        aggressive
            .learn_commit_with_flags("kai xin shu ru fa", phrase, LEARN_FLAG_COMPOSED_PHRASE)
            .expect("aggressive composed learn");
        assert!(aggressive
            .user_lexicon
            .lookup_input("kaixinshurufa")
            .is_some());

        let mut conservative = PinyinEngine::with_phrase_dir(None);
        conservative.clear_user_lexicon_for_test();
        conservative.set_mode_flags(MODE_LEARNING_CONSERVATIVE);
        for _ in 0..2 {
            conservative
                .learn_commit_with_flags("kai xin shu ru fa", phrase, LEARN_FLAG_COMPOSED_PHRASE)
                .expect("conservative composed observe");
        }
        assert!(conservative
            .user_lexicon
            .lookup_input("kaixinshurufa")
            .is_none());
        conservative
            .learn_commit_with_flags("kai xin shu ru fa", phrase, LEARN_FLAG_COMPOSED_PHRASE)
            .expect("conservative composed promote");
        let promoted = conservative
            .user_lexicon
            .lookup_input("kaixinshurufa")
            .expect("conservative composed promotion");
        assert_eq!(
            promoted[0].freq, 3,
            "observation frequency must be inherited"
        );
        assert!(promoted[0].confidence >= SuggestionConfidence::Confirmed);
        assert!(conservative
            .user_lexicon
            .lookup_observed_input("kaixinshurufa")
            .is_none());
    }

    #[test]
    fn direct_learning_adds_abbrev_and_mixed_keys() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";

        engine
            .learn_commit("kai xin shu ru fa", phrase)
            .expect("learn phrase");

        assert!(engine.user_lexicon.lookup_input("kaixinshurufa").is_some());
        assert!(engine.user_lexicon.lookup_input("kxsrf").is_some());
        assert!(engine
            .user_lexicon
            .lookup_mixed_input("kxshurufa")
            .is_some());
    }

    #[test]
    fn adjacent_commits_observe_then_promote_combined_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let first = "\u{591a}\u{6a21}\u{6001}";
        let second = "\u{5bf9}\u{9f50}";
        let combined = "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}";

        engine.learn_commit("duomotai", first).unwrap();
        engine.learn_commit("duiqi", second).unwrap();

        assert!(engine
            .user_lexicon
            .lookup_observed_input("duomotaiduiqi")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == combined)));
        assert!(engine.user_lexicon.lookup_input("duomotaiduiqi").is_none());

        engine.learn_commit("duomotai", first).unwrap();
        engine.learn_commit("duiqi", second).unwrap();

        assert!(engine
            .user_lexicon
            .lookup_input("duomotaiduiqi")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == combined)));
    }

    #[test]
    fn repeated_negative_feedback_demotes_exact_user_phrase_to_observed() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let old = "\u{6d4b}\u{8bd5}";
        let selected = "\u{6d4b}\u{9a8c}";

        engine.learn_commit("ceshi", old).unwrap();
        for _ in 0..2 {
            engine
                .learn_selection_feedback("ceshi", selected, 1, 0, &[old.to_string()])
                .unwrap();
        }

        assert!(!engine
            .user_lexicon
            .lookup_input("ceshi")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == old)));
        assert!(engine
            .user_lexicon
            .lookup_observed_input("ceshi")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == old)));
    }

    #[test]
    fn no_learn_commit_clears_runtime_context() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine.learn_commit("shanghai", "\u{4e0a}\u{6d77}").unwrap();
        assert_eq!(
            engine.user_lexicon.last_committed(),
            Some("\u{4e0a}\u{6d77}")
        );

        engine
            .learn_commit_with_flags("secret", "\u{79d8}\u{5bc6}", LEARN_FLAG_NO_LEARN)
            .unwrap();
        assert!(engine
            .user_lexicon
            .preceding_commits_for_context()
            .is_empty());
    }

    #[test]
    fn learn_commit_skips_sensitive_text_patterns() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine
            .learn_commit("xm", "\u{90ae}\u{7bb1}name@example.com")
            .unwrap();
        engine
            .learn_commit("dh", "\u{7535}\u{8bdd}13800138000")
            .unwrap();

        assert!(engine.user_lexicon.lookup_input("xm").is_none());
        assert!(engine.user_lexicon.lookup_input("dh").is_none());
    }

    #[test]
    fn pinned_candidate_promotes_exact_input_until_unpinned() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine
            .set_candidate_pin("bj", "\u{4fdd}\u{5065}", true)
            .unwrap();
        let (ranked, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4fdd}\u{5065}")
        );

        engine
            .set_candidate_pin("bj", "\u{4fdd}\u{5065}", false)
            .unwrap();
        let (ranked, err) = engine.lookup_full_detailed("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let unpinned = ranked
            .iter()
            .find(|item| item.phrase == "\u{4fdd}\u{5065}")
            .expect("unpinned candidate should keep learned entry");
        assert!(
            !unpinned.meta.pinned,
            "unpin should clear only the pin flag: {:?}",
            unpinned.meta
        );
    }

    #[test]
    fn pinned_candidate_meta_marks_exact_input_pin() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine
            .set_candidate_pin("bj", "\u{4fdd}\u{5065}", true)
            .unwrap();

        let (ranked, err) = engine.lookup_full_detailed("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let pinned = ranked
            .iter()
            .find(|item| item.phrase == "\u{4fdd}\u{5065}")
            .expect("pinned candidate");
        assert!(
            pinned
                .meta
                .display_text()
                .is_some_and(|meta| meta.split('\t').any(|token| token == "pinned=1")),
            "pinned candidate meta: {:?}",
            pinned.meta
        );
    }

    #[test]
    fn long_chunk_fallback_backtracks_to_shorter_prefixes() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 200,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "上海".into(),
            freq: 180,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "广州".into(),
            freq: 160,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);

        let mut merged = MergedCandidateMap::default();
        engine.merge_abbrev_chunk_fallback(
            "bjzzzzzzshyyyyyygzxxxxxxxxxxxxxxxxxxxx",
            &mut merged,
            0,
        );

        assert!(merged.contains_key("北京"));
        assert!(merged.contains_key("上海"));
        assert!(merged.contains_key("广州"));
    }

    #[test]
    fn system_candidates_merge_by_phrase_id_and_absorb_dynamic_entries() {
        let engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "北京".into(),
                freq: 200,
                code: None,
            },
            LexiconLayer::Core,
        ));
        let phrase_id = lex.phrase_id("北京").expect("system phrase id");
        let entries = lex.lookup("bj").expect("abbrev entries");
        let mut merged = MergedCandidateMap::default();

        merge_candidate(&mut merged, "北京".into(), 500.0, Some("dynamic"));
        merge_phrase_entries(
            &mut merged,
            entries.iter(),
            100.0,
            8,
            &engine.user_lexicon,
            0,
            Some(ABBREV_CANDIDATE_META),
            Some(&lex),
            Some(2),
        );
        merge_phrase_entries(
            &mut merged,
            entries.iter(),
            200.0,
            8,
            &engine.user_lexicon,
            0,
            Some(ABBREV_CANDIDATE_META),
            Some(&lex),
            Some(2),
        );

        assert_eq!(merged.len(), 1);
        assert!(merged.dynamic.is_empty());
        assert_eq!(merged.system_phrase_ids.get("北京"), Some(&phrase_id));
        assert_eq!(
            merged.system.get(&phrase_id).map(|item| item.score),
            Some(500.0)
        );

        merge_candidate(&mut merged, "北京".into(), 650.0, Some("dynamic-later"));
        assert_eq!(merged.len(), 1);
        assert!(merged.dynamic.is_empty());
        assert_eq!(
            merged.system.get(&phrase_id).map(|item| item.score),
            Some(650.0)
        );
    }

    #[test]
    fn long_invalid_input_still_returns_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 200,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "上海".into(),
            freq: 180,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "广州".into(),
            freq: 160,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);

        let input = "bjzzzzzzshyyyyyygzxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
        let (ranked, err) = engine.lookup_full(input);
        assert!(err.is_none() || err.as_deref() == Some("no candidates"));
        assert!(!ranked.is_empty());
        assert!(ranked.iter().any(|(phrase, _)| phrase == "北京"));
    }

    #[test]
    fn small_input_policy_places_expansions_at_page_tail() {
        let mut ranked = vec![
            ranked_candidate("北京", 20.0),
            ranked_candidate("\u{5317}\u{4eac}\u{5e02}", 19.0),
            ranked_candidate("上海", 18.0),
            ranked_candidate("\u{4e0a}", 17.0),
            ranked_candidate("\u{4e0a}\u{6d77}\u{5e02}", 16.0),
            ranked_candidate("\u{5e7f}", 15.0),
            ranked_candidate("\u{5e7f}\u{5dde}\u{5e02}", 14.0),
            ranked_candidate("北京城区", 13.0),
        ];

        arrange_small_input_candidates(&mut ranked, 2, 5);

        let first_page: Vec<&str> = ranked
            .iter()
            .take(5)
            .map(|item| item.phrase.as_str())
            .collect();
        assert_eq!(
            first_page,
            vec![
                "北京",
                "上海",
                "\u{4e0a}",
                "\u{5317}\u{4eac}\u{5e02}",
                "\u{4e0a}\u{6d77}\u{5e02}"
            ]
        );
        assert_eq!(
            ranked
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) == 3)
                .count(),
            2
        );
        assert!(ranked
            .iter()
            .all(|item| phrase_char_count(&item.phrase) <= 3));
    }

    #[test]
    fn small_input_policy_caps_four_char_candidates_for_three_char_intent() {
        let mut ranked = vec![
            ranked_candidate("\u{8f93}\u{5165}\u{6cd5}", 20.0),
            ranked_candidate("\u{8f93}", 19.0),
            ranked_candidate("输入法学", 18.0),
            ranked_candidate("输入", 17.0),
            ranked_candidate("输入法界", 16.0),
            ranked_candidate("输入法器", 15.0),
            ranked_candidate("\u{8f93}\u{5165}\u{6cd5}\u{5de5}\u{5177}", 14.0),
        ];

        arrange_small_input_candidates(&mut ranked, 3, 6);

        assert_eq!(
            ranked
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) == 4)
                .count(),
            SMALL_INPUT_THREE_CHAR_EXPANSION_LIMIT
        );
        assert!(ranked
            .iter()
            .all(|item| phrase_char_count(&item.phrase) <= 4));
        assert_eq!(ranked[0].phrase, "\u{8f93}\u{5165}\u{6cd5}");
        assert_eq!(ranked[1].phrase, "输入");
        assert_eq!(ranked[2].phrase, "\u{8f93}");
    }

    #[test]
    fn small_input_policy_reserves_single_chars_on_first_page_for_three_char_intent() {
        let mut ranked = vec![
            ranked_candidate("\u{8f93}\u{5165}\u{6cd5}", 30.0),
            ranked_candidate("\u{6536}\u{94f6}\u{53f0}", 29.0),
            ranked_candidate("\u{8bd5}\u{9a8c}\u{7530}", 28.0),
            ranked_candidate("\u{77f3}\u{6cb9}\u{57ce}", 27.0),
            ranked_candidate("\u{6444}\u{5f71}\u{5e08}", 26.0),
            ranked_candidate("\u{8f93}", 25.0),
            ranked_candidate("输入法学", 24.0),
            ranked_candidate("输入法界", 23.0),
        ];

        arrange_small_input_candidates(&mut ranked, 3, 6);

        assert!(ranked
            .iter()
            .take(6)
            .any(|item| phrase_char_count(&item.phrase) == 1));
    }

    #[test]
    fn short_input_density_moves_extra_low_freq_three_char_exact_after_short_candidates() {
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "输入法".into(),
            freq: 90_000,
            code: None,
        }));
        let mut ranked = vec![
            ranked_candidate("输入法", 30.0),
            ranked_candidate("输入分", 29.0),
            ranked_candidate("输入方", 28.0),
            ranked_candidate("输入发", 27.0),
            ranked_candidate("输", 26.0),
            ranked_candidate("输入", 25.0),
            ranked_candidate("书", 24.0),
        ];

        limit_short_input_first_page_density(&mut ranked, Some(3), Some(&lex));

        let pos_shu = ranked.iter().position(|item| item.phrase == "输").unwrap();
        let pos_shuru = ranked
            .iter()
            .position(|item| item.phrase == "输入")
            .unwrap();
        let pos_extra = ranked
            .iter()
            .position(|item| item.phrase == "输入方")
            .unwrap();
        assert!(pos_shu < pos_extra, "ranked: {ranked:?}");
        assert!(pos_shuru < pos_extra, "ranked: {ranked:?}");
    }

    #[test]
    fn final_short_intent_density_limits_total_three_char_expansions() {
        let mut ranked = vec![
            ranked_candidate("\u{7c73}\u{56fd}\u{5929}", 100.0),
            ranked_candidate("\u{660e}\u{5929}", 99.0),
            ranked_candidate("\u{5317}", 98.0),
            ranked_candidate("\u{5317}\u{4eac}\u{5e02}", 97.0),
            ranked_candidate("\u{4e0a}\u{6d77}\u{5e02}", 96.0),
            ranked_candidate("\u{5e7f}\u{5dde}\u{5e02}", 95.0),
            ranked_candidate("\u{6df1}\u{5733}\u{5e02}", 94.0),
            ranked_candidate("\u{5357}\u{4eac}\u{5e02}", 93.0),
            ranked_candidate("\u{4eca}", 92.0),
        ];

        apply_short_intent_final_candidate_density(&mut ranked, Some(2), None);

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{660e}\u{5929}"),
            "ranked: {ranked:?}"
        );
        assert_eq!(
            ranked
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) == 3)
                .count(),
            SHORT_INTENT_TOTAL_EXPANSION_LIMIT
        );
        let first_single = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 1)
            .unwrap();
        let first_three = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 3)
            .unwrap();
        assert!(first_single < first_three, "ranked: {ranked:?}");
        assert!(ranked
            .iter()
            .all(|item| phrase_char_count(&item.phrase) <= 3));
    }

    #[test]
    fn two_char_intent_page_density_reserves_only_last_slot_for_singles_on_first_pages() {
        let mut ranked = Vec::new();
        for idx in 0..(TSF_PAGE_SIZE - 1) * TWO_CHAR_INTENT_SINGLE_RESERVE_PAGES {
            let ch = char::from_u32(0x4e00 + idx as u32).unwrap();
            ranked.push(ranked_candidate(
                &format!("\u{8bcd}{ch}"),
                200.0 - idx as f64,
            ));
        }
        for idx in 0..6 {
            let ch = char::from_u32(0x3400 + idx as u32).unwrap();
            ranked.push(ranked_candidate(&ch.to_string(), 100.0 - idx as f64));
        }
        ranked.push(ranked_candidate("\u{4e09}\u{5b57}\u{8bcd}", 80.0));

        arrange_two_char_intent_page_density(&mut ranked, TSF_PAGE_SIZE);

        for page in ranked
            .chunks(TSF_PAGE_SIZE)
            .take(TWO_CHAR_INTENT_SINGLE_RESERVE_PAGES)
        {
            assert_eq!(page.len(), TSF_PAGE_SIZE, "page: {page:?}");
            assert!(
                page[..TSF_PAGE_SIZE - 1]
                    .iter()
                    .all(|item| phrase_char_count(&item.phrase) == 2),
                "page: {page:?}"
            );
            assert_eq!(
                phrase_char_count(&page[TSF_PAGE_SIZE - 1].phrase),
                1,
                "page: {page:?}"
            );
        }
        assert_eq!(ranked.len(), (TSF_PAGE_SIZE - 1) * 3 + 6 + 1);
    }

    #[test]
    fn two_char_intent_page_density_absorbs_short_leftover_group() {
        let mut ranked = vec![
            ranked_candidate("\u{4e00}\u{4e8c}", 100.0),
            ranked_candidate("\u{4e09}\u{56db}", 99.0),
            ranked_candidate("\u{4e94}\u{516d}", 98.0),
            ranked_candidate("\u{4e03}\u{516b}", 97.0),
            ranked_candidate("\u{4e5d}", 96.0),
            ranked_candidate("\u{5341}", 95.0),
        ];

        arrange_two_char_intent_page_density(&mut ranked, TSF_PAGE_SIZE);

        assert_eq!(
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .filter(|item| phrase_char_count(&item.phrase) == 2)
                .count(),
            4,
            "ranked: {ranked:?}"
        );
        assert_eq!(ranked.len(), 6);
    }

    #[test]
    fn two_char_intent_page_density_uses_singles_before_overlong_predictions() {
        let mut ranked = vec![
            ranked_candidate("\u{5317}\u{4eac}\u{5e02}", 100.0),
            ranked_candidate("\u{5317}", 99.0),
            ranked_candidate("\u{5317}\u{4eac}", 98.0),
            ranked_candidate("\u{6d25}", 97.0),
            ranked_candidate("\u{4e0a}\u{6d77}\u{5e02}", 96.0),
            ranked_candidate("\u{4e0a}", 95.0),
            ranked_candidate("\u{5e7f}", 94.0),
        ];

        arrange_two_char_intent_page_density(&mut ranked, 5);

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{5317}\u{4eac}"),
            "ranked: {ranked:?}"
        );
        assert!(
            ranked
                .iter()
                .take(5)
                .all(|item| phrase_char_count(&item.phrase) <= 2),
            "ranked: {ranked:?}"
        );
        assert!(
            ranked
                .iter()
                .position(|item| item.phrase == "\u{5317}\u{4eac}\u{5e02}")
                .is_some_and(|position| position >= 5),
            "ranked: {ranked:?}"
        );
        assert_eq!(ranked.len(), 7);
    }

    #[test]
    fn final_two_char_visible_guard_drops_ordinary_expansions_when_page_is_short() {
        let mut ranked = vec![
            ranked_candidate("北京市", 100.0),
            ranked_candidate("北京", 99.0),
            ranked_candidate("北", 98.0),
            ranked_candidate("津", 97.0),
            ranked_candidate("上海市", 96.0),
        ];

        enforce_short_intent_visible_page(&mut ranked, 2, 5);

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("北京")
        );
        assert!(
            ranked
                .iter()
                .all(|item| phrase_char_count(&item.phrase) <= 2),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn final_two_char_visible_guard_keeps_one_pin_after_exact_word() {
        let mut pinned = ranked_candidate("北京城", 110.0);
        pinned.meta.pinned = true;
        let mut ranked = vec![
            pinned,
            ranked_candidate("上海市", 109.0),
            ranked_candidate("北京", 100.0),
            ranked_candidate("北", 99.0),
            ranked_candidate("津", 98.0),
            ranked_candidate("上", 97.0),
        ];

        enforce_short_intent_visible_page(&mut ranked, 2, 5);

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("北京")
        );
        let first_page = ranked.iter().take(5).collect::<Vec<_>>();
        assert_eq!(
            first_page
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) > 2)
                .count(),
            1,
            "first page: {first_page:?}"
        );
        assert!(first_page.iter().any(|item| item.meta.pinned));
        assert!(
            ranked
                .iter()
                .position(|item| item.phrase == "上海市")
                .is_some_and(|position| position >= 5),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn multi_syllable_two_char_full_pinyin_layout_reapplies_short_density_guard() {
        let _page_guard = set_effective_page_size_for_test(4);
        let empty: &[String] = &[];
        let engine = PinyinEngine::with_phrase_dir(None);
        let mut ranked = vec![
            ranked_candidate("你好", 100.0),
            ranked_candidate("你", 99.0),
            ranked_candidate("你好啊", 98.0),
            ranked_candidate("妮", 97.0),
            ranked_candidate("尼", 96.0),
        ];

        engine.apply_candidate_postprocess(
            &mut ranked,
            CandidatePostprocessContext {
                raw: "nihao",
                compact_key: "nihao",
                correction_prefs: CorrectionPrefs::default(),
                suppress_correction: false,
                short_phrase_intent: Some(2),
                final_short_phrase_intent: Some(2),
                input_intent: InputIntent::FullPinyin,
                exact_guard_syllables: Some(2),
                overlong_pinned_exact_guard_syllables: None,
                skip_final_short_density: false,
                exact_single_syllable_input: false,
                applied_multi_syllable_phrase_layout: true,
                direct_input_shortcut: false,
                user_hotword_front_limit: 0,
                preserved_direct_phrases: empty,
                preserved_short_phrases: empty,
                preserved_daily_short_two_char_phrases: empty,
                preserved_daily_short_three_char_phrases: empty,
                preserved_exact_lexicon_phrases: empty,
                preserved_exact_user_phrases: empty,
                preserved_mixed_user_phrases: empty,
                preserved_pinned_user_phrases: empty,
                preserved_high_priority_two_char_phrases: empty,
                preserved_chat_priority_two_char_phrases: empty,
                preserved_separator_phrases: empty,
                final_preferred_phrases: empty,
            },
        );

        assert!(
            ranked
                .iter()
                .take(4)
                .all(|item| phrase_char_count(&item.phrase) <= 2),
            "ranked: {ranked:?}"
        );
        assert!(
            ranked
                .iter()
                .position(|item| item.phrase == "你好啊")
                .is_some_and(|position| position >= 4),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn exact_short_intent_guard_moves_two_char_before_three_char_expansion() {
        let mut ranked = vec![
            ranked_candidate("\u{7c73}\u{56fd}\u{5929}", 100.0),
            ranked_candidate("\u{660e}\u{5929}", 99.0),
            ranked_candidate("\u{4eca}", 98.0),
        ];

        ensure_exact_short_intent_before_expansion(&mut ranked, Some(2));

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{660e}\u{5929}"),
            "ranked: {ranked:?}"
        );
    }

    #[test]
    fn final_short_intent_density_limits_total_four_char_expansions() {
        let mut ranked = vec![
            ranked_candidate("\u{5317}\u{4eac}\u{5927}\u{5b66}", 100.0),
            ranked_candidate("\u{8f93}\u{5165}\u{6cd5}", 99.0),
            ranked_candidate("\u{8f93}\u{5165}", 98.0),
            ranked_candidate("\u{4e0a}\u{6d77}\u{673a}\u{573a}", 97.0),
            ranked_candidate("\u{5e7f}\u{5dde}\u{5730}\u{94c1}", 96.0),
            ranked_candidate("\u{6df1}\u{5733}\u{5317}\u{7ad9}", 95.0),
            ranked_candidate("\u{5357}\u{4eac}\u{5357}\u{7ad9}", 94.0),
            ranked_candidate("\u{676d}\u{5dde}\u{4e1c}\u{7ad9}", 93.0),
            ranked_candidate("\u{8f93}", 92.0),
        ];

        apply_short_intent_final_candidate_density(&mut ranked, Some(3), None);

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{8f93}\u{5165}\u{6cd5}"),
            "ranked: {ranked:?}"
        );
        assert_eq!(
            ranked
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) == 4)
                .count(),
            SHORT_INTENT_THREE_CHAR_EXPANSION_LIMIT
        );
        let first_two = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 2)
            .unwrap();
        let first_four = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 4)
            .unwrap();
        assert!(first_two < first_four, "ranked: {ranked:?}");
        assert!(ranked
            .iter()
            .all(|item| phrase_char_count(&item.phrase) <= 4));
    }

    #[test]
    fn multi_syllable_layout_moves_low_freq_head_tail_after_singles() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [
            ("教师", 90_000),
            ("教室", 80_000),
            ("礁石", 70_000),
            ("教士", 30_000),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }
        engine.phrase_lexicon = Some(lex);
        let mut ranked = vec![
            ranked_candidate("教师", 100.0),
            ranked_candidate("教室", 99.0),
            ranked_candidate("礁石", 98.0),
            ranked_candidate("教士", 97.0),
            ranked_candidate("教", 96.0),
        ];
        let syllables = vec!["jiao".to_string(), "shi".to_string()];

        engine.apply_multi_syllable_phrase_head_then_first_syllable_singles(
            &mut ranked,
            2,
            &syllables,
            "jiaoshi",
            0,
        );

        let first_single = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 1)
            .unwrap();
        let low_freq_tail = ranked
            .iter()
            .position(|item| item.phrase == "教士")
            .unwrap();
        assert!(first_single < low_freq_tail, "ranked: {ranked:?}");
    }

    #[test]
    fn exact_full_pinyin_layout_moves_composed_noise_after_exact_and_singles() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{6559}\u{5e08}".into(),
            freq: 90_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        let mut ranked = vec![
            ranked_candidate("\u{811a}\u{662f}", 200.0),
            ranked_candidate("\u{6559}\u{5e08}", 100.0),
            ranked_candidate("\u{6559}", 99.0),
        ];
        let syllables = vec!["jiao".to_string(), "shi".to_string()];

        engine.apply_multi_syllable_phrase_head_then_first_syllable_singles(
            &mut ranked,
            2,
            &syllables,
            "jiaoshi",
            0,
        );

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{6559}\u{5e08}"),
            "ranked: {ranked:?}"
        );
        let first_single = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 1)
            .unwrap();
        let composed_noise = ranked
            .iter()
            .position(|item| item.phrase == "\u{811a}\u{662f}")
            .unwrap();
        assert!(first_single < composed_noise, "ranked: {ranked:?}");
    }

    #[test]
    fn long_exact_full_pinyin_layout_keeps_prefix_phrase_before_singles() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4f60}\u{597d}\u{4e16}\u{754c}".into(),
            freq: 120_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4f60}\u{597d}".into(),
            freq: 110_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        let mut ranked = vec![
            ranked_candidate("\u{4f60}\u{597d}\u{4e16}\u{754c}", 100.0),
            ranked_candidate("\u{4f60}", 99.0),
        ];
        let syllables = vec![
            "ni".to_string(),
            "hao".to_string(),
            "shi".to_string(),
            "jie".to_string(),
        ];

        engine.apply_multi_syllable_phrase_head_then_first_syllable_singles(
            &mut ranked,
            4,
            &syllables,
            "nihaoshijie",
            0,
        );

        let full = ranked
            .iter()
            .position(|item| item.phrase == "\u{4f60}\u{597d}\u{4e16}\u{754c}")
            .unwrap();
        let prefix = ranked
            .iter()
            .position(|item| item.phrase == "\u{4f60}\u{597d}")
            .unwrap();
        let first_single = ranked
            .iter()
            .position(|item| phrase_char_count(&item.phrase) == 1)
            .unwrap();
        assert_eq!(full, 0, "ranked: {ranked:?}");
        assert!(prefix < first_single, "ranked: {ranked:?}");
    }

    #[test]
    fn real_lexicon_common_sentence_prefers_nihao_shijie() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("lexicon");
        if !dir.is_dir() {
            return;
        }
        let mut engine = PinyinEngine::with_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();
        let (ranked, err) = engine.lookup_full("nihaoshijie");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(ranked.first().map(|item| item.0.as_str()), Some("你好世界"));
    }

    #[test]
    fn short_intent_exact_bonus_is_stronger_than_shorter_words() {
        assert!(
            short_intent_exact_bonus(Some(3), "\u{8f93}\u{5165}\u{6cd5}")
                > short_intent_exact_bonus(Some(3), "输入")
        );
        assert_eq!(
            short_intent_exact_bonus(Some(2), "\u{8f93}\u{5165}\u{6cd5}"),
            0.0
        );
        assert!(
            short_intent_exact_bonus(Some(4), "\u{7532}\u{4e59}\u{4e19}\u{4e01}")
                > short_intent_exact_bonus(Some(4), "\u{7532}\u{4e59}\u{4e19}")
        );
    }

    #[test]
    fn three_and_four_char_intents_keep_high_frequency_exact_words_in_front() {
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [
            ("输入法", 90_000),
            ("候选词", 80_000),
            ("开心输入", 90_000),
            ("中文输入", 80_000),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }

        let mut three = vec![
            ranked_candidate("输入", 120.0),
            ranked_candidate("输", 119.0),
            ranked_candidate("输入法", 118.0),
            ranked_candidate("候选词", 117.0),
        ];
        apply_short_intent_final_candidate_density(&mut three, Some(3), Some(&lex));
        assert_eq!(
            three
                .iter()
                .take(2)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>(),
            vec!["输入法", "候选词"]
        );

        let mut four = vec![
            ranked_candidate("开心", 120.0),
            ranked_candidate("开", 119.0),
            ranked_candidate("开心输入", 118.0),
            ranked_candidate("中文输入", 117.0),
        ];
        apply_short_intent_final_candidate_density(&mut four, Some(4), Some(&lex));
        assert_eq!(
            four.iter()
                .take(2)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>(),
            vec!["开心输入", "中文输入"]
        );
    }

    #[test]
    fn confident_long_exact_phrase_moves_weak_corrections_out_of_first_page() {
        let mut ranked = vec![
            RankedCandidate {
                phrase: "中华人民共和国".into(),
                score: 500.0,
                meta: CandidateMeta::legacy(PINYIN_CANDIDATE_META),
            },
            RankedCandidate {
                phrase: "中华人面共和国".into(),
                score: 160.0,
                meta: CandidateMeta::legacy("纠错: test\ttypo=1\tcorrection=keyboard"),
            },
            ranked_candidate("中华", 150.0),
            ranked_candidate("中", 140.0),
        ];
        demote_corrections_after_confident_long_exact(&mut ranked);
        assert_eq!(ranked[0].phrase, "中华人民共和国");
        assert!(
            ranked
                .iter()
                .position(|item| item.phrase == "中华人面共和国")
                .unwrap()
                > ranked.iter().position(|item| item.phrase == "中").unwrap()
        );
    }

    #[test]
    fn exact_single_syllable_input_keeps_only_single_char_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.user_lexicon.learn("shu", "舒服").unwrap();

        let (ranked, err) = engine.lookup_full("shu");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert!(ranked
            .iter()
            .all(|(phrase, _)| phrase_char_count(phrase) == 1));
        assert!(!ranked.iter().any(|(phrase, _)| phrase == "舒服"));
    }

    #[test]
    fn exact_single_syllable_uses_explicit_lexicon_single_char_readings() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.dict = Arc::new(HashMap::new());
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed_with_layer(
            ThuoclEntry {
                phrase: "乐".into(),
                freq: 77_726,
                code: Some("yue".into()),
            },
            LexiconLayer::Core,
        ));
        engine.phrase_lexicon = Some(lex);

        let mut merged = MergedCandidateMap::default();
        engine.merge_exact_single_syllable_chars("yue", &mut merged, 0);

        assert!(merged.contains_key("乐"));
    }

    #[test]
    fn exact_ma_input_keeps_only_single_char_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.user_lexicon.learn("ma", "妈妈").unwrap();

        let (ranked, err) = engine.lookup_full("ma");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert!(ranked
            .iter()
            .all(|(phrase, _)| phrase_char_count(phrase) == 1));
        assert!(!ranked.iter().any(|(phrase, _)| phrase == "妈妈"));
    }

    #[test]
    fn exact_ye_input_keeps_only_single_char_candidates_even_with_abbrev_words() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [("幼儿", 9_000), ("婴儿", 8_800), ("鱼饵", 8_600)] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();
        engine.user_lexicon.learn("ye", "幼儿").unwrap();

        let (ranked, err) = engine.lookup_full("ye");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert!(ranked
            .iter()
            .all(|(phrase, _)| phrase_char_count(phrase) == 1));
        assert!(!ranked.iter().any(|(phrase, _)| phrase == "幼儿"));
        assert!(!ranked.iter().any(|(phrase, _)| phrase == "婴儿"));
        assert!(!ranked.iter().any(|(phrase, _)| phrase == "鱼饵"));
    }

    #[test]
    fn typo_corrected_candidates_expose_correction_meta() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 9_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "背景".into(),
            freq: 8_500,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("bejing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let beijing = ranked
            .iter()
            .find(|item| item.phrase == "北京")
            .expect("expected 北京 candidate");
        assert!(
            beijing
                .meta
                .display_text()
                .is_some_and(|meta| meta.starts_with("纠错: bejing->beijing")),
            "expected correction meta, got {:?}",
            beijing.meta
        );
        let typo_meta_count = ranked
            .iter()
            .filter(|item| {
                item.meta
                    .display_text()
                    .map(|meta| meta.starts_with("纠错"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(typo_meta_count, 1);
    }

    #[test]
    fn keyboard_typo_correction_is_visible_in_top_three() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("bejing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let beijing_pos = ranked
            .iter()
            .position(|item| item.phrase == "\u{5317}\u{4eac}")
            .expect("expected Beijing correction candidate");
        assert!(
            beijing_pos < 3,
            "expected Beijing typo correction in top3, got pos={beijing_pos}, top={:?}",
            ranked
                .iter()
                .take(5)
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            correction_style_kind(&ranked[beijing_pos].meta),
            Some("typo")
        );
    }

    #[test]
    fn exact_parse_suppresses_correction_meta() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let (ranked, _) = engine.lookup_full_detailed("han");
        assert!(!ranked.iter().any(|item| {
            item.meta
                .display_text()
                .is_some_and(|_| correction_style_kind(&item.meta).is_some())
        }));
    }

    #[test]
    fn phonetic_typo_exposes_correction_meta_with_corrected_reading() {
        let variant = crate::segment::ParseVariant {
            syllables: vec!["guang".to_string()],
            tail: None,
            source: crate::segment::ParseVariantSource::PhoneticTypo,
            corrected_input: Some("guang".to_string()),
        };
        let meta = correction_meta_for_parse_variant("guan", &variant);
        assert!(
            meta.as_deref().is_some_and(|m| {
                m.starts_with("纠错")
                    && m.contains("typo=1")
                    && m.contains("correction=phonetic")
                    && m.contains("corrected_reading=guang")
            }),
            "expected correction meta with corrected_reading, got {:?}",
            meta
        );
    }

    #[test]
    fn learned_correction_pair_boosts_future_candidate() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked_before, _) = engine.lookup_full_detailed("beijng");
        let before_pos = ranked_before
            .iter()
            .position(|item| item.phrase == "北京")
            .unwrap_or(ranked_before.len());

        engine.learn_correction("beijng", "beijing").unwrap();
        for _ in 0..4 {
            engine.learn_correction("beijng", "beijing").unwrap();
        }

        let (ranked_after, _) = engine.lookup_full_detailed("beijng");
        let after_pos = ranked_after
            .iter()
            .position(|item| item.phrase == "北京")
            .unwrap_or(ranked_after.len());

        assert!(
            after_pos < before_pos || after_pos < 3,
            "expected correction pair to boost 北京: before={before_pos}, after={after_pos}"
        );
    }

    #[test]
    fn correction_selection_learns_corrected_spelling_not_raw_typo() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine.learn_correction("bejing", "beijing").unwrap();
        engine
            .learn_commit("beijing", "\u{5317}\u{4eac}")
            .expect("learn corrected spelling");

        assert!(engine.user_lexicon.lookup_input("bejing").is_none());
        assert!(engine
            .user_lexicon
            .lookup_input("beijing")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{5317}\u{4eac}")));
        assert!(engine
            .user_lexicon
            .lookup_correction_pairs("bejing")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == "beijing")));
    }

    #[test]
    fn user_exact_input_suppresses_correction() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        // 把 "beijng" 当作一个合法的用户词多次学习，触发抑制门控。
        for _ in 0..USER_EXACT_SUPPRESS_CORRECTION_THRESHOLD {
            engine.user_lexicon.learn("beijng", "自定义").unwrap();
        }
        let (ranked, _) = engine.lookup_full_detailed("beijng");
        let typo_count = ranked
            .iter()
            .filter(|item| correction_style_kind(&item.meta) == Some("typo"))
            .count();
        assert_eq!(typo_count, 0, "expected typo corrections to be suppressed");
    }

    #[test]
    fn correction_candidates_stay_below_user_history_in_real_lookup() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.user_lexicon.learn("bejing", "自定义").unwrap();

        // Check both the full lookup and the subsequent cache-hit path.
        for attempt in 1..=2 {
            let (ranked, err) = engine.lookup_full_detailed("bejing");
            assert!(err.is_none(), "unexpected lookup error: {err:?}");
            let user_pos = ranked
                .iter()
                .position(|item| item.phrase == "自定义")
                .expect("user history candidate");
            let correction_pos = ranked
                .iter()
                .position(|item| correction_style_kind(&item.meta).is_some())
                .expect("correction candidate");

            assert!(
                user_pos < correction_pos,
                "user history should outrank correction on attempt {attempt}: user_pos={user_pos}, correction_pos={correction_pos}, top={:?}",
                ranked.iter().take(5).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selection_feedback_boosts_reselected_candidate() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (ranked_before, _) = engine.lookup_full_detailed("shurufa");
        let before_pos = ranked_before
            .iter()
            .position(|item| item.phrase == "\u{8f93}\u{5165}\u{6cd5}")
            .unwrap_or(ranked_before.len());

        engine
            .learn_selection_feedback(
                "shurufa",
                "\u{8f93}\u{5165}\u{6cd5}",
                3,
                0,
                &[
                    "\u{4e66}\u{6cd5}".to_string(),
                    "\u{8f93}\u{5165}".to_string(),
                    "\u{5c5e}".to_string(),
                ],
            )
            .unwrap();

        let feedback = engine
            .user_lexicon
            .selection_signal("shurufa")
            .expect("selection feedback");
        assert!(feedback
            .iter()
            .any(|entry| entry.phrase == "\u{4e66}\u{6cd5}" && entry.score < 0));

        let (ranked_after, _) = engine.lookup_full_detailed("shurufa");
        let after_pos = ranked_after
            .iter()
            .position(|item| item.phrase == "\u{8f93}\u{5165}\u{6cd5}")
            .unwrap_or(ranked_after.len());

        assert!(
            after_pos < before_pos || after_pos < 3,
            "expected selection feedback to boost selected candidate: before={before_pos}, after={after_pos}"
        );
    }

    #[test]
    fn repeated_four_char_selection_reaches_top_without_cross_key_pollution() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "实事求是";

        for _ in 0..2 {
            engine
                .learn_selection_feedback("ssqs", phrase, 1, 0, &["是是去世".to_string()])
                .unwrap();
        }

        let mut ranked = vec![
            ranked_candidate("是是去世", 100.0),
            ranked_candidate(phrase, 50.0),
        ];
        assert!(engine.apply_selection_feedback("ssqs", &mut ranked));
        ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some(phrase)
        );
        assert!(
            engine.user_lexicon.selection_signal("shisqs").is_none(),
            "selection feedback must stay bound to the spelling the user selected"
        );
    }

    #[test]
    fn removing_user_phrase_leaves_decaying_exact_key_tombstone() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "实事求是";
        engine.learn_commit("ssqs", phrase).unwrap();

        assert!(engine.remove_user_phrase("ssqs", phrase).unwrap());
        assert!(!engine
            .user_lexicon
            .lookup_input("ssqs")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == phrase)));
        let feedback = engine
            .user_lexicon
            .selection_signal("ssqs")
            .expect("removed phrase feedback");
        assert!(feedback.iter().any(|entry| {
            entry.phrase == phrase && entry.score == REMOVED_PHRASE_FEEDBACK_DELTA
        }));
    }

    #[test]
    fn selection_feedback_penalizes_previous_page_skipped_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let skipped = (0..=TSF_PAGE_SIZE)
            .map(|idx| format!("候选{idx}"))
            .collect::<Vec<_>>();
        engine
            .learn_selection_feedback("ceshi", "最终选择", TSF_PAGE_SIZE + 1, 1, &skipped)
            .unwrap();

        let feedback = engine
            .user_lexicon
            .selection_signal("ceshi")
            .expect("selection feedback");
        let previous_page_score = feedback
            .iter()
            .find(|entry| entry.phrase == "候选0")
            .expect("previous page candidate")
            .score;
        let current_page_score = feedback
            .iter()
            .find(|entry| entry.phrase == format!("候选{TSF_PAGE_SIZE}"))
            .expect("current page candidate")
            .score;

        assert!(
            previous_page_score < current_page_score,
            "previous page candidate should receive stronger negative feedback"
        );
    }

    #[test]
    fn visible_unselected_candidates_accumulate_weak_feedback() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        for _ in 0..WEAK_UNSELECTED_FEEDBACK_APPLY_THRESHOLD {
            engine
                .learn_selection_feedback("ceshi", "测试", 0, 0, &["侧视".to_string()])
                .unwrap();
        }

        assert!(
            engine.user_lexicon.selection_signal("ceshi").is_none(),
            "visible unselected candidates should not enter strong selection feedback"
        );
        let weak = engine
            .user_lexicon
            .weak_unselected_signal("ceshi")
            .expect("weak feedback");
        let side = weak
            .iter()
            .find(|entry| entry.phrase == "侧视")
            .expect("weakly unselected candidate");
        assert_eq!(side.score, -WEAK_UNSELECTED_FEEDBACK_APPLY_THRESHOLD);

        let mut ranked = vec![
            ranked_candidate("侧视", 100.0),
            ranked_candidate("测试", 96.0),
        ];
        assert!(engine.apply_selection_feedback("ceshi", &mut ranked));
        assert!(
            ranked[0].score < 100.0,
            "weak feedback should apply only after repeated observations: {:?}",
            ranked
        );
    }

    #[test]
    fn selection_feedback_records_context_path_signal() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine.learn_commit("dakai", "打开").unwrap();
        engine
            .learn_selection_feedback("shuru", "输入法", 2, 0, &["输入".into(), "数入".into()])
            .unwrap();

        let feedback = engine
            .user_lexicon
            .context_selection_signal("打开", "shuru")
            .expect("context selection feedback");
        assert!(feedback
            .iter()
            .any(|entry| entry.phrase == "输入法" && entry.score > 0));
        assert!(feedback
            .iter()
            .any(|entry| entry.phrase == "输入" && entry.score < 0));

        engine
            .learn_selection_feedback("shuru", "输入法", 0, 0, &["数入".into()])
            .unwrap();
        assert!(
            engine
                .user_lexicon
                .weak_unselected_signal("shuru")
                .is_none(),
            "contextual weak feedback should not fall back to global path feedback"
        );
        let context_weak = engine
            .user_lexicon
            .context_weak_unselected_signal("打开", "shuru")
            .expect("context weak feedback");
        assert!(context_weak
            .iter()
            .any(|entry| entry.phrase == "数入" && entry.score < 0));
    }

    #[test]
    fn no_learn_flag_skips_learning() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .learn_commit_with_flags("ceshi", "测试", LEARN_FLAG_NO_LEARN)
            .unwrap();
        assert!(engine.user_lexicon.lookup_input("ceshi").is_none());
    }

    #[test]
    fn learn_commit_rejects_pure_punctuation() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine.learn_commit("ceshi", "，。").unwrap();
        assert!(engine.user_lexicon.lookup_input("ceshi").is_none());
    }

    #[test]
    fn composed_novel_phrase_learns_whole_and_edge_subphrases_immediately() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine
            .learn_commit_with_flags(
                "duomotaiduiqi",
                "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}",
                LEARN_FLAG_COMPOSED_PHRASE,
            )
            .unwrap();

        assert!(engine
            .user_lexicon
            .lookup_input("duomotaiduiqi")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}")));
        assert!(engine
            .user_lexicon
            .lookup_input("duomotai")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{591a}\u{6a21}\u{6001}")));
        assert!(engine
            .user_lexicon
            .lookup_input("duiqi")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{5bf9}\u{9f50}")));
    }

    #[test]
    fn composed_long_novel_phrase_learns_stronger_boundaries_not_generic_edges() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        engine
            .learn_commit_with_flags(
                "duomotaiduiqisuanfa",
                "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}\u{7b97}\u{6cd5}",
                LEARN_FLAG_COMPOSED_PHRASE,
            )
            .unwrap();

        assert!(engine
            .user_lexicon
            .lookup_input("duomotaiduiqisuanfa")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase
                    == "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}\u{7b97}\u{6cd5}")));
        assert!(engine
            .user_lexicon
            .lookup_input("duomotaiduiqi")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}")));
        assert!(engine
            .user_lexicon
            .lookup_input("duiqisuanfa")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{5bf9}\u{9f50}\u{7b97}\u{6cd5}")));
        assert!(engine.user_lexicon.lookup_input("duomo").is_none());
        assert!(engine.user_lexicon.lookup_input("suanfa").is_none());
    }

    #[test]
    fn composed_novel_phrase_does_not_learn_blocked_subphrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .user_lexicon
            .block_phrase("\u{591a}\u{6a21}\u{6001}")
            .unwrap();

        engine
            .learn_commit_with_flags(
                "duomotaiduiqi",
                "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}",
                LEARN_FLAG_COMPOSED_PHRASE,
            )
            .unwrap();

        assert!(engine
            .user_lexicon
            .lookup_input("duomotaiduiqi")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{591a}\u{6a21}\u{6001}\u{5bf9}\u{9f50}")));
        assert!(engine.user_lexicon.lookup_input("duomotai").is_none());
        assert!(engine
            .user_lexicon
            .lookup_input("duiqi")
            .is_some_and(|entries| entries
                .iter()
                .any(|entry| entry.phrase == "\u{5bf9}\u{9f50}")));
    }

    #[test]
    fn correction_style_candidates_keep_at_most_one_total_after_normal_head() {
        let mut ranked = vec![
            RankedCandidate {
                phrase: "纠一".into(),
                score: 30.0,
                meta: CandidateMeta::legacy("纠错: a->b"),
            },
            RankedCandidate {
                phrase: "音一".into(),
                score: 29.0,
                meta: CandidateMeta::legacy("音近: a->c"),
            },
            RankedCandidate {
                phrase: "纠二".into(),
                score: 28.0,
                meta: CandidateMeta::legacy("纠错"),
            },
            RankedCandidate {
                phrase: "音二".into(),
                score: 27.0,
                meta: CandidateMeta::legacy("音近"),
            },
            ranked_candidate("正常", 26.0),
        ];

        limit_typo_corrections_first_page(
            &mut ranked,
            CorrectionPrefs {
                enabled: true,
                level: CorrectionLevel::Medium,
            },
            false,
        );

        assert_eq!(ranked[0].phrase, "正常");
        assert_eq!(ranked[TSF_PAGE_SIZE.min(ranked.len()) - 1].phrase, "纠一");
        assert_eq!(
            ranked
                .iter()
                .filter(|item| correction_style_kind(&item.meta).is_some())
                .count(),
            1
        );
    }

    #[test]
    fn typo_correction_is_preferred_over_near_sound_correction() {
        let mut ranked = vec![
            ranked_candidate("正常", 26.0),
            RankedCandidate {
                phrase: "音近项".into(),
                score: 999.0,
                meta: CandidateMeta::legacy("音近: ni->li\ttypo=1\tcorrection=syllable_confusion"),
            },
            RankedCandidate {
                phrase: "键盘项".into(),
                score: 10.0,
                meta: CandidateMeta::legacy("纠错: bejing->beijing\ttypo=1\tcorrection=keyboard"),
            },
        ];

        limit_typo_corrections_first_page(
            &mut ranked,
            CorrectionPrefs {
                enabled: true,
                level: CorrectionLevel::Medium,
            },
            false,
        );

        assert!(ranked.iter().any(|item| item.phrase == "键盘项"));
        assert!(!ranked.iter().any(|item| item.phrase == "音近项"));
    }

    #[test]
    fn two_char_intent_filters_single_char_correction_noise() {
        let mut ranked = vec![
            ranked_candidate("\u{8f93}\u{5165}", 100.0),
            RankedCandidate {
                phrase: "\u{4e66}".into(),
                score: 99.0,
                meta: CandidateMeta::legacy("纠错: shuru->shue"),
            },
            RankedCandidate {
                phrase: "\u{4e66}\u{5165}".into(),
                score: 98.0,
                meta: CandidateMeta::legacy("纠错: shuru->shuru"),
            },
            ranked_candidate("\u{6570}", 97.0),
        ];

        drop_single_char_corrections_for_two_char_intent(&mut ranked, Some(2));

        assert!(!ranked.iter().any(|item| {
            item.phrase == "\u{4e66}" && correction_style_kind(&item.meta).is_some()
        }));
        assert!(ranked.iter().any(|item| item.phrase == "\u{4e66}\u{5165}"));
        assert!(ranked.iter().any(|item| item.phrase == "\u{6570}"));
    }

    #[test]
    fn correction_stays_below_user_and_exact_full_pinyin_hits() {
        let mut exact_full = HashSet::new();
        exact_full.insert("\u{8f93}\u{5165}".to_string());
        let mut ranked = vec![
            RankedCandidate {
                phrase: "\u{4e66}\u{5165}".into(),
                score: 1000.0,
                meta: CandidateMeta::legacy("纠错: shuru->shulu\ttypo=1\tcorrection=keyboard"),
            },
            RankedCandidate {
                phrase: "\u{5e38}\u{7528}".into(),
                score: 10.0,
                meta: CandidateMeta::user(false),
            },
            RankedCandidate {
                phrase: "\u{8f93}\u{5165}".into(),
                score: 9.0,
                meta: CandidateMeta::legacy(PINYIN_CANDIDATE_META),
            },
        ];

        demote_corrections_below_priority_hits(&mut ranked, &exact_full, Some(2));

        let correction_pos = ranked
            .iter()
            .position(|item| item.phrase == "\u{4e66}\u{5165}")
            .unwrap();
        let user_pos = ranked
            .iter()
            .position(|item| item.phrase == "\u{5e38}\u{7528}")
            .unwrap();
        let exact_pos = ranked
            .iter()
            .position(|item| item.phrase == "\u{8f93}\u{5165}")
            .unwrap();
        assert!(user_pos < correction_pos);
        assert!(exact_pos < correction_pos);
    }

    #[test]
    fn strong_two_char_correction_moves_before_longer_expansion() {
        let mut ranked = vec![
            ranked_candidate("\u{4e09}\u{5b57}\u{8bcd}", 120.0),
            ranked_candidate("\u{5176}\u{4ed6}", 110.0),
            RankedCandidate {
                phrase: "\u{5317}\u{4eac}".into(),
                score: 130.0,
                meta: CandidateMeta::legacy("纠错: bejing->beijing"),
            },
        ];

        ensure_strong_two_char_correction_before_expansion(
            &mut ranked,
            CorrectionPrefs {
                enabled: true,
                level: CorrectionLevel::Strong,
            },
            false,
        );

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{5317}\u{4eac}")
        );
    }

    #[test]
    fn medium_two_char_correction_does_not_jump_to_first() {
        let mut ranked = vec![
            ranked_candidate("\u{4e09}\u{5b57}\u{8bcd}", 120.0),
            RankedCandidate {
                phrase: "\u{5317}\u{4eac}".into(),
                score: 100.0,
                meta: CandidateMeta::legacy("纠错: bejing->beijing"),
            },
        ];

        ensure_strong_two_char_correction_before_expansion(
            &mut ranked,
            CorrectionPrefs {
                enabled: true,
                level: CorrectionLevel::Medium,
            },
            false,
        );

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{4e09}\u{5b57}\u{8bcd}")
        );
    }

    #[test]
    fn low_score_strong_two_char_correction_does_not_jump_to_first() {
        let mut ranked = vec![
            ranked_candidate("\u{4e09}\u{5b57}\u{8bcd}", 120.0),
            RankedCandidate {
                phrase: "\u{5317}\u{4eac}".into(),
                score: 100.0,
                meta: CandidateMeta::legacy("纠错: bejing->beijing"),
            },
        ];

        ensure_strong_two_char_correction_before_expansion(
            &mut ranked,
            CorrectionPrefs {
                enabled: true,
                level: CorrectionLevel::Strong,
            },
            false,
        );

        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("\u{4e09}\u{5b57}\u{8bcd}")
        );
    }

    #[test]
    fn mixed_shortcuts_limit_three_char_expansions_for_two_char_input() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "输入".into(),
            freq: 5_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}\u{6cd5}".into(),
            freq: 6_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for input in ["sr", "sru", "shur"] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert!(ranked.iter().any(|(phrase, _)| phrase == "输入"));
            assert!(ranked
                .iter()
                .all(|(phrase, _)| phrase_char_count(phrase) <= 3));
            assert!(
                ranked
                    .iter()
                    .take(PAGE_SIZE)
                    .all(|(phrase, _)| phrase_char_count(phrase) <= 2),
                "ordinary three-char expansion mixed into the five-slot page for {input}: {:?}",
                ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>()
            );
            assert!(
                ranked
                    .iter()
                    .filter(|(phrase, _)| phrase_char_count(phrase) == 3)
                    .count()
                    <= 2
            );
        }
    }

    #[test]
    fn mixed_initial_then_full_pinyin_finds_two_char_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{91cd}\u{5e86}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("cqing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            ranked
                .iter()
                .any(|(phrase, _)| phrase == "\u{91cd}\u{5e86}"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn mixed_pinyin_path_scoring_prefers_one_initial_path() {
        let high = MixedPinyinKeyExpansion {
            key: "shuru".to_string(),
            syllables: vec!["shu".to_string(), "ru".to_string()],
            segment_count: 2,
            full_segments: 1,
            prefix_segments: 0,
            initial_segments: 1,
            leading_full: true,
            substantive_full_segments: 1,
            path_score: 0.0,
        };
        let low = MixedPinyinKeyExpansion {
            key: "shurufa".to_string(),
            syllables: vec!["shu".to_string(), "ru".to_string(), "fa".to_string()],
            segment_count: 3,
            full_segments: 1,
            prefix_segments: 0,
            initial_segments: 2,
            leading_full: true,
            substantive_full_segments: 1,
            path_score: 0.0,
        };

        assert!(is_high_confidence_mixed_key(&high));
        assert!(!is_high_confidence_mixed_key(&low));
        assert!(mixed_pinyin_path_adjustment(&high) > mixed_pinyin_path_adjustment(&low));
    }

    #[test]
    fn mixed_pinyin_can_be_disabled_independently_from_jianpin() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(0);

        let (ranked, err) = engine.lookup_full_detailed("sru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            !ranked.iter().any(|item| item.phrase == "\u{8f93}\u{5165}"
                && item
                    .meta
                    .display_text()
                    .is_some_and(|meta| meta.starts_with("混拼"))),
            "unexpected mixed candidate with mixed disabled: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );

        engine.set_mode_flags(MODE_MIXED_PINYIN);
        let (ranked, err) = engine.lookup_full_detailed("sru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(ranked.iter().any(|item| item.phrase == "\u{8f93}\u{5165}"
            && item
                .meta
                .display_text()
                .is_some_and(|meta| meta.starts_with("混拼"))));
    }

    #[test]
    fn high_priority_two_char_abbrev_promotes_priority_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4e0a}\u{6d77}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{751f}\u{6d3b}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("sh");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4e0a}\u{6d77}")
        );
    }

    #[test]
    fn high_priority_two_char_full_first_syllable_then_initial_promotes_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4e0a}\u{6d77}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4f24}\u{5bb3}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("shangh");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4e0a}\u{6d77}")
        );
    }

    #[test]
    fn high_priority_two_char_initial_then_full_second_syllable_promotes_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{91cd}\u{5e86}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{866b}\u{60c5}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("cqing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{91cd}\u{5e86}")
        );
    }

    #[test]
    fn high_priority_two_char_beats_learned_exact_abbrev() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();
        for _ in 0..8 {
            engine
                .learn_commit("bj", "\u{6807}\u{8bb0}")
                .expect("learn bj competitor");
        }

        let (ranked, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{5317}\u{4eac}")
        );
    }

    #[test]
    fn pinned_user_phrase_beats_high_priority_two_char() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        engine
            .set_candidate_pin("bj", "\u{6807}\u{8bb0}", true)
            .expect("pin bj competitor");

        let (ranked, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{6807}\u{8bb0}")
        );
    }

    #[test]
    fn high_priority_two_char_does_not_override_direct_datetime_shortcut() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{65f6}\u{95f4}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("sj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_ne!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{65f6}\u{95f4}")
        );
    }

    #[test]
    fn mixed_full_then_initial_and_partial_prefix_find_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{91cd}\u{5e86}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for input in ["chongq", "cqi"] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert!(
                ranked
                    .iter()
                    .any(|(phrase, _)| phrase == "\u{91cd}\u{5e86}"),
                "expected 重庆 for {input}"
            );
        }
    }

    #[test]
    fn long_mixed_input_composes_known_tail_phrase() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("lexicon");
        if !dir.is_dir() {
            return;
        }
        let mut engine = PinyinEngine::with_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(
            MODE_V_ASSIST
                | MODE_SYMBOL_TOOLBOX
                | MODE_EMOJI_INPUT
                | MODE_DATE_AUTO_FORMAT
                | MODE_JIANPIN
                | MODE_MIXED_PINYIN,
        );
        let expansions =
            engine.mixed_key_expansions("jisuanjwl", engine.user_lexicon.current_clock());
        let target_expansion = expansions
            .iter()
            .find(|item| item.key == "jisuanjiwangluo")
            .unwrap_or_else(|| {
                panic!(
                    "mixed expansion should retain the target key: {:?}",
                    expansions
                        .iter()
                        .take(16)
                        .map(|item| item.key.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(target_expansion.syllables.join(" "), "ji suan ji wang luo");

        let (ranked, err) = engine.lookup_full("jisuanjwl");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .any(|(candidate, _)| candidate == "计算机网络"),
            "long mixed path should compose 计算机 + 网络: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
        for noisy in [
            "计算技网络",
            "计算计网络",
            "计算鸡网络",
            "计算机威廉",
            "计算机王磊",
        ] {
            assert!(
                !ranked
                    .iter()
                    .take(TSF_PAGE_SIZE)
                    .any(|(candidate, _)| candidate == noisy),
                "broken 计算机 + 网络 boundary leaked into the first page: {noisy}: {:?}",
                ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn four_char_mixed_patterns_recall_exact_phrases() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq, code) in [
            ("实事求是", 82_000, "shi shi qiu shi"),
            ("无论如何", 80_000, "wu lun ru he"),
            ("理所当然", 78_000, "li suo dang ran"),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: Some(code.into()),
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for (input, expected) in [
            ("shisqs", "实事求是"),
            ("shisqius", "实事求是"),
            ("shishqish", "实事求是"),
            ("wulrh", "无论如何"),
            ("lisdr", "理所当然"),
        ] {
            let expansions =
                engine.mixed_key_expansions(input, engine.user_lexicon.current_clock());
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert!(
                ranked
                    .iter()
                    .take(TSF_MAX_CANDIDATES)
                    .any(|(phrase, _)| phrase == expected),
                "expected {expected} for {input}, expansions: {:?}, ranked: {ranked:?}",
                expansions
                    .iter()
                    .map(|item| item.key.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn mixed_three_char_intent_limits_four_and_five_char_prefix_noise() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq, code) in [
            ("输入法", 90_000, "shu ru fa"),
            ("输入法器", 9_000_000, "shu ru fa qi"),
            ("输入法学", 8_000_000, "shu ru fa xue"),
            ("输入法界", 7_000_000, "shu ru fa jie"),
            ("输入法工具", 6_000_000, "shu ru fa gong ju"),
            ("输入法教程", 5_000_000, "shu ru fa jiao cheng"),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: Some(code.into()),
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for pass in ["initial", "cache"] {
            let (ranked, err) = engine.lookup_full("shuruf");
            assert!(err.is_none(), "unexpected {pass} error: {err:?}");
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase.as_str()),
                Some("输入法"),
                "{pass} ranked: {:?}",
                ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>()
            );
            let first_page = ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>();
            assert_eq!(first_page.len(), PAGE_SIZE, "{pass} ranked: {ranked:?}");
            assert!(
                first_page
                    .iter()
                    .filter(|(phrase, _)| phrase_char_count(phrase) == 4)
                    .count()
                    <= 1,
                "{pass} four-char prefix noise crowded the five-slot page: {first_page:?}"
            );
            assert!(
                first_page
                    .iter()
                    .all(|(phrase, _)| phrase_char_count(phrase) < 5),
                "{pass} five-char prefix noise should stay off the first page: {first_page:?}"
            );
            assert!(
                first_page
                    .iter()
                    .any(|(phrase, _)| phrase_char_count(phrase) <= 2),
                "{pass} three-char intent should leave room for shorter composition candidates: {first_page:?}"
            );
        }
    }

    #[test]
    fn three_char_abbrev_lookup_keeps_single_char_candidates_on_first_page() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}\u{6cd5}".into(),
            freq: 9_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{6444}\u{5165}\u{6cd5}".into(),
            freq: 8_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{79c1}\u{4eba}\u{623f}".into(),
            freq: 7_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("srf");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(ranked
            .iter()
            .any(|(phrase, _)| phrase == "\u{8f93}\u{5165}\u{6cd5}"));
        assert!(
            ranked
                .iter()
                .take(PAGE_SIZE)
                .any(|(phrase, _)| phrase_char_count(phrase) == 1),
            "five-slot first page should contain a single-char candidate: {:?}",
            ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>()
        );
    }

    #[test]
    fn three_char_intent_keeps_two_char_and_single_candidates_on_later_pages() {
        let mut ranked = Vec::new();
        for idx in 0..24 {
            ranked.push(ranked_candidate(&format!("三字{idx}"), 300.0 - idx as f64));
        }
        for phrase in ["输入", "收入", "深入", "生日", "诗人", "私人"] {
            ranked.push(ranked_candidate(phrase, 200.0));
        }
        for phrase in ["输", "入", "法"] {
            ranked.push(ranked_candidate(phrase, 100.0));
        }

        arrange_three_char_intent_page_density(&mut ranked, TSF_PAGE_SIZE);

        for page in 0..3 {
            let candidates = ranked
                .iter()
                .skip(page * TSF_PAGE_SIZE)
                .take(TSF_PAGE_SIZE)
                .collect::<Vec<_>>();
            assert!(
                candidates
                    .iter()
                    .any(|item| phrase_char_count(&item.phrase) == 2),
                "page {} should contain a two-character candidate: {:?}",
                page + 1,
                candidates
            );
            assert!(
                candidates
                    .iter()
                    .any(|item| phrase_char_count(&item.phrase) == 1),
                "page {} should contain a single-character candidate: {:?}",
                page + 1,
                candidates
            );
        }
    }

    #[test]
    fn exact_three_char_abbrev_beats_two_char_prefix_intent() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}".into(),
            freq: 99_000,
            code: None,
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}\u{6cd5}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("srf");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}\u{6cd5}")
        );
    }

    #[test]
    fn repeated_three_char_abbrev_cache_preserves_exact_front_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [
            ("\u{8f93}\u{5165}\u{6cd5}", 45_338),
            ("\u{4e09}\u{65e5}\u{4efd}", 36_000),
            ("\u{53d7}\u{8ba9}\u{65b9}", 33_146),
            ("\u{53cc}\u{4eba}\u{623f}", 30_953),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for pass in 1..=2 {
            let (ranked, err) = engine.lookup_full("srf");
            assert!(err.is_none(), "pass {pass} unexpected error: {err:?}");
            assert!(
                ranked
                    .iter()
                    .take(3)
                    .any(|(phrase, _)| phrase == "\u{4e09}\u{65e5}\u{4efd}"),
                "pass {pass} should keep 三日份 in top three: {:?}",
                ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn two_char_abbrev_lookup_keeps_initial_single_chars_for_user_phrase_composition() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5c0f}\u{96e8}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("xy");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert!(ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .any(|(phrase, _)| phrase_char_count(phrase) == 1));
    }

    #[test]
    fn long_abbrev_promotes_two_char_prefix_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{4e0d}".into(),
            freq: 9_000,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("abbb");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].0, "\u{963f}\u{4e0d}");
    }

    #[test]
    fn long_abbrev_two_char_intent_moves_three_char_expansion_after_first_page() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{7532}\u{4e59}".into(),
            freq: 9_000,
            code: Some("a bu".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{7532}\u{4e59}\u{4e19}".into(),
            freq: 99_000,
            code: Some("a bu bian".into()),
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        for pass in ["initial", "cache"] {
            let (ranked, err) = engine.lookup_full("abbb");
            assert!(err.is_none(), "unexpected {pass} error: {err:?}");
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase.as_str()),
                Some("\u{7532}\u{4e59}")
            );
            let expansion_pos = ranked
                .iter()
                .position(|(phrase, _)| phrase == "\u{7532}\u{4e59}\u{4e19}")
                .unwrap_or_else(|| {
                    panic!(
                        "{pass} three-char expansion should remain available after page one: {ranked:?}"
                    )
                });
            assert!(
                expansion_pos >= PAGE_SIZE,
                "{pass} three-char expansion mixed into the two-char first page at {expansion_pos}: {:?}",
                ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>()
            );
            assert!(
                ranked
                    .iter()
                    .take(PAGE_SIZE)
                    .all(|(phrase, _)| phrase_char_count(phrase) <= 2),
                "{pass} two-char intent first page contains an overlong candidate: {:?}",
                ranked.iter().take(PAGE_SIZE).collect::<Vec<_>>()
            );
            assert!(
                ranked
                    .iter()
                    .filter(|(phrase, _)| phrase_char_count(phrase) == 3)
                    .count()
                    <= SHORT_INTENT_TOTAL_EXPANSION_LIMIT
            );
        }
    }

    #[test]
    fn repeated_initial_long_abbrev_keeps_two_char_prefix_first() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{963f}".into(),
            freq: 9_000,
            code: Some("a a".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{963f}\u{5b89}".into(),
            freq: 99_000,
            code: Some("a a an".into()),
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("aaab");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{963f}\u{963f}")
        );
        assert!(ranked
            .iter()
            .take(PAGE_SIZE)
            .all(|(phrase, _)| phrase_char_count(phrase) <= 2));
        let expansion_position = ranked
            .iter()
            .position(|(phrase, _)| phrase == "\u{963f}\u{963f}\u{5b89}")
            .expect("three-char expansion should remain on a later page");
        assert!(expansion_position >= PAGE_SIZE, "ranked: {ranked:?}");
    }

    #[test]
    fn exact_four_char_abbrev_promotes_lexicon_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4e00}\u{4e2a}\u{4eba}\u{5417}".into(),
            freq: 111,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("ygrm");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].0, "\u{4e00}\u{4e2a}\u{4eba}\u{5417}");
    }

    #[test]
    fn exact_multi_syllable_input_keeps_multi_phrase_head_then_singles() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [
            ("事实", 9_000),
            ("实施", 8_900),
            ("时事", 8_800),
            ("试试", 8_700),
            ("世事", 8_600),
            ("失实", 8_500),
            ("石室", 8_400),
            ("诗史", 8_300),
            ("师事", 8_200),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("shishi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());

        // 交错插入后：前若干条仍是长度 == 2 的整词 head，但第一页内应尽早出现单字候选，便于拼凑。

        let first_page: Vec<_> = ranked.iter().take(TSF_PAGE_SIZE).collect();
        let head_in_first_page = first_page
            .iter()
            .take_while(|(phrase, _)| phrase_char_count(phrase) > 1)
            .count();
        assert!(
            head_in_first_page <= MULTI_SYLL_PHRASE_HEAD_MIN,
            "expected low-frequency multi-syllable phrases to stop before crowding out singles, got {head_in_first_page}"
        );
        assert!(first_page
            .iter()
            .take(head_in_first_page)
            .all(|(phrase, _)| phrase_char_count(phrase) == 2));
        assert!(
            first_page
                .iter()
                .any(|(phrase, _)| phrase_char_count(phrase) == 1),
            "expected at least one single-char candidate on page 1"
        );
    }

    #[test]
    fn long_request_returns_partial_first_batch_then_full_on_idle_retry() {
        const PAGE_SIZE: usize = 5;
        let _page_size_guard = set_effective_page_size_for_test(PAGE_SIZE);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (idx, phrase) in ["输入", "书入", "述入", "输乳", "舒入", "树入"]
            .into_iter()
            .enumerate()
        {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq: 90_000 - idx as u64,
                code: Some("shu ru".into()),
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();
        let cache_key = final_lookup_cache_key("shuru", engine.mode_flags, engine.cache_epoch);

        let (first, first_err) = engine.lookup_full_detailed_with_request_id("shuru", 10_001);
        assert!(first_err.is_none(), "unexpected first error: {first_err:?}");
        assert!(!first.is_empty());
        assert!(first.len() <= PAGE_SIZE);
        assert!(
            first.iter().any(|item| item.meta.partial),
            "first request should be marked partial: {first:?}"
        );
        assert!(
            first
                .iter()
                .take(PAGE_SIZE)
                .any(|item| phrase_char_count(&item.phrase) == 1),
            "partial five-slot page should retain a single-char candidate: {first:?}"
        );
        assert!(engine.pending_lookup_continuation.is_some());
        assert!(
            engine.final_lookup_cache.get(&cache_key).is_none(),
            "partial first batch must not enter the final lookup cache"
        );

        // Simulate another TSF client changing per-app modes between the
        // partial result and its idle retry. This clears the single
        // continuation slot, but must not clear the budget-free retry ticket.
        let original_mode_flags = engine.mode_flags;
        engine.set_mode_flags(original_mode_flags ^ MODE_FUZZY_PINYIN);
        engine.set_mode_flags(original_mode_flags);
        assert!(engine.pending_lookup_continuation.is_none());
        assert!(engine.has_pending_full_lookup_retry("shuru"));
        let retry_cache_key =
            final_lookup_cache_key("shuru", engine.mode_flags, engine.cache_epoch);

        let (second, second_err) = engine.lookup_full_detailed_with_request_id("shuru", 10_002);
        assert!(
            second_err.is_none(),
            "unexpected retry error: {second_err:?}"
        );
        assert!(!second.is_empty());
        assert!(
            second.iter().all(|item| !item.meta.partial),
            "idle retry for same key should complete full lookup: {second:?}"
        );
        let second_page = second.iter().take(PAGE_SIZE).collect::<Vec<_>>();
        assert!(
            second_page
                .iter()
                .any(|item| phrase_char_count(&item.phrase) == 1),
            "full idle retry should run single-char layout: {second_page:?}"
        );
        assert_eq!(second_page.len(), PAGE_SIZE, "second page: {second_page:?}");
        assert!(
            second_page[..PAGE_SIZE - 1]
                .iter()
                .all(|item| phrase_char_count(&item.phrase) == 2),
            "idle retry should keep four exact two-char words before the reserved single: {second_page:?}"
        );
        assert_eq!(
            phrase_char_count(&second_page[PAGE_SIZE - 1].phrase),
            1,
            "idle retry should reserve the fifth slot for a single: {second_page:?}"
        );
        assert!(engine.pending_lookup_continuation.is_none());
        assert!(!engine.has_pending_full_lookup_retry("shuru"));
        assert!(
            engine.final_lookup_cache.get(&retry_cache_key).is_some(),
            "full idle retry should populate the final lookup cache"
        );

        let (third, third_err) = engine.lookup_full_detailed_with_request_id("shuru", 10_003);
        assert!(
            third_err.is_none(),
            "unexpected cache-hit error: {third_err:?}"
        );
        assert!(
            third.iter().all(|item| !item.meta.partial),
            "cached complete lookup must not regain the partial marker: {third:?}"
        );
        let third_page = third.iter().take(PAGE_SIZE).collect::<Vec<_>>();
        assert_eq!(third_page.len(), PAGE_SIZE, "third page: {third_page:?}");
        assert!(
            third_page
                .iter()
                .all(|item| phrase_char_count(&item.phrase) <= 2),
            "cache rerank reintroduced an overlong two-char-intent candidate: {third_page:?}"
        );
        assert!(
            third_page
                .iter()
                .any(|item| phrase_char_count(&item.phrase) == 1),
            "cache hit should preserve a visible single-char candidate: {third_page:?}"
        );
    }

    #[test]
    fn missing_touchui_phrase_still_decodes_exact_full_pinyin() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full_detailed("touchui");
        assert!(
            err.is_none(),
            "touchui should be a valid tou+chui parse: {err:?}"
        );
        assert!(
            ranked.iter().any(|item| item.phrase == "头锤"),
            "missing whole-word entries must still use exact character decoding: {ranked:?}"
        );
        assert!(ranked.iter().all(|item| !item.meta.partial));
    }

    #[test]
    fn user_learning_invalidation_retains_system_prefix_caches() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "北京".into(),
            freq: 100_000,
            code: Some("bei jing".into()),
        }));

        assert!(!engine.cached_prefix_flat(&lex, "bj").is_empty());
        assert!(!engine.cached_prefix_pinyin_flat(&lex, "bei").is_empty());
        engine.clear_user_sensitive_lookup_caches();
        assert!(engine
            .prefix_cache_abbrev
            .lock()
            .ok()
            .and_then(|mut cache| cache.get("bj"))
            .is_some());
        assert!(engine
            .prefix_cache_pinyin
            .lock()
            .ok()
            .and_then(|mut cache| cache.get("bei"))
            .is_some());

        engine.clear_lookup_caches();
        assert!(engine
            .prefix_cache_abbrev
            .lock()
            .ok()
            .and_then(|mut cache| cache.get("bj"))
            .is_none());
    }

    #[test]
    fn exact_multi_syllable_input_second_page_still_shows_single_char_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        for (phrase, freq) in [
            ("事实", 9_000),
            ("实施", 8_900),
            ("时事", 8_800),
            ("试试", 8_700),
            ("世事", 8_600),
            ("失实", 8_500),
            ("石室", 8_400),
            ("诗史", 8_300),
            ("师事", 8_200),
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: None,
            }));
        }
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("shishi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(
            ranked.len() > TSF_PAGE_SIZE,
            "expected enough candidates to exercise page 2, got {}",
            ranked.len()
        );
        let head_n = ranked
            .iter()
            .take_while(|(phrase, _)| phrase_char_count(phrase) > 1)
            .count();
        assert!((2..=3).contains(&head_n));
        assert!(ranked
            .iter()
            .take(head_n)
            .all(|(phrase, _)| phrase_char_count(phrase) == 2));
        // 第 2 页应仍能看到单字候选，保证翻页后仍可逐字拼词。

        let page2: Vec<_> = ranked
            .iter()
            .skip(TSF_PAGE_SIZE)
            .take(TSF_PAGE_SIZE)
            .collect();
        assert!(!page2.is_empty());
        assert!(page2
            .iter()
            .any(|(phrase, _)| phrase_char_count(phrase) == 1));
    }

    #[test]
    fn exact_two_syllable_full_pinyin_keeps_primary_two_char_words_dense() {
        let lexicon_dir =
            default_phrase_lexicon_dir().expect("test requires repository lexicon directory");
        let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir));
        engine.clear_user_lexicon_for_test();

        for input in ["xianshi", "jiaoshi"] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert!(!ranked.is_empty(), "expected candidates for {input}");

            let two_char_count = ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .filter(|(phrase, _)| phrase_char_count(phrase) == 2)
                .count();
            assert!(
                two_char_count >= 6,
                "expected at least 6 two-char words on page 1 for {input}, got {two_char_count}: {ranked:?}"
            );
        }
    }

    #[test]
    fn exact_multi_syllable_input_limits_single_chars_to_first_syllable() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let (ranked, err) = engine.lookup_full("nihao");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty());

        let first_syllable_chars = engine
            .dict
            .get("ni")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<char>>();
        let later_syllable_chars = engine
            .dict
            .get("hao")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<char>>();
        assert!(
            !later_syllable_chars.is_empty(),
            "dict should contain chars for later syllable"
        );
        let later_only_chars = later_syllable_chars
            .difference(&first_syllable_chars)
            .copied()
            .collect::<HashSet<char>>();
        assert!(
            !later_only_chars.is_empty(),
            "test requires chars that only belong to the later syllable"
        );
        assert!(ranked.iter().any(|(phrase, _)| {
            phrase_char_count(phrase) == 1
                && phrase
                    .chars()
                    .next()
                    .is_some_and(|ch| first_syllable_chars.contains(&ch))
        }));
        assert!(
            ranked.iter().all(|(phrase, _)| {
                phrase_char_count(phrase) != 1
                    || phrase
                        .chars()
                        .next()
                        .map_or(true, |ch| !later_only_chars.contains(&ch))
            }),
            "later-syllable-only single chars should not appear in ranked candidates: {:?}",
            ranked
                .iter()
                .filter(|(phrase, _)| phrase_char_count(phrase) == 1)
                .take(TSF_PAGE_SIZE * 2)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn exact_two_syllable_input_with_one_letter_syllable_prefers_two_char_words() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        for input in ["aba", "aer"] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase_char_count(phrase)),
                Some(2),
                "ranked for {input}: {:?}",
                ranked
                    .iter()
                    .take(TSF_PAGE_SIZE)
                    .map(|(phrase, _)| phrase.as_str())
                    .collect::<Vec<_>>()
            );
            assert_ne!(
                ranked.first().map(|(phrase, _)| phrase_char_count(phrase)),
                Some(3),
                "three-char candidate should not be first for {input}"
            );
        }
    }

    #[test]
    fn exact_two_syllable_full_pinyin_beats_three_char_abbrev_expansion() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{7238}".into(),
            freq: 50_000,
            code: Some("a ba".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{4e0d}\u{5b89}".into(),
            freq: 9_000_000,
            code: Some("a bu an".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("aba");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{963f}\u{7238}"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn final_daily_three_char_preserve_does_not_override_exact_two_syllable_full_pinyin() {
        let _page_guard = candidate_prefs::set_effective_page_size_for_test(5);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{7238}".into(),
            freq: 50_000,
            code: Some("a ba".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{963f}\u{4e0d}\u{5b89}".into(),
            freq: DAILY_SHORT_FREQ_MAX,
            code: Some("a bu an".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("aba");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{963f}\u{7238}"),
            "daily three-char preserve should not override exact full-pinyin intent: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
        let expansion_position = ranked
            .iter()
            .position(|(phrase, _)| phrase == "\u{963f}\u{4e0d}\u{5b89}")
            .unwrap_or_else(|| {
                panic!(
                    "test setup should retain the preserved three-char candidate: {:?}",
                    ranked
                        .iter()
                        .map(|(phrase, _)| phrase.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert!(
            expansion_position >= 5,
            "ordinary three-char prediction should move behind the visible page: {:?}",
            ranked
                .iter()
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn hot_lexicon_short_full_pinyin_prefers_exact_two_char_phrase_over_large_noise() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("lexicon");
        if !dir.is_dir() {
            return;
        }
        let mut engine = PinyinEngine::with_hot_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}"),
            "unexpected top candidate for shuru: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn short_full_pinyin_prefers_exact_two_char_phrase_over_mixed_three_char_noise() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}".into(),
            freq: 900_000,
            code: Some("shu ru".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5c5e}\u{864e}\u{5973}".into(),
            freq: 9_000_000,
            code: Some("shu hu ru".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}"),
            "unexpected top candidate for shuru: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shuru_without_phrase_lexicon_does_not_promote_three_char_composition() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_ne!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{5e02}\u{6237}\u{5165}"),
            "unexpected top candidate for shuru without lexicon: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            ranked
                .first()
                .is_some_and(|(phrase, _)| phrase_char_count(phrase) <= 2),
            "top candidate should stay within the two-syllable full-pinyin intent: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shuru_all_pinyin_modes_prefers_exact_two_char_phrase() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("lexicon");
        if !dir.is_dir() {
            return;
        }
        let mut engine = PinyinEngine::with_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();
        engine.set_mode_flags(
            MODE_FUZZY_PINYIN
                | MODE_V_ASSIST
                | MODE_DATE_AUTO_FORMAT
                | MODE_JIANPIN
                | MODE_MIXED_PINYIN
                | MODE_MIXED_PINYIN_AGGRESSIVE,
        );

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}"),
            "unexpected top candidate for shuru with all pinyin modes: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn observed_three_char_composition_does_not_beat_exact_shuru_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}".into(),
            freq: 900_000,
            code: Some("shu ru".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        for _ in 0..3 {
            engine
                .learn_commit_with_flags(
                    "shuru",
                    "\u{5e02}\u{6237}\u{5165}",
                    LEARN_FLAG_COMPOSED_PHRASE,
                )
                .expect("observe composed phrase");
        }

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}"),
            "observed composed phrase should not beat exact shuru phrase: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pinned_three_char_phrase_does_not_beat_exact_shuru_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8f93}\u{5165}".into(),
            freq: 900_000,
            code: Some("shu ru".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        engine
            .set_candidate_pin("shuru", "\u{5e02}\u{6237}\u{5165}", true)
            .expect("pin overlong shuru candidate");

        let (ranked, err) = engine.lookup_full("shuru");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{8f93}\u{5165}"),
            "pinned overlong phrase should not beat exact shuru phrase: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
        assert!(
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .any(|(phrase, _)| phrase == "\u{5e02}\u{6237}\u{5165}"),
            "pinned phrase should stay available after the exact full-pinyin word: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn exact_two_syllable_full_pinyin_beats_unpinned_long_user_phrase() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4f60}\u{597d}".into(),
            freq: 800_000,
            code: Some("ni hao".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        engine
            .learn_commit("nihao", "\u{4f60}\u{597d}\u{554a}")
            .expect("learn non-pinned long phrase");

        let (ranked, err) = engine.lookup_full("nihao");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4f60}\u{597d}"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn learned_commit_returns_as_top_candidate_for_same_short_key() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .learn_commit("bj", "\u{5317}\u{4eac}")
            .expect("learn user phrase");

        let (ranked, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{5317}\u{4eac}")
        );
    }

    #[test]
    fn single_char_commit_learns_exact_input_without_global_phrase_signal() {
        let _hotword_guard = set_user_hotword_level_for_test(UserHotwordBoostLevel::Strong);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "\u{6d32}";

        engine
            .learn_commit("zhou", phrase)
            .expect("learn single char");

        assert!(engine
            .user_lexicon
            .lookup_input("zhou")
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == phrase)));
        assert_eq!(engine.user_lexicon.phrase_signal(phrase).freq, 0);

        let (ranked, err) = engine.lookup_full("zhou");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(candidate, _)| candidate.as_str()),
            Some(phrase)
        );
    }

    #[test]
    fn exact_two_char_user_hotword_moves_into_front_candidates() {
        let _hotword_guard = set_user_hotword_level_for_test(UserHotwordBoostLevel::Strong);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{52a0}\u{4ee5}".into(),
            freq: 900_000,
            code: Some("jia yi".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5609}\u{4e49}".into(),
            freq: 800_000,
            code: Some("jia yi".into()),
        }));
        engine.phrase_lexicon = Some(lex);
        let phrase = "\u{7532}\u{4e59}";

        for _ in 0..3 {
            engine
                .learn_commit("jia yi", phrase)
                .expect("learn two-char hotword");
        }

        let (ranked, err) = engine.lookup_full("jiayi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let pos = ranked
            .iter()
            .position(|(candidate, _)| candidate == phrase)
            .expect("expected learned two-char hotword");
        assert!(
            pos < 3,
            "learned two-char hotword should be in the first few candidates: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exact_three_char_user_hotword_moves_into_front_candidates() {
        let _hotword_guard = set_user_hotword_level_for_test(UserHotwordBoostLevel::Strong);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{52a0}\u{4ee5}\u{5e76}".into(),
            freq: 900_000,
            code: Some("jia yi bing".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5bb6}\u{4e00}\u{5e76}".into(),
            freq: 800_000,
            code: Some("jia yi bing".into()),
        }));
        engine.phrase_lexicon = Some(lex);
        let phrase = "\u{7532}\u{4e59}\u{4e19}";

        for _ in 0..3 {
            engine
                .learn_commit("jia yi bing", phrase)
                .expect("learn three-char hotword");
        }

        let (ranked, err) = engine.lookup_full("jiayibing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let pos = ranked
            .iter()
            .position(|(candidate, _)| candidate == phrase)
            .expect("expected learned three-char hotword");
        assert!(
            pos < 3,
            "learned three-char hotword should be in the first few candidates: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exact_four_char_user_hotword_moves_into_front_candidates() {
        let _hotword_guard = set_user_hotword_level_for_test(UserHotwordBoostLevel::Strong);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{52a0}\u{4ee5}\u{7ed1}\u{5b9a}".into(),
            freq: 900_000,
            code: Some("jia yi bang ding".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{8bb0}\u{5fc6}\u{5b9d}\u{5178}".into(),
            freq: 800_000,
            code: Some("ji yi bao dian".into()),
        }));
        engine.phrase_lexicon = Some(lex);
        let phrase = "\u{7532}\u{4e59}\u{4e19}\u{4e01}";

        for _ in 0..3 {
            engine
                .learn_commit("jia yi bing ding", phrase)
                .expect("learn four-char hotword");
        }

        let (ranked, err) = engine.lookup_full("jybd");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let pos = ranked
            .iter()
            .position(|(candidate, _)| candidate == phrase)
            .expect("expected learned four-char hotword");
        assert!(
            pos < 3,
            "learned four-char hotword should be in the first few candidates: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
    }

    #[test]
    fn overlong_three_char_user_hotword_does_not_beat_exact_two_syllable_word() {
        let _hotword_guard = set_user_hotword_level_for_test(UserHotwordBoostLevel::Aggressive);
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{4f60}\u{597d}".into(),
            freq: 800_000,
            code: Some("ni hao".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        for _ in 0..5 {
            engine
                .learn_commit("nihao", "\u{4f60}\u{597d}\u{554a}")
                .expect("learn overlong hotword");
        }

        let (ranked, err) = engine.lookup_full("nihao");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{4f60}\u{597d}"),
            "overlong user phrase should not beat exact two-syllable word: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
    }

    #[test]
    fn short_abbrev_learning_does_not_backfill_full_pinyin_key() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .learn_commit("bj", "\u{5317}\u{4eac}")
            .expect("learn short abbrev");

        assert!(engine.user_lexicon.lookup_input("bj").is_some());
        assert!(engine.user_lexicon.lookup_input("beijing").is_none());
        assert!(engine.user_lexicon.lookup_mixed_input("beijing").is_none());
    }

    #[test]
    fn first_mixed_pinyin_learning_promotes_user_hotword() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let phrase = "\u{91cd}\u{5e86}";

        engine
            .learn_commit("cqing", phrase)
            .expect("first mixed learn");
        assert!(engine.user_lexicon.lookup_input("cqing").is_none());
        assert!(engine.user_lexicon.lookup_mixed_input("cqing").is_some());
        let (first_ranked, first_err) = engine.lookup_full_detailed("cqing");
        assert!(first_err.is_none(), "unexpected error: {first_err:?}");
        let first_pos = first_ranked
            .iter()
            .position(|item| {
                item.phrase == phrase && item.meta.display_text() == Some(MIXED_USER_CANDIDATE_META)
            })
            .expect("first mixed-pinyin selection should be recalled as a user hotword");
        assert!(
            first_pos < 3,
            "first mixed-pinyin user hotword should be near the front: {:?}",
            first_ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );

        engine
            .learn_commit("cqing", phrase)
            .expect("second mixed learn");
        let (second_ranked, second_err) = engine.lookup_full_detailed("cqing");
        assert!(second_err.is_none(), "unexpected error: {second_err:?}");
        assert!(second_ranked.iter().any(|item| {
            item.phrase == phrase && item.meta.display_text() == Some(MIXED_USER_CANDIDATE_META)
        }));
    }

    #[test]
    fn top_candidate_survives_backspace_and_retype() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let (first, err) = engine.lookup_full("shurufa");
        assert!(err.is_none(), "unexpected error: {err:?}");
        let expected = first
            .first()
            .map(|(phrase, _)| phrase.clone())
            .expect("expected initial candidate");
        let _ = engine.lookup_full("shuruf");
        let (again, err) = engine.lookup_full("shurufa");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(again.first().map(|(phrase, _)| phrase), Some(&expected));
    }

    #[test]
    fn stability_bonus_applies_to_previous_top_five_candidates() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.last_lookup = Some(LookupStabilityState {
            key: "shur".to_string(),
            top_phrases: vec![
                "甲".into(),
                "乙".into(),
                "输入".into(),
                "丁".into(),
                "戊".into(),
            ],
        });

        let third_bonus = engine.stability_bonus_for_candidate("shuru", "输入");
        let first_bonus = engine.stability_bonus_for_candidate("shuru", "甲");

        assert!(third_bonus > 0.0);
        assert!(first_bonus > third_bonus);
        assert_eq!(
            engine.stability_bonus_for_candidate("shuru", "不在前五"),
            0.0
        );
    }

    #[test]
    fn confident_candidate_can_beat_sticky_previous_top1() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.last_lookup = Some(LookupStabilityState {
            key: "shu".to_string(),
            top_phrases: vec!["旧候选".into()],
        });
        let stability = engine.stability_bonus_for_candidate("shur", "旧候选");
        assert!(stability > 0.0);

        let mut ranked = vec![
            ranked_candidate("旧候选", 100.0 + stability),
            ranked_candidate("新候选", 111.0),
        ];
        assert!(ranked[0].score > ranked[1].score);

        assert!(engine.promote_confident_top1_over_stability("shur", &mut ranked));
        assert_eq!(
            ranked.first().map(|item| item.phrase.as_str()),
            Some("新候选")
        );
    }

    #[test]
    fn cache_epoch_changes_when_mode_or_user_data_changes() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let initial = engine.cache_epoch;

        engine.set_mode_flags(engine.mode_flags ^ MODE_FUZZY_PINYIN);
        assert!(engine.cache_epoch > initial);
        let after_mode = engine.cache_epoch;

        engine.learn_commit("ceshi", "测试").unwrap();
        assert!(engine.cache_epoch > after_mode);
    }

    #[test]
    fn runtime_config_generation_change_invalidates_lookup_caches() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let old_generation = engine.runtime_config_generation;
        let old_epoch = engine.cache_epoch;
        let key = final_lookup_cache_key("nihao", engine.mode_flags, old_epoch);
        engine.final_lookup_cache.insert(
            key.clone(),
            vec![ranked_candidate("旧配置候选", 100.0)],
            None,
        );
        engine.last_lookup = Some(LookupStabilityState {
            key: "nihao".to_string(),
            top_phrases: vec!["旧配置候选".to_string()],
        });

        assert!(engine.refresh_runtime_config_generation_to(old_generation.wrapping_add(1)));
        assert!(engine.cache_epoch > old_epoch);
        assert!(engine.final_lookup_cache.get(&key).is_none());
        assert!(engine.last_lookup.is_none());
        assert!(!engine.refresh_runtime_config_generation_to(old_generation.wrapping_add(1)));
    }

    #[test]
    fn reset_learning_context_clears_lookup_dependent_caches() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let initial = engine.cache_epoch;

        engine.reset_learning_context();

        assert!(engine.cache_epoch > initial);
        assert!(engine.last_lookup.is_none());
    }

    #[test]
    fn phrase_reload_failure_keeps_existing_lexicon_and_epoch() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "保留".into(),
            freq: 100,
            code: Some("baoliu".into()),
        }));
        engine.install_phrase_lexicon(Some(lex));
        let epoch = engine.cache_epoch;
        let missing =
            std::env::temp_dir().join(format!("kaixin-missing-lexicon-{}", std::process::id()));

        assert!(engine.reload_phrase_dir(Some(&missing)).is_err());
        assert!(engine.has_phrase_lexicon());
        assert_eq!(engine.cache_epoch, epoch);
    }

    #[test]
    fn empty_lookup_result_falls_back_to_raw_input_candidate() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.install_phrase_lexicon(None);
        engine.clear_user_lexicon_for_test();

        let input = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let (ranked, err) = engine.lookup_full_detailed(input);

        assert!(err.is_some());
        assert_eq!(ranked.first().map(|item| item.phrase.as_str()), Some(input));
        assert!(ranked
            .first()
            .and_then(|item| item.meta.display_text())
            .is_some_and(|meta| meta.contains("no_learn=1")));
    }

    #[test]
    fn pinned_user_phrase_survives_backspace_and_retype() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{5317}\u{4eac}".into(),
            freq: HIGH_PRIORITY_TWO_CHAR_FREQ_MIN,
            code: None,
        }));
        engine.phrase_lexicon = Some(lex);
        engine.clear_user_lexicon_for_test();
        engine
            .set_candidate_pin("bj", "\u{6807}\u{8bb0}", true)
            .expect("pin bj competitor");

        let (first, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            first.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{6807}\u{8bb0}")
        );
        let _ = engine.lookup_full("b");
        let (again, err) = engine.lookup_full("bj");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            again.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{6807}\u{8bb0}")
        );
    }

    #[test]
    fn common_typo_keeps_expected_phrase_in_top_three() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .learn_commit("beijing", "\u{5317}\u{4eac}")
            .expect("learn user phrase");

        let (ranked, err) = engine.lookup_full_detailed("bejing");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(ranked
            .iter()
            .take(3)
            .any(|candidate| candidate.phrase == "\u{5317}\u{4eac}"));
    }

    #[test]
    fn common_missing_letter_mingtian_keeps_expected_phrase_in_top_three() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        engine
            .learn_commit("mingtian", "\u{660e}\u{5929}")
            .expect("learn user phrase");

        let (ranked, err) = engine.lookup_full_detailed("migtian");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(ranked
            .iter()
            .take(3)
            .any(|candidate| candidate.phrase == "\u{660e}\u{5929}"));
    }

    #[test]
    fn lve_phrase_code_surfaces_jianlve() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{7b80}\u{7565}".into(),
            freq: 17_915,
            code: Some("jian lve".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("jianlve");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .any(|(candidate, _)| candidate == "\u{7b80}\u{7565}"));
    }

    #[test]
    fn nue_lue_aliases_surface_umlaut_entries() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();
        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{66b4}\u{8650}".into(),
            freq: 10_000,
            code: Some("bao nve".into()),
        }));
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{7b80}\u{7565}".into(),
            freq: 17_915,
            code: Some("jian lve".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        for (input, expected) in [
            ("nue", "\u{8650}"),
            ("lue", "\u{7565}"),
            ("baonue", "\u{66b4}\u{8650}"),
            ("jianlue", "\u{7b80}\u{7565}"),
        ] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "{input} unexpected error: {err:?}");
            assert!(
                ranked
                    .iter()
                    .take(TSF_PAGE_SIZE)
                    .any(|(candidate, _)| candidate == expected),
                "{input} did not surface {expected}; got {:?}",
                ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
            );
        }
    }

    /// 全拼两字词不应被三字词置顶：当输入可以被切分为 2 音节和 3 音节两种完整变体时，
    /// 2 字词（exact pinyin match）应在候选栏首位，三字组合词不应排在前面。
    #[test]
    fn full_pinyin_two_char_exact_beats_three_char_from_alternate_split() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        // "xianshi" → Exact: ["xian","shi"] (2音节) + Alternate: ["xi","an","shi"] (3音节)
        // 高频双字词 "显示" 应在首位，即使三音节切分能产生三字组合候选。
        let mut lex = AbbrevLexicon::default();
        // 2 字词：显示 (xian+shi)
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{663e}\u{793a}".into(), // 显示
            freq: 800_000,
            code: Some("xian shi".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("xianshi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty(), "should have candidates for xianshi");

        let top_phrase = ranked.first().map(|(phrase, _)| phrase.as_str());
        let top_char_count = ranked.first().map(|(phrase, _)| phrase_char_count(phrase));

        assert_eq!(
            top_char_count,
            Some(2),
            "top candidate should be 2-char for full pinyin 'xianshi', got {top_phrase:?} with {top_char_count:?} chars.\nranked: {:?}",
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );
        assert_eq!(
            top_phrase,
            Some("\u{663e}\u{793a}"),
            "expected 显示 as top for 'xianshi', got {top_phrase:?}"
        );

        // 确认前三里没有三字词排在双字词前面
        let first_three_char_pos = ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .position(|(phrase, _)| phrase_char_count(phrase) >= 3);
        let first_two_char_pos = ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .position(|(phrase, _)| phrase_char_count(phrase) == 2);
        if let (Some(three_pos), Some(two_pos)) = (first_three_char_pos, first_two_char_pos) {
            assert!(
                two_pos < three_pos,
                "2-char word should appear before 3-char word for 'xianshi', but 2-char at pos {two_pos} vs 3-char at pos {three_pos}"
            );
        }
    }

    #[test]
    fn exact_2_3_4_char_phrase_accuracy_regression_samples() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let mut lex = AbbrevLexicon::default();
        for (phrase, freq, code) in [
            ("\u{663e}\u{793a}", 800_000, "xian shi"),          // 显示
            ("\u{8f93}\u{5165}\u{6cd5}", 780_000, "shu ru fa"), // 输入法
            (
                "\u{5f00}\u{5fc3}\u{8f93}\u{5165}",
                760_000,
                "kai xin shu ru",
            ), // 开心输入
        ] {
            assert!(lex.insert_parsed(ThuoclEntry {
                phrase: phrase.into(),
                freq,
                code: Some(code.into()),
            }));
        }
        engine.phrase_lexicon = Some(lex);

        for (input, expected, expected_len) in [
            ("xianshi", "\u{663e}\u{793a}", 2),
            ("shurufa", "\u{8f93}\u{5165}\u{6cd5}", 3),
            ("kaixinshuru", "\u{5f00}\u{5fc3}\u{8f93}\u{5165}", 4),
        ] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "{input} unexpected error: {err:?}");
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase.as_str()),
                Some(expected),
                "{input} top candidate regression failed: {:?}",
                ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
            );
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase_char_count(phrase)),
                Some(expected_len),
                "{input} should keep exact {expected_len}-char phrase intent"
            );
        }
    }

    #[test]
    fn exact_two_syllable_split_is_preferred_over_high_freq_three_char_alternate() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{897f}\u{5b89}\u{5e02}".into(), // 西安市: xi + an + shi
            freq: 9_000_000,
            code: Some("xi an shi".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let variants = parse_input_variants_detailed(
            "xianshi",
            engine.syllables.as_ref(),
            engine.parse_options(),
        )
        .unwrap();
        let preferred = engine
            .preferred_exact_complete_multi_variant(&variants)
            .expect("expected preferred variant");
        assert_eq!(preferred.source, ParseVariantSource::Exact);
        assert_eq!(
            preferred.syllables,
            vec!["xian".to_string(), "shi".to_string()]
        );
    }

    #[test]
    fn exact_three_syllable_split_is_preferred_over_high_freq_four_char_alternate() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        let mut lex = AbbrevLexicon::default();
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{897f}\u{5b89}\u{5e02}\u{5b89}".into(), // 西安市安: xi + an + shi + an
            freq: 9_000_000,
            code: Some("xi an shi an".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let variants = parse_input_variants_detailed(
            "xianshian",
            engine.syllables.as_ref(),
            engine.parse_options(),
        )
        .unwrap();
        let preferred = engine
            .preferred_exact_complete_multi_variant(&variants)
            .expect("expected preferred variant");
        assert_eq!(preferred.source, ParseVariantSource::Exact);
        assert_eq!(
            preferred.syllables,
            vec!["xian".to_string(), "shi".to_string(), "an".to_string()]
        );
    }

    #[test]
    fn real_lexicon_exact_three_syllable_beats_four_char_alternate() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("lexicon");
        if !dir.is_dir() {
            return;
        }
        let mut engine = PinyinEngine::with_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();

        for input in ["gongzuoyuan", "xianshichang", "xinlianxiang"] {
            let (ranked, err) = engine.lookup_full(input);
            assert!(err.is_none(), "unexpected error for {input}: {err:?}");
            assert_eq!(
                ranked.first().map(|(phrase, _)| phrase_char_count(phrase)),
                Some(3),
                "{input} should keep 3-syllable intent before 4-char alternate: {:?}",
                ranked
                    .iter()
                    .take(TSF_PAGE_SIZE)
                    .map(|(phrase, _)| phrase.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn standard_large_short_phrase_fills_missing_three_char_runtime_candidate() {
        let dir = std::env::temp_dir().join("pinyin_ime_standard_large_short_engine_test");
        let ext_dir = dir.join("ext");
        let large_dir = dir.join("large");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::create_dir_all(&large_dir).unwrap();
        std::fs::write(
            ext_dir.join("ext.txt"),
            "\u{83ab}\u{91cc}\u{5c3c}\u{5965}\tmo li ni ao\t100\n",
        )
        .unwrap();
        std::fs::write(
            large_dir.join("tencent.txt"),
            "\u{9b54}\u{529b}\u{9e1f}\t100\n",
        )
        .unwrap();

        let mut engine = PinyinEngine::with_phrase_dir(Some(&dir));
        engine.clear_user_lexicon_for_test();
        let (ranked, err) = engine.lookup_full("moliniao");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert_eq!(
            ranked.first().map(|(phrase, _)| phrase.as_str()),
            Some("\u{9b54}\u{529b}\u{9e1f}"),
            "ranked: {:?}",
            ranked
                .iter()
                .take(TSF_PAGE_SIZE)
                .map(|(phrase, _)| phrase.as_str())
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 类似场景但双字词词频较低，应仍然不会被三字词置顶。
    #[test]
    fn full_pinyin_low_freq_two_char_still_beats_three_char_composed() {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_test();

        // "jiaoshi" → Exact: ["jiao","shi"] (2音节) + Alternate: ["ji","ao","shi"] (3音节)
        let mut lex = AbbrevLexicon::default();
        // 低频双字词
        assert!(lex.insert_parsed(ThuoclEntry {
            phrase: "\u{6559}\u{5e08}".into(), // 教师
            freq: 50_000,
            code: Some("jiao shi".into()),
        }));
        engine.phrase_lexicon = Some(lex);

        let (ranked, err) = engine.lookup_full("jiaoshi");
        assert!(err.is_none(), "unexpected error: {err:?}");
        assert!(!ranked.is_empty(), "should have candidates for jiaoshi");

        let top_char_count = ranked.first().map(|(phrase, _)| phrase_char_count(phrase));
        assert_eq!(
            top_char_count,
            Some(2),
            "top candidate should be 2-char for 'jiaoshi', got {:?} chars.\nranked: {:?}",
            top_char_count,
            ranked.iter().take(TSF_PAGE_SIZE).collect::<Vec<_>>()
        );

        // 三字词排在双字词后面的位置
        let first_two = ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .position(|(phrase, _)| phrase_char_count(phrase) == 2);
        let first_three = ranked
            .iter()
            .take(TSF_PAGE_SIZE)
            .position(|(phrase, _)| phrase_char_count(phrase) == 3);
        assert!(
            first_two.is_some(),
            "should have at least one 2-char candidate"
        );
        if let (Some(two), Some(three)) = (first_two, first_three) {
            assert!(
                two < three,
                "2-char should appear before 3-char for 'jiaoshi'"
            );
        }
    }
}
