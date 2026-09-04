use super::*;

#[derive(Clone, Debug)]
pub(in crate::core) struct KnownMixedWordPartition {
    score: f64,
    split_syllable_index: usize,
    split_key_byte: usize,
    left_freq: u64,
    right_freq: u64,
}

pub(crate) fn load_phrase_lexicon_core(dir: &Path) -> Result<AbbrevLexicon, String> {
    load_phrase_lexicon_core_with_prefs(
        dir,
        crate::lexicon_prefs::has_custom_optional_lexicon_prefs(),
    )
}

pub(super) fn load_phrase_lexicon_core_with_prefs(
    dir: &Path,
    has_custom_optional_prefs: bool,
) -> Result<AbbrevLexicon, String> {
    let options = crate::thuocl::LexiconLoadOptions::default();
    if !has_custom_optional_prefs {
        if let Ok(Some(lex)) = try_load_prebaked_near(dir) {
            return Ok(lex.with_load_options(options));
        }
    }
    let load = if has_custom_optional_prefs {
        load_dir_txt_with_profile_filtered_by_prefs
    } else {
        load_dir_txt_with_profile_default_enabled
    };
    let lex = load(dir, LexiconBuildProfile::Standard)
        .map(|(a, _)| a)
        .map_err(|e| e.to_string())?;
    Ok(lex.with_load_options(options))
}

pub(crate) fn load_hot_phrase_lexicon_core(dir: &Path) -> Result<AbbrevLexicon, String> {
    let options = crate::thuocl::LexiconLoadOptions::default();
    let has_custom_optional_prefs = crate::lexicon_prefs::has_custom_optional_lexicon_prefs();
    if !has_custom_optional_prefs {
        if let Ok(Some(lex)) = try_load_hot_prebaked_near(dir) {
            return Ok(lex.with_load_options(options));
        }
    }
    let load = if has_custom_optional_prefs {
        load_dir_txt_with_profile_filtered_by_prefs
    } else {
        load_dir_txt_with_profile_default_enabled
    };
    let lex = load(dir, LexiconBuildProfile::Hot)
        .map(|(a, _)| a)
        .map_err(|e| e.to_string())?;
    Ok(lex.with_load_options(options))
}

struct LookupPipelineState {
    raw: String,
    compact_key: String,
    direct_input_shortcut: bool,
    user_clock: u64,
    soft_budget: LookupSoftBudget,
    short_cache_key: Option<ShortLookupCacheKey>,
    final_cache_key: Option<FinalLookupCacheKey>,
    // Date/shortcut disambiguation needs the same parse variants as the main
    // lookup.  Keep them alive across the prepare/lookup boundary so the
    // interactive path does not parse the same reading twice.
    preparsed_variants: Option<Vec<ParseVariant>>,
}

enum LookupPipelineStart {
    Continue(LookupPipelineState),
    Finished(Vec<RankedCandidate>, Option<String>),
}

fn clipboard_background_enabled_for_mode_flags(flags: u32) -> bool {
    flags & MODE_CLIPBOARD_BACKGROUND != 0 && flags & MODE_CLIPBOARD_DISABLED == 0
}

impl PinyinEngine {
    pub fn new() -> Self {
        Self::with_phrase_dir(trusted_default_phrase_dir().as_deref())
    }

    pub fn with_phrase_dir(dir: Option<&Path>) -> Self {
        let started = Instant::now();
        let lex_started = Instant::now();
        let phrase_lexicon = dir.and_then(load_phrase_lexicon);
        let single_char_common = dir.map(load_single_char_common_index).unwrap_or_default();
        let lex_us = lex_started.elapsed().as_micros();
        Self::with_phrase_lexicon_and_single_chars(
            phrase_lexicon,
            single_char_common,
            "full",
            started,
            lex_us,
        )
    }

    pub(crate) fn with_hot_phrase_dir(dir: Option<&Path>) -> Self {
        let started = Instant::now();
        let lex_started = Instant::now();
        let phrase_lexicon = dir.and_then(load_hot_phrase_lexicon);
        let single_char_common = dir.map(load_single_char_common_index).unwrap_or_default();
        let lex_us = lex_started.elapsed().as_micros();
        Self::with_phrase_lexicon_and_single_chars(
            phrase_lexicon,
            single_char_common,
            "hot",
            started,
            lex_us,
        )
    }

