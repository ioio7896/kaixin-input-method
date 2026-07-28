use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct EngineTuning {
    pub(super) prefix_cache_capacity: usize,
    pub(super) final_lookup_cache_capacity: usize,
    pub(super) short_lookup_cache_capacity: usize,
    pub(super) long_lookup_soft_budget: Duration,
    pub(super) long_lookup_min_first_batch_candidates: usize,
}

impl Default for EngineTuning {
    fn default() -> Self {
        Self {
            prefix_cache_capacity: DEFAULT_PREFIX_CACHE_CAPACITY,
            final_lookup_cache_capacity: DEFAULT_FINAL_LOOKUP_CACHE_CAPACITY,
            short_lookup_cache_capacity: DEFAULT_SHORT_LOOKUP_CACHE_CAPACITY,
            long_lookup_soft_budget: Duration::from_millis(DEFAULT_LONG_LOOKUP_SOFT_BUDGET_MS),
            long_lookup_min_first_batch_candidates: DEFAULT_LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES,
        }
    }
}

pub(super) static LATEST_LOOKUP_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
static LOOKUP_PROFILE_FAST_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cancellation domain owned by one lookup session. IPC clients use this
/// instead of the process-global request id so activity in one application
/// cannot cancel a still-current lookup in another application.
#[derive(Clone, Debug)]
pub(crate) struct LookupCancellationToken {
    latest_generation: Arc<AtomicU64>,
}

impl Default for LookupCancellationToken {
    fn default() -> Self {
        Self {
            latest_generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl LookupCancellationToken {
    pub(crate) fn next_generation(&self) -> u64 {
        self.latest_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1)
    }

    pub(crate) fn publish(&self, generation: u64) {
        if generation == 0 {
            return;
        }
        let _ =
            self.latest_generation
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (generation > current).then_some(generation)
                });
    }

    #[inline]
    pub(crate) fn is_superseded(&self, generation: u64) -> bool {
        generation != 0 && self.latest_generation.load(Ordering::Acquire) > generation
    }
}

thread_local! {
    static LOOKUP_CANCELLATION_SCOPE: RefCell<Option<(LookupCancellationToken, u64)>> =
        const { RefCell::new(None) };
}

pub(crate) struct LookupCancellationScope {
    previous: Option<(LookupCancellationToken, u64)>,
}

impl LookupCancellationScope {
    pub(crate) fn enter(token: LookupCancellationToken, generation: u64) -> Self {
        token.publish(generation);
        let previous =
            LOOKUP_CANCELLATION_SCOPE.with(|scope| scope.borrow_mut().replace((token, generation)));
        Self { previous }
    }
}

impl Drop for LookupCancellationScope {
    fn drop(&mut self) {
        LOOKUP_CANCELLATION_SCOPE.with(|scope| {
            *scope.borrow_mut() = self.previous.take();
        });
    }
}

const LOOKUP_PROFILE_SLOW_THRESHOLD_US: u128 = 20_000;
const LOOKUP_PROFILE_FAST_SAMPLE_MASK: u64 = 0x3f;

pub(super) fn engine_tuning() -> EngineTuning {
    crate::runtime_config::snapshot().engine_tuning
}

pub(crate) fn parse_engine_tuning_ini(text: &str) -> EngineTuning {
    let mut tuning = EngineTuning::default();
    let mut in_engine_section = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_engine_section = section.trim().eq_ignore_ascii_case("engine");
            continue;
        }
        if !in_engine_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .split([';', '#'])
            .next()
            .unwrap_or("")
            .trim()
            .parse::<usize>();
        let Ok(value) = value else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "prefix_cache_capacity" => tuning.prefix_cache_capacity = value.clamp(8, 512),
            "final_lookup_cache_capacity" => {
                tuning.final_lookup_cache_capacity = value.clamp(8, 512)
            }
            "short_lookup_cache_capacity" => {
                tuning.short_lookup_cache_capacity = value.clamp(8, 512)
            }
            "long_lookup_soft_budget_ms" => {
                tuning.long_lookup_soft_budget = Duration::from_millis(value.clamp(1, 50) as u64)
            }
            "long_lookup_min_first_batch_candidates" => {
                tuning.long_lookup_min_first_batch_candidates = value.clamp(1, TSF_MAX_CANDIDATES)
            }
            _ => {}
        }
    }
    tuning
}

