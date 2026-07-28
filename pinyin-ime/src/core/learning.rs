use super::*;

fn selection_feedback_decay(now: u64, last_used: u64, half_life_ticks: f64) -> f64 {
    let age = now.saturating_sub(last_used) as f64;
    2.0f64.powf(-age / half_life_ticks.max(1.0))
}

pub(super) fn user_lexicon_disk_stamp() -> UserLexiconDiskStamp {
    fn file_stamp(path: &Path) -> (Option<SystemTime>, Option<u64>) {
        match std::fs::metadata(path) {
            Ok(meta) => (meta.modified().ok(), Some(meta.len())),
            Err(_) => (None, None),
        }
    }

    let path = default_user_dict_path();
    let (dict_mtime, dict_len) = file_stamp(&path);
    let reset_stamp = user_dict_reset_stamp(&path);
    UserLexiconDiskStamp {
        dict_mtime,
        dict_len,
        reset_mtime: reset_stamp.modified,
        reset_len: reset_stamp.len,
    }
}

static USER_LEXICON_GENERATION: AtomicU64 = AtomicU64::new(1);
static USER_LEXICON_WATCHER: OnceLock<()> = OnceLock::new();
static USER_LEXICON_OBSERVED_STAMP: OnceLock<Mutex<UserLexiconDiskStamp>> = OnceLock::new();
static USER_LEXICON_PREPARED_RELOAD: OnceLock<Mutex<Option<PreparedUserLexiconReload>>> =
    OnceLock::new();

struct PreparedUserLexiconReload {
    stamp: UserLexiconDiskStamp,
    lexicon: UserLexicon,
}

fn observed_user_lexicon_stamp() -> &'static Mutex<UserLexiconDiskStamp> {
    USER_LEXICON_OBSERVED_STAMP.get_or_init(|| Mutex::new(UserLexiconDiskStamp::default()))
}

fn prepared_user_lexicon_reload() -> &'static Mutex<Option<PreparedUserLexiconReload>> {
    USER_LEXICON_PREPARED_RELOAD.get_or_init(|| Mutex::new(None))
}

fn retire_user_lexicon(lexicon: UserLexicon) {
    static RETIRE_TX: OnceLock<std::sync::mpsc::Sender<UserLexicon>> = OnceLock::new();
    let tx = RETIRE_TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<UserLexicon>();
        let _ = std::thread::Builder::new()
            .name("kaixin-user-dict-retire".to_string())
            .spawn(move || {
                while let Ok(lexicon) = rx.recv() {
                    drop(lexicon);
                }
            });
        tx
    });
    if let Err(err) = tx.send(lexicon) {
        // The retire thread is unavailable; preserving correctness is more
        // important than leaking the old persistence worker.
        drop(err.0);
    }
}

pub(super) fn start_user_lexicon_watcher(initial: UserLexiconDiskStamp) -> u64 {
    if let Ok(mut observed) = observed_user_lexicon_stamp().lock() {
        *observed = initial;
    }
    USER_LEXICON_WATCHER.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("kaixin-user-dict-watch".to_string())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_millis(800));
                let next = user_lexicon_disk_stamp();
                let changed = observed_user_lexicon_stamp()
                    .lock()
                    .map(|observed| *observed != next)
                    .unwrap_or(false);
                if !changed {
                    continue;
                }

                // Parse external changes away from the interactive lookup
                // thread. Publish the generation only after the replacement is
                // ready, so the next key only performs a cheap move/swap.
                let lexicon = UserLexicon::load_default();
                let loaded_stamp = user_lexicon_disk_stamp();
                if loaded_stamp != next {
                    continue;
                }
                if let Ok(mut prepared) = prepared_user_lexicon_reload().lock() {
                    *prepared = Some(PreparedUserLexiconReload {
                        stamp: next.clone(),
                        lexicon,
                    });
                }
                if let Ok(mut observed) = observed_user_lexicon_stamp().lock() {
                    *observed = next;
                }
                USER_LEXICON_GENERATION.fetch_add(1, Ordering::AcqRel);
            });
    });
    USER_LEXICON_GENERATION.load(Ordering::Acquire)
}

pub(super) fn notify_user_lexicon_changed(stamp: UserLexiconDiskStamp) -> u64 {
    // An in-process learning operation already updated the live UserLexicon.
    // Teach the watcher about its new disk stamp so it does not report the
    // same write as an external change 800ms later and force a full reload.
    if let Ok(mut observed) = observed_user_lexicon_stamp().lock() {
        *observed = stamp;
    }
    let stale = prepared_user_lexicon_reload()
        .lock()
        .ok()
        .and_then(|mut prepared| prepared.take());
    if let Some(stale) = stale {
        retire_user_lexicon(stale.lexicon);
    }
    USER_LEXICON_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
}