    fn with_phrase_lexicon_and_single_chars(
        phrase_lexicon: Option<AbbrevLexicon>,
        single_char_common: Arc<single_char_common::SingleCharCommonIndex>,
        lexicon_mode: &str,
        started: Instant,
        lex_us: u128,
    ) -> Self {
        let model_started = Instant::now();
        let model = shared_engine_model();
        let model_us = model_started.elapsed().as_micros();
        let user_lexicon = UserLexicon::load_default();
        let user_lexicon_stamp = user_lexicon_disk_stamp();
        let user_lexicon_generation = start_user_lexicon_watcher(user_lexicon_stamp.clone());
        let total_us = started.elapsed().as_micros();
        if perf_log_enabled() {
            let lex_state = if phrase_lexicon.is_some() {
                "loaded"
            } else {
                "none"
            };
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_engine_load",
                format!(
                    "char_count={} phrase_lexicon={} model={}us lexicon={}us total={}us",
                    model.char_count, lex_state, model_us, lex_us, total_us
                ),
            );
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_engine_lexicon_mode",
                format!("mode={lexicon_mode} phrase_lexicon={lex_state}"),
            );
        }
        let shared_data_generation = next_shared_data_generation();
        let runtime_config_generation = crate::runtime_config::snapshot().generation;
        Self {
            dict: Arc::clone(&model.dict),
            syllables: Arc::clone(&model.syllables),
            lm: Arc::clone(&model.lm),
            single_char_common,
            phrase_lexicon,
            user_lexicon,
            user_lexicon_stamp,
            user_lexicon_generation,
            mode_flags: DEFAULT_MODE_FLAGS,
            char_count: model.char_count,
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
            runtime_config_generation,
            merged_buf: MergedCandidateMap::with_capacity(256),
            ranked_buf: Vec::with_capacity(128),
            last_lookup: None,
            shared_data_generation,
            active_session_generation: shared_data_generation,
        }
    }

    pub(super) fn advance_shared_data_generation(&mut self) {
        self.shared_data_generation = next_shared_data_generation();
        self.active_session_generation = self.shared_data_generation;
    }

    fn swap_lookup_session(&mut self, session: &mut LookupSession) {
        std::mem::swap(&mut self.mode_flags, &mut session.mode_flags);
        std::mem::swap(
            &mut self.prefix_cache_abbrev,
            &mut session.prefix_cache_abbrev,
        );
        std::mem::swap(
            &mut self.prefix_cache_pinyin,
            &mut session.prefix_cache_pinyin,
        );
        std::mem::swap(
            &mut self.mixed_expansion_cache,
            &mut session.mixed_expansion_cache,
        );
        std::mem::swap(
            &mut self.mixed_incremental_cache,
            &mut session.mixed_incremental_cache,
        );
        std::mem::swap(
            &mut self.mixed_completion_score_cache,
            &mut session.mixed_completion_score_cache,
        );
        std::mem::swap(
            &mut self.incremental_parse_cache,
            &mut session.incremental_parse_cache,
        );
        std::mem::swap(
            &mut self.incremental_decode_cache,
            &mut session.incremental_decode_cache,
        );
        std::mem::swap(
            &mut self.incremental_word_graph_cache,
            &mut session.incremental_word_graph_cache,
        );
        std::mem::swap(
            &mut self.rerank_score_cache,
            &mut session.rerank_score_cache,
        );
        std::mem::swap(
            &mut self.final_lookup_cache,
            &mut session.final_lookup_cache,
        );
        std::mem::swap(
            &mut self.short_lookup_cache,
            &mut session.short_lookup_cache,
        );
        std::mem::swap(
            &mut self.pending_lookup_continuation,
            &mut session.pending_lookup_continuation,
        );
        std::mem::swap(
            &mut self.pending_full_retry_keys,
            &mut session.pending_full_retry_keys,
        );
        std::mem::swap(&mut self.cache_epoch, &mut session.cache_epoch);
        std::mem::swap(
            &mut self.runtime_config_generation,
            &mut session.runtime_config_generation,
        );
        std::mem::swap(&mut self.merged_buf, &mut session.merged_buf);
        std::mem::swap(&mut self.ranked_buf, &mut session.ranked_buf);
        std::mem::swap(&mut self.last_lookup, &mut session.last_lookup);
        std::mem::swap(
            &mut self.active_session_generation,
            &mut session.shared_data_generation,
        );
    }

    /// Run one lookup with composition-local mutable state while retaining the
    /// engine's global model and learned dictionary. The swap is restored even
    /// if a decoder panics so the shared engine remains usable.
    pub(crate) fn lookup_with_session(
        &mut self,
        session: &mut LookupSession,
        mode_flags: u32,
        reading: &str,
        request_id: u64,
    ) -> (Vec<RankedCandidate>, Option<String>) {
        let _cancellation_scope =
            LookupCancellationScope::enter(session.cancellation_token(), request_id);
        self.swap_lookup_session(session);
        if self.active_session_generation != self.shared_data_generation {
            self.clear_lookup_caches();
            self.last_lookup = None;
            self.active_session_generation = self.shared_data_generation;
        }
        self.set_mode_flags(mode_flags);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.lookup_full_detailed_with_request_id(reading, request_id)
        }));
        self.active_session_generation = self.shared_data_generation;
        self.swap_lookup_session(session);
        if self.active_session_generation != self.shared_data_generation {
            self.clear_lookup_caches();
            self.last_lookup = None;
            self.active_session_generation = self.shared_data_generation;
        }
        match result {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(super) fn clear_lookup_caches(&mut self) {
        self.cache_epoch = self.cache_epoch.wrapping_add(1);
        if let Ok(mut c) = self.prefix_cache_abbrev.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.prefix_cache_pinyin.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mixed_expansion_cache.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mixed_incremental_cache.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mixed_completion_score_cache.lock() {
            c.clear();
        }
        self.incremental_parse_cache.borrow_mut().clear();
        self.incremental_decode_cache.borrow_mut().clear();
        self.incremental_word_graph_cache.borrow_mut().clear();
        if let Ok(mut c) = self.rerank_score_cache.lock() {
            c.clear();
        }
        self.final_lookup_cache.clear();
        self.short_lookup_cache.clear();
        self.pending_lookup_continuation = None;
    }

    /// Invalidate caches whose result depends on user learning or selection
    /// feedback while retaining system-lexicon prefix work. Prefix lookup and
    /// mixed-key expansion are independent of the user dictionary and are
    /// expensive to rebuild after every committed phrase.
    pub(super) fn clear_user_sensitive_lookup_caches(&mut self) {
        self.cache_epoch = self.cache_epoch.wrapping_add(1);
        if let Ok(mut c) = self.rerank_score_cache.lock() {
            c.clear();
        }
        if let Ok(mut c) = self.mixed_completion_score_cache.lock() {
            c.clear();
        }
        self.final_lookup_cache.clear();
        self.short_lookup_cache.clear();
        self.pending_lookup_continuation = None;
    }

    fn refresh_runtime_config_generation(&mut self) -> bool {
        let generation = crate::runtime_config::snapshot().generation;
        self.refresh_runtime_config_generation_to(generation)
    }

    pub(super) fn refresh_runtime_config_generation_to(&mut self, generation: u64) -> bool {
        if self.runtime_config_generation == generation {
            return false;
        }
        self.runtime_config_generation = generation;
        self.clear_lookup_caches();
        self.last_lookup = None;
        true
    }

    pub fn reload_phrase_dir(&mut self, dir: Option<&Path>) -> Result<(), String> {
        let (next_lexicon, next_single_chars) = match dir {
            Some(path) => {
                let lexicon = Some(
                    load_phrase_lexicon_core(path).map_err(|e| format!("load lexicon: {}", e))?,
                );
                let single_chars = load_single_char_common_index(path);
                (lexicon, single_chars)
            }
            None => {
                let trusted = trusted_default_phrase_dir();
                let lexicon = trusted.as_deref().and_then(load_phrase_lexicon);
                let single_chars = trusted
                    .as_deref()
                    .map(load_single_char_common_index)
                    .unwrap_or_default();
                (lexicon, single_chars)
            }
        };
        self.clear_lookup_caches();
        self.phrase_lexicon = next_lexicon;
        self.single_char_common = next_single_chars;
        self.last_lookup = None;
        self.advance_shared_data_generation();
        Ok(())
    }

    pub(crate) fn install_phrase_lexicon(&mut self, lexicon: Option<AbbrevLexicon>) {
        self.clear_lookup_caches();
        self.phrase_lexicon = lexicon;
        self.last_lookup = None;
        self.advance_shared_data_generation();
    }

    pub(crate) fn take_phrase_lexicon(&mut self) -> Option<AbbrevLexicon> {
        self.phrase_lexicon.take()
    }

    pub(crate) fn has_phrase_lexicon(&self) -> bool {
        self.phrase_lexicon.is_some()
    }

    pub fn set_mode_flags(&mut self, flags: u32) {
        if self.mode_flags != flags {
            self.clear_lookup_caches();
            self.last_lookup = None;
        }
        self.mode_flags = flags;
        crate::clipboard_store::set_background_polling_enabled(
            clipboard_background_enabled_for_mode_flags(flags),
        );
    }

    pub fn syllable_set(&self) -> &HashSet<String> {
        self.syllables.as_ref()
    }

    pub(super) fn note_user_lexicon_updated(&mut self) {
        self.user_lexicon_stamp = user_lexicon_disk_stamp();
        self.user_lexicon_generation = notify_user_lexicon_changed(self.user_lexicon_stamp.clone());
        self.clear_user_sensitive_lookup_caches();
        self.last_lookup = None;
        self.advance_shared_data_generation();
    }

    pub(super) fn note_selection_feedback_updated(&mut self, affected: &[String]) {
        self.note_user_phrases_updated(affected);
    }

    /// 已知词条的用户词典变化（提交/置顶/删除/反馈等）只影响这些词条
    /// 参与的缓存结果：按短语定向驱逐，而不是整表清空。外部磁盘重载等
    /// 未知范围的变化仍走 note_user_lexicon_updated 的整表失效。
    pub(super) fn note_user_phrases_updated(&mut self, affected: &[String]) {
        self.user_lexicon_stamp = user_lexicon_disk_stamp();
        self.user_lexicon_generation = notify_user_lexicon_changed(self.user_lexicon_stamp.clone());
        self.invalidate_user_sensitive_lookup_caches_for(affected);
        self.last_lookup = None;
        self.advance_shared_data_generation();
    }

    /// 提交路径的定向失效：短语本身 + 其分词 token 都算受影响词条。
    pub(super) fn note_phrase_and_tokens_updated(&mut self, phrase: &str, tokens: &[String]) {
        let mut affected = Vec::with_capacity(1 + tokens.len());
        affected.push(phrase.to_string());
        for token in tokens {
            if !affected.iter().any(|existing| existing == token) {
                affected.push(token.clone());
            }
        }
        self.note_user_phrases_updated(&affected);
    }

    /// 定向失效：只驱逐依赖受影响词条的缓存项，保留其余输入的结果，
    /// 让"选词/上屏后的下一键"继续命中已有缓存而不是全冷启动。
    pub(super) fn invalidate_user_sensitive_lookup_caches_for(&mut self, affected: &[String]) {
        if let Ok(mut c) = self.rerank_score_cache.lock() {
            c.evict_affected(affected);
        }
        if let Ok(mut c) = self.mixed_completion_score_cache.lock() {
            // 该缓存只存 f64 分值，无法按短语反查 key；容量小、重建便宜，
            // 保持整表清空。
            c.clear();
        }
        self.final_lookup_cache.evict_affected(affected);
        self.short_lookup_cache.evict_affected(affected);
        self.pending_lookup_continuation = None;
    }

    pub(crate) fn warmup_correction_indexes(&self) {
        if let Some(lexicon) = self.phrase_lexicon.as_ref() {
            lexicon.warmup_approx_indexes();
        }
    }

    /// Preload representative lookup paths before serving interactive input.
    pub fn warmup_interactive_paths(&mut self) {
        self.warmup_interactive_paths_until(|| true);
    }

    pub(crate) fn warmup_interactive_paths_until<F>(&mut self, mut should_continue: F)
    where
        F: FnMut() -> bool,
    {
        if !should_continue() {
            return;
        }
        self.warmup_correction_indexes();
        for reading in [
            "ni",
            "nihao",
            "jintian",
            "mingtian",
            "shur",
            "shurufa",
            "bejing",
            "beijing",
            "anguan",
            "angui",
            "hangzhou",
            "nihaoshijie",
            "zhonghuarenmingongheguo",
        ] {
            if !should_continue() {
                return;
            }
            self.warmup_interactive_prefixes_until(reading, &mut should_continue);
        }
    }

    /// Warm every prefix a user produces while typing a representative hot
    /// reading. This fills the exact hot index, prefix LRUs, parse/final caches,
    /// and rerank caches in the same order as real incremental composition.
    pub(crate) fn warmup_interactive_prefixes_until<F>(
        &mut self,
        reading: &str,
        mut should_continue: F,
    ) where
        F: FnMut() -> bool,
    {
        for (end, _) in reading
            .char_indices()
            .skip(1)
            .chain(std::iter::once((reading.len(), '\0')))
        {
            if !should_continue() {
                return;
            }
            self.warmup_interactive_reading(&reading[..end]);
        }
    }

    pub(crate) fn warmup_interactive_reading(&mut self, reading: &str) {
        let _ = self.lookup_full_detailed(reading);
        // Warmup must not affect the stability context of the user's first
        // real composition. The useful prefix/final caches remain populated.
        self.last_lookup = None;
        self.pending_lookup_continuation = None;
    }

    pub(crate) fn lookup_full_detailed(
        &mut self,
        pinyin_buffer: &str,
    ) -> (Vec<RankedCandidate>, Option<String>) {
        self.lookup_full_detailed_with_request_id(pinyin_buffer, 0)
    }

    fn finish_lookup_pipeline(
        profiler: &mut Option<LookupProfiler>,
        candidates: Vec<RankedCandidate>,
        error: Option<String>,
    ) -> LookupPipelineStart {
        if let Some(profiler) = profiler.take() {
            profiler.finish(candidates.len());
        }
        LookupPipelineStart::Finished(candidates, error)
    }

    pub(super) fn finish_partial_first_batch(
        &mut self,
        merged: &mut MergedCandidateMap,
        compact_key: &str,
        raw: &str,
        now: u64,
        request_id: u64,
        mut profiler: Option<LookupProfiler>,
        stage: &'static str,
    ) -> (Vec<RankedCandidate>, Option<String>) {
        self.arm_full_lookup_retry(raw);
        let mut ranked = std::mem::take(&mut self.ranked_buf);
        ranked.clear();
        ranked.reserve(merged.len().min(TSF_MAX_CANDIDATES));
        let rerank_prefs = rerank_prefs::get_rerank_prefs();
        for (candidate_index, (phrase, merged_candidate)) in
            merged.drain_candidates().into_iter().enumerate()
        {
            if candidate_index % 32 == 0 && lookup_request_superseded(request_id) {
                self.ranked_buf = ranked;
                self.merged_buf = std::mem::take(merged);
                if let Some(profiler) = profiler {
                    profiler.finish(0);
                }
                return (Vec::new(), Some("lookup superseded".to_string()));
            }
            let score = merged_candidate.score
                + self.stability_bonus_for_candidate(compact_key, &phrase)
                + self.blended_rerank_with_prefs(&phrase, now, rerank_prefs);
            ranked.push(RankedCandidate {
                phrase,
                score,
                meta: merged_candidate.meta,
            });
        }
        let partial_mixed_hot_phrases = self
            .phrase_lexicon
            .as_ref()
            .filter(|_| self.mixed_pinyin_enabled() && compact_key.chars().count() >= 4)
            .and_then(|lex| lex.lookup_mixed_hot(compact_key))
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| {
                        (2..=SHORT_HOTWORD_MAX_CHARS).contains(&phrase_char_count(&entry.phrase))
                    })
                    .take(MIXED_HOT_DIRECT_FRONT_LIMIT)
                    .map(|entry| entry.phrase.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let partial_keep = TSF_MAX_CANDIDATES.min(TSF_PAGE_SIZE * 2);
        self.apply_selection_feedback(compact_key, &mut ranked);
        sort_ranked_candidates_top_k_preserving(
            &mut ranked,
            partial_keep,
            &partial_mixed_hot_phrases,
        );
        drop_disabled_english_word_candidates(
            &mut ranked,
            self.english_word_input_enabled(),
            self.phrase_lexicon.as_ref(),
        );
        self.shape_partial_first_page_candidates(&mut ranked, raw, compact_key, now);
        if lookup_request_superseded(request_id) {
            self.ranked_buf = ranked;
            self.merged_buf = std::mem::take(merged);
            if let Some(profiler) = profiler {
                profiler.finish(0);
            }
            return (Vec::new(), Some("lookup superseded".to_string()));
        }
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            apply_traditional_output(&mut ranked);
        }
        if lookup_request_superseded(request_id) {
            self.ranked_buf = ranked;
            self.merged_buf = std::mem::take(merged);
            if let Some(profiler) = profiler {
                profiler.finish(0);
            }
            return (Vec::new(), Some("lookup superseded".to_string()));
        }
        self.pending_lookup_continuation = Some(PendingLookupContinuation {
            raw: raw.to_string(),
            mode_flags: self.mode_flags,
            cache_epoch: self.cache_epoch,
            user_clock: now,
        });
        let page_size =
            candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        if ranked.len() > page_size {
            ranked.truncate(page_size);
        }
        if let Some(first) = ranked.first_mut() {
            first.meta = first.meta.clone().with_partial();
        }
        if ranked.is_empty() {
            ranked.push(RankedCandidate {
                phrase: raw.to_string(),
                score: 0.0,
                meta: CandidateMeta::legacy(ASCII_DIRECT_META).with_partial(),
            });
        }
        self.last_lookup = lookup_stability_state_from_candidates(compact_key, &ranked);
        let out = std::mem::take(&mut ranked);
        self.ranked_buf = ranked;
        self.merged_buf = std::mem::take(merged);
        if let Some(profiler) = profiler.as_mut() {
            profiler.mark(stage);
        }
        if let Some(profiler) = profiler {
            profiler.finish(out.len());
        }
        (out, None)
    }

    fn arm_full_lookup_retry(&mut self, raw: &str) {
        let mode_flags = self.mode_flags;
        self.pending_full_retry_keys
            .retain(|(key, mode)| key != raw || *mode != mode_flags);
        self.pending_full_retry_keys
            .push_back((raw.to_string(), mode_flags));
        while self.pending_full_retry_keys.len() > MAX_PENDING_FULL_RETRY_KEYS {
            self.pending_full_retry_keys.pop_front();
        }
    }

    pub(super) fn has_pending_full_lookup_retry(&self, raw: &str) -> bool {
        self.pending_full_retry_keys
            .iter()
            .any(|(key, mode)| key == raw && *mode == self.mode_flags)
    }

    fn complete_full_lookup_retry(&mut self, raw: &str) {
        let mode_flags = self.mode_flags;
        self.pending_full_retry_keys
            .retain(|(key, mode)| key != raw || *mode != mode_flags);
    }

    fn ranked_direct_candidates(
        &self,
        candidates: Vec<crate::v_mode::DirectCandidate>,
    ) -> Vec<RankedCandidate> {
        let mut detailed = candidates
            .into_iter()
            .map(|candidate| RankedCandidate {
                phrase: candidate.phrase,
                score: candidate.score,
                meta: candidate_meta_from_legacy(candidate.meta.as_deref()),
            })
            .collect::<Vec<_>>();
        detailed.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            apply_traditional_output(&mut detailed);
        }
        detailed.truncate(TSF_MAX_CANDIDATES);
        detailed
    }

    fn reserve_datetime_collision_slot(&self, raw: &str, ranked: &mut Vec<RankedCandidate>) {
        if !crate::v_mode::is_live_datetime_top_shortcut(raw)
            || !self.has_hot_two_char_abbrev_conflict(raw)
        {
            return;
        }
        let Some(candidates) = crate::v_mode::direct_input_candidates_detailed(raw) else {
            return;
        };
        if !ranked.first().is_some_and(|item| item.meta.pinned) {
            let mut preferred = Vec::new();
            promote_preferred_short_abbrev(&compact_lookup_key(raw), &mut preferred);
            preferred.retain(|phrase| ranked.iter().any(|item| item.phrase == *phrase));
            promote_preserved_candidates(ranked, &preferred);
        }
        let shortcuts = self.ranked_direct_candidates(candidates);
        let shortcut_phrases = shortcuts
            .iter()
            .map(|item| item.phrase.clone())
            .collect::<HashSet<_>>();
        ranked.retain(|item| !shortcut_phrases.contains(&item.phrase));
        if let Some(shortcut) = shortcuts.into_iter().next() {
            // Keep the common-word Top3 intact while making the explicit
            // utility candidate consistently reachable on the first page.
            let slot = ranked.len().min(3);
            ranked.insert(slot, shortcut);
        }
    }

    fn effective_direct_input_shortcut_with_preparse(
        &self,
        raw: &str,
    ) -> (bool, Option<Vec<ParseVariant>>) {
        if crate::v_mode::is_function_key_shortcut(raw) {
            return (true, None);
        }
        // `sj` is also a common abbreviation for 手机/睡觉. Keep the live
        // time forms in the candidate list, but let the normal abbreviation
        // pipeline run when a hot phrase collision exists so ordinary words
        // remain reachable on the first page.
        if crate::v_mode::is_live_datetime_top_shortcut(raw)
            && !self.has_hot_two_char_abbrev_conflict(raw)
        {
            return (true, None);
        }
        if self.has_hot_two_char_abbrev_conflict(raw) {
            return (false, self.parse_variants_cached(raw).ok());
        }
        if self.mode_flags & MODE_DATE_AUTO_FORMAT == 0 {
            return (false, None);
        }
        if !crate::v_mode::is_direct_input_shortcut(raw) {
            return (false, None);
        }
        let (conflicts, variants) = self.direct_datetime_shortcut_conflicts_with_exact_pinyin(raw);
        (!conflicts, variants)
    }

    fn has_hot_two_char_abbrev_conflict(&self, raw: &str) -> bool {
        let Some(lexicon) = self.phrase_lexicon.as_ref() else {
            return false;
        };
        let key = compact_lookup_key(raw);
        lexicon.lookup_hot(&key).is_some_and(|entries| {
            entries.iter().any(|entry| match key.as_str() {
                "sj" => matches!(entry.phrase.as_str(), "手机" | "睡觉"),
                "dt" => entry.phrase == "地铁",
                _ => false,
            })
        })
    }

    pub(super) fn parse_variants_cached(&self, raw: &str) -> Result<Vec<ParseVariant>, String> {
        self.incremental_parse_cache.borrow_mut().parse(
            raw,
            self.syllables.as_ref(),
            self.parse_options(),
        )
    }

    pub(super) fn parse_exact_variants_cached(
        &self,
        raw: &str,
    ) -> Result<Vec<ParseVariant>, String> {
        self.incremental_parse_cache.borrow_mut().parse_exact(
            raw,
            self.syllables.as_ref(),
            self.parse_options(),
        )
    }

    pub(super) fn is_effective_direct_input_shortcut(&self, raw: &str) -> bool {
        self.effective_direct_input_shortcut_with_preparse(raw).0
    }

    fn direct_datetime_shortcut_conflicts_with_exact_pinyin(
        &self,
        raw: &str,
    ) -> (bool, Option<Vec<ParseVariant>>) {
        let normalized = normalize_pinyin_line(raw);
        let trimmed = normalized.trim();
        let compact_key = compact_lookup_key(trimmed);
        if compact_key.len() < 2 {
            return (false, None);
        }
        let Ok(variants) = self.parse_variants_cached(trimmed) else {
            return (false, None);
        };
        let conflicts = variants.iter().any(|variant| {
            variant.source == ParseVariantSource::Exact
                && variant.tail.is_none()
                && !variant.syllables.is_empty()
                && variant.syllables.join("") == compact_key
        });
        (conflicts, Some(variants))
    }

    fn prepare_lookup_pipeline(
        &mut self,
        pinyin_buffer: &str,
        request_id: u64,
        profiler: &mut Option<LookupProfiler>,
    ) -> LookupPipelineStart {
        self.refresh_runtime_config_generation();
        self.reload_user_lexicon_if_changed();
        // Try a fast swap-in of a background-loaded lexicon first.
        // If none is ready but a reload is pending, trigger a deferred
        // background load and fall through with the current lexicon.
        if let Some(prepared) = crate::lexicon_prefs::take_prepared_lexicon() {
            self.install_phrase_lexicon(Some(prepared));
        } else if crate::lexicon_prefs::take_phrase_lexicon_reload_flag() {
            let default_dir = trusted_default_phrase_dir();
            if let Some(dir) = default_dir.as_deref().map(|p| p.to_path_buf()) {
                std::thread::Builder::new()
                    .name("srf-lexicon-bg-load".to_string())
                    .spawn(move || {
                        if let Ok(lex) = load_phrase_lexicon_core(&dir) {
                            if let Ok(mut prepared) = crate::lexicon_prefs::PREPARED_LEXICON.lock()
                            {
                                *prepared = Some(lex);
                            }
                        }
                    })
                    .ok();
            }
        }

        let normalized = normalize_pinyin_line(pinyin_buffer);
        let raw = normalized.trim();
        if raw.is_empty() {
            return Self::finish_lookup_pipeline(profiler, Vec::new(), None);
        }
        if self.mode_flags & MODE_U_MODE != 0 && raw.starts_with('u') && raw.len() > 1 {
            let candidates = radical_pinyin_candidates(raw);
            if !candidates.is_empty() {
                return Self::finish_lookup_pipeline(profiler, candidates, None);
            }
        }
        if let Some(custom) = crate::custom_shortcuts::lookup_detailed(raw) {
            let detailed = custom
                .into_iter()
                .enumerate()
                .map(|(idx, candidate)| RankedCandidate {
                    phrase: candidate.phrase,
                    score: 10_000.0 - idx as f64,
                    meta: CandidateMeta::legacy(format!("自定义: {}", candidate.source_key)),
                })
                .collect::<Vec<_>>();
            return Self::finish_lookup_pipeline(profiler, detailed, None);
        }

        let (direct_input_shortcut, preparsed_variants) =
            self.effective_direct_input_shortcut_with_preparse(raw);

        if self.mode_flags & MODE_V_ASSIST != 0 && raw.starts_with("vv") {
            let stripped = raw[2..].trim_start();
            if self.mode_flags & MODE_CLIPBOARD_DISABLED != 0
                && crate::v_mode::is_clipboard_command(stripped)
            {
                return Self::finish_lookup_pipeline(profiler, Vec::new(), None);
            }
            let options = crate::v_mode::LookupOptions {
                symbol_toolbox_enabled: self.mode_flags & MODE_SYMBOL_TOOLBOX != 0,
                emoji_input_enabled: self.mode_flags & MODE_EMOJI_INPUT != 0,
            };
            if let Some(ranked) = crate::v_mode::lookup_detailed_with_options(stripped, options) {
                let detailed = self.ranked_direct_candidates(ranked);
                return Self::finish_lookup_pipeline(profiler, detailed, None);
            }
        }
        if direct_input_shortcut {
            if let Some(candidates) = crate::v_mode::direct_input_candidates_detailed(raw) {
                let detailed = self.ranked_direct_candidates(candidates);
                self.last_lookup = None;
                return Self::finish_lookup_pipeline(profiler, detailed, None);
            }
        }

        let compact_key = compact_lookup_key(raw);
        let user_clock = self.user_lexicon.current_clock();
        let guaranteed_full_retry = self.has_pending_full_lookup_retry(raw);
        let continuation_retry =
            self.pending_lookup_continuation
                .take()
                .is_some_and(|continuation| {
                    continuation.raw == raw
                        && continuation.mode_flags == self.mode_flags
                        && continuation.cache_epoch == self.cache_epoch
                        && continuation.user_clock == user_clock
                });
        if continuation_retry || guaranteed_full_retry {
            if let Some(profiler) = profiler.as_mut() {
                profiler.mark(if guaranteed_full_retry {
                    "guaranteed_full_retry"
                } else {
                    "continuation_retry"
                });
            }
        }
        let repeated_lookup_for_same_key = guaranteed_full_retry
            || continuation_retry
            || self
                .last_lookup
                .as_ref()
                .is_some_and(|state| state.key == compact_key);
        let mut soft_budget = LookupSoftBudget::new(
            compact_key.chars().count(),
            direct_input_shortcut || repeated_lookup_for_same_key,
            request_id,
        );
        if soft_budget.should_yield(0) {
            return Self::finish_lookup_pipeline(
                profiler,
                Vec::new(),
                Some("lookup superseded".to_string()),
            );
        }

        let short_cache_key = if is_short_lookup_cacheable(raw, &compact_key, direct_input_shortcut)
        {
            Some(short_lookup_cache_key(
                &compact_key,
                self.mode_flags,
                self.cache_epoch,
            ))
        } else {
            None
        };
        if let Some(cache_key) = short_cache_key.as_ref() {
            if let Some((mut cached, visible_short_intent, two_char_density_replay)) =
                self.short_lookup_cache.get(cache_key)
            {
                self.apply_cached_context_rerank(&compact_key, &mut cached);
                drop_disabled_english_word_candidates(
                    &mut cached,
                    self.english_word_input_enabled(),
                    self.phrase_lexicon.as_ref(),
                );
                if let Some(intent_chars) = visible_short_intent {
                    let page_size = candidate_prefs::get_effective_candidate_page_size()
                        .clamp(3, TSF_PAGE_SIZE);
                    apply_short_intent_final_candidate_density(
                        &mut cached,
                        Some(intent_chars),
                        self.phrase_lexicon.as_ref(),
                    );
                    replay_two_char_intent_density(
                        &mut cached,
                        two_char_density_replay,
                        intent_chars,
                        page_size,
                    );
                    enforce_short_intent_visible_page(&mut cached, intent_chars, page_size);
                }
                // Cached context reranking must not let a typo correction jump
                // ahead of an explicit user-history candidate.
                demote_corrections_below_priority_hits(&mut cached, &HashSet::new(), None);
                if !cached.first().is_some_and(|item| item.meta.pinned) {
                    promote_ranked_high_priority_two_char_candidates(
                        &mut cached,
                        raw,
                        &compact_key,
                        self.syllables.as_ref(),
                        self.phrase_lexicon.as_ref(),
                    );
                }
                self.reserve_datetime_collision_slot(raw, &mut cached);
                if let Some((intent, phrase_len)) =
                    self.strict_lexicon_frequency_order_context(raw, &compact_key)
                {
                    if phrase_len >= 2 {
                        enforce_strict_system_lexicon_frequency_order(
                            &mut cached,
                            &compact_key,
                            intent,
                            phrase_len,
                            self.phrase_lexicon.as_ref(),
                        );
                    }
                    demote_cold_non_full_lexicon_candidates(
                        &mut cached,
                        intent,
                        self.phrase_lexicon.as_ref(),
                    );
                }
                self.last_lookup = lookup_stability_state_from_candidates(&compact_key, &cached);
                if let Some(profiler) = profiler.as_mut() {
                    profiler.mark("short_cache_hit");
                }
                return Self::finish_lookup_pipeline(profiler, cached, None);
            }
        }

        let final_cache_key = if is_final_lookup_cacheable(raw, direct_input_shortcut) {
            Some(final_lookup_cache_key(
                raw,
                self.mode_flags,
                self.cache_epoch,
            ))
        } else {
            None
        };
        if let Some(cache_key) = final_cache_key.as_ref() {
            if let Some((mut cached, visible_short_intent, two_char_density_replay)) =
                self.final_lookup_cache.get(cache_key)
            {
                self.apply_cached_context_rerank(&compact_key, &mut cached);
                drop_disabled_english_word_candidates(
                    &mut cached,
                    self.english_word_input_enabled(),
                    self.phrase_lexicon.as_ref(),
                );
                if let Some(intent_chars) = visible_short_intent {
                    let page_size = candidate_prefs::get_effective_candidate_page_size()
                        .clamp(3, TSF_PAGE_SIZE);
                    apply_short_intent_final_candidate_density(
                        &mut cached,
                        Some(intent_chars),
                        self.phrase_lexicon.as_ref(),
                    );
                    replay_two_char_intent_density(
                        &mut cached,
                        two_char_density_replay,
                        intent_chars,
                        page_size,
                    );
                    enforce_short_intent_visible_page(&mut cached, intent_chars, page_size);
                }
                // Cached context reranking must not let a typo correction jump
                // ahead of an explicit user-history candidate.
                demote_corrections_below_priority_hits(&mut cached, &HashSet::new(), None);
                if !cached.first().is_some_and(|item| item.meta.pinned) {
                    promote_ranked_high_priority_two_char_candidates(
                        &mut cached,
                        raw,
                        &compact_key,
                        self.syllables.as_ref(),
                        self.phrase_lexicon.as_ref(),
                    );
                }
                self.reserve_datetime_collision_slot(raw, &mut cached);
                if let Some((intent, phrase_len)) =
                    self.strict_lexicon_frequency_order_context(raw, &compact_key)
                {
                    if phrase_len >= 2 {
                        enforce_strict_system_lexicon_frequency_order(
                            &mut cached,
                            &compact_key,
                            intent,
                            phrase_len,
                            self.phrase_lexicon.as_ref(),
                        );
                    }
                    demote_cold_non_full_lexicon_candidates(
                        &mut cached,
                        intent,
                        self.phrase_lexicon.as_ref(),
                    );
                }
                self.last_lookup = lookup_stability_state_from_candidates(&compact_key, &cached);
                if let Some(profiler) = profiler.as_mut() {
                    profiler.mark("final_cache_hit");
                }
                return Self::finish_lookup_pipeline(profiler, cached, None);
            }
        }

        if let Some(profiler) = profiler.as_mut() {
            profiler.mark("prepare");
        }

        LookupPipelineStart::Continue(LookupPipelineState {
            raw: raw.to_string(),
            compact_key,
            direct_input_shortcut,
            user_clock,
            soft_budget,
            short_cache_key,
            final_cache_key,
            preparsed_variants,
        })
    }

    pub(crate) fn lookup_full_detailed_with_request_id(
        &mut self,
        pinyin_buffer: &str,
        request_id: u64,
    ) -> (Vec<RankedCandidate>, Option<String>) {
        publish_latest_lookup_request_id(request_id);
        let mut profiler = LookupProfiler::maybe_start_with_request_id(pinyin_buffer, request_id);
        let LookupPipelineState {
            raw,
            compact_key,
            direct_input_shortcut,
            user_clock,
            mut soft_budget,
            short_cache_key,
            final_cache_key,
            preparsed_variants,
        } = match self.prepare_lookup_pipeline(pinyin_buffer, request_id, &mut profiler) {
            LookupPipelineStart::Continue(state) => state,
            LookupPipelineStart::Finished(candidates, error) => return (candidates, error),
        };
        let raw = raw.as_str();
        // Parse once before abbreviation work. A complete full-pinyin path is
        // allowed through the exact decoder even if abbreviation candidates
        // have already exhausted the first-batch soft budget.
        // Keep correction/fuzzy expansion out of the critical path. Exact and
        // alternate splits cover full-pinyin, jianpin and mixed routing; the
        // expanded parser is requested below only when that first page is
        // sparse (or when exact parsing failed).
        let preparsed_variants =
            preparsed_variants.or_else(|| self.parse_exact_variants_cached(raw).ok());
        let exact_full_pinyin_syllables = preparsed_variants.as_ref().and_then(|variants| {
            variants
                .iter()
                .find(|variant| {
                    variant.source == ParseVariantSource::Exact
                        && variant.tail.is_none()
                        && !variant.syllables.is_empty()
                        && variant.syllables.join("") == compact_key
                })
                .map(|variant| variant.syllables.clone())
        });
        let exact_complete_full_pinyin = exact_full_pinyin_syllables.is_some();
        let prefix_pinyin_route = preparsed_variants.as_ref().and_then(|variants| {
            variants
                .iter()
                .find(|variant| {
                    variant.source == ParseVariantSource::Exact
                        && variant.tail.as_ref().is_some_and(|tail| !tail.is_empty())
                        && !variant.syllables.is_empty()
                        && format!(
                            "{}{}",
                            variant.syllables.join(""),
                            variant.tail.as_deref().unwrap_or_default()
                        ) == compact_key
                })
                .and_then(|variant| {
                    variant
                        .tail
                        .as_ref()
                        .map(|tail| (variant.syllables.clone(), tail.clone()))
                })
        });
        let compact_is_exact_single_syllable =
            is_exact_complete_single_syllable_input(raw, &compact_key, self.syllables.as_ref());
        let exact_user_hotword_syllables = exact_full_pinyin_syllables
            .as_ref()
            .map(Vec::len)
            .filter(|len| (1..=SHORT_HOTWORD_MAX_CHARS).contains(len));
        let jianpin_enabled = self.jianpin_enabled();
        // 将可复用容器临时取出，避免与 `&self` 方法（如 cached_prefix_*）发生借用冲突。

        let mut merged = std::mem::take(&mut self.merged_buf);
        merged.clear();
        let now = user_clock;
        let mut mixed_intent_char_count: Option<usize> = None;
        let mut mixed_intent_syllables: Option<Vec<String>> = None;
        let mut mixed_prefix_intent = false;
        let mut mixed_direct_cross_length_collision = false;
        let mut strong_long_mixed_composition = false;
        let short_ascii_input = !compact_key.is_empty()
            && compact_key.len() <= 4
            && raw.chars().all(|ch| ch.is_ascii_alphabetic())
            && raw.chars().count() == compact_key.chars().count();
        let mut has_short_compact_phrase_candidates = false;
        let mut short_two_char_prefix_intent = false;
        let mut preserved_direct_phrases = Vec::new();
        let mut preserved_short_phrases = Vec::new();
        let mut preserved_mixed_hot_phrases = Vec::new();
        let mut preserved_daily_short_two_char_phrases = Vec::new();
        let mut preserved_daily_short_three_char_phrases = Vec::new();
        let mut preserved_chat_priority_two_char_phrases = Vec::new();
        let mut preserved_high_priority_two_char_phrases = Vec::new();
        let mut preserved_exact_lexicon_phrases = Vec::new();
        let mut preserved_exact_user_phrases = Vec::new();
        let mut preserved_mixed_user_phrases = Vec::new();
        let mut preserved_pinned_user_phrases = Vec::new();
        let mut final_preferred_phrases = Vec::new();
        let mut deferred_first_batch_stage: Option<&'static str> = None;
        let mut exact_primary_decoded = false;
        let mut prefix_primary_decoded = false;
        let mut exact_route_candidate_count = 0usize;
        promote_preferred_short_abbrev(&compact_key, &mut final_preferred_phrases);

        macro_rules! return_lookup_superseded {
            () => {{
                self.merged_buf = merged;
                if let Some(profiler) = profiler.take() {
                    profiler.finish(0);
                }
                return (Vec::new(), Some("lookup superseded".to_string()));
            }};
        }

        if should_preserve_ascii_input(raw) {
            preserved_direct_phrases.push(raw.to_string());
            merge_candidate(
                &mut merged,
                raw.to_string(),
                12_000.0,
                Some(ASCII_DIRECT_META),
            );
        }

        // A live date/time alias that collides with a hot abbreviation should
        // contribute its own candidates without short-circuiting the normal
        // lexicon lookup. This keeps `sj` useful for both time and everyday
        // phrases such as 手机/睡觉.
        let datetime_collision_shortcut = !direct_input_shortcut
            && crate::v_mode::is_live_datetime_top_shortcut(raw)
            && self.has_hot_two_char_abbrev_conflict(raw);
        if datetime_collision_shortcut {
            if let Some(candidates) = crate::v_mode::direct_input_candidates_detailed(raw) {
                for candidate in candidates {
                    preserved_direct_phrases.push(candidate.phrase.clone());
                    merge_candidate(
                        &mut merged,
                        candidate.phrase,
                        candidate.score,
                        candidate.meta.as_deref(),
                    );
                }
            }
        }

        if direct_input_shortcut {
            if let Some(candidates) = crate::v_mode::direct_input_candidates_detailed(raw) {
                for candidate in candidates {
                    preserved_direct_phrases.push(candidate.phrase.clone());
                    merge_candidate(
                        &mut merged,
                        candidate.phrase,
                        candidate.score,
                        candidate.meta.as_deref(),
                    );
                }
            }
        }

        if !compact_key.is_empty() {
            if !direct_input_shortcut {
                if let Some(entries) = self.user_lexicon.lookup_input(&compact_key) {
                    preserved_exact_user_phrases.extend(
                        entries
                            .iter()
                            .take(TSF_PAGE_SIZE)
                            .map(|entry| entry.phrase.clone()),
                    );
                    preserved_pinned_user_phrases.extend(
                        entries
                            .iter()
                            .filter(|entry| entry.is_pinned_exact_input())
                            .take(TSF_PAGE_SIZE)
                            .map(|entry| entry.phrase.clone()),
                    );
                    merge_user_entries(
                        &mut merged,
                        entries.iter(),
                        USER_EXACT_INPUT_BONUS,
                        12,
                        now,
                        Some(USER_CANDIDATE_META),
                        self.phrase_lexicon.as_ref(),
                        exact_user_hotword_syllables,
                    );
                }
                if let Some(entries) = self.user_lexicon.lookup_mixed_input(&compact_key) {
                    // A mixed-pinyin key is already more specific than a pure
                    // abbreviation.  Do not hide a newly selected user phrase
                    // until it has been selected a second time: doing so makes
                    // the first learned hotword appear to have no effect in the
                    // candidate window.
                    preserved_mixed_user_phrases.extend(
                        entries
                            .iter()
                            .take(user_hotword_prefs::get_user_hotword_prefs().front_limit)
                            .map(|entry| entry.phrase.clone()),
                    );
                    merge_user_entries(
                        &mut merged,
                        entries.iter(),
                        USER_MIXED_INPUT_BONUS,
                        8,
                        now,
                        Some(MIXED_USER_CANDIDATE_META),
                        self.phrase_lexicon.as_ref(),
                        None,
                    );
                }
                if let Some(entries) = self.user_lexicon.lookup_observed_input(&compact_key) {
                    merge_user_entries(
                        &mut merged,
                        entries.iter(),
                        USER_OBSERVED_INPUT_BONUS,
                        4,
                        now,
                        Some(OBSERVED_USER_CANDIDATE_META),
                        self.phrase_lexicon.as_ref(),
                        None,
                    );
                }
            }

            // Intent router: a complete full-pinyin reading gets its exact
            // channel before abbreviation/mixed expansion. This is both more
            // deterministic and avoids spending the first-screen budget on a
            // lower-confidence interpretation of an already complete input.
            if let Some(syllables) = exact_full_pinyin_syllables.as_ref() {
                let before_exact = merged.len();
                if let Some(lex) = self.phrase_lexicon.as_ref() {
                    if let Some(entries) = lex.lookup_pinyin(&compact_key) {
                        preserved_exact_lexicon_phrases.extend(
                            entries
                                .iter()
                                .filter(|entry| phrase_char_count(&entry.phrase) == syllables.len())
                                .take(8)
                                .map(|entry| entry.phrase.clone()),
                        );
                        merge_phrase_entries(
                            &mut merged,
                            entries.iter().filter(|entry| {
                                phrase_char_count(&entry.phrase) == syllables.len()
                            }),
                            PHRASE_EXACT_PINYIN_BONUS,
                            20,
                            &self.user_lexicon,
                            now,
                            Some(PINYIN_CANDIDATE_META),
                            Some(lex),
                            None,
                        );
                    }
                }

                let word_graph = self.decode_word_graph_until(syllables, now, || {
                    soft_budget.should_stop_expensive_work("exact_word_graph")
                });
                merge_scored_candidates(
                    &mut merged,
                    word_graph.into_iter(),
                    0.0,
                    &self.user_lexicon,
                    now,
                    Some(PINYIN_CANDIDATE_META),
                );
                let (decoded, decode_complete) = self.decode_chars_incremental_until(
                    syllables,
                    self.effective_decode_beam_for_key(&compact_key),
                    decode_candidate_limit(syllables.len()),
                    || {
                        lookup_request_superseded(request_id)
                            || soft_budget.should_stop_expensive_work("exact_char_decode")
                    },
                );
                if !decode_complete {
                    if lookup_request_superseded(request_id) {
                        return_lookup_superseded!();
                    }
                    deferred_first_batch_stage.get_or_insert("first_batch_exact_decode");
                }
                merge_scored_candidates(
                    &mut merged,
                    decoded.into_iter(),
                    0.0,
                    &self.user_lexicon,
                    now,
                    Some(PINYIN_CANDIDATE_META),
                );
                exact_primary_decoded = true;
                exact_route_candidate_count = merged.len().saturating_sub(before_exact);
                if let Some(profiler) = profiler.as_mut() {
                    profiler.mark("exact_intent");
                }
                if lookup_request_superseded(request_id) {
                    return_lookup_superseded!();
                }
            } else if compact_key.len() >= 5 {
                if let Some((syllables, tail)) = prefix_pinyin_route
                    .as_ref()
                    .filter(|(_, tail)| tail.chars().count() >= 2)
                {
                    let before_prefix = merged.len();
                    if let Some(lex) = self.phrase_lexicon.as_ref() {
                        let entries = self.cached_prefix_pinyin_flat(lex, &compact_key);
                        preserved_exact_lexicon_phrases.extend(
                            entries
                                .iter()
                                .filter(|entry| {
                                    phrase_char_count(&entry.phrase) == syllables.len() + 1
                                })
                                .take(8)
                                .map(|entry| entry.phrase.clone()),
                        );
                        merge_phrase_entries(
                            &mut merged,
                            entries.iter(),
                            PHRASE_PREFIX_PINYIN_BONUS,
                            18,
                            &self.user_lexicon,
                            now,
                            Some(PINYIN_CANDIDATE_META),
                            Some(lex),
                            None,
                        );
                    }
                    let predictions = self.rank_syllable_completions(syllables, tail, now, 40);
                    let prefix_prediction_limit = if soft_budget.is_enabled() {
                        // Four top syllable completions are enough to fill a
                        // nine-item page; the continuation retry can expand
                        // the remaining predictions without blocking typing.
                        4
                    } else {
                        PREDICT_DECODE_TOP
                    };
                    for (rank, prediction) in
                        predictions.iter().take(prefix_prediction_limit).enumerate()
                    {
                        let mut full = syllables.clone();
                        full.push(prediction.clone());
                        if rank < PREDICT_WORD_GRAPH_TOP {
                            let word_graph = self.decode_word_graph_until(&full, now, || {
                                soft_budget.should_stop_expensive_work("prefix_word_graph")
                            });
                            merge_scored_candidates(
                                &mut merged,
                                word_graph.into_iter(),
                                rank as f64 * PREDICTION_PENALTY,
                                &self.user_lexicon,
                                now,
                                Some(PINYIN_CANDIDATE_META),
                            );
                        }
                        let (decoded, decode_complete) = self.decode_chars_incremental_until(
                            &full,
                            self.effective_decode_beam_for_key(&compact_key),
                            decode_candidate_limit(full.len()),
                            || {
                                lookup_request_superseded(request_id)
                                    || soft_budget.should_stop_expensive_work("prefix_decode")
                            },
                        );
                        if !decode_complete {
                            if lookup_request_superseded(request_id) {
                                return_lookup_superseded!();
                            }
                            deferred_first_batch_stage.get_or_insert("first_batch_prefix_decode");
                            break;
                        }
                        merge_scored_candidates(
                            &mut merged,
                            decoded.into_iter(),
                            rank as f64 * PREDICTION_PENALTY,
                            &self.user_lexicon,
                            now,
                            Some(PINYIN_CANDIDATE_META),
                        );
                        if merged.len().saturating_sub(before_prefix) >= TSF_PAGE_SIZE * 2 {
                            break;
                        }
                    }
                    exact_route_candidate_count = merged.len().saturating_sub(before_prefix);
                    prefix_primary_decoded = true;
                    if let Some(profiler) = profiler.as_mut() {
                        profiler.mark("prefix_intent");
                    }
                    if lookup_request_superseded(request_id) {
                        return_lookup_superseded!();
                    }
                }
            }

            let prefix_route_needs_mixed_disambiguation = prefix_pinyin_route
                .as_ref()
                .is_some_and(|(_, tail)| tail.chars().count() <= 1);
            let exact_route_needs_fallback = exact_route_candidate_count
                < candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE)
                || prefix_route_needs_mixed_disambiguation;
            let english_lexicon_intent = self.english_word_input_enabled()
                && exact_full_pinyin_syllables.is_none()
                && prefix_pinyin_route.is_none()
                && is_plain_ascii_word_input(raw, &compact_key)
                && !crate::english_words::lookup_prefix(&compact_key, 1).is_empty();
            // Do not fan URL/code/English-shaped input into the mixed beam.
            // A one- or two-letter incomplete pinyin tail still benefits from
            // mixed completion, but it only needs a small tail-focused beam.
            let routed_mixed_limit = if direct_input_shortcut
                || should_preserve_ascii_input(raw)
                || english_lexicon_intent
                || (exact_complete_full_pinyin && !exact_route_needs_fallback)
            {
                0
            } else {
                let base = self.mixed_pinyin_key_limit(&compact_key);
                if prefix_pinyin_route
                    .as_ref()
                    .is_some_and(|(_, tail)| tail.chars().count() <= 2)
                {
                    base.min(8)
                } else {
                    base
                }
            };

            // Abbreviation/prefix scans are the largest remaining tail before
            // mixed decoding. If the exact route already has a visible page,
            // stop before entering that tail and let the continuation retry
            // finish it without delaying the current candidate window.
            if exact_route_needs_fallback
                && soft_budget.should_defer_first_batch_stage("first_batch_abbrev", merged.len())
            {
                deferred_first_batch_stage = Some("first_batch_abbrev");
            }

            // Keep only the tiny, protected daily-use abbreviation tail for a
            // complete short full-pinyin input. The expensive prefix/mixed
            // expansion remains skipped when the exact route filled a page.
            if exact_complete_full_pinyin
                && !exact_route_needs_fallback
                && !compact_is_exact_single_syllable
                && jianpin_enabled
                && short_ascii_input
                && (2..=3).contains(&compact_key.chars().count())
            {
                if let Some(lex) = self.phrase_lexicon.as_ref() {
                    if let Some(entries) = lex.lookup_hot(&compact_key) {
                        let abbrev_char_count = compact_key.chars().count();
                        let daily_entries = entries
                            .iter()
                            .filter(|entry| match abbrev_char_count {
                                2 => is_daily_short_two_char_entry(entry),
                                3 => is_daily_short_three_char_entry(entry),
                                _ => false,
                            })
                            .collect::<Vec<_>>();
                        let preserved = if abbrev_char_count == 2 {
                            &mut preserved_daily_short_two_char_phrases
                        } else {
                            &mut preserved_daily_short_three_char_phrases
                        };
                        preserved.extend(daily_entries.iter().map(|entry| entry.phrase.clone()));
                        merge_phrase_entries(
                            &mut merged,
                            daily_entries.into_iter(),
                            self.exact_abbrev_bonus() + SHORT_COMPACT_EXACT_BOOST,
                            8,
                            &self.user_lexicon,
                            now,
                            Some(ABBREV_CANDIDATE_META),
                            Some(lex),
                            Some(abbrev_char_count),
                        );
                    }
                }
            }
            if !compact_is_exact_single_syllable
                && exact_route_needs_fallback
                && deferred_first_batch_stage.is_none()
            {
                if let Some(lex) = &self.phrase_lexicon {
                    if jianpin_enabled {
                        if !direct_input_shortcut {
                            if let Some(priority_abbrev) = high_priority_two_char_abbrev_key(
                                raw,
                                &compact_key,
                                self.syllables.as_ref(),
                            ) {
                                if let Some(entries) = lex.lookup(&priority_abbrev) {
                                    let priority_entries = entries
                                        .iter()
                                        .filter(|entry| is_protected_two_char_entry(entry))
                                        .collect::<Vec<_>>();
                                    if !priority_entries.is_empty() {
                                        has_short_compact_phrase_candidates = true;
                                        let (high_priority_entries, chat_priority_entries): (
                                            Vec<_>,
                                            Vec<_>,
                                        ) = priority_entries.into_iter().partition(|entry| {
                                            is_high_priority_two_char_entry(entry)
                                        });
                                        preserved_high_priority_two_char_phrases.extend(
                                            high_priority_entries
                                                .iter()
                                                .map(|entry| entry.phrase.clone()),
                                        );
                                        preserved_chat_priority_two_char_phrases.extend(
                                            chat_priority_entries
                                                .iter()
                                                .map(|entry| entry.phrase.clone()),
                                        );
                                        merge_phrase_entries(
                                            &mut merged,
                                            high_priority_entries.into_iter(),
                                            self.exact_abbrev_bonus() + SHORT_COMPACT_EXACT_BOOST,
                                            8,
                                            &self.user_lexicon,
                                            now,
                                            Some(ABBREV_CANDIDATE_META),
                                            Some(lex),
                                            Some(compact_key.chars().count()),
                                        );
                                        merge_phrase_entries(
                                            &mut merged,
                                            chat_priority_entries.into_iter(),
                                            self.exact_abbrev_bonus() + SHORT_COMPACT_EXACT_BOOST,
                                            8,
                                            &self.user_lexicon,
                                            now,
                                            Some(ABBREV_CANDIDATE_META),
                                            Some(lex),
                                            Some(compact_key.chars().count()),
                                        );
                                    }
                                }
                            }
                        }

                        let abbrev_char_count = compact_key.chars().count();
                        let expanded_three_char_abbrev_recall = short_ascii_input
                            && abbrev_char_count == 3
                            && exact_full_pinyin_syllables.is_none();
                        let exact_abbrev_entries = if expanded_three_char_abbrev_recall {
                            lex.lookup(&compact_key)
                        } else {
                            lex.lookup_hot(&compact_key)
                        };

                        // Daily-use short phrase protection for 2/3-letter abbreviation input.
                        if !direct_input_shortcut
                            && short_ascii_input
                            && (2..=3).contains(&abbrev_char_count)
                        {
                            if let Some(entries) = exact_abbrev_entries.as_ref() {
                                let daily_entries: Vec<_> = entries
                                    .iter()
                                    .filter(|entry| match abbrev_char_count {
                                        2 => is_daily_short_two_char_entry(entry),
                                        3 => is_daily_short_three_char_entry(entry),
                                        _ => false,
                                    })
                                    .collect();
                                if !daily_entries.is_empty() {
                                    has_short_compact_phrase_candidates = true;
                                    let preserved = match abbrev_char_count {
                                        2 => &mut preserved_daily_short_two_char_phrases,
                                        3 => &mut preserved_daily_short_three_char_phrases,
                                        _ => &mut preserved_daily_short_two_char_phrases,
                                    };
                                    preserved.extend(
                                        daily_entries.iter().map(|entry| entry.phrase.clone()),
                                    );
                                    merge_phrase_entries(
                                        &mut merged,
                                        daily_entries.into_iter(),
                                        self.exact_abbrev_bonus() + SHORT_COMPACT_EXACT_BOOST,
                                        8,
                                        &self.user_lexicon,
                                        now,
                                        Some(ABBREV_CANDIDATE_META),
                                        Some(lex),
                                        Some(abbrev_char_count),
                                    );
                                }
                            }
                        }

                        if let Some(entries) = exact_abbrev_entries.as_ref() {
                            if short_ascii_input && !entries.is_empty() {
                                has_short_compact_phrase_candidates = true;
                            }
                            let exact_abbrev_recall_limit = if expanded_three_char_abbrev_recall
                                && entries
                                    .iter()
                                    .filter(|entry| phrase_char_count(&entry.phrase) == 3)
                                    .take(THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT + 1)
                                    .count()
                                    > THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT
                            {
                                THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT_HIGH_COLLISION
                            } else if expanded_three_char_abbrev_recall {
                                THREE_CHAR_EXACT_ABBREV_RECALL_LIMIT
                            } else {
                                18
                            };
                            let exact_abbrev_bonus = self.exact_abbrev_bonus()
                                + if short_ascii_input
                                    && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&abbrev_char_count)
                                {
                                    SHORT_COMPACT_EXACT_BOOST
                                } else {
                                    0.0
                                };
                            if short_ascii_input && (2..=8).contains(&abbrev_char_count) {
                                preserved_short_phrases.extend(
                                    entries
                                        .iter()
                                        .filter(|entry| {
                                            phrase_char_count(&entry.phrase) == abbrev_char_count
                                        })
                                        .map(|entry| entry.phrase.clone()),
                                );
                            }
                            merge_phrase_entries(
                                &mut merged,
                                entries.iter().filter(|entry| {
                                    phrase_char_count(&entry.phrase) == abbrev_char_count
                                }),
                                exact_abbrev_bonus,
                                exact_abbrev_recall_limit,
                                &self.user_lexicon,
                                now,
                                Some(ABBREV_CANDIDATE_META),
                                Some(lex),
                                Some(abbrev_char_count),
                            );
                        } else if compact_key.len() >= 2 {
                            let entries = self.cached_prefix_flat(lex, &compact_key);
                            if short_ascii_input && !entries.is_empty() {
                                has_short_compact_phrase_candidates = true;
                            }
                            merge_phrase_entries(
                                &mut merged,
                                entries.iter(),
                                self.prefix_abbrev_bonus(),
                                18,
                                &self.user_lexicon,
                                now,
                                Some(ABBREV_CANDIDATE_META),
                                Some(lex),
                                Some(compact_key.chars().count()),
                            );
                        }

                        if short_ascii_input
                            && exact_abbrev_entries.is_none()
                            && compact_key.len() >= 3
                        {
                            for prefix_len in (2..compact_key.len()).rev() {
                                let prefix = &compact_key[..prefix_len];
                                let Some(entries) = lex.lookup(prefix) else {
                                    continue;
                                };
                                if entries.is_empty() {
                                    continue;
                                }
                                has_short_compact_phrase_candidates = true;
                                if prefix_len == 2
                                    && entries
                                        .iter()
                                        .any(|entry| phrase_char_count(&entry.phrase) == 2)
                                {
                                    short_two_char_prefix_intent = true;
                                }
                                preserved_short_phrases.extend(
                                    entries
                                        .iter()
                                        .filter(|entry| {
                                            phrase_char_count(&entry.phrase) == prefix_len
                                        })
                                        .map(|entry| entry.phrase.clone()),
                                );
                                merge_phrase_entries(
                                    &mut merged,
                                    entries.iter().filter(|entry| {
                                        phrase_char_count(&entry.phrase) == prefix_len
                                    }),
                                    self.exact_abbrev_bonus()
                                        - short_compact_backoff_penalty(
                                            compact_key.len() - prefix_len,
                                            entries
                                                .iter()
                                                .filter(|entry| {
                                                    phrase_char_count(&entry.phrase) == prefix_len
                                                })
                                                .map(|entry| entry.freq)
                                                .max()
                                                .unwrap_or(0),
                                        ),
                                    12,
                                    &self.user_lexicon,
                                    now,
                                    Some(ABBREV_CANDIDATE_META),
                                    Some(lex),
                                    Some(prefix_len),
                                );
                                break;
                            }
                        }

                        if short_ascii_input && compact_key.len() >= 3 {
                            let prefix = &compact_key[..2];
                            if let Some(entries) = lex.lookup(prefix) {
                                let prefix_phrases: Vec<String> = entries
                                    .iter()
                                    .filter(|entry| phrase_char_count(&entry.phrase) == 2)
                                    .map(|entry| entry.phrase.clone())
                                    .collect();
                                if !prefix_phrases.is_empty() {
                                    has_short_compact_phrase_candidates = true;
                                    short_two_char_prefix_intent = true;
                                    preserved_short_phrases
                                        .splice(0..0, prefix_phrases.iter().cloned());
                                    merge_phrase_entries(
                                        &mut merged,
                                        entries
                                            .iter()
                                            .filter(|entry| phrase_char_count(&entry.phrase) == 2),
                                        self.exact_abbrev_bonus() + SHORT_COMPACT_EXACT_BOOST,
                                        12,
                                        &self.user_lexicon,
                                        now,
                                        Some(ABBREV_CANDIDATE_META),
                                        Some(lex),
                                        Some(2),
                                    );
                                }
                            }
                        }
                    }

                    if compact_key.len() >= 3 {
                        if let Some(entries) = lex.lookup_pinyin_hot(&compact_key) {
                            if short_ascii_input && !entries.is_empty() {
                                has_short_compact_phrase_candidates = true;
                            }
                            preserved_exact_lexicon_phrases.extend(
                                entries
                                    .iter()
                                    .filter(|entry| phrase_char_count(&entry.phrase) <= 4)
                                    .map(|entry| entry.phrase.clone()),
                            );
                            merge_phrase_entries(
                                &mut merged,
                                entries
                                    .iter()
                                    .filter(|entry| phrase_char_count(&entry.phrase) <= 4),
                                PHRASE_DIRECT_PINYIN_LOOKUP_BONUS + SHORT_COMPACT_EXACT_BOOST,
                                16,
                                &self.user_lexicon,
                                now,
                                Some(PINYIN_CANDIDATE_META),
                                Some(lex),
                                None,
                            );
                        }
                    }

                    // Curated 2–4 character system phrases have an exact mixed
                    // key index. Query it before the bounded runtime expansion
                    // beam so inputs such as `weiyh` and `jisj` cannot be lost
                    // merely because their syllable path fell outside the
                    // first-batch expansion budget.
                    if self.mixed_pinyin_enabled()
                        && !direct_input_shortcut
                        && compact_key.chars().count() >= 4
                    {
                        if let Some(entries) = lex.lookup_mixed_hot(&compact_key) {
                            let mut direct_intent_chars = None;
                            let mut has_cross_length_collision = false;
                            for entry in entries.iter() {
                                let chars = phrase_char_count(&entry.phrase);
                                if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&chars) {
                                    continue;
                                }
                                match direct_intent_chars {
                                    None => direct_intent_chars = Some(chars),
                                    Some(current) if current != chars => {
                                        has_cross_length_collision = true;
                                        mixed_direct_cross_length_collision = true;
                                        break;
                                    }
                                    Some(_) => {}
                                }
                            }
                            if let Some(direct_chars) = direct_intent_chars {
                                mixed_prefix_intent = true;
                                if !has_cross_length_collision {
                                    mixed_intent_char_count.get_or_insert(direct_chars);
                                }
                                has_short_compact_phrase_candidates = true;
                                preserved_short_phrases.extend(
                                    entries
                                        .iter()
                                        .filter(|entry| {
                                            (2..=SHORT_HOTWORD_MAX_CHARS)
                                                .contains(&phrase_char_count(&entry.phrase))
                                        })
                                        .map(|entry| entry.phrase.clone()),
                                );
                                preserved_mixed_hot_phrases.extend(
                                    entries
                                        .iter()
                                        .filter(|entry| {
                                            (2..=SHORT_HOTWORD_MAX_CHARS)
                                                .contains(&phrase_char_count(&entry.phrase))
                                        })
                                        .take(MIXED_HOT_DIRECT_FRONT_LIMIT)
                                        .map(|entry| entry.phrase.clone()),
                                );
                                merge_phrase_entries(
                                    &mut merged,
                                    entries.iter().filter(|entry| {
                                        (2..=SHORT_HOTWORD_MAX_CHARS)
                                            .contains(&phrase_char_count(&entry.phrase))
                                    }),
                                    self.mixed_pinyin_bonus() + MIXED_PINYIN_HIGH_CONFIDENCE_BONUS,
                                    24,
                                    &self.user_lexicon,
                                    now,
                                    Some(MIXED_HIGH_CANDIDATE_META),
                                    Some(lex),
                                    None,
                                );
                            }
                        }
                    }

                    if soft_budget
                        .should_defer_first_batch_stage("first_batch_lexicon", merged.len())
                    {
                        deferred_first_batch_stage = Some("first_batch_lexicon");
                    }

                    if deferred_first_batch_stage.is_none() {
                        let first_batch_mixed_limit = if soft_budget.is_enabled() {
                            let full_limit = routed_mixed_limit;
                            match compact_key.chars().count() {
                                len if len >= 10 => full_limit.min(8),
                                len if len >= 6 => full_limit.min(12),
                                _ => full_limit.min(16),
                            }
                        } else {
                            routed_mixed_limit
                        };
                        let mixed_start_candidates = merged.len();
                        let (mixed_keys, mixed_complete) =
                            if soft_budget.should_yield_stage("mixed", mixed_start_candidates) {
                                (Vec::new(), false)
                            } else {
                                self.mixed_key_expansions_until(
                                    &compact_key,
                                    now,
                                    first_batch_mixed_limit,
                                    || soft_budget.should_stop_expensive_work("mixed_expand"),
                                )
                            };
                        if !mixed_complete {
                            deferred_first_batch_stage.get_or_insert("first_batch_mixed");
                        }
                        mixed_prefix_intent |= mixed_keys.iter().any(is_mixed_prefix_input_intent);
                        let mixed_intent_expansion = mixed_keys
                            .iter()
                            .find(|item| is_mixed_prefix_input_intent(item))
                            .or_else(|| mixed_keys.first());
                        if let Some(mixed_char_count) =
                            mixed_intent_expansion.map(|item| item.segment_count)
                        {
                            if !mixed_direct_cross_length_collision
                                && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&mixed_char_count)
                            {
                                if mixed_intent_char_count.is_none() {
                                    mixed_intent_char_count = Some(mixed_char_count);
                                }
                                if mixed_intent_char_count == Some(mixed_char_count)
                                    && mixed_intent_syllables.is_none()
                                {
                                    mixed_intent_syllables =
                                        mixed_intent_expansion.map(|item| item.syllables.clone());
                                }
                            }
                        }
                        let mut mixed_graph_expansions = 0usize;
                        let skip_mixed_graph_decode_for_short_input = short_ascii_input
                            && compact_key.len() <= 4
                            && merged.len() >= TSF_PAGE_SIZE;
                        let skip_mixed_graph_decode_for_populated_long_input =
                            compact_key.chars().count() >= 6 && merged.len() >= TSF_PAGE_SIZE;
                        let mut mixed_composition_graph_expansions = 0usize;
                        let prioritize_known_mixed_compositions = compact_key.chars().count() >= 9;
                        let known_partitions = mixed_keys
                            .iter()
                            .map(|expansion| {
                                prioritize_known_mixed_compositions
                                    .then(|| {
                                        self.mixed_expansion_known_word_partition_score(expansion)
                                    })
                                    .flatten()
                            })
                            .collect::<Vec<_>>();
                        let best_known_partition_score = known_partitions
                            .iter()
                            .filter_map(|partition| partition.as_ref().map(|item| item.score))
                            .max_by(f64::total_cmp);
                        strong_long_mixed_composition = prioritize_known_mixed_compositions
                            && best_known_partition_score.is_some();
                        for (expansion, known_partition) in
                            mixed_keys.iter().zip(known_partitions.iter())
                        {
                            if soft_budget.should_yield(merged.len()) {
                                break;
                            }
                            let segment_count = expansion.segment_count;
                            let high_confidence = is_high_confidence_mixed_key(expansion);
                            if !high_confidence && !self.mixed_pinyin_aggressive() {
                                continue;
                            }
                            let mixed_bonus = self.mixed_pinyin_bonus()
                                + if short_ascii_input
                                    && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&segment_count)
                                {
                                    SHORT_COMPACT_EXACT_BOOST
                                } else {
                                    0.0
                                }
                                + mixed_pinyin_path_adjustment(expansion)
                                - known_partition
                                    .as_ref()
                                    .zip(best_known_partition_score)
                                    .map(|(partition, best)| {
                                        ((best - partition.score).max(0.0)
                                            * MIXED_PINYIN_PARTITION_SCORE_GAP_SCALE)
                                            .min(MIXED_PINYIN_PARTITION_SCORE_GAP_CAP)
                                    })
                                    .unwrap_or(0.0);
                            let mixed_meta = Some(mixed_pinyin_candidate_meta(expansion));
                            if let Some(entries) = lex.lookup_pinyin(&expansion.key) {
                                if short_ascii_input && !entries.is_empty() && high_confidence {
                                    has_short_compact_phrase_candidates = true;
                                }
                                if short_ascii_input
                                    && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&segment_count)
                                    && high_confidence
                                {
                                    preserved_short_phrases.extend(
                                        entries
                                            .iter()
                                            .filter(|entry| {
                                                phrase_char_count(&entry.phrase) == segment_count
                                            })
                                            .map(|entry| entry.phrase.clone()),
                                    );
                                }
                                merge_phrase_entries(
                                    &mut merged,
                                    entries.iter().filter(|entry| {
                                        phrase_char_count(&entry.phrase) == segment_count
                                    }),
                                    mixed_bonus,
                                    14,
                                    &self.user_lexicon,
                                    now,
                                    mixed_meta,
                                    Some(lex),
                                    None,
                                );
                            }
                        }
                        let mut graph_mixed_keys = mixed_keys
                            .into_iter()
                            .zip(known_partitions)
                            .enumerate()
                            .map(|(original_index, (expansion, partition))| {
                                (original_index, partition, expansion)
                            })
                            .collect::<Vec<_>>();
                        if prioritize_known_mixed_compositions {
                            // Long mixed input has a deliberately small graph budget. Rank
                            // compositional paths by the evidence of both known words first;
                            // parser completion order alone can otherwise spend the budget on
                            // “计算机+物理/万里” before reaching “计算机+网络”.
                            graph_mixed_keys.sort_by(|left, right| {
                                right
                                    .1
                                    .as_ref()
                                    .map(|item| item.score)
                                    .unwrap_or(f64::NEG_INFINITY)
                                    .total_cmp(
                                        &left
                                            .1
                                            .as_ref()
                                            .map(|item| item.score)
                                            .unwrap_or(f64::NEG_INFINITY),
                                    )
                                    .then_with(|| left.0.cmp(&right.0))
                            });
                        }
                        for (_, known_partition, expansion) in graph_mixed_keys {
                            if soft_budget.should_yield(merged.len()) {
                                break;
                            }
                            if prioritize_known_mixed_compositions
                                && best_known_partition_score
                                    .zip(known_partition.as_ref().map(|item| item.score))
                                    .is_some_and(|(best, score)| {
                                        best - score > MIXED_PINYIN_COMPOSITION_GRAPH_SCORE_GAP
                                    })
                            {
                                // The direct lexicon channel above still keeps exact
                                // full-phrase hits. Restrict only synthesized word-graph
                                // combinations to the strongest known boundary, otherwise
                                // high-frequency names overwhelm a clearly typed tail.
                                continue;
                            }
                            let high_confidence = is_high_confidence_mixed_key(&expansion);
                            if !high_confidence && !self.mixed_pinyin_aggressive() {
                                continue;
                            }
                            let path_adjustment = mixed_pinyin_path_adjustment(&expansion);
                            let mixed_bonus = self.mixed_pinyin_bonus()
                                + if short_ascii_input
                                    && (2..=SHORT_HOTWORD_MAX_CHARS)
                                        .contains(&expansion.segment_count)
                                {
                                    SHORT_COMPACT_EXACT_BOOST
                                } else {
                                    0.0
                                }
                                + if known_partition.is_some() {
                                    // Two independently known words are stronger evidence than
                                    // three raw initial segments. Refund the ambiguity penalty so
                                    // the selected composition survives pruning and is visible.
                                    path_adjustment
                                        .max(MIXED_PINYIN_KNOWN_COMPOSITION_ADJUSTMENT_FLOOR)
                                } else {
                                    path_adjustment
                                }
                                - known_partition
                                    .as_ref()
                                    .zip(best_known_partition_score)
                                    .map(|(partition, best)| {
                                        ((best - partition.score).max(0.0)
                                            * MIXED_PINYIN_PARTITION_SCORE_GAP_SCALE)
                                            .min(MIXED_PINYIN_PARTITION_SCORE_GAP_CAP)
                                    })
                                    .unwrap_or(0.0);
                            let mixed_meta = Some(mixed_pinyin_candidate_meta(&expansion));
                            if soft_budget.should_yield(merged.len()) {
                                break;
                            }
                            let compositional_long_path =
                                skip_mixed_graph_decode_for_populated_long_input
                                    && compact_key.chars().count() >= 9
                                    && mixed_composition_graph_expansions
                                        < MIXED_PINYIN_COMPOSITION_GRAPH_LIMIT
                                    && known_partition.is_some();
                            if !skip_mixed_graph_decode_for_short_input
                                && (!skip_mixed_graph_decode_for_populated_long_input
                                    || compositional_long_path)
                                && expansion.syllables.len() >= 2
                                && mixed_graph_expansions < MIXED_PINYIN_WORD_GRAPH_LIMIT
                            {
                                mixed_graph_expansions += 1;
                                if compositional_long_path {
                                    mixed_composition_graph_expansions += 1;
                                }
                                let mut graph_limited = false;
                                let mut word_graph =
                                    self.decode_word_graph_until(&expansion.syllables, now, || {
                                        graph_limited =
                                            soft_budget.should_stop_expensive_work("mixed_graph");
                                        graph_limited
                                    });
                                if let Some(partition) = known_partition.as_ref() {
                                    word_graph.retain_mut(|(phrase, score)| {
                                        let adjustment = self
                                            .mixed_known_boundary_candidate_adjustment(
                                                &expansion, partition, phrase,
                                            );
                                        if adjustment < 0.0 {
                                            return false;
                                        }
                                        *score += adjustment;
                                        true
                                    });
                                }
                                merge_scored_candidates(
                                    &mut merged,
                                    word_graph.into_iter(),
                                    MIXED_PINYIN_WORD_GRAPH_PENALTY - mixed_bonus,
                                    &self.user_lexicon,
                                    now,
                                    mixed_meta,
                                );

                                if graph_limited
                                    || soft_budget.should_stop_expensive_work("mixed_decode")
                                {
                                    deferred_first_batch_stage
                                        .get_or_insert("first_batch_mixed_graph");
                                    break;
                                }

                                let (mut decoded, decode_complete) = self
                                    .decode_chars_incremental_until(
                                        &expansion.syllables,
                                        self.effective_decode_beam_for_key(&compact_key),
                                        decode_candidate_limit(expansion.syllables.len()),
                                        || {
                                            soft_budget
                                                .should_stop_expensive_work("mixed_char_decode")
                                        },
                                    );
                                if let Some(partition) = known_partition.as_ref() {
                                    decoded.retain_mut(|(phrase, score)| {
                                        let adjustment = self
                                            .mixed_known_boundary_candidate_adjustment(
                                                &expansion, partition, phrase,
                                            );
                                        if adjustment < 0.0 {
                                            return false;
                                        }
                                        *score += adjustment;
                                        true
                                    });
                                }
                                merge_scored_candidates(
                                    &mut merged,
                                    decoded.into_iter(),
                                    MIXED_PINYIN_DECODE_PENALTY - mixed_bonus,
                                    &self.user_lexicon,
                                    now,
                                    mixed_meta,
                                );
                                if !decode_complete {
                                    deferred_first_batch_stage
                                        .get_or_insert("first_batch_mixed_char_decode");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        // 长输入若首批只有单字/短片段，宁可继续精确解码，也不向候选窗发送
        // “干、感、甘……”一类误导性的闪屏。预算已经触发且首屏已满时，
        // 直接交付首批，避免长拼音继续占用共享引擎。
        let partial_min_phrase_chars = if compact_key.len() >= 18 { 6 } else { 5 };
        let partial_has_long_phrase = merged
            .phrases()
            .any(|phrase| phrase_char_count(phrase) >= partial_min_phrase_chars);
        let partial_has_first_page = merged.len()
            >= candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        let interactive_page_size =
            candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        if (!exact_complete_full_pinyin || deferred_first_batch_stage.is_some())
            && (partial_has_long_phrase
                || (deferred_first_batch_stage.is_some() && partial_has_first_page))
        {
            if let Some(stage) = deferred_first_batch_stage {
                return self.finish_partial_first_batch(
                    &mut merged,
                    &compact_key,
                    raw,
                    now,
                    request_id,
                    profiler.take(),
                    stage,
                );
            }
        }
        // A long reading that already has a visible page must not enter the
        // expanded parse/correction tail.  This is the hard hand-off point
        // that keeps one held composition from monopolising the shared engine.
        if compact_key.chars().count() > LONG_INPUT_COMPACT_CHAR_LIMIT
            && soft_budget.should_yield_stage("first_batch_ready", merged.len())
            && merged.len() >= interactive_page_size
        {
            return self.finish_partial_first_batch(
                &mut merged,
                &compact_key,
                raw,
                now,
                request_id,
                profiler.take(),
                "first_batch_ready",
            );
        }
        if let Some(profiler) = profiler.as_mut() {
            profiler.mark("abbrev_phrase");
        }
        if lookup_request_superseded(request_id) {
            return_lookup_superseded!();
        }

        let mut error = None;
        let mut typo_variant_merged = MergedCandidateMap::default();
        let parse_result = if let Some(preparsed) = preparsed_variants {
            let page_size =
                candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
            let has_exact_variant = preparsed
                .iter()
                .any(|variant| variant.source == ParseVariantSource::Exact);
            if has_exact_variant && merged.len() >= page_size {
                Ok(preparsed)
            } else if compact_key.chars().count() > LONG_INPUT_COMPACT_CHAR_LIMIT {
                Err("long lookup skipped expanded parse".to_string())
            } else if soft_budget.should_yield_stage("parse", merged.len()) {
                Err("lookup soft budget yielded before expanded parse".to_string())
            } else {
                self.parse_variants_cached(raw)
            }
        } else if compact_key.chars().count() > LONG_INPUT_COMPACT_CHAR_LIMIT {
            Err("long lookup skipped expanded parse".to_string())
        } else if soft_budget.should_yield_stage("parse", merged.len()) {
            Err("lookup soft budget yielded before parse".to_string())
        } else {
            self.parse_variants_cached(raw)
        };
        let parse_failed = parse_result.is_err();
        let correction_prefs = crate::correction_prefs::get_correction_prefs();
        let correction_enabled = correction_prefs.enabled;
        let has_exact_parse = parse_result
            .as_ref()
            .ok()
            .map(|variants| {
                variants
                    .iter()
                    .any(|variant| variant.source == ParseVariantSource::Exact)
            })
            .unwrap_or(false);
        if parse_failed || (!has_exact_parse && compact_key.len() >= 4) {
            preserved_direct_phrases.extend(merge_ascii_completion_candidates(
                raw,
                &compact_key,
                &mut merged,
                self.english_word_input_enabled(),
            ));
        }
        let primary_variant = parse_result
            .as_ref()
            .ok()
            .and_then(|variants| variants.first().cloned());
        let fallback_exact_complete_short_parse_intent = parse_result
            .as_ref()
            .ok()
            .and_then(|variants| infer_exact_complete_short_parse_intent(variants));
        let parse_intent_char_count = parse_result
            .as_ref()
            .ok()
            .and_then(|variants| infer_uniform_variant_char_count_detailed(variants));
        let explicit_separator_preferred = parse_result
            .as_ref()
            .ok()
            .and_then(|variants| explicit_separator_phrase_key_from_variants(raw, variants));
        let explicit_separator_preferred_len = explicit_separator_preferred
            .as_ref()
            .map(|(_, syllable_count)| *syllable_count);
        let mut preserved_separator_phrases = Vec::new();
        if let Some((exact_key, preferred_len)) = explicit_separator_preferred.as_ref() {
            if let Some(lex) = &self.phrase_lexicon {
                if let Some(entries) = lex.lookup_pinyin(exact_key) {
                    preserved_separator_phrases.extend(
                        entries
                            .iter()
                            .filter(|entry| phrase_char_count(&entry.phrase) == *preferred_len)
                            .take(8)
                            .map(|entry| entry.phrase.clone()),
                    );
                }
            }
        }
        let exact_single_syllable_input = explicit_separator_preferred_len.is_none()
            && (compact_is_exact_single_syllable
                || parse_result
                    .as_ref()
                    .ok()
                    .map(|variants| {
                        has_exact_complete_single_syllable_variant(variants, &compact_key)
                    })
                    .unwrap_or(false));
        let exact_complete_multi_variant = if exact_single_syllable_input {
            None
        } else {
            parse_result
                .as_ref()
                .ok()
                .and_then(|variants| self.preferred_exact_complete_multi_variant(variants))
        };
        // A plain ASCII word can also be a syntactically valid pinyin
        // sequence (for example `wifi` is accepted as `wi` + `fi`).  Do not
        // let that incidental parse suppress the dedicated English-word
        // channel; otherwise words such as `python` disappear completely
        // while `wifi` is misclassified as a Chinese pinyin candidate.
        let english_word_input_intent = self.english_word_input_enabled()
            && is_plain_ascii_word_input(raw, &compact_key)
            && !exact_single_syllable_input
            && exact_complete_multi_variant.is_none();
        if english_word_input_intent {
            preserved_direct_phrases.extend(merge_ascii_completion_candidates(
                raw,
                &compact_key,
                &mut merged,
                true,
            ));
        }
        let exact_complete_short_parse_intent = exact_complete_multi_variant
            .as_ref()
            .map(|variant| variant.syllables.len())
            .filter(|len| (2..=SHORT_HOTWORD_MAX_CHARS).contains(len))
            .or(fallback_exact_complete_short_parse_intent);
        match parse_result {
            Ok(variants) => {
                for (variant_rank, variant) in
                    variants.into_iter().take(MAX_PARSE_VARIANTS).enumerate()
                {
                    let exact_complete_variant = variant.source == ParseVariantSource::Exact
                        && variant.tail.is_none()
                        && !variant.syllables.is_empty()
                        && variant.syllables.join("") == compact_key;
                    if exact_complete_variant && exact_primary_decoded {
                        continue;
                    }
                    let primary_prefix_variant = prefix_primary_decoded
                        && variant.source == ParseVariantSource::Exact
                        && variant.tail.as_ref().is_some_and(|tail| {
                            format!("{}{}", variant.syllables.join(""), tail) == compact_key
                        });
                    if primary_prefix_variant {
                        continue;
                    }
                    if !exact_complete_variant
                        && soft_budget.should_yield_stage("parse", merged.len())
                    {
                        break;
                    }
                    if variant_rank > 0 && soft_budget.should_yield(merged.len()) {
                        break;
                    }
                    if short_ascii_input
                        && compact_key.chars().count() <= 3
                        && matches!(
                            variant.source,
                            ParseVariantSource::KeyboardTypo
                                | ParseVariantSource::SyllableConfusion
                                | ParseVariantSource::PhoneticTypo
                        )
                    {
                        continue;
                    }
                    if short_ascii_input
                        && has_short_compact_phrase_candidates
                        && variant.source == ParseVariantSource::KeyboardTypo
                    {
                        continue;
                    }
                    if has_exact_parse
                        && matches!(
                            variant.source,
                            ParseVariantSource::KeyboardTypo
                                | ParseVariantSource::SyllableConfusion
                                | ParseVariantSource::PhoneticTypo
                        )
                    {
                        continue;
                    }
                    if !correction_enabled
                        && matches!(
                            variant.source,
                            ParseVariantSource::KeyboardTypo
                                | ParseVariantSource::SyllableConfusion
                                | ParseVariantSource::PhoneticTypo
                        )
                    {
                        continue;
                    }
                    let variant_penalty = variant_rank as f64 * VARIANT_PENALTY
                        + correction_variant_source_penalty(variant.source, compact_key.len());
                    let correction_meta = correction_meta_for_parse_variant(raw, &variant);
                    let meta = correction_meta.as_deref();
                    let target = if variant.source == ParseVariantSource::KeyboardTypo {
                        &mut typo_variant_merged
                    } else {
                        &mut merged
                    };
                    match variant.tail {
                        None => {
                            if let Some(lex) = &self.phrase_lexicon {
                                let exact_key = variant.syllables.join("");
                                if let Some(entries) = lex.lookup_pinyin(&exact_key) {
                                    preserved_exact_lexicon_phrases.extend(
                                        entries
                                            .iter()
                                            .filter(|entry| {
                                                phrase_char_count(&entry.phrase)
                                                    == variant.syllables.len()
                                            })
                                            .take(8)
                                            .map(|entry| entry.phrase.clone()),
                                    );
                                    merge_phrase_entries(
                                        target,
                                        entries.iter(),
                                        PHRASE_EXACT_PINYIN_BONUS - variant_penalty,
                                        20,
                                        &self.user_lexicon,
                                        now,
                                        meta.or(Some(PINYIN_CANDIDATE_META)),
                                        Some(lex),
                                        None,
                                    );
                                }
                            }

                            let skip_heavy_decode = !exact_complete_variant
                                && target.len() >= TSF_PAGE_SIZE
                                && soft_budget.should_yield_stage("decode", target.len());
                            if !skip_heavy_decode {
                                let word_graph =
                                    self.decode_word_graph_until(&variant.syllables, now, || false);
                                merge_scored_candidates(
                                    target,
                                    word_graph.into_iter(),
                                    variant_penalty,
                                    &self.user_lexicon,
                                    now,
                                    meta,
                                );

                                let out = self.decode_chars_incremental(
                                    &variant.syllables,
                                    self.effective_decode_beam_for_key(&compact_key),
                                    decode_candidate_limit(variant.syllables.len()),
                                );
                                merge_scored_candidates(
                                    target,
                                    out.into_iter(),
                                    variant_penalty,
                                    &self.user_lexicon,
                                    now,
                                    meta,
                                );
                            }
                        }
                        Some(tail) => {
                            if let Some(lex) = &self.phrase_lexicon {
                                // 词组（特别是成语）前缀加权：必须已完整输入前两个音节，
                                // 且正在输入第 3 个音节（出现 tail）时才显著抬权。
                                // 这样避免仅输入 1-2 个音节时，长词组/成语过早压过常用单字。

                                if variant.syllables.len() >= 2 {
                                    let prefix_key =
                                        format!("{}{}", variant.syllables.join(""), tail);
                                    let entries = self.cached_prefix_pinyin_flat(lex, &prefix_key);
                                    merge_phrase_entries(
                                        target,
                                        entries.iter(),
                                        PHRASE_PREFIX_PINYIN_BONUS - variant_penalty,
                                        18,
                                        &self.user_lexicon,
                                        now,
                                        meta.or(Some(PINYIN_CANDIDATE_META)),
                                        Some(lex),
                                        None,
                                    );
                                }
                            }

                            let preds =
                                self.rank_syllable_completions(&variant.syllables, &tail, now, 40);
                            if preds.is_empty() {
                                if error.is_none() {
                                    error = Some(format!("no syllable starts with {}", tail));
                                }
                                continue;
                            }

                            for (pred_rank, pred) in
                                preds.iter().take(PREDICT_DECODE_TOP).enumerate()
                            {
                                if pred_rank > 0 && soft_budget.should_yield(target.len()) {
                                    break;
                                }
                                let mut full = variant.syllables.clone();
                                full.push(pred.clone());
                                if pred_rank < PREDICT_WORD_GRAPH_TOP {
                                    let lattice =
                                        self.decode_word_graph_until(&full, now, || false);
                                    merge_scored_candidates(
                                        target,
                                        lattice.into_iter(),
                                        variant_penalty + pred_rank as f64 * PREDICTION_PENALTY,
                                        &self.user_lexicon,
                                        now,
                                        meta,
                                    );
                                }
                                let part = self.decode_chars_incremental(
                                    &full,
                                    self.effective_decode_beam_for_key(&compact_key),
                                    decode_candidate_limit(full.len()),
                                );
                                merge_scored_candidates(
                                    target,
                                    part.into_iter(),
                                    variant_penalty + pred_rank as f64 * PREDICTION_PENALTY,
                                    &self.user_lexicon,
                                    now,
                                    meta,
                                );
                            }
                        }
                    }
                }
            }
            Err(ref e) => {
                error = Some(e.clone());
            }
        }
        merge_best_candidate_map(&mut merged, typo_variant_merged);
        if let Some(profiler) = profiler.as_mut() {
            profiler.mark("decode");
        }
        if lookup_request_superseded(request_id) {
            return_lookup_superseded!();
        }
        let decoded_partial_has_long_phrase = merged
            .phrases()
            .any(|phrase| phrase_char_count(phrase) >= partial_min_phrase_chars);
        if soft_budget.should_defer_first_batch_stage("first_batch_decode", merged.len())
            && (decoded_partial_has_long_phrase || merged.len() >= TSF_PAGE_SIZE)
        {
            return self.finish_partial_first_batch(
                &mut merged,
                &compact_key,
                raw,
                now,
                request_id,
                profiler.take(),
                "first_batch_decode",
            );
        }

        // 词库近似纠错：与主通道并行，始终参与合并（由 merge_candidate 与重排决定去留）。

        let user_exact_signal = if compact_key.len() >= 4 {
            self.user_lexicon
                .lookup_input(&compact_key)
                .map(|entries| entries.iter().map(|e| e.freq).sum::<u64>())
                .unwrap_or(0)
        } else {
            0
        };
        let suppress_correction = user_exact_signal >= USER_EXACT_SUPPRESS_CORRECTION_THRESHOLD;

        let mut correction_budget =
            CorrectionWorkBudget::new(request_id, compact_key.chars().count());
        if correction_enabled
            && compact_key.chars().count() <= LONG_INPUT_COMPACT_CHAR_LIMIT
            && !compact_is_exact_single_syllable
            && compact_key.len() >= 4
        {
            let allow_correction =
                correction_prefs.level != CorrectionLevel::Light || !has_exact_parse;
            let candidate_shortage = merged.len() < TSF_PAGE_SIZE;
            let skip_correction_for_budget =
                soft_budget.should_yield_stage("correction", merged.len());
            if allow_correction
                && !has_exact_parse
                && !skip_correction_for_budget
                && !suppress_correction
            {
                self.merge_typo_corrections(
                    &compact_key,
                    &mut merged,
                    now,
                    suppress_correction,
                    &mut correction_budget,
                );
            }
            if allow_correction
                && (!has_exact_parse || candidate_shortage)
                && !skip_correction_for_budget
                && !suppress_correction
            {
                self.merge_missing_letter_pinyin_corrections(
                    &compact_key,
                    &mut merged,
                    now,
                    &mut correction_budget,
                );
            }
        }
        if let Some(profiler) = profiler.as_mut() {
            profiler.mark("correction");
        }
        if correction_budget.was_cancelled() || lookup_request_superseded(request_id) {
            return_lookup_superseded!();
        }

        // 如果无法解析为精确音节，或常规候选不足，仍尽量给出候选：
        // 将 compact_key 按 7 字母分组，从左到右连续拆分做简拼前缀候选合并。

        if !compact_is_exact_single_syllable
            && (merged.len() < TSF_PAGE_SIZE || parse_failed || !has_exact_parse)
            && !compact_key.is_empty()
            && compact_key.len() >= ABBREV_FALLBACK_CHUNK * 2
        {
            self.merge_abbrev_chunk_fallback(&compact_key, &mut merged, now);
        }

        if exact_single_syllable_input {
            let syllable = primary_variant
                .as_ref()
                .filter(|variant| variant.tail.is_none() && variant.syllables.len() == 1)
                .map(|variant| variant.syllables[0].as_str())
                .unwrap_or(compact_key.as_str());
            self.merge_exact_single_syllable_chars(syllable, &mut merged, now);
        }

        let abbrev_phrase_intent =
            infer_short_abbrev_phrase_intent(raw, &compact_key, merged.phrases());
        let input_chars = compact_key.chars().count();
        let has_input_length_phrase_candidate = {
            preserved_short_phrases
                .iter()
                .chain(preserved_daily_short_two_char_phrases.iter())
                .chain(preserved_daily_short_three_char_phrases.iter())
                .chain(preserved_exact_lexicon_phrases.iter())
                .chain(preserved_exact_user_phrases.iter())
                .chain(preserved_high_priority_two_char_phrases.iter())
                .chain(preserved_chat_priority_two_char_phrases.iter())
                .chain(preserved_separator_phrases.iter())
                .any(|phrase| phrase_char_count(phrase) == input_chars)
        };
        let exact_length_phrase_intent = has_input_length_phrase_candidate.then_some(input_chars);
        let inferred_short_phrase_intent = choose_short_phrase_intent([
            (exact_complete_short_parse_intent, 90),
            (exact_length_phrase_intent, 82),
            (abbrev_phrase_intent, 78),
            (mixed_intent_char_count, 70),
            (parse_intent_char_count, 64),
        ]);
        let short_phrase_intent =
            if mixed_direct_cross_length_collision && exact_complete_short_parse_intent.is_none() {
                None
            } else if short_two_char_prefix_intent && !has_input_length_phrase_candidate {
                Some(2)
            } else {
                inferred_short_phrase_intent
            };
        let input_intent = classify_input_intent(
            raw,
            &compact_key,
            direct_input_shortcut,
            self.english_word_input_enabled(),
            exact_single_syllable_input,
            exact_complete_multi_variant.is_some(),
            short_phrase_intent,
            mixed_prefix_intent,
        );
        if !exact_single_syllable_input {
            self.merge_short_abbrev_leading_initial_singles(raw, &compact_key, &mut merged, now);
        }
        if lookup_request_superseded(request_id) {
            return_lookup_superseded!();
        }

        if merged.len() > RERANK_INPUT_PRUNE_TRIGGER {
            let rerank_soft_cap = if compact_key.chars().count() > LONG_INPUT_COMPACT_CHAR_LIMIT {
                LONG_INPUT_RERANK_SOFT_CAP
            } else {
                RERANK_INPUT_SOFT_CAP
            };
            let pruned = prune_merged_candidates_for_rerank(
                &mut merged,
                rerank_soft_cap,
                preserved_direct_phrases
                    .iter()
                    .chain(preserved_short_phrases.iter())
                    .chain(preserved_daily_short_two_char_phrases.iter())
                    .chain(preserved_daily_short_three_char_phrases.iter())
                    .chain(preserved_exact_lexicon_phrases.iter())
                    .chain(preserved_exact_user_phrases.iter())
                    .chain(preserved_pinned_user_phrases.iter())
                    .chain(preserved_high_priority_two_char_phrases.iter())
                    .chain(preserved_chat_priority_two_char_phrases.iter())
                    .chain(preserved_separator_phrases.iter())
                    .chain(final_preferred_phrases.iter()),
            );
            if pruned > 0 {
                if let Some(profiler) = profiler.as_mut() {
                    profiler.mark("pre_rerank_prune");
                }
            }
        }

        let mut ranked = std::mem::take(&mut self.ranked_buf);
        ranked.clear();
        ranked.reserve(merged.len());
        let exact_complete_intent_syllables = exact_complete_multi_variant
            .as_ref()
            .map(|variant| variant.syllables.len())
            .or(exact_complete_short_parse_intent);
        let exact_full_pinyin_score_phrases = exact_complete_intent_syllables
            .map(|syllable_count| {
                self.exact_full_pinyin_phrases(&compact_key, syllable_count)
                    .into_iter()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let explicit_separator_score_phrases = preserved_separator_phrases
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let rerank_prefs = rerank_prefs::get_rerank_prefs();
        for (candidate_index, (phrase, merged_candidate)) in
            merged.drain_candidates().into_iter().enumerate()
        {
            if candidate_index % 32 == 0 && lookup_request_superseded(request_id) {
                self.ranked_buf = ranked;
                return_lookup_superseded!();
            }
            let reranked = merged_candidate.score
                + self.blended_rerank_with_prefs(&phrase, now, rerank_prefs)
                + self.stability_bonus_for_candidate(&compact_key, &phrase)
                + exact_full_pinyin_base_bonus(
                    &phrase,
                    &exact_full_pinyin_score_phrases,
                    &merged_candidate.meta,
                )
                + short_intent_exact_bonus(short_phrase_intent, &phrase)
                + self.abbrev_context_bonus(short_ascii_input, input_chars, &phrase, now)
                + self.short_abbrev_exact_length_rerank_bonus(
                    short_ascii_input,
                    input_chars,
                    &phrase,
                    &merged_candidate.meta,
                )
                + explicit_separator_phrase_bonus(&phrase, &explicit_separator_score_phrases)
                + intent_layer_score_adjustment(
                    input_intent,
                    compact_key.chars().count(),
                    &merged_candidate.meta,
                )
                + primary_lexicon_score_adjustment(
                    &phrase,
                    &merged_candidate.meta,
                    self.phrase_lexicon.as_ref(),
                    input_intent,
                )
                + self.single_syllable_common_char_prior(&phrase, input_intent)
                + mixed_prefix_core_phrase_bonus(input_intent, &phrase, &merged_candidate.meta)
                - abbrev_length_penalty(short_ascii_input, input_chars, &phrase)
                - short_ascii_multi_char_rerank_discount(short_ascii_input, input_chars, &phrase);
            ranked.push(RankedCandidate {
                phrase,
                score: reranked,
                meta: merged_candidate.meta,
            });
        }
        if lookup_request_superseded(request_id) {
            self.ranked_buf = ranked;
            return_lookup_superseded!();
        }
        sort_ranked_candidates_top_k(&mut ranked, TSF_MAX_CANDIDATES * 2);
        if lookup_request_superseded(request_id) {
            self.ranked_buf = ranked;
            return_lookup_superseded!();
        }
        let selection_feedback_applied = self.apply_selection_feedback(&compact_key, &mut ranked);
        if selection_feedback_applied {
            sort_ranked_candidates_top_k(&mut ranked, TSF_MAX_CANDIDATES * 2);
        }
        drop_disabled_english_word_candidates(
            &mut ranked,
            self.english_word_input_enabled(),
            self.phrase_lexicon.as_ref(),
        );
        if exact_single_syllable_input {
            keep_only_single_char_candidates(&mut ranked);
        }
        dedup_preserved_phrase_order(&mut preserved_direct_phrases);
        dedup_preserved_phrase_order(&mut preserved_short_phrases);
        dedup_preserved_phrase_order(&mut preserved_mixed_hot_phrases);
        sort_preserved_phrases_by_lexicon_frequency(
            &mut preserved_short_phrases,
            self.phrase_lexicon.as_ref(),
        );
        dedup_preserved_phrase_order(&mut preserved_high_priority_two_char_phrases);
        sort_preserved_phrases_by_lexicon_frequency(
            &mut preserved_high_priority_two_char_phrases,
            self.phrase_lexicon.as_ref(),
        );
        dedup_preserved_phrase_order(&mut preserved_chat_priority_two_char_phrases);
        sort_preserved_phrases_by_lexicon_frequency(
            &mut preserved_chat_priority_two_char_phrases,
            self.phrase_lexicon.as_ref(),
        );
        dedup_preserved_phrase_order(&mut preserved_daily_short_two_char_phrases);
        sort_preserved_phrases_by_lexicon_frequency(
            &mut preserved_daily_short_two_char_phrases,
            self.phrase_lexicon.as_ref(),
        );
        dedup_preserved_phrase_order(&mut preserved_daily_short_three_char_phrases);
        sort_preserved_phrases_by_lexicon_frequency(
            &mut preserved_daily_short_three_char_phrases,
            self.phrase_lexicon.as_ref(),
        );
        prioritize_preserved_phrase_length(
            &mut preserved_short_phrases,
            compact_key.chars().count(),
        );
        promote_preferred_short_abbrev(&compact_key, &mut preserved_short_phrases);
        dedup_preserved_phrase_order(&mut preserved_exact_lexicon_phrases);
        if let Some(preferred_len) = explicit_separator_preferred_len {
            prioritize_preserved_phrase_length(&mut preserved_exact_lexicon_phrases, preferred_len);
        }
        dedup_preserved_phrase_order(&mut preserved_separator_phrases);
        dedup_preserved_phrase_order(&mut preserved_exact_user_phrases);
        dedup_preserved_phrase_order(&mut preserved_mixed_user_phrases);
        dedup_preserved_phrase_order(&mut preserved_pinned_user_phrases);
        dedup_preserved_phrase_order(&mut final_preferred_phrases);
        if !preserved_pinned_user_phrases.is_empty() {
            final_preferred_phrases.clear();
        }
        let mut applied_multi_syllable_phrase_layout = false;
        if let Some(variant) = &exact_complete_multi_variant {
            // 完整多音节输入优先走「少量高频整词 + 后续首音节单字」。
            // 短键可能同时命中三字简拼和两字全拼，不能只看 primary variant。
            self.apply_multi_syllable_phrase_head_then_first_syllable_singles(
                &mut ranked,
                variant.syllables.len(),
                &variant.syllables,
                &compact_key,
                now,
            );
            applied_multi_syllable_phrase_layout = true;
        }
        if !applied_multi_syllable_phrase_layout {
            if let Some(syllables) = mixed_intent_syllables.as_deref() {
                self.ensure_short_intent_support_candidates(
                    &mut ranked,
                    syllables,
                    &compact_key,
                    now,
                );
            }
        }
        let final_short_phrase_intent = choose_short_phrase_intent([
            (exact_complete_short_parse_intent, 92),
            (short_phrase_intent, 86),
            (
                (!mixed_prefix_intent)
                    .then(|| {
                        mixed_intent_syllables
                            .as_deref()
                            .map(|syllables| syllables.len())
                    })
                    .flatten(),
                72,
            ),
        ]);
        let final_short_phrase_intent = match final_short_phrase_intent {
            Some(intent) if (2..=SHORT_HOTWORD_MAX_CHARS).contains(&intent) => Some(intent),
            _ if compact_key.chars().count() <= 4
                && !has_input_length_phrase_candidate
                && (short_two_char_prefix_intent
                    || ranked
                        .iter()
                        .take(TSF_PAGE_SIZE)
                        .any(|item| phrase_char_count(&item.phrase) == 2)) =>
            {
                Some(2)
            }
            _ => None,
        };
        let mixed_prefix_unbounded_long_input = input_intent == InputIntent::MixedPrefix
            && compact_key.chars().count() >= 4
            && final_short_phrase_intent.is_none();
        let postprocess_short_phrase_intent = if mixed_prefix_unbounded_long_input {
            None
        } else {
            short_phrase_intent
        };
        let postprocess_final_short_phrase_intent = if mixed_prefix_unbounded_long_input {
            None
        } else {
            final_short_phrase_intent
        };
        let skip_final_short_density = direct_input_shortcut
            || should_preserve_ascii_input(raw)
            || mixed_prefix_unbounded_long_input
            || mixed_direct_cross_length_collision;
        let user_hotword_front_limit = user_hotword_prefs::get_user_hotword_prefs().front_limit;
        let exact_guard_syllables = if !direct_input_shortcut
            && !should_preserve_ascii_input(raw)
            && preserved_pinned_user_phrases.is_empty()
            && !exact_single_syllable_input
        {
            exact_complete_intent_syllables
        } else {
            None
        };
        let overlong_pinned_exact_guard_syllables = if !direct_input_shortcut
            && !should_preserve_ascii_input(raw)
            && !preserved_pinned_user_phrases.is_empty()
            && !exact_single_syllable_input
        {
            exact_complete_intent_syllables
                .filter(|count| (2..=SHORT_HOTWORD_MAX_CHARS).contains(count))
        } else {
            None
        };
        if lookup_request_superseded(request_id) {
            self.ranked_buf = ranked;
            self.merged_buf = merged;
            if let Some(profiler) = profiler {
                profiler.finish(0);
            }
            return (Vec::new(), Some("lookup superseded".to_string()));
        }

        self.apply_candidate_postprocess(
            &mut ranked,
            CandidatePostprocessContext {
                raw,
                compact_key: &compact_key,
                correction_prefs,
                suppress_correction,
                short_phrase_intent: postprocess_short_phrase_intent,
                final_short_phrase_intent: postprocess_final_short_phrase_intent,
                input_intent,
                exact_guard_syllables,
                overlong_pinned_exact_guard_syllables,
                skip_final_short_density,
                exact_single_syllable_input,
                applied_multi_syllable_phrase_layout,
                direct_input_shortcut,
                user_hotword_front_limit,
                preserved_direct_phrases: &preserved_direct_phrases,
                preserved_short_phrases: &preserved_short_phrases,
                preserved_daily_short_two_char_phrases: &preserved_daily_short_two_char_phrases,
                preserved_daily_short_three_char_phrases: &preserved_daily_short_three_char_phrases,
                preserved_exact_lexicon_phrases: &preserved_exact_lexicon_phrases,
                preserved_exact_user_phrases: &preserved_exact_user_phrases,
                preserved_mixed_user_phrases: &preserved_mixed_user_phrases,
                preserved_pinned_user_phrases: &preserved_pinned_user_phrases,
                preserved_high_priority_two_char_phrases: &preserved_high_priority_two_char_phrases,
                preserved_chat_priority_two_char_phrases: &preserved_chat_priority_two_char_phrases,
                preserved_separator_phrases: &preserved_separator_phrases,
                final_preferred_phrases: &final_preferred_phrases,
            },
        );
        stabilize_top1_by_confidence(
            &mut ranked,
            &exact_full_pinyin_score_phrases,
            exact_guard_syllables.or(overlong_pinned_exact_guard_syllables),
            input_intent,
        );
        if !direct_input_shortcut && !ranked.first().is_some_and(|item| item.meta.pinned) {
            promote_ranked_high_priority_two_char_candidates(
                &mut ranked,
                raw,
                &compact_key,
                self.syllables.as_ref(),
                self.phrase_lexicon.as_ref(),
            );
        }
        demote_corrections_below_priority_hits(
            &mut ranked,
            &exact_full_pinyin_score_phrases,
            exact_guard_syllables.or(overlong_pinned_exact_guard_syllables),
        );
        if !has_exact_parse {
            ensure_typo_correction_visible_after_priority_hits(
                &mut ranked,
                correction_prefs,
                suppress_correction,
                &exact_full_pinyin_score_phrases,
                exact_guard_syllables.or(overlong_pinned_exact_guard_syllables),
            );
            drop_single_char_corrections_for_two_char_intent(
                &mut ranked,
                (postprocess_final_short_phrase_intent == Some(2)
                    || exact_guard_syllables == Some(2)
                    || overlong_pinned_exact_guard_syllables == Some(2))
                .then_some(2),
            );
        }
        if explicit_separator_preferred_len.is_some() {
            promote_preserved_candidates(&mut ranked, &preserved_separator_phrases);
        }
        promote_preserved_candidates(&mut ranked, &final_preferred_phrases);
        drop_disabled_english_word_candidates(
            &mut ranked,
            self.english_word_input_enabled(),
            self.phrase_lexicon.as_ref(),
        );
        limit_english_candidates_for_non_english_intent(&mut ranked, input_intent);
        let final_negative_feedback_applied =
            self.apply_final_negative_selection_feedback(&compact_key, &mut ranked);
        // Candidate-layout policies run after the first feedback sort and may
        // restore the static lexicon order. Re-sort once at the end whenever
        // any per-reading feedback was applied so repeated choices and weak
        // top-two misses remain authoritative on both cold and cached paths.
        if selection_feedback_applied || final_negative_feedback_applied {
            sort_ranked_candidates_top_k(&mut ranked, TSF_MAX_CANDIDATES * 2);
        }

        // The dedicated 8,105-character list is the authoritative default
        // order for single-character input. Apply it after general reranking
        // so phrase/English frequency scales, LM counts and pronunciation
        // aliases cannot rewrite that order. Explicit user priority and
        // learned per-reading feedback remain in control.
        if !self.has_selection_feedback_for_reading(&compact_key) {
            let common_full_pinyin_single = exact_single_syllable_input
                || (compact_key.chars().count() == 1 && self.syllables.contains(&compact_key));
            let common_order = if common_full_pinyin_single {
                self.single_char_common.pinyin_order(&compact_key).to_vec()
            } else if self.jianpin_enabled()
                && raw.chars().count() == 1
                && compact_key.chars().count() == 1
                && raw.chars().all(|ch| ch.is_ascii_alphabetic())
            {
                compact_key
                    .chars()
                    .next()
                    .map(|initial| self.single_char_common.initial_order(initial).to_vec())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if !common_order.is_empty() {
                promote_common_single_chars_after_user_priority(
                    &mut ranked,
                    &common_order,
                    common_full_pinyin_single,
                );
            }
        }

        if exact_single_syllable_input {
            keep_only_single_char_candidates(&mut ranked);
        }
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            apply_traditional_output(&mut ranked);
            for phrase in &mut preserved_mixed_hot_phrases {
                *phrase = crate::traditional::to_traditional(phrase);
            }
            dedup_preserved_phrase_order(&mut preserved_mixed_hot_phrases);
        }
        self.promote_confident_top1_over_stability(&compact_key, &mut ranked);
        // Explicit high-confidence routing exceptions (for example fengh →
        // 奉化) are stronger evidence than the generic anti-stickiness pass.
        // Reapply them after stability has compared raw scores.
        promote_preserved_candidates(&mut ranked, &final_preferred_phrases);

        // Priority, feedback and stability passes above may move a prediction
        // back into the visible page after the main postprocess has already
        // shaped it. Reapply a non-destructive visible-page guard here: normal
        // overlong predictions move behind the page, while one explicit pinned
        // candidate may remain as a user-controlled exception.
        let visible_short_intent = (!skip_final_short_density)
            .then(|| {
                exact_guard_syllables
                    .or(overlong_pinned_exact_guard_syllables)
                    .or(postprocess_final_short_phrase_intent)
                    .filter(|intent| matches!(intent, 2 | 3))
            })
            .flatten();
        if let Some(intent_chars) = visible_short_intent {
            let page_size =
                candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
            enforce_short_intent_visible_page(&mut ranked, intent_chars, page_size);
        }
        // Final priority promotions (including explicit correction routes such
        // as bejing -> 北京) must still respect learned user history.
        demote_corrections_below_priority_hits(
            &mut ranked,
            &exact_full_pinyin_score_phrases,
            exact_guard_syllables.or(overlong_pinned_exact_guard_syllables),
        );
        if !self.has_selection_feedback_for_reading(&compact_key) {
            promote_first_preserved_after_user_priority(&mut ranked, &preserved_mixed_hot_phrases);
        }

        if strong_long_mixed_composition
            && ranked
                .first()
                .is_some_and(|item| item.meta.source == CandidateSource::Mixed)
        {
            let visible_floor = ranked[0].score - MIXED_PINYIN_STRONG_COMPOSITION_VISIBLE_GAP;
            let top_phrase = ranked[0].phrase.clone();
            ranked.retain(|item| {
                item.phrase == top_phrase
                    || item.meta.source != CandidateSource::Mixed
                    || item.score >= visible_floor
            });
        }

        if datetime_collision_shortcut {
            // Reserve one visible slot for the shortcut without allowing its
            // multiple formatting variants to evict ordinary abbreviation
            // candidates. The fourth slot keeps the common-word Top3 intact.
            self.reserve_datetime_collision_slot(raw, &mut ranked);
        }

        // Several late system-priority passes run after the main postprocess.
        // Reassert learned exact-input intent at the presentation boundary so
        // a phrase explicitly assembled by the user is visible and frontmost
        // on the very next lookup. Pinned/direct candidates remain anchors.
        let final_exact_len_guard = exact_guard_syllables.or(overlong_pinned_exact_guard_syllables);
        let final_exact_user_phrases = preserved_exact_user_phrases
            .iter()
            .filter(|phrase| {
                final_exact_len_guard.is_none_or(|guard| phrase_char_count(phrase) == guard)
            })
            .cloned()
            .collect::<Vec<_>>();
        promote_exact_user_hotwords_front(
            &mut ranked,
            &final_exact_user_phrases,
            final_exact_len_guard,
            user_hotword_front_limit,
        );

        let strict_frequency_phrase_len = match input_intent {
            InputIntent::FullPinyin => exact_complete_intent_syllables,
            InputIntent::ShortAbbrev if jianpin_enabled => Some(compact_key.chars().count()),
            _ => None,
        };
        if !self.has_selection_feedback_for_reading(&compact_key) {
            if let Some(phrase_len) = strict_frequency_phrase_len {
                enforce_strict_system_lexicon_frequency_order(
                    &mut ranked,
                    &compact_key,
                    input_intent,
                    phrase_len,
                    self.phrase_lexicon.as_ref(),
                );
            }
        }
        demote_cold_non_full_lexicon_candidates(
            &mut ranked,
            input_intent,
            self.phrase_lexicon.as_ref(),
        );

        let lookup_was_limited = soft_budget.was_limited();
        if lookup_was_limited {
            if let Some(first) = ranked.first_mut() {
                first.meta = first.meta.clone().with_partial();
            }
            if let Some(stage) = soft_budget.limited_stage() {
                if let Some(profiler) = profiler.as_mut() {
                    profiler.mark(stage);
                }
            }
        }

        if ranked.len() > LOOKUP_FULL_MAX_CANDIDATES {
            ranked.truncate(LOOKUP_FULL_MAX_CANDIDATES);
        }
        if let Some(profiler) = profiler.as_mut() {
            profiler.mark("rerank_sort");
        }

        if ranked.is_empty() {
            let fallback = raw.to_string();
            let out = vec![RankedCandidate {
                phrase: fallback,
                score: 0.0,
                meta: CandidateMeta::legacy(ASCII_DIRECT_META),
            }];
            self.last_lookup = lookup_stability_state_from_candidates(&compact_key, &out);
            if !lookup_was_limited && !soft_budget.was_cancelled() {
                self.complete_full_lookup_retry(raw);
            }
            self.ranked_buf = ranked;
            self.merged_buf = merged;
            if let Some(profiler) = profiler {
                profiler.finish(out.len());
            }
            (
                out,
                Some(error.unwrap_or_else(|| "no candidates".to_string())),
            )
        } else {
            let out = std::mem::take(&mut ranked);
            let out_len = out.len();
            let mut base_out = None;
            if !lookup_was_limited
                && !soft_budget.was_cancelled()
                && (short_cache_key.is_some() || final_cache_key.is_some())
            {
                let mut cached_base = out.clone();
                self.remove_context_rerank_before_cache(&compact_key, &mut cached_base);
                base_out = Some(cached_base);
            }
            // 记录冷路径是否对二字意图应用了完整分页编排，缓存命中路径据此
            // 重放同样的编排，保持两条路径的页面结构一致。
            let two_char_density_replay = if !skip_final_short_density
                && preserved_pinned_user_phrases.is_empty()
                && postprocess_final_short_phrase_intent == Some(2)
            {
                Some(if input_intent == InputIntent::FullPinyin {
                    TwoCharDensityReplay::FullPinyin
                } else {
                    TwoCharDensityReplay::Minimum
                })
            } else {
                None
            };
            self.last_lookup = lookup_stability_state_from_candidates(&compact_key, &out);
            if !lookup_was_limited && !soft_budget.was_cancelled() {
                self.complete_full_lookup_retry(raw);
            }
            if let Some(base_out) = base_out {
                if let Some(cache_key) = short_cache_key {
                    self.short_lookup_cache.insert(
                        cache_key,
                        base_out.clone(),
                        visible_short_intent,
                        two_char_density_replay,
                    );
                }
                if let Some(cache_key) = final_cache_key {
                    self.final_lookup_cache.insert(
                        cache_key,
                        base_out,
                        visible_short_intent,
                        two_char_density_replay,
                    );
                }
            }
            self.ranked_buf = ranked;
            self.merged_buf = merged;
            if let Some(profiler) = profiler {
                profiler.finish(out_len);
            }
            (out, None)
        }
    }
}

impl Default for PinyinEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn compact_lookup_key(s: &str) -> String {
    let compact: String = s
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    normalize_compact_pinyin_aliases(&compact)
}

pub(super) fn load_phrase_lexicon(dir: &Path) -> Option<AbbrevLexicon> {
    load_phrase_lexicon_core(dir).ok()
}

pub(super) fn load_hot_phrase_lexicon(dir: &Path) -> Option<AbbrevLexicon> {
    load_hot_phrase_lexicon_core(dir).ok()
}

fn load_single_char_common_index(dir: &Path) -> Arc<single_char_common::SingleCharCommonIndex> {
    match single_char_common::SingleCharCommonIndex::from_lexicon_dir(dir) {
        Ok(index) => Arc::new(index),
        Err(error) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "single_char_common_load_failed",
                format!("dir={} error={error}", dir.display()),
            );
            Arc::default()
        }
    }
}

pub(super) fn normalize_existing_path(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return None;
    }
    std::fs::canonicalize(path)
        .ok()
        .or_else(|| Some(path.to_path_buf()))
}