pub(super) fn nonzero_capacity(capacity: usize) -> NonZeroUsize {
    NonZeroUsize::new(capacity.max(1)).expect("LRU capacity is clamped above zero")
}

pub(crate) fn publish_latest_lookup_request_id(request_id: u64) {
    if request_id == 0 {
        return;
    }
    let published_to_session = LOOKUP_CANCELLATION_SCOPE.with(|scope| {
        let scope = scope.borrow();
        if let Some((token, _)) = scope.as_ref() {
            token.publish(request_id);
            true
        } else {
            false
        }
    });
    if published_to_session {
        return;
    }
    let _ = LATEST_LOOKUP_REQUEST_ID.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        (request_id > current).then_some(request_id)
    });
}

/// Allocate a process-local generation for IPC lookup work.
///
/// Wire request ids are useful for diagnostics, but older clients and
/// internal background lookups may send zero. A server-side generation makes
/// every IPC lookup cancellable and avoids comparing ids from different
/// client processes.
pub(crate) fn next_lookup_request_generation() -> u64 {
    let mut current = LATEST_LOOKUP_REQUEST_ID.load(Ordering::Acquire);
    loop {
        let next = current.wrapping_add(1).max(1);
        match LATEST_LOOKUP_REQUEST_ID.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

#[inline]
pub(crate) fn lookup_request_superseded(request_id: u64) -> bool {
    if request_id == 0 {
        return false;
    }
    LOOKUP_CANCELLATION_SCOPE.with(|scope| {
        scope.borrow().as_ref().map_or_else(
            || LATEST_LOOKUP_REQUEST_ID.load(Ordering::Acquire) > request_id,
            |(token, generation)| *generation != request_id || token.is_superseded(request_id),
        )
    })
}

#[inline]
pub(crate) fn current_lookup_request_superseded() -> bool {
    LOOKUP_CANCELLATION_SCOPE.with(|scope| {
        scope
            .borrow()
            .as_ref()
            .is_some_and(|(token, generation)| token.is_superseded(*generation))
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct UserLexiconDiskStamp {
    pub(super) dict_mtime: Option<SystemTime>,
    pub(super) dict_len: Option<u64>,
    pub(super) reset_mtime: Option<SystemTime>,
    pub(super) reset_len: Option<u64>,
}

/// LRU cache keyed by abbreviation or pinyin prefix, yielding pre-ranked
/// phrase lists.  Backed by the `lru` crate for O(1) access and eviction.
pub(super) struct PrefixLruCache {
    inner: LruCache<String, Arc<[ThuoclEntry]>>,
}

impl Default for PrefixLruCache {
    fn default() -> Self {
        // Capacity set by `engine_tuning()` at construction time; provide a
        // reasonable floor so the default-constructed cache is usable.
        Self {
            inner: LruCache::new(nonzero_capacity(DEFAULT_PREFIX_CACHE_CAPACITY)),
        }
    }
}

#[derive(Default)]
pub(super) struct MixedExpansionLruCache {
    order: VecDeque<String>,
    map: HashMap<String, Arc<[MixedPinyinKeyExpansion]>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct MixedCompletionScoreCacheKey {
    full_key: String,
    compositional_suffix: bool,
    user_clock: u64,
    cache_epoch: u64,
}

pub(super) struct MixedCompletionScoreCache {
    map: LruCache<MixedCompletionScoreCacheKey, f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FinalLookupCacheKey {
    input: String,
    mode_flags: u32,
    cache_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ShortLookupCacheKey {
    input: String,
    mode_flags: u32,
    user_clock: u64,
    cache_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum RerankScoreKind {
    PhrasePath,
    ContextBonus,
    WordTransition,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct RerankScoreCacheKey {
    kind: RerankScoreKind,
    phrase: String,
    user_clock: u64,
    cache_epoch: u64,
    context: Vec<String>,
}

pub(super) struct FinalLookupCache {
    map: LruCache<FinalLookupCacheKey, CachedLookupResult>,
}

pub(super) struct ShortLookupCache {
    map: LruCache<ShortLookupCacheKey, CachedLookupResult>,
}

#[derive(Clone)]
struct CachedLookupResult {
    candidates: Arc<[RankedCandidate]>,
    visible_short_intent: Option<usize>,
}

pub(super) struct RerankScoreCache {
    map: LruCache<RerankScoreCacheKey, f64>,
}

impl MixedCompletionScoreCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            map: LruCache::new(nonzero_capacity(capacity)),
        }
    }

    pub(super) fn clear(&mut self) {
        self.map.clear();
    }

    pub(super) fn get(&mut self, key: &MixedCompletionScoreCacheKey) -> Option<f64> {
        self.map.get(key).copied()
    }

    pub(super) fn insert(&mut self, key: MixedCompletionScoreCacheKey, score: f64) {
        if score.is_finite() {
            self.map.put(key, score);
        }
    }
}

impl Default for MixedCompletionScoreCache {
    fn default() -> Self {
        Self::new(MIXED_COMPLETION_SCORE_CACHE_CAPACITY)
    }
}

#[derive(Clone, Debug)]
pub(super) struct LookupStabilityState {
    pub(super) key: String,
    pub(super) top_phrases: Vec<String>,
}

impl FinalLookupCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            map: LruCache::new(nonzero_capacity(capacity)),
        }
    }

    pub(super) fn clear(&mut self) {
        self.map.clear();
    }

    pub(super) fn get(
        &mut self,
        key: &FinalLookupCacheKey,
    ) -> Option<(Vec<RankedCandidate>, Option<usize>)> {
        let cached = self.map.get(key)?.clone();
        Some((
            cached.candidates.as_ref().to_vec(),
            cached.visible_short_intent,
        ))
    }

    pub(super) fn insert(
        &mut self,
        key: FinalLookupCacheKey,
        candidates: Vec<RankedCandidate>,
        visible_short_intent: Option<usize>,
    ) {
        if candidates.is_empty() {
            return;
        }
        self.map.put(
            key,
            CachedLookupResult {
                candidates: Arc::from(candidates),
                visible_short_intent,
            },
        );
    }
}

impl ShortLookupCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            map: LruCache::new(nonzero_capacity(capacity)),
        }
    }

    pub(super) fn clear(&mut self) {
        self.map.clear();
    }

    pub(super) fn get(
        &mut self,
        key: &ShortLookupCacheKey,
    ) -> Option<(Vec<RankedCandidate>, Option<usize>)> {
        let cached = self.map.get(key)?.clone();
        Some((
            cached.candidates.as_ref().to_vec(),
            cached.visible_short_intent,
        ))
    }

    pub(super) fn insert(
        &mut self,
        key: ShortLookupCacheKey,
        candidates: Vec<RankedCandidate>,
        visible_short_intent: Option<usize>,
    ) {
        if candidates.is_empty() {
            return;
        }
        self.map.put(
            key,
            CachedLookupResult {
                candidates: Arc::from(candidates),
                visible_short_intent,
            },
        );
    }
}

impl RerankScoreCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            map: LruCache::new(nonzero_capacity(capacity)),
        }
    }

    pub(super) fn clear(&mut self) {
        self.map.clear();
    }

    pub(super) fn get(&mut self, key: &RerankScoreCacheKey) -> Option<f64> {
        self.map.get(key).copied()
    }

    pub(super) fn insert(&mut self, key: RerankScoreCacheKey, score: f64) {
        if score.is_finite() {
            self.map.put(key, score);
        }
    }
}