#[derive(Clone, Copy)]
struct LearnCommitProfile {
    exact_delta: u64,
    composed_observe_threshold: u64,
    source: CommitSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LearningSensitivity {
    Conservative,
    Standard,
    Aggressive,
}

fn log_learning_aux_error(event: &str, err: &std::io::Error) {
    crate::runtime_log::log_engine(
        crate::runtime_log::RuntimeLogLevel::Error,
        event,
        format!(
            "auxiliary user dictionary learning failed: kind={:?}",
            err.kind()
        ),
    );
}

impl PinyinEngine {
    pub fn learn_commit(&mut self, reading: &str, phrase: &str) -> Result<(), String> {
        self.learn_commit_with_flags(reading, phrase, 0)
    }

    pub fn reset_learning_context(&mut self) {
        self.user_lexicon.clear_context_history();
        self.clear_user_sensitive_lookup_caches();
        self.last_lookup = None;
    }

    pub fn learn_commit_with_flags(
        &mut self,
        reading: &str,
        phrase: &str,
        flags: u32,
    ) -> Result<(), String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(());
        }
        if flags & LEARN_FLAG_NO_LEARN != 0 {
            self.user_lexicon.clear_context_history();
            return Ok(());
        }
        let phrase = phrase.trim();
        let key = compact_lookup_key(reading);
        if key.is_empty() || phrase.is_empty() {
            return Ok(());
        }
        if self.is_effective_direct_input_shortcut(&key) {
            return Ok(());
        }
        if is_learnable_single_user_char(phrase) {
            self.reload_user_lexicon_if_changed_now();
            let delta = self.single_user_char_delta(flags);
            self.user_lexicon
                .learn_exact_input_without_phrase_signal(&key, phrase, delta)
                .map_err(|e| format!("save single-char user dict: {}", e))?;
            self.note_user_lexicon_updated();
            return Ok(());
        }
        if !is_learnable_user_phrase(phrase) {
            return Ok(());
        }
        self.reload_user_lexicon_if_changed_now();
        let weak_learning = flags & LEARN_FLAG_WEAK != 0;
        let composed_phrase = flags & LEARN_FLAG_COMPOSED_PHRASE != 0;
        let auto_novel = self.auto_novel_phrase_parts(&key, reading, phrase);
        let full_pinyin_learning_syllables = self.full_pinyin_learning_syllables(reading, &key);
        let profile = self.learn_commit_profile(
            flags,
            phrase,
            auto_novel.is_some(),
            full_pinyin_learning_syllables.as_deref(),
        );
        let mut composed_observation = None;
        if composed_phrase {
            if !(2..=AUTO_NOVEL_MAX_CHARS).contains(&phrase_char_count(phrase)) {
                return Ok(());
            }
            let observed = self
                .user_lexicon
                .observe_input(&key, phrase)
                .map_err(|e| format!("observe user dict: {}", e))?;
            if observed.freq < profile.composed_observe_threshold && auto_novel.is_none() {
                self.note_user_lexicon_updated();
                return Ok(());
            }
            composed_observation = Some(observed);
        }
        if full_pinyin_learning_syllables.is_none() && is_mixed_pinyin_learning_key(&key, phrase) {
            self.user_lexicon
                .learn_mixed_input_with_delta(&key, phrase, 1)
                .map_err(|e| format!("save mixed user dict: {}", e))?;
            self.user_lexicon
                .record_commit_observation(&key, phrase, CommitSource::Mixed)
                .map_err(|e| format!("observe mixed commit: {}", e))?;
            self.note_user_lexicon_updated();
            return Ok(());
        }

        // 多字词更依赖学习提权：对完整拼写给一点额外增量。

        if let Some(observed) = composed_observation {
            self.user_lexicon
                .promote_observed_input_with_source_and_delta(
                    &key,
                    phrase,
                    profile.exact_delta,
                    profile.source,
                    observed,
                )
                .map_err(|e| format!("promote observed user dict: {}", e))?;
        } else {
            self.user_lexicon
                .learn_with_source_and_delta(&key, phrase, profile.exact_delta, profile.source)
                .map_err(|e| format!("save user dict: {}", e))?;
        }

        if let Some(novel_parts) = auto_novel {
            if let Err(err) = self.user_lexicon.record_novel_phrase(&key, phrase) {
                log_learning_aux_error("record_novel_phrase", &err);
            }
            for (sub_key, sub_phrase) in novel_parts {
                if let Err(err) =
                    self.user_lexicon
                        .learn_input_only_with_delta(&sub_key, &sub_phrase, 1)
                {
                    log_learning_aux_error("learn_novel_subphrase", &err);
                }
            }
        }