#[cfg(windows)]
pub(super) fn current_module_dir() -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    let mut module: HMODULE = 0;
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let ok = unsafe {
        GetModuleHandleExW(
            flags,
            current_module_dir as *const () as usize as *const u16,
            &mut module,
        )
    };
    if ok == 0 || module == 0 {
        return None;
    }

    let mut buf = vec![0u16; 260];
    loop {
        let len =
            unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if len == 0 {
            return None;
        }
        if len + 1 < buf.len() {
            let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
            return path.parent().map(Path::to_path_buf);
        }
        if buf.len() >= 32 * 1024 {
            return None;
        }
        buf.resize(buf.len() * 2, 0);
    }
}

#[cfg(not(windows))]
pub(super) fn current_module_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub(super) fn trusted_user_lexicon_root() -> Option<PathBuf> {
    crate::app_paths::local_data_dir().map(|dir| dir.join("lexicon"))
}

pub(super) fn known_app_path_names() -> &'static [&'static str] {
    &["kaixin"]
}

pub(super) fn known_install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push_unique = |root: PathBuf| {
        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root);
        }
    };

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let programs = PathBuf::from(local_app_data).join("Programs");
        for app_name in known_app_path_names() {
            push_unique(programs.join(app_name));
        }
    }

    for env_name in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env_name) {
            let base = PathBuf::from(base);
            for app_name in known_app_path_names() {
                push_unique(base.join(app_name));
            }
        }
    }

    roots
}