impl Default for FinalLookupCache {
    fn default() -> Self {
        Self::new(engine_tuning().final_lookup_cache_capacity)
    }
}

impl Default for ShortLookupCache {
    fn default() -> Self {
        Self::new(engine_tuning().short_lookup_cache_capacity)
    }
}

impl Default for RerankScoreCache {
    fn default() -> Self {
        Self::new(RERANK_SCORE_CACHE_CAPACITY)
    }
}

pub(super) struct LookupProfiler {
    input: String,
    request_id: u64,
    started: Instant,
    last_mark: Instant,
    marks: Vec<(&'static str, u128)>,
}

pub(super) struct LookupSoftBudget {
    started: Instant,
    request_id: u64,
    enabled: bool,
    limited: bool,
    cancelled: bool,
    limited_stage: Option<&'static str>,
    soft_budget: Duration,
    min_first_batch_candidates: usize,
}

pub(super) struct CorrectionWorkBudget {
    started: Instant,
    request_id: u64,
    limit: Duration,
    cancelled: bool,
}

impl CorrectionWorkBudget {
    pub(super) fn new(request_id: u64, input_chars: usize) -> Self {
        Self {
            started: Instant::now(),
            request_id,
            // Correction is a fallback path. On long readings cap its work
            // more aggressively so the full retry cannot monopolize the
            // shared engine after the visible first page is already shown.
            limit: Duration::from_millis(if input_chars >= 8 { 4 } else { 10 }),
            cancelled: false,
        }
    }