        if (2..=4).contains(&phrase_char_count(phrase))
            && crate::thuocl::mixed_pinyin_keys_for_phrase(phrase)
                .iter()
                .any(|mixed_key| mixed_key == &key)
        {
            self.user_lexicon
                .learn_input_only_with_delta(&key, phrase, 2)
                .map_err(|e| format!("save user dict: {}", e))?;
        }

        // 让“缩写也能打出常用词”：如果 reading 可解析为多个完整音节，则额外学习其首字母串（如 bei jing -> bj）。

        if let Some(syllables) = full_pinyin_learning_syllables.as_ref() {
            if syllables.len() >= 2 {
                let mut abbrev = String::with_capacity(syllables.len());
                for syl in syllables {
                    if let Some(ch) = syl.chars().next() {
                        if ch.is_ascii_alphabetic() {
                            abbrev.push(ch.to_ascii_lowercase());
                        }
                    }
                }
                if !abbrev.is_empty()
                    && abbrev != key
                    && !self.is_effective_direct_input_shortcut(&abbrev)
                {
                    // 缩写学习力度比全拼弱一点，避免压过明确的短语/单字输入意图。

                    if let Err(err) = self
                        .user_lexicon
                        .learn_input_only_with_delta(&abbrev, phrase, 1)
                    {
                        log_learning_aux_error("learn_abbrev", &err);
                    }
                    if (2..=SHORT_HOTWORD_MAX_CHARS).contains(&syllables.len()) {
                        if let Err(err) = self
                            .user_lexicon
                            .learn_input_only_with_delta(&abbrev, phrase, 1)
                        {
                            log_learning_aux_error("learn_abbrev_boost", &err);
                        }
                    }
                }
                let mixed_delta = if weak_learning {
                    1
                } else {
                    USER_MIXED_INPUT_MIN_FREQ
                };
                for mixed_key in mixed_pinyin_keys_from_syllables(syllables, 64) {
                    if mixed_key != key
                        && mixed_key != abbrev
                        && !self.is_effective_direct_input_shortcut(&mixed_key)
                    {
                        if let Err(err) = self.user_lexicon.learn_mixed_input_with_delta(
                            &mixed_key,
                            phrase,
                            mixed_delta,
                        ) {
                            log_learning_aux_error("learn_syllable_mixed", &err);
                        }
                    }
                }
            }
        }

        if full_pinyin_learning_syllables.is_some() {
            let mixed_delta = if weak_learning {
                1
            } else {
                USER_MIXED_INPUT_MIN_FREQ
            };
            for mixed_key in crate::thuocl::mixed_pinyin_keys_for_phrase(phrase) {
                if mixed_key != key && !self.is_effective_direct_input_shortcut(&mixed_key) {
                    if let Err(err) = self.user_lexicon.learn_mixed_input_with_delta(
                        &mixed_key,
                        phrase,
                        mixed_delta,
                    ) {
                        log_learning_aux_error("learn_phrase_mixed", &err);
                    }
                }
            }
        }

