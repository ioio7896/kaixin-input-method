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
    LexiconLayer, ThuoclEntry, MAX_LEXICON_FREQ,
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
mod single_char_common;

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
/// 完整候选加载的上限。精确单音节输入（si/shi/yi 等）的候选全部是单字，
/// 允许用户翻页直达冷门单字，不能再按高频词场景截断在 128 条。
/// 必须与 TSF 侧 kRustRows（响应行数上限）保持一致，否则完整加载结果
/// 会在某一环被砍回 128。
pub const LOOKUP_FULL_MAX_CANDIDATES: usize = 256;

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
const USER_LOW_LEXICON_FREQ_THRESHOLD: u64 = MAX_LEXICON_FREQ * 80 / 100;
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
const WORD_TRANSITION_RECENCY_SCALE: f64 = 3.0;
const WORD_TRANSITION_CAP: f64 = 18.0;
const WORD_NGRAM_BIGRAM_BACKOFF_STRENGTH: f64 = 4.0;
const WORD_NGRAM_TRIGRAM_BACKOFF_STRENGTH: f64 = 3.0;
const WORD_NGRAM_LOG_LIFT_SCALE: f64 = 3.2;
const WORD_NGRAM_RECENCY_SCALE: f64 = 0.55;
const WORD_GRAPH_NGRAM_SCALE: f64 = 0.55;
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
// zh-ext/large 词库允许在完整全拼中召回长尾词；简拼和混拼则只让
// 低于这个归一化词频阈值的长尾词落到高频候选之后，避免动物、地名等
// 扩展词在短输入下抢占首屏。
const NON_FULL_COLD_LEXICON_FREQ_THRESHOLD: u64 = MAX_LEXICON_FREQ * 60 / 100;
const NON_FULL_COLD_LEXICON_PENALTY_BASE: f64 = 48.0;
const NON_FULL_COLD_LEXICON_PENALTY_SCALE: f64 = 60.0;
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
/// Three-letter exact abbreviations are a bounded, explicit phrase intent.
/// Recall a wider bucket before reranking so high-collision keys such as
/// `dsj` do not permanently lose candidates behind the generic hot head.
const THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT: usize = 48;
const THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT_HIGH_COLLISION: usize = 64;
/// Exact mixed keys are curated and collision-bounded. Keep their hottest
/// candidates ahead of later generic two-character prefix promotions.
const MIXED_HOT_DIRECT_FRONT_LIMIT: usize = 7;
const SHORT_COMPACT_EXACT_BOOST: f64 = 56.0;
const SHORT_ABBREV_EXACT_LENGTH_RERANK_BONUS: f64 = 42.0;
const SHORT_ABBREV_TOP1_FREQ_BONUS_SCALE: f64 = 5.5;
const SHORT_ABBREV_TOP1_FREQ_BONUS_OFFSET: f64 = -42.0;
const SHORT_ABBREV_TOP1_FREQ_BONUS_CAP: f64 = 26.0;
const SHORT_ASCII_MULTI_CHAR_RERANK_DISCOUNT: f64 = 8.0;
const EXPLICIT_SEPARATOR_PHRASE_BONUS: f64 = 72.0;
const SHORT_COMPACT_BACKOFF_STEP_PENALTY: f64 = 26.0;
/// Metadata from a higher-priority source may remain attached while merging
/// the same phrase, but the candidate score is always raised to the maximum.
const META_STICKY_SCORE_MARGIN: f64 = 96.0;
const HIGH_PRIORITY_TWO_CHAR_FREQ_MIN: u64 = MAX_LEXICON_FREQ * 98 / 100;
const DAILY_SHORT_FREQ_MAX: u64 = MAX_LEXICON_FREQ;
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
// This is the legacy learned-user-stat counter, not curated lexicon frequency.
const USER_PINNED_SHORT_ABBREV_FREQ_MIN: u64 = 100_000;
const SHORT_FALLBACK_FREQ_RELIEF_CAP: f64 = 16.0;
const SHORT_ABBREV_SINGLE_CONTEXT_SCALE: f64 = 0.45;
/// 多音节整词头部上限（实际条数还受分数断崖与词频门槛约束，见 `apply_multi_syllable_phrase_head_then_first_syllable_singles`）。

const MULTI_SYLL_PHRASE_HEAD_MAX: usize = 3;
/// 分数断崖：相邻两条整词候选分差超过该值且已保留最少条数时，停止继续收整词。

const MULTI_SYLL_PHRASE_SCORE_GAP: f64 = 22.0;
/// 断崖截断前至少保留的整词条数（在有多条候选时生效）。

const MULTI_SYLL_PHRASE_HEAD_MIN: usize = 2;
const MULTI_SYLL_PHRASE_HEAD_FREQ_MIN: u64 = MAX_LEXICON_FREQ * 60 / 100;
const FULL_PINYIN_PREFIX_SUPPORT_LIMIT: usize = 3;
const SHORT_INPUT_EXACT_FRONT_FREQ_MIN: u64 = MAX_LEXICON_FREQ * 50 / 100;
const SHORT_INPUT_EXACT_FRONT_LIMIT_TWO: usize = 4;
const SHORT_INPUT_EXACT_FRONT_LIMIT_THREE: usize = 3;
const SHORT_INPUT_EXACT_FRONT_LIMIT_FOUR: usize = 3;
const SHORT_INPUT_LOW_FREQ_EXACT_FRONT_LIMIT: usize = 1;
const SHORT_PHRASE_SINGLE_RERANK_EXTRA: f64 = 20.0;
const SHORT_PHRASE_TWO_CHAR_RERANK_BONUS: f64 = 22.0;
// Do not give generic two-character candidates a head start over equally
// strong three-character candidates. Exact-intent bonuses remain separate.
const SHORT_PHRASE_THREE_CHAR_RERANK_BONUS: f64 = 22.0;
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
    } else if syllable_count >= 6 {
        // Long readings are usually settled by the word graph and exact
        // lexicon channels. Keeping a small character tail avoids materializing
        // dozens of low-probability strings before reranking.
        24
    } else if syllable_count >= 4 {
        32
    } else {
        48
    }
}

pub struct PinyinEngine {
    dict: Arc<HashMap<String, Vec<char>>>,
    syllables: Arc<HashSet<String>>,
    lm: Arc<Lm>,
    single_char_common: Arc<single_char_common::SingleCharCommonIndex>,
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
    /// 同词的系统候选在并入用户学习信号时保留该标记：即使系统分数高出
    /// META_STICKY_SCORE_MARGIN，hotword 前置逻辑仍应识别它为用户热词。
    pub user_signal: bool,
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
        let source = if lower.contains("english") || text.contains("英文") {
            CandidateSource::English
        } else if lower.contains("no_learn=1") {
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
            user_signal: tokens.contains(&"user_signal=1"),
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

    pub fn with_user_signal(mut self) -> Self {
        self.user_signal = true;
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