    #[inline]
    pub(super) fn should_stop(&mut self, iteration: usize) -> bool {
        if iteration != 0 && iteration % 8 != 0 {
            return false;
        }
        if lookup_request_superseded(self.request_id) {
            self.cancelled = true;
            return true;
        }
        self.started.elapsed() >= self.limit
    }

    pub(super) fn was_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl LookupSoftBudget {
    pub(super) fn new(input_chars: usize, disabled: bool, request_id: u64) -> Self {
        let tuning = engine_tuning();
        Self {
            started: Instant::now(),
            request_id,
            enabled: request_id != 0
                && !disabled
                && input_chars >= LONG_LOOKUP_SOFT_BUDGET_INPUT_LEN,
            limited: false,
            cancelled: false,
            limited_stage: None,
            soft_budget: tuning.long_lookup_soft_budget,
            min_first_batch_candidates: tuning.long_lookup_min_first_batch_candidates,
        }
    }

    pub(super) fn should_yield(&mut self, candidate_count: usize) -> bool {
        if lookup_request_superseded(self.request_id) {
            self.limited = true;
            self.cancelled = true;
            self.limited_stage.get_or_insert("superseded");
            return true;
        }
        if !self.enabled || candidate_count < self.min_first_batch_candidates {
            return false;
        }
        if self.started.elapsed() < self.soft_budget {
            return false;
        }
        self.limited = true;
        self.limited_stage.get_or_insert("candidate_budget");
        true
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Stops a single expensive inner loop once the first-batch deadline has
    /// elapsed. Unlike `should_yield_stage`, this does not require the outer
    /// candidate map to be full because mixed expansions materialize their
    /// candidates only after the inner loop returns.
    pub(super) fn should_stop_expensive_work(&mut self, stage: &'static str) -> bool {
        if lookup_request_superseded(self.request_id) {
            self.limited = true;
            self.cancelled = true;
            self.limited_stage.get_or_insert("superseded");
            return true;
        }
        if !self.enabled || self.started.elapsed() < self.soft_budget {
            return false;
        }
        self.limited = true;
        self.limited_stage.get_or_insert(stage);
        true
    }

    pub(super) fn should_yield_stage(
        &mut self,
        stage: &'static str,
        candidate_count: usize,
    ) -> bool {
        if lookup_request_superseded(self.request_id) {
            self.limited = true;
            self.cancelled = true;
            self.limited_stage.get_or_insert("superseded");
            return true;
        }
        if !self.enabled || candidate_count == 0 {
            return false;
        }
        if self.started.elapsed() < self.soft_budget {
            return false;
        }
        self.limited = true;
        self.limited_stage.get_or_insert(stage);
        true
    }

    pub(super) fn should_defer_first_batch_stage(
        &mut self,
        stage: &'static str,
        candidate_count: usize,
    ) -> bool {
        if lookup_request_superseded(self.request_id) {
            self.limited = true;
            self.cancelled = true;
            self.limited_stage.get_or_insert("superseded");
            return true;
        }
        if !self.enabled || candidate_count < self.min_first_batch_candidates {
            return false;
        }
        if self.limited && !self.cancelled {
            self.limited_stage.get_or_insert(stage);
            return true;
        }
        let entering_expensive_tail = stage == "first_batch_decode";
        if !entering_expensive_tail && self.started.elapsed() < self.soft_budget {
            return false;
        }
        self.limited = true;
        self.limited_stage.get_or_insert(stage);
        true
    }

    pub(super) fn was_limited(&self) -> bool {
        self.limited
    }

    pub(super) fn was_cancelled(&self) -> bool {
        self.cancelled
    }

    pub(super) fn limited_stage(&self) -> Option<&'static str> {
        self.limited_stage
    }
}

pub(super) fn perf_log_enabled() -> bool {
    runtime_log::perf_enabled()
}

pub(super) fn final_lookup_cache_key(
    raw: &str,
    mode_flags: u32,
    cache_epoch: u64,
) -> FinalLookupCacheKey {
    FinalLookupCacheKey {
        input: raw.to_string(),
        mode_flags,
        cache_epoch,
    }
}

pub(super) fn short_lookup_cache_key(
    compact_key: &str,
    mode_flags: u32,
    user_clock: u64,
    cache_epoch: u64,
) -> ShortLookupCacheKey {
    ShortLookupCacheKey {
        input: compact_key.to_string(),
        mode_flags,
        user_clock,
        cache_epoch,
    }
}

pub(super) fn rerank_score_cache_key(
    kind: RerankScoreKind,
    phrase: &str,
    user_clock: u64,
    cache_epoch: u64,
    context: Vec<String>,
) -> RerankScoreCacheKey {
    RerankScoreCacheKey {
        kind,
        phrase: phrase.to_string(),
        user_clock,
        cache_epoch,
        context,
    }
}

pub(super) fn mixed_completion_score_cache_key(
    full_key: &str,
    compositional_suffix: bool,
    user_clock: u64,
    cache_epoch: u64,
) -> MixedCompletionScoreCacheKey {
    MixedCompletionScoreCacheKey {
        full_key: full_key.to_string(),
        compositional_suffix,
        user_clock,
        cache_epoch,
    }
}

pub(super) fn lookup_stability_state_from_candidates(
    compact_key: &str,
    candidates: &[RankedCandidate],
) -> Option<LookupStabilityState> {
    let top_phrases = candidates
        .iter()
        .take(LOOKUP_STABILITY_TOP_N)
        .map(|item| item.phrase.clone())
        .collect::<Vec<_>>();
    (!top_phrases.is_empty()).then(|| LookupStabilityState {
        key: compact_key.to_string(),
        top_phrases,
    })
}

pub(super) fn is_short_lookup_cacheable(
    raw: &str,
    compact_key: &str,
    direct_input_shortcut: bool,
) -> bool {
    !direct_input_shortcut
        && !compact_key.is_empty()
        && compact_key.chars().count() <= SHORT_LOOKUP_CACHE_MAX_INPUT_CHARS
        && raw.chars().all(|ch| ch.is_ascii_alphabetic())
        && raw.chars().count() == compact_key.chars().count()
}

pub(super) fn is_final_lookup_cacheable(raw: &str, direct_input_shortcut: bool) -> bool {
    !raw.is_empty()
        && !direct_input_shortcut
        && !raw.starts_with("vv")
        && raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '\'' || ch == ' ')
}

impl LookupProfiler {
    pub(super) fn maybe_start_with_request_id(input: &str, request_id: u64) -> Option<Self> {
        if !perf_log_enabled() {
            return None;
        }
        let now = Instant::now();
        Some(Self {
            input: input.to_string(),
            request_id,
            started: now,
            last_mark: now,
            marks: Vec::with_capacity(8),
        })
    }