        self.note_user_lexicon_updated();
        Ok(())
    }

    // Convert commit source and confidence into concrete learning strength.
    fn learn_commit_profile(
        &self,
        flags: u32,
        phrase: &str,
        auto_novel: bool,
        full_pinyin_syllables: Option<&[String]>,
    ) -> LearnCommitProfile {
        let weak = flags & LEARN_FLAG_WEAK != 0;
        let composed = flags & LEARN_FLAG_COMPOSED_PHRASE != 0;
        let chars = phrase_char_count(phrase);
        let full_exact =
            full_pinyin_syllables.is_some_and(|syllables| syllables.len() == chars && chars >= 2);

        let source = if weak {
            CommitSource::Weak
        } else if composed {
            CommitSource::Composed
        } else {
            CommitSource::Direct
        };
        let mut exact_delta = if weak || composed { 1 } else { 2 };
        if !weak && full_exact && chars >= 4 {
            exact_delta += 1;
        }
        if auto_novel && (4..=AUTO_NOVEL_MAX_CHARS).contains(&chars) {
            exact_delta += 1;
        }

        let mut composed_observe_threshold: u64 = if chars <= 2 { 3 } else { 2 };
        if auto_novel && chars >= 4 {
            composed_observe_threshold = 1;
        } else if full_exact && chars >= 4 {
            composed_observe_threshold = 2;
        }
        match self.learning_sensitivity() {
            LearningSensitivity::Aggressive => {
                if composed {
                    composed_observe_threshold = 1;
                    exact_delta += 1;
                }
            }
            LearningSensitivity::Conservative => {
                if composed {
                    composed_observe_threshold = composed_observe_threshold.saturating_add(1);
                }
            }
            // 逐字选出的完整词组是明确意图，不应在默认模式下继续作为
            // “观察词”等待多次。保守模式仍保留额外确认门槛。
            LearningSensitivity::Standard => {
                if composed {
                    composed_observe_threshold = 1;
                }
            }
        }

        LearnCommitProfile {
            exact_delta,
            composed_observe_threshold,
            source,
        }
    }

    fn learning_sensitivity(&self) -> LearningSensitivity {
        if self.mode_flags & MODE_LEARNING_AGGRESSIVE != 0 {
            LearningSensitivity::Aggressive
        } else if self.mode_flags & MODE_LEARNING_CONSERVATIVE != 0 {
            LearningSensitivity::Conservative
        } else {
            LearningSensitivity::Standard
        }
    }

    fn single_user_char_delta(&self, flags: u32) -> u64 {
        if flags & LEARN_FLAG_WEAK != 0 {
            return 1;
        }
        match self.learning_sensitivity() {
            LearningSensitivity::Conservative => 1,
            LearningSensitivity::Standard => 2,
            LearningSensitivity::Aggressive => 3,
        }
    }

    /// Learn one correction pair after the user commits a correction candidate.
    pub fn learn_correction(&mut self, raw: &str, corrected: &str) -> Result<(), String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(());
        }
        let raw = raw.trim();
        let corrected = corrected.trim();
        if raw.is_empty() || corrected.is_empty() {
            return Ok(());
        }
        self.reload_user_lexicon_if_changed_now();
        self.user_lexicon
            .learn_correction(raw, corrected)
            .map_err(|e| format!("save correction pair: {}", e))?;
        self.note_user_lexicon_updated();
        Ok(())
    }

    /// 记录候选交互反馈：用户从非首项选择、翻页后选择或置顶候选时，
    /// 把该选择信号持久化到用户词库，用于后续同输入下的候选排序加权。
    pub fn learn_selection_feedback(
        &mut self,
        reading: &str,
        phrase: &str,
        selected_index: usize,
        page: usize,
        skipped_candidates: &[String],
    ) -> Result<(), String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(());
        }
        let key = compact_lookup_key(reading);
        let phrase = phrase.trim();
        if key.is_empty() || phrase.is_empty() {
            return Ok(());
        }
        if !is_learnable_user_phrase(phrase) {
            return Ok(());
        }

        let context_prev = self
            .user_lexicon
            .preceding_commits_for_context()
            .into_iter()
            .find(|prev| *prev != phrase)
            .map(str::to_string);

        let mut adjustments: Vec<(String, i64)> = Vec::new();
        if selected_index != 0 || page != 0 {
            let selected_delta = SELECTION_SELECTED_BASE_DELTA
                + selected_index.min(TSF_PAGE_SIZE * 2) as i64
                + page.min(8) as i64 * SELECTION_PAGE_SELECTED_DELTA
                + if phrase_char_count(phrase) == 4 {
                    FOUR_CHAR_SELECTION_EXTRA_DELTA
                } else {
                    0
                };
            adjustments.push((phrase.to_string(), selected_delta));
        }

        let mut weak_unselected: Vec<(String, i64)> = Vec::new();
        let mut seen_skipped = HashSet::new();
        let previous_page_cutoff = page.saturating_mul(TSF_PAGE_SIZE);
        for (rank, skipped) in skipped_candidates.iter().enumerate() {
            let skipped = skipped.trim();
            if skipped.is_empty() || skipped == phrase || !seen_skipped.insert(skipped.to_string())
            {
                continue;
            }
            if !is_learnable_user_phrase(skipped) {
                continue;
            }
            if rank >= selected_index {
                weak_unselected.push((skipped.to_string(), -1));
                continue;
            }
            let mut penalty = if rank == 0 {
                SELECTION_FRONT_PENALTY
            } else {
                SELECTION_SKIPPED_PENALTY
            };
            if rank < previous_page_cutoff {
                penalty = penalty.saturating_add(SELECTION_PREVIOUS_PAGE_EXTRA_PENALTY);
            }
            adjustments.push((skipped.to_string(), penalty));
        }

        if adjustments.is_empty() && weak_unselected.is_empty() {
            return Ok(());
        }

        self.reload_user_lexicon_if_changed_now();
        if !adjustments.is_empty() {
            self.user_lexicon
                .record_selection_feedback(
                    &key,
                    adjustments
                        .iter()
                        .map(|(candidate, delta)| (candidate.as_str(), *delta)),
                )
                .map_err(|e| format!("save selection feedback: {}", e))?;
            if let Some(prev) = context_prev.as_deref() {
                self.user_lexicon
                    .record_context_selection_feedback(
                        prev,
                        &key,
                        adjustments
                            .iter()
                            .map(|(candidate, delta)| (candidate.as_str(), *delta)),
                    )
                    .map_err(|e| format!("save context selection feedback: {}", e))?;
            }
        }
        if !weak_unselected.is_empty() {
            if let Some(prev) = context_prev.as_deref() {
                self.user_lexicon
                    .record_context_weak_unselected_feedback(
                        prev,
                        &key,
                        weak_unselected
                            .iter()
                            .map(|(candidate, delta)| (candidate.as_str(), *delta)),
                    )
                    .map_err(|e| format!("save context weak unselected feedback: {}", e))?;
            } else {
                self.user_lexicon
                    .record_weak_unselected_feedback(
                        &key,
                        weak_unselected
                            .iter()
                            .map(|(candidate, delta)| (candidate.as_str(), *delta)),
                    )
                    .map_err(|e| format!("save weak unselected feedback: {}", e))?;
            }
        }
        self.note_selection_feedback_updated();
        Ok(())
    }

    /// 应用候选交互反馈到候选分数；调用方会在加权后做一次 top-k 重排，
    /// 让反馈尽快生效，同时仍保留后续 promote / short-intent 链路的收尾处理。
    pub(super) fn apply_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        self.adjust_selection_feedback(reading, ranked, 1.0)
    }

    pub(super) fn remove_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        self.adjust_selection_feedback(reading, ranked, -1.0)
    }

    pub(super) fn apply_final_negative_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        self.adjust_final_negative_selection_feedback(reading, ranked, 1.0)
    }

    pub(super) fn remove_final_negative_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        self.adjust_final_negative_selection_feedback(reading, ranked, -1.0)
    }

    pub(super) fn adjust_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
        direction: f64,
    ) -> bool {
        let mut changed = false;
        if let Some(feedback) = self.user_lexicon.selection_signal(reading) {
            changed |= self.adjust_selection_feedback_entries(ranked, feedback, direction, 1.0);
        }
        if let Some(prev) = self.rerank_context_key().first() {
            if let Some(feedback) = self.user_lexicon.context_selection_signal(prev, reading) {
                changed |= self.adjust_selection_feedback_entries(
                    ranked,
                    feedback,
                    direction,
                    SELECTION_CONTEXT_FEEDBACK_SCALE,
                );
            }
        }
        if let Some(feedback) = self.user_lexicon.weak_unselected_signal(reading) {
            changed |=
                self.adjust_weak_unselected_feedback_entries(ranked, feedback, direction, 1.0);
        }
        if let Some(prev) = self.rerank_context_key().first() {
            if let Some(feedback) = self
                .user_lexicon
                .context_weak_unselected_signal(prev, reading)
            {
                changed |= self.adjust_weak_unselected_feedback_entries(
                    ranked,
                    feedback,
                    direction,
                    SELECTION_CONTEXT_FEEDBACK_SCALE,
                );
            }
        }
        changed
    }

    fn adjust_weak_unselected_feedback_entries(
        &self,
        ranked: &mut [RankedCandidate],
        feedback: &[crate::user_dict::SelectionFeedbackEntry],
        direction: f64,
        scale: f64,
    ) -> bool {
        let mut changed = false;
        let now = self.user_lexicon.current_clock();
        for item in ranked.iter_mut() {
            if item.meta.pinned {
                continue;
            }
            let Some(entry) = feedback
                .iter()
                .find(|entry| entry.phrase == item.phrase && entry.score < 0)
            else {
                continue;
            };
            let count = entry.score.abs().min(WEAK_UNSELECTED_FEEDBACK_SCORE_CAP);
            if count < WEAK_UNSELECTED_FEEDBACK_APPLY_THRESHOLD {
                continue;
            }
            let effective = count - WEAK_UNSELECTED_FEEDBACK_APPLY_THRESHOLD + 1;
            let decay = selection_feedback_decay(
                now,
                entry.last_used,
                WEAK_UNSELECTED_FEEDBACK_HALF_LIFE_TICKS,
            );
            item.score +=
                WEAK_UNSELECTED_FEEDBACK_PENALTY * effective as f64 * decay * scale * direction;
            changed = true;
        }
        changed
    }

    fn adjust_selection_feedback_entries(
        &self,
        ranked: &mut [RankedCandidate],
        feedback: &[crate::user_dict::SelectionFeedbackEntry],
        direction: f64,
        scale: f64,
    ) -> bool {
        let mut changed = false;
        let now = self.user_lexicon.current_clock();
        for item in ranked.iter_mut() {
            let Some(entry) = feedback.iter().find(|e| e.phrase == item.phrase) else {
                continue;
            };
            let capped = entry
                .score
                .clamp(-SELECTION_FEEDBACK_SCORE_CAP, SELECTION_FEEDBACK_SCORE_CAP);
            // 负反馈在最终整理后统一应用，避免同一次查询重复降权。
            if capped <= 0 {
                continue;
            }
            let decay =
                selection_feedback_decay(now, entry.last_used, SELECTION_FEEDBACK_HALF_LIFE_TICKS);
            let mut bonus = SELECTION_POSITIVE_FEEDBACK_BONUS * capped as f64 * decay;
            if !item.meta.pinned
                && self.phrase_lexicon.as_ref().is_some_and(|lex| {
                    lex.phrase_frequency(&item.phrase) < USER_LOW_LEXICON_FREQ_THRESHOLD
                })
            {
                bonus *= USER_LOW_LEXICON_FREQ_DISCOUNT;
            }
            item.score += bonus * scale * direction;
            changed = true;
        }
        changed
    }

    pub(super) fn adjust_final_negative_selection_feedback(
        &self,
        reading: &str,
        ranked: &mut [RankedCandidate],
        direction: f64,
    ) -> bool {
        let mut changed = false;
        if let Some(feedback) = self.user_lexicon.selection_signal(reading) {
            changed |= self
                .adjust_final_negative_selection_feedback_entries(ranked, feedback, direction, 1.0);
        }
        if let Some(prev) = self.rerank_context_key().first() {
            if let Some(feedback) = self.user_lexicon.context_selection_signal(prev, reading) {
                changed |= self.adjust_final_negative_selection_feedback_entries(
                    ranked,
                    feedback,
                    direction,
                    SELECTION_CONTEXT_FEEDBACK_SCALE,
                );
            }
        }
        changed
    }

    fn adjust_final_negative_selection_feedback_entries(
        &self,
        ranked: &mut [RankedCandidate],
        feedback: &[crate::user_dict::SelectionFeedbackEntry],
        direction: f64,
        scale: f64,
    ) -> bool {
        let mut changed = false;
        let now = self.user_lexicon.current_clock();
        for item in ranked.iter_mut() {
            if item.meta.pinned {
                continue;
            }
            let Some(entry) = feedback
                .iter()
                .find(|entry| entry.phrase == item.phrase && entry.score < 0)
            else {
                continue;
            };
            let capped = entry
                .score
                .clamp(-SELECTION_FEEDBACK_SCORE_CAP, SELECTION_FEEDBACK_SCORE_CAP);
            if capped >= 0 {
                continue;
            }
            let decay =
                selection_feedback_decay(now, entry.last_used, SELECTION_FEEDBACK_HALF_LIFE_TICKS);
            item.score += SELECTION_NEGATIVE_FEEDBACK_PENALTY
                * capped.abs() as f64
                * SELECTION_FINAL_NEGATIVE_FEEDBACK_SCALE
                * decay
                * scale
                * direction;
            changed = true;
        }
        changed
    }

    pub(super) fn auto_novel_phrase_parts(
        &self,
        key: &str,
        reading: &str,
        phrase: &str,
    ) -> Option<Vec<(String, String)>> {
        if reading
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '\'' || ch == '-')
        {
            return None;
        }
        let char_count = phrase_char_count(phrase);
        if !(AUTO_NOVEL_MIN_CHARS..=AUTO_NOVEL_MAX_CHARS).contains(&char_count) {
            return None;
        }
        if self.system_phrase_exists(key, phrase) {
            return None;
        }
        let Ok((syllables, tail)) =
            split_syllables_with_tail_mode(reading, self.syllables.as_ref(), self.parse_options())
        else {
            return None;
        };
        if tail.is_some() || syllables.len() != char_count {
            return None;
        }

        let chars: Vec<char> = phrase.chars().collect();
        let mut spans = Vec::new();
        if (4..=5).contains(&char_count) {
            spans.push((0usize, 2usize));
            spans.push((char_count - 2, char_count));
        }
        if char_count >= 5 {
            spans.push((0usize, 3usize));
            spans.push((char_count - 3, char_count));
        }
        if char_count >= 6 {
            spans.push((0usize, char_count - 2));
            spans.push((char_count - 4, char_count));
        }

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (start, end) in spans {
            if start >= end || end > chars.len() {
                continue;
            }
            let sub_phrase: String = chars[start..end].iter().collect();
            if sub_phrase == phrase
                || !is_learnable_user_phrase(&sub_phrase)
                || self.user_lexicon.phrase_is_blocked(&sub_phrase)
            {
                continue;
            }
            let sub_key = syllables[start..end].join("");
            if sub_key.is_empty() || !seen.insert((sub_key.clone(), sub_phrase.clone())) {
                continue;
            }
            out.push((sub_key, sub_phrase));
        }
        Some(out)
    }

    pub(super) fn full_pinyin_learning_syllables(
        &self,
        reading: &str,
        compact_key: &str,
    ) -> Option<Vec<String>> {
        let mut options = self.parse_options();
        options.fuzzy_enabled = false;
        options.correction_enabled = false;
        let (syllables, tail) =
            split_syllables_with_tail_mode(reading, self.syllables.as_ref(), options).ok()?;
        if tail.is_some() || syllables.len() < 2 || syllables.join("") != compact_key {
            return None;
        }
        Some(syllables)
    }

    pub(super) fn system_phrase_exists(&self, key: &str, phrase: &str) -> bool {
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return false;
        };
        if lex.phrase_frequency(phrase) > 0 {
            return true;
        }
        lex.lookup_pinyin(key)
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == phrase))
    }

    pub fn set_candidate_pin(
        &mut self,
        reading: &str,
        phrase: &str,
        pinned: bool,
    ) -> Result<(), String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(());
        }
        let phrase = phrase.trim();
        let key = compact_lookup_key(reading);
        if key.is_empty() || phrase.is_empty() {
            return Ok(());
        }
        if self.is_effective_direct_input_shortcut(&key) {
            return Ok(());
        }
        self.reload_user_lexicon_if_changed_now();

        if pinned {
            self.user_lexicon
                .pin_exact_input(&key, phrase)
                .map_err(|e| format!("pin user dict: {}", e))?;
        } else {
            self.user_lexicon
                .unpin_exact_input(&key, phrase)
                .map_err(|e| format!("unpin user dict: {}", e))?;
        }
        self.user_lexicon
            .flush_pending()
            .map_err(|e| format!("flush user dict: {}", e))?;
        self.note_user_lexicon_updated();
        Ok(())
    }

    pub fn remove_user_phrase(&mut self, reading: &str, phrase: &str) -> Result<bool, String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(false);
        }
        let phrase = phrase.trim();
        let key = compact_lookup_key(reading);
        if key.is_empty() || phrase.is_empty() {
            return Ok(false);
        }
        self.reload_user_lexicon_if_changed_now();
        let removed = self
            .user_lexicon
            .remove_exact_input(&key, phrase)
            .map_err(|e| format!("remove user phrase: {}", e))?;
        if removed {
            self.user_lexicon
                .record_selection_feedback(&key, [(phrase, REMOVED_PHRASE_FEEDBACK_DELTA)])
                .map_err(|e| format!("remember removed user phrase: {}", e))?;
            self.note_user_lexicon_updated();
        }
        Ok(removed)
    }

    pub fn block_user_phrase(&mut self, phrase: &str) -> Result<bool, String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(false);
        }
        self.reload_user_lexicon_if_changed_now();
        let blocked = self
            .user_lexicon
            .block_phrase(phrase)
            .map_err(|e| format!("block user phrase: {}", e))?;
        if blocked {
            self.note_user_lexicon_updated();
        }
        Ok(blocked)
    }

    pub fn unblock_user_phrase(&mut self, phrase: &str) -> Result<bool, String> {
        if self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0 {
            return Ok(false);
        }
        self.reload_user_lexicon_if_changed_now();
        let unblocked = self
            .user_lexicon
            .unblock_phrase(phrase)
            .map_err(|e| format!("unblock user phrase: {}", e))?;
        if unblocked {
            self.note_user_lexicon_updated();
        }
        Ok(unblocked)
    }

    pub(super) fn reload_user_lexicon_if_changed(&mut self) {
        self.reload_user_lexicon_if_changed_impl(false);
    }

    pub(super) fn reload_user_lexicon_if_changed_now(&mut self) {
        self.reload_user_lexicon_if_changed_impl(true);
    }

    pub fn clear_user_lexicon_for_eval(&mut self) {
        self.user_lexicon = UserLexicon::default();
        self.user_lexicon_stamp = user_lexicon_disk_stamp();
        self.user_lexicon_generation = notify_user_lexicon_changed(self.user_lexicon_stamp.clone());
        self.clear_user_sensitive_lookup_caches();
        self.advance_shared_data_generation();
    }

    /// Advances the event clock without manufacturing learning records. Used
    /// by the offline replay evaluator to test recency/decay behavior.
    pub fn advance_learning_clock_for_eval(&mut self, ticks: u64) {
        self.user_lexicon.advance_clock_for_eval(ticks);
        self.clear_user_sensitive_lookup_caches();
        self.last_lookup = None;
        self.advance_shared_data_generation();
    }

    pub(super) fn reload_user_lexicon_if_changed_impl(&mut self, force_check: bool) {
        let generation = USER_LEXICON_GENERATION.load(Ordering::Acquire);
        if !force_check && generation == self.user_lexicon_generation {
            return;
        }
        let stamp = user_lexicon_disk_stamp();
        self.user_lexicon_generation = generation;
        if stamp == self.user_lexicon_stamp {
            return;
        }
        let prepared = (!force_check)
            .then(|| {
                prepared_user_lexicon_reload()
                    .lock()
                    .ok()
                    .and_then(|mut slot| slot.take())
            })
            .flatten();
        let prepared = match prepared {
            Some(prepared) if prepared.stamp == stamp => Some(prepared.lexicon),
            Some(stale) => {
                retire_user_lexicon(stale.lexicon);
                None
            }
            None => None,
        };
        let replacement = prepared.unwrap_or_else(UserLexicon::load_default);
        let retired = std::mem::replace(&mut self.user_lexicon, replacement);
        retire_user_lexicon(retired);
        self.user_lexicon_stamp = stamp;
        self.clear_user_sensitive_lookup_caches();
        self.last_lookup = None;
        self.advance_shared_data_generation();
    }
}