pub(super) fn development_lexicon_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut push = |path: PathBuf| {
        if path.is_dir() && !dirs.iter().any(|existing| existing == &path) {
            dirs.push(path);
        }
    };
    if let Ok(cwd) = std::env::current_dir() {
        push(cwd.join("lexicon"));
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest = PathBuf::from(manifest_dir);
        push(manifest.join("..").join("lexicon"));
    }
    dirs
}

pub(super) fn is_path_within(path: &Path, root: &Path) -> bool {
    let Some(path) = normalize_existing_path(path) else {
        return false;
    };
    let Some(root) = normalize_existing_path(root) else {
        return false;
    };
    path.starts_with(root)
}

pub(super) fn find_trusted_lexicon_under(root: &Path) -> Option<PathBuf> {
    let lexicon_dir = root.join("lexicon");
    if lexicon_dir.is_dir() {
        return Some(lexicon_dir);
    }
    if root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case("lexicon"))
        .unwrap_or(false)
    {
        return Some(root.to_path_buf());
    }
    None
}

pub(crate) fn validate_trusted_phrase_dir(path: &Path) -> Option<PathBuf> {
    let path = normalize_existing_path(path)?;
    let module_dir = current_module_dir();
    let user_root = trusted_user_lexicon_root();
    let trusted = module_dir
        .into_iter()
        .chain(user_root)
        .chain(known_install_roots())
        .filter_map(|root| find_trusted_lexicon_under(&root))
        .chain(development_lexicon_dirs())
        .any(|root| is_path_within(&path, &root));
    trusted.then_some(path)
}