    pub(super) fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        self.marks
            .push((name, now.duration_since(self.last_mark).as_micros()));
        self.last_mark = now;
    }

    pub(super) fn finish(mut self, candidate_count: usize) {
        self.mark("finish");
        let total = self.started.elapsed().as_micros();
        if !should_log_lookup_profile(total, &self.marks) {
            return;
        }
        let stages = self
            .marks
            .iter()
            .map(|(name, us)| format!("{name}={us}us"))
            .collect::<Vec<_>>()
            .join(" ");
        runtime_log::log_engine(
            RuntimeLogLevel::Perf,
            "srf_lookup_profile",
            format!(
                "request_id={} {} candidates={} total={}us {}",
                self.request_id,
                runtime_log::input_fingerprint(&self.input),
                candidate_count,
                total,
                stages
            ),
        );
    }
}

fn should_log_lookup_profile(total_us: u128, marks: &[(&'static str, u128)]) -> bool {
    if total_us >= LOOKUP_PROFILE_SLOW_THRESHOLD_US {
        return true;
    }
    if marks.iter().any(|(name, _)| {
        matches!(*name, "candidate_budget" | "mixed" | "parse" | "superseded")
            || name.starts_with("first_batch")
    }) || repeated_mark(marks, "correction")
        || repeated_mark(marks, "decode")
    {
        return true;
    }
    LOOKUP_PROFILE_FAST_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed)
        & LOOKUP_PROFILE_FAST_SAMPLE_MASK
        == 0
}

fn repeated_mark(marks: &[(&'static str, u128)], needle: &'static str) -> bool {
    marks
        .iter()
        .filter(|(name, _)| *name == needle)
        .take(2)
        .count()
        > 1
}

impl PrefixLruCache {
    pub(super) fn clear(&mut self) {
        self.inner.clear();
    }

    pub(super) fn get(&mut self, key: &str) -> Option<Arc<[ThuoclEntry]>> {
        self.inner.get(key).cloned()
    }

    pub(super) fn put(&mut self, key: String, value: Arc<[ThuoclEntry]>) {
        // Truncate per-entry results before caching.
        let value = if value.len() > PREFIX_CACHE_MAX_ITEMS {
            Arc::from(
                value
                    .iter()
                    .take(PREFIX_CACHE_MAX_ITEMS)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        } else {
            value
        };
        self.inner.put(key, value);
    }
}

impl MixedExpansionLruCache {
    pub(super) fn clear(&mut self) {
        self.order.clear();
        self.map.clear();
    }

    pub(super) fn get(&mut self, key: &str) -> Option<Arc<[MixedPinyinKeyExpansion]>> {
        let v = Arc::clone(self.map.get(key)?);
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_front(key.to_string());
        Some(v)
    }

    pub(super) fn put(&mut self, key: String, value: Arc<[MixedPinyinKeyExpansion]>) {
        if self.map.contains_key(&key) {
            self.map.insert(key.clone(), value);
            if let Some(pos) = self.order.iter().position(|k| k == &key) {
                self.order.remove(pos);
            }
            self.order.push_front(key);
            return;
        }
        self.order.push_front(key.clone());
        self.map.insert(key, value);

        while self.order.len() > MIXED_PINYIN_EXPANSION_CACHE_CAPACITY {
            if let Some(old) = self.order.pop_back() {
                self.map.remove(&old);
            }
        }
    }
}