pub(super) fn phrase_char_count(phrase: &str) -> usize {
    phrase.chars().filter(|ch| !ch.is_whitespace()).count()
}

pub(super) fn is_learnable_user_phrase(phrase: &str) -> bool {
    let count = phrase_char_count(phrase);
    if !(2..=128).contains(&count) {
        return false;
    }
    if looks_like_sensitive_user_phrase(phrase) {
        return false;
    }
    let mut has_cjk = false;
    let mut all_ascii_punctuation = true;
    let mut has_any_printable = false;
    for ch in phrase.chars() {
        if ch.is_control() {
            return false;
        }
        if is_cjk_unified(ch) {
            has_cjk = true;
        }
        if !ch.is_ascii_punctuation() {
            all_ascii_punctuation = false;
        }
        if !ch.is_whitespace() {
            has_any_printable = true;
        }
    }
    // 纯标点、空白、或纯 ASCII 非英文单词（无 CJK、无空格）不学入用户词库。
    if all_ascii_punctuation || !has_any_printable {
        return false;
    }
    if !has_cjk && !phrase.chars().any(|ch| ch.is_whitespace()) {
        return false;
    }
    has_cjk
}

pub(super) fn is_learnable_single_user_char(phrase: &str) -> bool {
    if looks_like_sensitive_user_phrase(phrase) {
        return false;
    }
    let mut chars = phrase.chars().filter(|ch| !ch.is_whitespace());
    let Some(ch) = chars.next() else {
        return false;
    };
    if chars.next().is_some() || ch.is_control() {
        return false;
    }
    is_cjk_unified(ch)
}