/// 与拼音引擎默认词库目录一致（安装目录 `lexicon/`、`%LOCALAPPDATA%\kaixin\lexicon` 等），供设置程序枚举分项词库。

pub fn default_phrase_lexicon_dir() -> Option<PathBuf> {
    trusted_default_phrase_dir()
}

pub(super) fn trusted_default_phrase_dir() -> Option<PathBuf> {
    let module_dir = current_module_dir();
    let user_root = trusted_user_lexicon_root();

    if let Ok(path) = std::env::var("SRF_LEXICON_DIR") {
        if let Some(trusted) = validate_trusted_phrase_dir(Path::new(&path)) {
            return Some(trusted);
        }
    }

    for path in development_lexicon_dirs() {
        if path.is_dir() {
            return Some(path);
        }
    }

    for root in module_dir
        .into_iter()
        .chain(user_root)
        .chain(known_install_roots())
    {
        if let Some(path) = find_trusted_lexicon_under(&root) {
            return Some(path);
        }
    }
    None
}

/// 缓存命中路径重放冷路径对二字意图的完整分页编排。冷路径在
/// postprocess 内依次应用 `apply_short_intent_final_candidate_density` 再
/// `apply_two_char_intent_page_density`/`apply_two_char_intent_minimum_page_density`;
/// 此处按同样的顺序重放，避免热路径上二字词退化为整块 exact_later。
fn replay_two_char_intent_density(
    cached: &mut Vec<RankedCandidate>,
    replay: Option<TwoCharDensityReplay>,
    intent_chars: usize,
    page_size: usize,
) {
    if intent_chars != 2 {
        return;
    }
    match replay {
        Some(TwoCharDensityReplay::FullPinyin) => {
            arrange_two_char_intent_page_density(cached, page_size);
        }
        Some(TwoCharDensityReplay::Minimum) => {
            arrange_two_char_intent_minimum_page_density(cached, page_size);
        }
        None => {}
    }
}

#[cfg(test)]
#[path = "lookup_tests.rs"]
mod tests;

#[path = "lookup/corrections.rs"]
mod corrections;
#[path = "lookup/facade.rs"]
mod facade;
#[path = "lookup/mixed.rs"]
mod mixed;