pub(super) fn looks_like_sensitive_user_phrase(phrase: &str) -> bool {
    let trimmed = phrase.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("://") || lower.starts_with("www.") {
        return true;
    }
    if looks_like_email(trimmed) || looks_like_windows_or_url_path(trimmed) {
        return true;
    }
    let digits: String = trimmed.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if digits.len() >= 11 {
        return true;
    }
    if looks_like_mainland_id(trimmed) {
        return true;
    }
    if digits.len() >= 8 {
        return true;
    }
    false
}

pub(super) fn looks_like_email(text: &str) -> bool {
    let Some(at) = text.find('@') else {
        return false;
    };
    at > 0 && text[at + 1..].contains('.')
}

pub(super) fn looks_like_windows_or_url_path(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("\\\\") {
        return true;
    }
    let bytes = lower.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        return true;
    }
    lower.contains("\\") || (lower.contains('/') && lower.contains('.'))
}

pub(super) fn looks_like_mainland_id(text: &str) -> bool {
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.len() != 18 {
        return false;
    }
    let mut chars = compact.chars();
    chars.by_ref().take(17).all(|ch| ch.is_ascii_digit())
        && chars
            .next()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == 'x' || ch == 'X')
}

pub(super) fn is_cjk_unified(ch: char) -> bool {
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
}

pub(super) fn is_mixed_pinyin_learning_key(key: &str, phrase: &str) -> bool {
    if !(2..=4).contains(&phrase_char_count(phrase)) {
        return false;
    }
    crate::thuocl::mixed_pinyin_keys_for_phrase(phrase)
        .iter()
        .any(|mixed_key| mixed_key == key)
}

pub(super) fn mixed_pinyin_keys_from_syllables(syllables: &[String], limit: usize) -> Vec<String> {
    let len = syllables.len();
    if !(2..=6).contains(&len) || limit == 0 {
        return Vec::new();
    }
    let all_full_mask = (1usize << len) - 1;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for mask in 1usize..all_full_mask {
        let mut key = String::new();
        let mut valid = true;
        for (idx, syllable) in syllables.iter().enumerate() {
            if (mask & (1usize << idx)) != 0 {
                key.push_str(syllable);
            } else if let Some(initial) = syllable.chars().next() {
                if initial.is_ascii_alphabetic() {
                    key.push(initial.to_ascii_lowercase());
                } else {
                    valid = false;
                    break;
                }
            } else {
                valid = false;
                break;
            }
        }
        if valid && !key.is_empty() && seen.insert(key.clone()) {
            out.push(key);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}
