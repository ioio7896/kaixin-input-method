use super::*;

#[inline]
pub(super) fn phrase_length_rerank_prior(n_chars: usize) -> f64 {
    match n_chars {
        0 => 0.0,
        1 => SHORT_PHRASE_SINGLE_RERANK_EXTRA,
        2 => SHORT_PHRASE_TWO_CHAR_RERANK_BONUS,
        3 => SHORT_PHRASE_THREE_CHAR_RERANK_BONUS,
        _ => (MULTI_CHAR_PHRASE_RERANK_BONUS
            + (n_chars.saturating_sub(4) as f64) * MULTI_CHAR_PHRASE_RERANK_STEP)
            .min(MULTI_CHAR_PHRASE_RERANK_CAP),
    }
}

#[derive(Clone)]
pub(super) struct WordGraphEdge {
    end: usize,
    phrase: String,
    base_score: f64,
    last_char: char,
    single_char: bool,
}

#[derive(Clone)]
pub(super) struct WordGraphState {
    phrase: String,
    score: f64,
    last_char: Option<char>,
    prev_token: Option<String>,
    last_token: Option<String>,
    token_count: usize,
    single_token_count: usize,
    last_token_was_single: bool,
}

#[derive(Clone)]
struct CachedWordGraphState {
    syllables: Vec<String>,
    user_clock: u64,
    cache_epoch: u64,
    initial_prev_token: Option<String>,
    initial_last_token: Option<String>,
    edges_by_pos: Vec<Vec<WordGraphEdge>>,
    beams: Vec<Vec<WordGraphState>>,
    results: Vec<(String, f64)>,
}

/// Composition-local word-lattice snapshots. On append, prior beams stay
/// valid; only edges that cross the old suffix and the newly appended suffix
/// are expanded. Exact older snapshots make Backspace an O(result) restore.
pub(super) struct IncrementalWordGraphCache {
    states: VecDeque<CachedWordGraphState>,
    capacity: usize,
}

impl Default for IncrementalWordGraphCache {
    fn default() -> Self {
        Self {
            states: VecDeque::new(),
            capacity: 16,
        }
    }
}

impl IncrementalWordGraphCache {
    pub(super) fn clear(&mut self) {
        self.states.clear();
    }

    fn decode<Populate, Transition>(
        &mut self,
        syllables: &[String],
        user_clock: u64,
        cache_epoch: u64,
        initial_prev_token: Option<String>,
        initial_last_token: Option<String>,
        mut populate: Populate,
        mut transition: Transition,
    ) -> Vec<(String, f64)>
    where
        Populate: FnMut(&[String], usize, &mut Vec<WordGraphEdge>),
        Transition: FnMut(Option<char>, Option<&str>, Option<&str>, &str) -> f64,
    {
        if syllables.is_empty() {
            return Vec::new();
        }

        if let Some(index) = self.states.iter().position(|state| {
            state.user_clock == user_clock
                && state.cache_epoch == cache_epoch
                && state.syllables == syllables
                && state.initial_prev_token == initial_prev_token
                && state.initial_last_token == initial_last_token
        }) {
            let state = self.states.remove(index).expect("word graph state exists");
            let results = state.results.clone();
            self.states.push_front(state);
            return results;
        }

        let reusable = self
            .states
            .iter()
            .enumerate()
            .filter(|(_, state)| {
                state.user_clock == user_clock
                    && state.cache_epoch == cache_epoch
                    && state.initial_prev_token == initial_prev_token
                    && state.initial_last_token == initial_last_token
                    && state.syllables.len() < syllables.len()
                    && syllables.starts_with(&state.syllables)
            })
            .max_by_key(|(_, state)| state.syllables.len())
            .map(|(index, _)| index);

        let mut state = if let Some(index) = reusable {
            let previous = self.states[index].clone();
            self.extend(previous, syllables, &mut populate, &mut transition)
        } else {
            self.build_cold(
                syllables,
                user_clock,
                cache_epoch,
                initial_prev_token,
                initial_last_token,
                &mut populate,
                &mut transition,
            )
        };
        state.results = word_graph_results(&state.beams, syllables.len());
        let results = state.results.clone();
        self.states.push_front(state);
        while self.states.len() > self.capacity {
            self.states.pop_back();
        }
        results
    }

    fn build_cold<Populate, Transition>(
        &self,
        syllables: &[String],
        user_clock: u64,
        cache_epoch: u64,
        initial_prev_token: Option<String>,
        initial_last_token: Option<String>,
        populate: &mut Populate,
        transition: &mut Transition,
    ) -> CachedWordGraphState
    where
        Populate: FnMut(&[String], usize, &mut Vec<WordGraphEdge>),
        Transition: FnMut(Option<char>, Option<&str>, Option<&str>, &str) -> f64,
    {
        let mut edges_by_pos = vec![Vec::<WordGraphEdge>::new(); syllables.len()];
        for (start, edges) in edges_by_pos.iter_mut().enumerate() {
            populate(syllables, start, edges);
        }
        let mut beams = empty_word_graph_beams(
            syllables.len(),
            initial_prev_token.clone(),
            initial_last_token.clone(),
        );
        expand_word_graph_range(&edges_by_pos, &mut beams, 0, 0, transition);
        CachedWordGraphState {
            syllables: syllables.to_vec(),
            user_clock,
            cache_epoch,
            initial_prev_token,
            initial_last_token,
            edges_by_pos,
            beams,
            results: Vec::new(),
        }
    }

    fn extend<Populate, Transition>(
        &self,
        mut state: CachedWordGraphState,
        syllables: &[String],
        populate: &mut Populate,
        transition: &mut Transition,
    ) -> CachedWordGraphState
    where
        Populate: FnMut(&[String], usize, &mut Vec<WordGraphEdge>),
        Transition: FnMut(Option<char>, Option<&str>, Option<&str>, &str) -> f64,
    {
        let prefix_len = state.syllables.len();
        state.edges_by_pos.resize_with(syllables.len(), Vec::new);
        state.beams.resize_with(syllables.len() + 1, Vec::new);

        // Only starts close enough to span into the new suffix can gain edges.
        let affected_start = prefix_len.saturating_sub(WORD_GRAPH_MAX_SPAN.saturating_sub(1));
        for start in affected_start..syllables.len() {
            state.edges_by_pos[start].clear();
            populate(syllables, start, &mut state.edges_by_pos[start]);
        }
        for bucket in state.beams.iter_mut().skip(prefix_len + 1) {
            bucket.clear();
        }

        // Old beams are complete. For old positions, expand only newly possible
        // edges crossing the former end; from the old end onward expand all.
        expand_word_graph_range(
            &state.edges_by_pos,
            &mut state.beams,
            affected_start,
            prefix_len,
            transition,
        );
        state.syllables = syllables.to_vec();
        state
    }
}

fn empty_word_graph_beams(
    syllable_count: usize,
    initial_prev_token: Option<String>,
    initial_last_token: Option<String>,
) -> Vec<Vec<WordGraphState>> {
    let mut beams = vec![Vec::<WordGraphState>::new(); syllable_count + 1];
    beams[0].push(WordGraphState {
        phrase: String::new(),
        score: 0.0,
        last_char: None,
        prev_token: initial_prev_token,
        last_token: initial_last_token,
        token_count: 0,
        single_token_count: 0,
        last_token_was_single: false,
    });
    beams
}

fn expand_word_graph_range(
    edges_by_pos: &[Vec<WordGraphEdge>],
    beams: &mut [Vec<WordGraphState>],
    start_at: usize,
    old_end: usize,
    transition: &mut impl FnMut(Option<char>, Option<&str>, Option<&str>, &str) -> f64,
) {
    for start in start_at..edges_by_pos.len() {
        let edge_count = edges_by_pos[start].len();
        if beams[start].is_empty() || edge_count == 0 {
            continue;
        }
        // Split `beams` so we can read position `start` while writing later
        // positions.  Edges always go forward (end > start), so the slices
        // are guaranteed disjoint.
        let (left, right) = beams.split_at_mut(start + 1);
        let current = &mut left[start];
        let state_count = current.len();
        for state_idx in 0..state_count {
            let state = &current[state_idx];
            for edge in &edges_by_pos[start] {
                if start < old_end && edge.end <= old_end {
                    continue;
                }
                let mut phrase = String::with_capacity(state.phrase.len() + edge.phrase.len());
                phrase.push_str(&state.phrase);
                phrase.push_str(&edge.phrase);
                let consecutive_single = state.last_token_was_single && edge.single_char;
                let dest = &mut right[edge.end - (start + 1)];
                dest.push(WordGraphState {
                    phrase,
                    score: state.score
                        + edge.base_score
                        + transition(
                            state.last_char,
                            state.prev_token.as_deref(),
                            state.last_token.as_deref(),
                            &edge.phrase,
                        )
                        - if consecutive_single {
                            WORD_GRAPH_CONSECUTIVE_SINGLE_PENALTY
                        } else {
                            0.0
                        },
                    last_char: Some(edge.last_char),
                    prev_token: state.last_token.clone(),
                    last_token: Some(edge.phrase.clone()),
                    token_count: state.token_count + 1,
                    single_token_count: state.single_token_count + usize::from(edge.single_char),
                    last_token_was_single: edge.single_char,
                });
            }
        }
        for bucket in right.iter_mut() {
            if !bucket.is_empty() {
                truncate_top_k_word_states(bucket, WORD_GRAPH_BEAM);
            }
        }
    }
}

fn word_graph_results(beams: &[Vec<WordGraphState>], syllable_count: usize) -> Vec<(String, f64)> {
    let mut results = beams
        .get(syllable_count)
        .into_iter()
        .flatten()
        .map(|state| {
            (
                state.phrase.clone(),
                state.score + WORD_GRAPH_SENTENCE_BONUS
                    - state.token_count as f64 * WORD_GRAPH_TOKEN_PENALTY
                    - state.single_token_count as f64 * WORD_GRAPH_SINGLE_TOKEN_PENALTY,
            )
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    results.dedup_by(|a, b| a.0 == b.0);
    results.truncate(BASE_DECODE_LIMIT);
    results
}

impl PinyinEngine {
    fn cached_rerank_score(&self, key: &RerankScoreCacheKey) -> Option<f64> {
        self.rerank_score_cache.lock().ok()?.get(key)
    }

    fn remember_rerank_score(&self, key: RerankScoreCacheKey, score: f64) {
        if let Ok(mut cache) = self.rerank_score_cache.lock() {
            cache.insert(key, score);
        }
    }

    pub(super) fn rerank_context_key(&self) -> Vec<String> {
        self.user_lexicon
            .preceding_commits_for_context()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    pub(super) fn rerank_word_context_key(&self) -> Vec<String> {
        self.user_lexicon
            .preceding_word_tokens_for_context()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    pub(super) fn rerank_sentence(&self, phrase: &str, now: u64) -> f64 {
        self.best_phrase_path_score(phrase, now)
            + self.context_bonus(phrase, now)
            + self.word_level_transition_bonus(phrase, now)
    }

    /// 按字平均的语言模型 unigram log 概率（越高表示字越常见，通常为负值）。

    pub(super) fn lm_mean_unigram_log(&self, phrase: &str) -> f64 {
        let chars: Vec<char> = phrase.chars().filter(|c| !c.is_whitespace()).collect();
        if chars.is_empty() {
            return -25.0;
        }
        chars.iter().map(|c| self.lm.log_unigram(*c)).sum::<f64>() / chars.len() as f64
    }

    /// 单字通道：常用字得分高，与候选字数无关（按均值，避免长串天然占优）。

    pub(super) fn single_char_channel_boost(&self, phrase: &str, lm_scale: f64) -> f64 {
        let avg = self.lm_mean_unigram_log(phrase);
        (avg + 25.0).clamp(0.0, 28.0) * lm_scale
    }

    pub(super) fn single_syllable_common_char_prior(
        &self,
        phrase: &str,
        intent: InputIntent,
    ) -> f64 {
        if intent != InputIntent::SingleSyllable || phrase_char_count(phrase) != 1 {
            return 0.0;
        }

        let Some(lexicon) = self.phrase_lexicon.as_ref() else {
            return 0.0;
        };
        let freq = lexicon.phrase_frequency(phrase);
        let frequency_strength = short_phrase_priority_score(freq, 1);
        if frequency_strength <= 0.0 {
            return 0.0;
        }

        let layer_scale = match lexicon.phrase_layer(phrase) {
            LexiconLayer::Core => 1.0,
            LexiconLayer::Base => 0.92,
            LexiconLayer::Unknown => 0.65,
            LexiconLayer::Ext | LexiconLayer::Large | LexiconLayer::En => return 0.0,
        };
        let freq_score = frequency_strength * SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_SCALE * 7.0;
        (SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_BASE * frequency_strength + freq_score)
            .min(SINGLE_SYLLABLE_COMMON_CHAR_PRIOR_CAP)
            * layer_scale
    }

    /// 单字（LM）+ 词组路径；短候选额外加常数偏置，便于 1/2/3 字候选在候选栏更靠前。

    pub(super) fn blended_rerank_with_prefs(
        &self,
        phrase: &str,
        now: u64,
        prefs: rerank_prefs::RerankPrefs,
    ) -> f64 {
        let single = self.single_char_channel_boost(phrase, prefs.lm_single_scale);
        let phrase_ch = self.rerank_sentence(phrase, now);
        let mut score = prefs.w_single_lm * single + prefs.w_phrase_path * phrase_ch;
        let n_chars = phrase.chars().filter(|c| !c.is_whitespace()).count();
        score += phrase_length_rerank_prior(n_chars);
        score
    }

    pub(super) fn best_phrase_path_score(&self, phrase: &str, now: u64) -> f64 {
        let chars: Vec<char> = phrase.chars().collect();
        if chars.len() <= 1 {
            return 0.0;
        }

        let cache_key = rerank_score_cache_key(
            RerankScoreKind::PhrasePath,
            phrase,
            self.cache_epoch,
            Vec::new(),
        );
        if let Some(score) = self.cached_rerank_score(&cache_key) {
            return score;
        }

        let mut dp = vec![f64::NEG_INFINITY; chars.len() + 1];
        dp[chars.len()] = 0.0;

        for start in (0..chars.len()).rev() {
            dp[start] = dp[start + 1] - SENTENCE_SINGLE_CHAR_PENALTY;

            let mut piece = String::new();
            let limit = (start + MAX_PHRASE_SPAN_CHARS).min(chars.len());
            for end in start..limit {
                piece.push(chars[end]);
                if end == start {
                    continue;
                }

                let lex_freq = self
                    .phrase_lexicon
                    .as_ref()
                    .map(|lex| lex.phrase_frequency(&piece))
                    .unwrap_or(0);
                let user_signal = self.user_lexicon.phrase_signal(&piece);
                if lex_freq == 0 && user_signal.freq == 0 {
                    continue;
                }

                let score = phrase_span_bonus(piece.chars().count(), lex_freq, user_signal, now)
                    + dp[end + 1];
                if score > dp[start] {
                    dp[start] = score;
                }
            }
        }

        let score = if dp[0].is_finite() { dp[0] } else { 0.0 };
        self.remember_rerank_score(cache_key, score);
        score
    }

    pub(super) fn context_bonus(&self, phrase: &str, now: u64) -> f64 {
        let context = self.rerank_context_key();
        if context.is_empty() {
            return 0.0;
        }
        let cache_key = rerank_score_cache_key(
            RerankScoreKind::ContextBonus,
            phrase,
            self.cache_epoch,
            context.clone(),
        );
        if let Some(score) = self.cached_rerank_score(&cache_key) {
            return score;
        }

        const CONTEXT_PREV_WEIGHTS: [f64; 3] = [1.0, 0.58, 0.34];
        let chars: Vec<char> = phrase.chars().collect();
        let mut weighted_sum = 0.0_f64;
        if context.len() >= 2 {
            weighted_sum += signal_bonus(
                self.user_lexicon
                    .context_trigram_signal(&context[1], &context[0], phrase),
                now,
                CONTEXT_TRIGRAM_FREQ_SCALE,
                CONTEXT_TRIGRAM_RECENCY_SCALE,
            )
            .min(CONTEXT_TRIGRAM_CAP);
        }
        for (i, prev) in context.iter().take(3).enumerate() {
            let w = CONTEXT_PREV_WEIGHTS.get(i).copied().unwrap_or(0.22);
            let mut bonus = signal_bonus(
                self.user_lexicon.context_signal(prev, phrase),
                now,
                CONTEXT_FREQ_SCALE * w,
                CONTEXT_RECENCY_SCALE * w,
            );

            for len in 2..=chars.len().min(CONTEXT_HEAD_MAX_LEN) {
                let head: String = chars[..len].iter().collect();
                let head_bonus = signal_bonus(
                    self.user_lexicon.context_signal(prev, &head),
                    now,
                    CONTEXT_FREQ_SCALE * w,
                    CONTEXT_RECENCY_SCALE * w,
                ) * CONTEXT_HEAD_DISCOUNT;
                bonus = bonus.max(head_bonus);
            }
            weighted_sum += bonus * CONTEXT_SUM_DECAY.powi(i as i32);
        }

        let score = weighted_sum.clamp(0.0, CONTEXT_SUM_CAP);
        self.remember_rerank_score(cache_key, score);
        score
    }

    pub(super) fn word_level_transition_bonus(&self, phrase: &str, now: u64) -> f64 {
        let context = self.rerank_word_context_key();
        let cache_key = rerank_score_cache_key(
            RerankScoreKind::WordTransition,
            phrase,
            self.cache_epoch,
            context.clone(),
        );
        if let Some(score) = self.cached_rerank_score(&cache_key) {
            return score;
        }
        let tokens = self.best_phrase_tokens(phrase, now);
        if tokens.is_empty() || (tokens.len() == 1 && context.is_empty()) {
            return 0.0;
        }

        let mut score = 0.0_f64;
        let mut prev1 = context.first().cloned();
        let mut prev2 = context.get(1).cloned();
        for (idx, token) in tokens.into_iter().enumerate() {
            score +=
                self.word_ngram_transition_score(prev2.as_deref(), prev1.as_deref(), &token, now)
                    * CONTEXT_TOKEN_DECAY.powi(idx as i32);
            prev2 = prev1;
            prev1 = Some(token);
        }

        let score = score.clamp(0.0, WORD_TRANSITION_CAP);
        self.remember_rerank_score(cache_key, score);
        score
    }

    pub(super) fn abbrev_context_bonus(
        &self,
        short_ascii_input: bool,
        input_chars: usize,
        phrase: &str,
        now: u64,
    ) -> f64 {
        if !short_ascii_input || phrase_char_count(phrase) <= 1 {
            return 0.0;
        }
        let scale = if input_chars == 4 && phrase_char_count(phrase) == 4 {
            FOUR_CHAR_ABBREV_CONTEXT_EXTRA_SCALE
        } else {
            ABBREV_CONTEXT_EXTRA_SCALE
        };
        (self.context_bonus(phrase, now) + self.word_level_transition_bonus(phrase, now)) * scale
    }

    pub(super) fn short_abbrev_exact_length_rerank_bonus(
        &self,
        short_ascii_input: bool,
        input_chars: usize,
        phrase: &str,
        meta: &CandidateMeta,
    ) -> f64 {
        if !short_ascii_input
            || !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&input_chars)
            || phrase_char_count(phrase) != input_chars
            || correction_style_kind(meta).is_some()
        {
            return 0.0;
        }
        if !matches!(
            meta.match_kind,
            CandidateMatchKind::ShortAbbrev
                | CandidateMatchKind::User
                | CandidateMatchKind::MixedPrefix
                | CandidateMatchKind::FullPinyin
        ) {
            return 0.0;
        }

        SHORT_ABBREV_EXACT_LENGTH_RERANK_BONUS
    }

    pub(super) fn word_ngram_transition_score(
        &self,
        prev2: Option<&str>,
        prev1: Option<&str>,
        next: &str,
        now: u64,
    ) -> f64 {
        if next.is_empty() {
            return 0.0;
        }
        let Some(prev1) = prev1.filter(|prev| !prev.is_empty() && *prev != next) else {
            return 0.0;
        };

        let next_stats = self.user_lexicon.phrase_signal(next);
        let vocabulary = self.user_lexicon.phrase_vocabulary_size().max(1) as f64;
        let unigram_total = self
            .user_lexicon
            .current_clock()
            .max(next_stats.freq)
            .saturating_add(vocabulary as u64)
            .max(1) as f64;
        let unigram = ((next_stats.freq as f64 + 1.0) / unigram_total).clamp(1e-9, 0.5);

        let pair = self.user_lexicon.context_signal(prev1, next);
        let (bigram_total, _) = self.user_lexicon.context_outgoing_stats(prev1);
        let mut probability = (pair.freq as f64 + WORD_NGRAM_BIGRAM_BACKOFF_STRENGTH * unigram)
            / (bigram_total as f64 + WORD_NGRAM_BIGRAM_BACKOFF_STRENGTH);
        let mut evidence = pair.freq as f64 / (pair.freq as f64 + 2.0);
        let mut recency = signal_bonus(
            pair,
            now,
            WORD_NGRAM_RECENCY_SCALE,
            WORD_TRANSITION_RECENCY_SCALE * 0.25,
        );

        if let Some(prev2) = prev2.filter(|prev| !prev.is_empty() && *prev != next) {
            let trigram = self.user_lexicon.context_trigram_signal(prev2, prev1, next);
            let (trigram_total, _) = self
                .user_lexicon
                .context_trigram_outgoing_stats(prev2, prev1);
            probability = (trigram.freq as f64 + WORD_NGRAM_TRIGRAM_BACKOFF_STRENGTH * probability)
                / (trigram_total as f64 + WORD_NGRAM_TRIGRAM_BACKOFF_STRENGTH);
            evidence = evidence.max(trigram.freq as f64 / (trigram.freq as f64 + 1.5));
            recency += signal_bonus(
                trigram,
                now,
                WORD_NGRAM_RECENCY_SCALE * 1.25,
                WORD_TRANSITION_RECENCY_SCALE * 0.35,
            );
        }

        let log_lift = (probability.max(1e-9) / unigram).ln().max(0.0);
        (log_lift * WORD_NGRAM_LOG_LIFT_SCALE * evidence + recency).clamp(0.0, WORD_TRANSITION_CAP)
    }

    pub(super) fn best_phrase_tokens(&self, phrase: &str, now: u64) -> Vec<String> {
        let chars: Vec<char> = phrase.chars().collect();
        if chars.len() <= 1 {
            return vec![phrase.to_string()];
        }

        let n = chars.len();
        let mut dp = vec![f64::NEG_INFINITY; n + 1];
        let mut next = vec![n; n];
        dp[n] = 0.0;

        for start in (0..n).rev() {
            dp[start] = dp[start + 1] - SENTENCE_SINGLE_CHAR_PENALTY;
            next[start] = start + 1;

            let mut piece = String::new();
            let limit = (start + MAX_PHRASE_SPAN_CHARS).min(n);
            for end in start..limit {
                piece.push(chars[end]);
                if end == start {
                    continue;
                }
                let lex_freq = self
                    .phrase_lexicon
                    .as_ref()
                    .map(|lex| lex.phrase_frequency(&piece))
                    .unwrap_or(0);
                let user_signal = self.user_lexicon.phrase_signal(&piece);
                if lex_freq == 0 && user_signal.freq == 0 {
                    continue;
                }
                let score = phrase_span_bonus(piece.chars().count(), lex_freq, user_signal, now)
                    + dp[end + 1];
                if score > dp[start] {
                    dp[start] = score;
                    next[start] = end + 1;
                }
            }
        }

        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let j = next[i].max(i + 1).min(n);
            let token: String = chars[i..j].iter().collect();
            out.push(token);
            i = j;
        }
        out
    }

    pub(super) fn decode_word_graph(&self, syls: &[String], now: u64) -> Vec<(String, f64)> {
        let (initial_prev_token, initial_last_token) = self.word_graph_initial_context();
        self.incremental_word_graph_cache.borrow_mut().decode(
            syls,
            now,
            self.cache_epoch,
            initial_prev_token,
            initial_last_token,
            |syllables, start, out| self.populate_word_graph_edges(syllables, start, now, out),
            |previous_char, prev2, prev1, phrase| {
                self.word_graph_transition_score(previous_char, prev2, prev1, phrase, now)
            },
        )
    }

    pub(super) fn decode_word_graph_until(
        &self,
        syls: &[String],
        now: u64,
        mut should_stop: impl FnMut() -> bool,
    ) -> Vec<(String, f64)> {
        if syls.is_empty() {
            return Vec::new();
        }

        let mut edges_by_pos = vec![Vec::<WordGraphEdge>::new(); syls.len()];
        for start in 0..syls.len() {
            if should_stop() {
                return Vec::new();
            }
            self.populate_word_graph_edges(syls, start, now, &mut edges_by_pos[start]);
        }

        if edges_by_pos.iter().all(|edges| edges.is_empty()) {
            return Vec::new();
        }

        let (initial_prev_token, initial_last_token) = self.word_graph_initial_context();
        let mut beams = empty_word_graph_beams(syls.len(), initial_prev_token, initial_last_token);

        for start in 0..syls.len() {
            if should_stop() {
                return Vec::new();
            }
            if beams[start].is_empty() || edges_by_pos[start].is_empty() {
                continue;
            }
            let states = beams[start].clone();
            for (state_index, state) in states.into_iter().enumerate() {
                if state_index % 8 == 0 && should_stop() {
                    return Vec::new();
                }
                for edge in &edges_by_pos[start] {
                    let mut phrase = state.phrase.clone();
                    phrase.push_str(&edge.phrase);
                    let consecutive_single = state.last_token_was_single && edge.single_char;
                    beams[edge.end].push(WordGraphState {
                        phrase,
                        score: state.score
                            + edge.base_score
                            + self.word_graph_transition_score(
                                state.last_char,
                                state.prev_token.as_deref(),
                                state.last_token.as_deref(),
                                &edge.phrase,
                                now,
                            )
                            - if consecutive_single {
                                WORD_GRAPH_CONSECUTIVE_SINGLE_PENALTY
                            } else {
                                0.0
                            },
                        last_char: Some(edge.last_char),
                        prev_token: state.last_token.clone(),
                        last_token: Some(edge.phrase.clone()),
                        token_count: state.token_count + 1,
                        single_token_count: state.single_token_count
                            + usize::from(edge.single_char),
                        last_token_was_single: edge.single_char,
                    });
                }
            }
            for bucket in beams.iter_mut().skip(start + 1) {
                if !bucket.is_empty() {
                    truncate_top_k_word_states(bucket, WORD_GRAPH_BEAM);
                }
            }
        }

        let final_states = std::mem::take(&mut beams[syls.len()]);
        let mut results: Vec<(String, f64)> = final_states
            .into_iter()
            .map(|state| {
                (
                    state.phrase,
                    state.score + WORD_GRAPH_SENTENCE_BONUS
                        - state.token_count as f64 * WORD_GRAPH_TOKEN_PENALTY
                        - state.single_token_count as f64 * WORD_GRAPH_SINGLE_TOKEN_PENALTY,
                )
            })
            .collect();
        results.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        results.dedup_by(|a, b| a.0 == b.0);
        results.truncate(BASE_DECODE_LIMIT);
        results
    }

    pub(super) fn populate_word_graph_edges(
        &self,
        syls: &[String],
        start: usize,
        now: u64,
        out: &mut Vec<WordGraphEdge>,
    ) {
        let mut merged: HashMap<String, WordGraphEdge> = HashMap::new();

        if let Some(raw) = self.dict.get(&syls[start]) {
            for ch in self
                .lm
                .rank_chars_by_unigram(raw)
                .into_iter()
                .take(WORD_GRAPH_CHAR_LIMIT)
            {
                merge_word_graph_edge(
                    &mut merged,
                    WordGraphEdge {
                        end: start + 1,
                        phrase: ch.to_string(),
                        base_score: self.word_graph_single_char_bonus(ch, now),
                        last_char: ch,
                        single_char: true,
                    },
                );
            }
        }

        let mut key = String::new();
        let max_end = (start + WORD_GRAPH_MAX_SPAN).min(syls.len());
        for end in start..max_end {
            key.push_str(&syls[end]);
            let span_len = end - start + 1;
            if span_len < 2 {
                continue;
            }

            if let Some(entries) = self.user_lexicon.lookup_input(&key) {
                for entry in entries.iter().take(WORD_GRAPH_PHRASE_LIMIT) {
                    if entry.phrase.chars().count() != span_len {
                        continue;
                    }
                    let Some(last_char) = entry.phrase.chars().last() else {
                        continue;
                    };
                    merge_word_graph_edge(
                        &mut merged,
                        WordGraphEdge {
                            end: end + 1,
                            phrase: entry.phrase.clone(),
                            base_score: self.word_graph_user_edge_bonus(entry, now),
                            last_char,
                            single_char: false,
                        },
                    );
                }
            }

            if let Some(lex) = &self.phrase_lexicon {
                if let Some(entries) = lex.lookup_pinyin(&key) {
                    for entry in entries.iter().take(WORD_GRAPH_PHRASE_LIMIT) {
                        if entry.phrase.chars().count() != span_len {
                            continue;
                        }
                        let Some(last_char) = entry.phrase.chars().last() else {
                            continue;
                        };
                        merge_word_graph_edge(
                            &mut merged,
                            WordGraphEdge {
                                end: end + 1,
                                phrase: entry.phrase.clone(),
                                base_score: self.word_graph_phrase_edge_bonus(entry, now),
                                last_char,
                                single_char: false,
                            },
                        );
                    }
                }
            }
        }

        out.extend(merged.into_values());
        out.sort_by(|a, b| {
            b.base_score
                .partial_cmp(&a.base_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
    }

    pub(super) fn word_graph_phrase_edge_bonus(&self, entry: &ThuoclEntry, now: u64) -> f64 {
        let len = entry.phrase.chars().count();
        phrase_span_bonus(
            len,
            entry.freq,
            self.user_lexicon.phrase_signal(&entry.phrase),
            now,
        ) + WORD_GRAPH_PHRASE_EDGE_BONUS
    }

    pub(super) fn word_graph_user_edge_bonus(&self, entry: &LearnedEntry, now: u64) -> f64 {
        let lex_freq = self
            .phrase_lexicon
            .as_ref()
            .map(|lex| lex.phrase_frequency(&entry.phrase))
            .unwrap_or(0);
        phrase_span_bonus(entry.phrase.chars().count(), lex_freq, entry.stats(), now)
            + input_entry_bonus(entry, now)
            + WORD_GRAPH_USER_EDGE_BONUS
    }

    pub(super) fn word_graph_single_char_bonus(&self, ch: char, now: u64) -> f64 {
        let phrase = ch.to_string();
        self.single_char_channel_boost(&phrase, 1.25)
            + signal_bonus(
                self.user_lexicon.phrase_signal(&phrase),
                now,
                USER_PHRASE_SCORE_SCALE * 0.35,
                USER_RECENCY_SCALE * 0.35,
            )
    }

    pub(super) fn phrase_transition_lm_score(&self, prev_last: Option<char>, phrase: &str) -> f64 {
        let mut chars = phrase.chars();
        let Some(first) = chars.next() else {
            return 0.0;
        };
        let mut score = if let Some(prev) = prev_last {
            self.lm.log_bigram(prev, first)
        } else {
            self.lm.log_unigram(first)
        };
        let mut prev = first;
        for ch in chars {
            score += self.lm.log_bigram(prev, ch);
            prev = ch;
        }
        score
    }

    pub(super) fn word_graph_initial_context(&self) -> (Option<String>, Option<String>) {
        let context = self.rerank_word_context_key();
        (context.get(1).cloned(), context.first().cloned())
    }

    pub(super) fn word_graph_transition_score(
        &self,
        prev_last: Option<char>,
        prev2: Option<&str>,
        prev1: Option<&str>,
        phrase: &str,
        now: u64,
    ) -> f64 {
        self.phrase_transition_lm_score(prev_last, phrase)
            + self.word_ngram_transition_score(prev2, prev1, phrase, now) * WORD_GRAPH_NGRAM_SCALE
    }

    pub(super) fn stability_bonus_for_candidate(&self, current_key: &str, phrase: &str) -> f64 {
        stability_bonus_from_state(self.last_lookup.as_ref(), current_key, phrase)
    }

    pub(super) fn remove_stability_bonus_from_candidates(
        &self,
        current_key: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        let mut changed = false;
        for candidate in ranked {
            let bonus = stability_bonus_from_state(
                self.last_lookup.as_ref(),
                current_key,
                &candidate.phrase,
            );
            if bonus != 0.0 {
                candidate.score -= bonus;
                changed = true;
            }
        }
        changed
    }

    pub(super) fn apply_stability_bonus_to_candidates(
        &self,
        current_key: &str,
        ranked: &mut [RankedCandidate],
    ) -> bool {
        let mut changed = false;
        for candidate in ranked {
            let bonus = stability_bonus_from_state(
                self.last_lookup.as_ref(),
                current_key,
                &candidate.phrase,
            );
            if bonus != 0.0 {
                candidate.score += bonus;
                changed = true;
            }
        }
        changed
    }
}

fn stability_bonus_from_state(
    previous: Option<&LookupStabilityState>,
    current_key: &str,
    phrase: &str,
) -> f64 {
    let Some(prev) = previous else {
        return 0.0;
    };
    if current_key.is_empty() || prev.key.is_empty() {
        return 0.0;
    }
    let Some(prev_rank) = prev
        .top_phrases
        .iter()
        .position(|prev_phrase| prev_phrase == phrase)
    else {
        return 0.0;
    };
    let delta = current_key.len().abs_diff(prev.key.len());
    if delta > 2 {
        return 0.0;
    }
    if !(current_key.starts_with(&prev.key) || prev.key.starts_with(current_key)) {
        return 0.0;
    }
    let rank_weight = ((LOOKUP_STABILITY_TOP_N - prev_rank) as f64 / LOOKUP_STABILITY_TOP_N as f64)
        .clamp(0.2, 1.0);
    let mut bonus = (LOOKUP_STABILITY_BONUS - delta as f64 * 2.5).max(0.0) * rank_weight;
    if current_key.chars().all(|ch| ch.is_ascii_alphabetic()) && current_key.chars().count() <= 4 {
        bonus += ABBREV_STABILITY_EXTRA_BONUS * rank_weight;
    }
    bonus
}

impl PinyinEngine {
    pub(super) fn promote_confident_top1_over_stability(
        &self,
        current_key: &str,
        ranked: &mut Vec<RankedCandidate>,
    ) -> bool {
        if ranked.len() < 2 || ranked[0].meta.pinned {
            return false;
        }
        let top_stability = self.stability_bonus_for_candidate(current_key, &ranked[0].phrase);
        if top_stability <= 0.0 {
            return false;
        }

        let top_without_stability = ranked[0].score - top_stability;
        let mut promote_idx = None;
        let mut best_margin = f64::NEG_INFINITY;
        for idx in 1..ranked.len().min(TOP1_STABILITY_RECHECK_LIMIT) {
            if ranked[idx].meta.pinned {
                continue;
            }
            let candidate_stability =
                self.stability_bonus_for_candidate(current_key, &ranked[idx].phrase);
            let candidate_without_stability = ranked[idx].score - candidate_stability;
            let margin = candidate_without_stability - top_without_stability;
            // Exact user and full-pinyin candidates are stronger evidence than
            // the previous prefix's display order.  They still need to win on
            // their raw score, but use a lower confidence threshold so a stale
            // stability bonus cannot keep an evidently better Top-1 stuck
            // behind it while a word is being completed.
            let confidence_margin = if matches!(
                ranked[idx].meta.match_kind,
                CandidateMatchKind::User | CandidateMatchKind::FullPinyin
            ) && !ranked[idx].meta.partial
            {
                TOP1_STABILITY_CONFIDENCE_MARGIN * 0.45
            } else {
                TOP1_STABILITY_CONFIDENCE_MARGIN
            };
            if margin >= confidence_margin && margin > best_margin {
                best_margin = margin;
                promote_idx = Some(idx);
            }
        }

        let Some(idx) = promote_idx else {
            return false;
        };
        let candidate = ranked.remove(idx);
        ranked.insert(0, candidate);
        true
    }
}

pub(super) fn merge_word_graph_edge(out: &mut HashMap<String, WordGraphEdge>, edge: WordGraphEdge) {
    if let Some(existing) = out.get_mut(&edge.phrase) {
        if edge.base_score > existing.base_score {
            *existing = edge;
        }
    } else {
        out.insert(edge.phrase.clone(), edge);
    }
}

pub(super) fn truncate_top_k_word_states(items: &mut Vec<WordGraphState>, k: usize) {
    if k == 0 || items.is_empty() {
        items.clear();
        return;
    }
    if items.len() <= k {
        items.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        return;
    }
    let nth = k - 1;
    items.select_nth_unstable_by(nth, |a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(k);
    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

pub(super) fn sort_ranked_candidates_top_k(items: &mut Vec<RankedCandidate>, k: usize) {
    if k == 0 {
        items.clear();
        return;
    }
    if items.len() <= 1 {
        return;
    }
    let compare = |a: &RankedCandidate, b: &RankedCandidate| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.phrase.cmp(&b.phrase))
    };
    if items.len() <= k {
        items.sort_by(compare);
        return;
    }
    let nth = k - 1;
    items.select_nth_unstable_by(nth, compare);
    items.truncate(k);
    items.sort_by(compare);
}

pub(super) fn sort_ranked_candidates_top_k_preserving(
    items: &mut Vec<RankedCandidate>,
    k: usize,
    protected_phrases: &[String],
) {
    if protected_phrases.is_empty() {
        sort_ranked_candidates_top_k(items, k);
        return;
    }
    if k == 0 {
        items.clear();
        return;
    }

    let protected_set = protected_phrases
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut protected = Vec::with_capacity(protected_phrases.len().min(k));
    let mut ordinary = Vec::with_capacity(items.len());
    for item in std::mem::take(items) {
        if protected_set.contains(item.phrase.as_str()) {
            protected.push(item);
        } else {
            ordinary.push(item);
        }
    }

    if protected.len() > k {
        let desired_order = protected_phrases
            .iter()
            .enumerate()
            .map(|(index, phrase)| (phrase.as_str(), index))
            .collect::<HashMap<_, _>>();
        protected.sort_by_key(|item| {
            desired_order
                .get(item.phrase.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        });
        protected.truncate(k);
        ordinary.clear();
    } else {
        sort_ranked_candidates_top_k(&mut ordinary, k - protected.len());
    }

    items.extend(ordinary);
    items.extend(protected);
    // Protection only controls top-k membership. Every retained candidate is
    // still ordered by the same score comparator as the non-partial path.
    let total = items.len();
    sort_ranked_candidates_top_k(items, total);
}

pub(super) fn prune_merged_candidates_for_rerank<'a>(
    merged: &mut MergedCandidateMap,
    keep: usize,
    protected_phrases: impl Iterator<Item = &'a String>,
) -> usize {
    if keep == 0 {
        let removed = merged.len();
        merged.clear();
        return removed;
    }
    if merged.len() <= keep {
        return 0;
    }

    let protected = protected_phrases.cloned().collect::<HashSet<_>>();
    let protected_existing = protected
        .iter()
        .filter(|phrase| merged.contains_phrase(phrase))
        .count();
    let scored_keep = keep
        .saturating_sub(protected_existing)
        .max(TSF_PAGE_SIZE.min(keep));

    let mut scored = merged
        .candidates()
        .filter(|(phrase, _)| !protected.contains(*phrase))
        .map(|(phrase, candidate)| (phrase.to_string(), candidate.score))
        .collect::<Vec<_>>();
    if scored.len() <= scored_keep {
        return 0;
    }

    let nth = scored_keep - 1;
    scored.select_nth_unstable_by(nth, |a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.truncate(scored_keep);
    let score_keep_phrases = scored
        .into_iter()
        .map(|(phrase, _)| phrase)
        .collect::<HashSet<_>>();

    let before = merged.len();
    merged
        .retain_phrases(|phrase| protected.contains(phrase) || score_keep_phrases.contains(phrase));
    before.saturating_sub(merged.len())
}

pub(super) fn merge_phrase_entries<'a>(
    out: &mut MergedCandidateMap,
    entries: impl Iterator<Item = &'a ThuoclEntry>,
    bonus: f64,
    limit: usize,
    user_lexicon: &UserLexicon,
    now: u64,
    meta: Option<&str>,
    source_lexicon: Option<&AbbrevLexicon>,
    short_abbrev_input_chars: Option<usize>,
) {
    for (rank, entry) in entries.take(limit).enumerate() {
        let score = bonus
            + system_frequency_prior(entry, meta, short_abbrev_input_chars)
            + short_abbrev_user_bonus(user_lexicon, &entry.phrase, now)
            + signal_bonus(
                user_lexicon.phrase_signal(&entry.phrase),
                now,
                USER_PHRASE_SCORE_SCALE,
                USER_RECENCY_SCALE,
            )
            - rank as f64 * 0.01;
        let candidate_meta = candidate_meta_from_legacy(meta).with_source_layer(
            source_lexicon
                .map(|lexicon| lexicon.phrase_layer(&entry.phrase))
                .unwrap_or_default(),
        );
        if let Some(phrase_id) = source_lexicon.and_then(|lexicon| lexicon.phrase_id(&entry.phrase))
        {
            merge_system_candidate_meta(
                out,
                phrase_id,
                entry.phrase.clone(),
                score,
                candidate_meta,
            );
        } else {
            merge_candidate_meta(out, entry.phrase.clone(), score, candidate_meta);
        }
    }
}

/// 系统词频只形成一个先验。短简拼的长度保护和 Top1 校准取较强者，
/// 不再与基础词频逐项叠加，避免高频二字词被重复放大。
fn system_frequency_prior(
    entry: &ThuoclEntry,
    meta: Option<&str>,
    short_abbrev_input_chars: Option<usize>,
) -> f64 {
    let base = (entry.freq as f64 + 1.0).ln();
    let short_prior = short_abbrev_frequency_bonus(entry).max(short_abbrev_top1_frequency_bonus(
        entry,
        meta,
        short_abbrev_input_chars,
    ));
    base + short_prior
}

pub(super) fn short_abbrev_top1_frequency_bonus(
    entry: &ThuoclEntry,
    meta: Option<&str>,
    input_chars: Option<usize>,
) -> f64 {
    let Some(input_chars) = input_chars else {
        return 0.0;
    };
    if meta != Some(ABBREV_CANDIDATE_META)
        || !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&input_chars)
        || phrase_char_count(&entry.phrase) != input_chars
        || entry.freq == 0
    {
        return 0.0;
    }
    ((entry.freq as f64 + 1.0).ln() * SHORT_ABBREV_TOP1_FREQ_BONUS_SCALE
        + SHORT_ABBREV_TOP1_FREQ_BONUS_OFFSET)
        .clamp(0.0, SHORT_ABBREV_TOP1_FREQ_BONUS_CAP)
}

pub(super) fn merge_user_entries<'a>(
    out: &mut MergedCandidateMap,
    entries: impl Iterator<Item = &'a LearnedEntry>,
    bonus: f64,
    limit: usize,
    now: u64,
    meta: Option<&str>,
    source_lexicon: Option<&AbbrevLexicon>,
    exact_hotword_syllables: Option<usize>,
) {
    let hotword_prefs = user_hotword_prefs::get_user_hotword_prefs();
    for (rank, entry) in entries.take(limit).enumerate() {
        let phrase_chars = phrase_char_count(&entry.phrase);
        let exact_hotword =
            is_exact_user_hotword(entry, meta, phrase_chars, exact_hotword_syllables);
        let mut trust = user_entry_lexicon_trust(entry, now, source_lexicon);
        if exact_hotword {
            trust = if phrase_chars == 1 {
                1.0
            } else {
                trust.max(hotword_prefs.trust_floor)
            };
        }
        let mut effective_bonus_base = bonus;
        let mut learned_bonus = input_entry_bonus(entry, now);
        if exact_hotword {
            effective_bonus_base *= hotword_prefs.exact_bonus_scale;
            learned_bonus *= hotword_prefs.signal_scale;
        }
        let confidence_scale = entry.suggestion_score_scale();
        effective_bonus_base *= confidence_scale;
        learned_bonus *= confidence_scale;
        let effective_bonus = if entry.is_pinned_exact_input() {
            effective_bonus_base
        } else {
            effective_bonus_base * trust
        };
        let score = effective_bonus + learned_bonus * trust - rank as f64 * 0.02;
        if meta == Some(USER_CANDIDATE_META) {
            merge_candidate_meta(
                out,
                entry.phrase.clone(),
                score,
                CandidateMeta::user_with_freq(entry.is_pinned_exact_input(), entry.freq),
            );
        } else {
            merge_candidate(out, entry.phrase.clone(), score, meta);
        }
    }
}

fn is_exact_user_hotword(
    entry: &LearnedEntry,
    meta: Option<&str>,
    phrase_chars: usize,
    exact_hotword_syllables: Option<usize>,
) -> bool {
    if !(1..=SHORT_HOTWORD_MAX_CHARS).contains(&phrase_chars) {
        return false;
    }
    let exact_user_source =
        meta == Some(USER_CANDIDATE_META) || (meta.is_none() && phrase_chars == 1);
    if !exact_user_source {
        return false;
    }
    if entry.is_pinned_exact_input() {
        return true;
    }
    match exact_hotword_syllables {
        Some(count) => count == phrase_chars,
        None => true,
    }
}

pub(super) fn merge_scored_candidates(
    out: &mut MergedCandidateMap,
    entries: impl Iterator<Item = (String, f64)>,
    penalty: f64,
    user_lexicon: &UserLexicon,
    now: u64,
    meta: Option<&str>,
) {
    for (phrase, base_score) in entries {
        let score = base_score
            + signal_bonus(
                user_lexicon.phrase_signal(&phrase),
                now,
                USER_PHRASE_SCORE_SCALE,
                USER_RECENCY_SCALE,
            )
            - penalty;
        merge_candidate(out, phrase, score, meta);
    }
}

pub(super) fn input_entry_bonus(entry: &LearnedEntry, now: u64) -> f64 {
    let bonus = signal_bonus(
        entry.stats(),
        now,
        USER_EXACT_INPUT_SCALE,
        USER_RECENCY_SCALE,
    );
    if !entry.is_pinned_exact_input() && entry.stats().freq <= 1 {
        bonus.min(FIRST_LEARNED_USER_SOFT_CAP)
    } else {
        bonus
    }
}

pub(super) fn user_entry_lexicon_trust(
    entry: &LearnedEntry,
    now: u64,
    source_lexicon: Option<&AbbrevLexicon>,
) -> f64 {
    if entry.is_pinned_exact_input() {
        return 1.0;
    }
    if now.saturating_sub(entry.last_used) <= 32 {
        return 1.0;
    }
    let Some(lexicon) = source_lexicon else {
        return 1.0;
    };
    let freq = lexicon.phrase_frequency(&entry.phrase);
    if freq == 0 || freq < USER_LOW_LEXICON_FREQ_THRESHOLD {
        USER_LEXICON_LOW_TRUST_MULTIPLIER
    } else {
        1.0
    }
}

pub(super) fn short_abbrev_frequency_bonus(entry: &ThuoclEntry) -> f64 {
    let chars = phrase_char_count(&entry.phrase);
    if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&chars) || entry.freq == 0 {
        return 0.0;
    }
    let cap = match chars {
        2 => SHORT_ABBREV_FREQ_BONUS_CAP_TWO,
        3 => SHORT_ABBREV_FREQ_BONUS_CAP_THREE,
        4 => SHORT_ABBREV_FREQ_BONUS_CAP_FOUR,
        _ => return 0.0,
    };
    ((entry.freq as f64 + 1.0).ln() * SHORT_ABBREV_FREQ_BONUS_SCALE
        + SHORT_ABBREV_FREQ_BONUS_OFFSET)
        .clamp(0.0, cap)
}

pub(super) fn short_abbrev_user_bonus(lexicon: &UserLexicon, phrase: &str, now: u64) -> f64 {
    let signal = lexicon.phrase_signal(phrase);
    if signal.freq == 0 {
        return 0.0;
    }
    let mut bonus = signal_bonus(
        signal,
        now,
        USER_SHORT_ABBREV_FREQ_SCALE,
        USER_RECENCY_SCALE,
    )
    .min(USER_SHORT_ABBREV_BONUS_CAP);
    if signal.freq >= USER_PINNED_SHORT_ABBREV_FREQ_MIN {
        bonus += USER_PINNED_SHORT_ABBREV_BONUS;
    }
    bonus
}

pub(super) fn short_compact_backoff_penalty(missing_chars: usize, best_freq: u64) -> f64 {
    let base = missing_chars as f64 * SHORT_COMPACT_BACKOFF_STEP_PENALTY;
    let relief = ((best_freq as f64 + 1.0).ln() * 1.15).clamp(0.0, SHORT_FALLBACK_FREQ_RELIEF_CAP);
    (base - relief).max(0.0)
}

pub(super) fn signal_bonus(
    stats: LearnedStats,
    now: u64,
    freq_scale: f64,
    recency_scale: f64,
) -> f64 {
    if stats.freq == 0 {
        return 0.0;
    }

    let age = now.saturating_sub(stats.last_used) as f64;
    let (short_count, long_count) = stats.effective_counts(
        now,
        SHORT_TERM_FREQ_DECAY_HALF_LIFE,
        LONG_TERM_FREQ_DECAY_HALF_LIFE,
    );
    let short_freq_bonus = (short_count + 1.0).ln() * freq_scale * SHORT_TERM_SIGNAL_WEIGHT;
    let long_freq_bonus = (long_count + 1.0).ln() * freq_scale * LONG_TERM_SIGNAL_WEIGHT;
    let recency_bonus = if stats.last_used == 0 {
        0.0
    } else {
        recency_scale * RECENCY_SIGNAL_WEIGHT / (age + 1.0).sqrt()
    };
    short_freq_bonus + long_freq_bonus + recency_bonus
}

pub(super) fn phrase_span_bonus(
    len: usize,
    lex_freq: u64,
    user_signal: LearnedStats,
    now: u64,
) -> f64 {
    let lex_bonus = if lex_freq == 0 {
        0.0
    } else {
        (lex_freq as f64 + 1.0).ln() * SENTENCE_WORD_FREQ_SCALE
    };
    (len as f64 - 1.0) * SENTENCE_WORD_LEN_SCALE
        + lex_bonus
        + signal_bonus(
            user_signal,
            now,
            USER_PHRASE_SCORE_SCALE * 0.8,
            USER_RECENCY_SCALE,
        )
}

pub(super) fn merge_candidate(
    out: &mut MergedCandidateMap,
    phrase: String,
    score: f64,
    meta: Option<&str>,
) {
    let incoming = MergedCandidate {
        phrase: phrase.clone(),
        score,
        meta: candidate_meta_from_legacy(meta),
    };
    if let Some(&phrase_id) = out.system_phrase_ids.get(&phrase) {
        match out.system.entry(phrase_id) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                merge_into_candidate(slot.get_mut(), incoming);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(incoming);
            }
        }
        return;
    }
    match out.dynamic.entry(phrase) {
        std::collections::hash_map::Entry::Occupied(mut slot) => {
            merge_into_candidate(slot.get_mut(), incoming);
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(incoming);
        }
    }
}

pub(super) fn merge_candidate_meta(
    out: &mut MergedCandidateMap,
    phrase: String,
    score: f64,
    meta: CandidateMeta,
) {
    let incoming = MergedCandidate {
        phrase: phrase.clone(),
        score,
        meta,
    };
    if let Some(&phrase_id) = out.system_phrase_ids.get(&phrase) {
        match out.system.entry(phrase_id) {
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                merge_into_candidate(slot.get_mut(), incoming);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(incoming);
            }
        }
        return;
    }
    match out.dynamic.entry(phrase) {
        std::collections::hash_map::Entry::Occupied(mut slot) => {
            merge_into_candidate(slot.get_mut(), incoming);
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(incoming);
        }
    }
}

pub(super) fn merge_best_candidate_map(
    out: &mut MergedCandidateMap,
    mut incoming: MergedCandidateMap,
) {
    let Some((phrase, candidate)) = incoming.drain_candidates().into_iter().max_by(|a, b| {
        a.1.score
            .partial_cmp(&b.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.0.cmp(&a.0))
    }) else {
        return;
    };

    merge_candidate_meta(out, phrase, candidate.score, candidate.meta);
}

fn merge_system_candidate_meta(
    out: &mut MergedCandidateMap,
    phrase_id: u32,
    phrase: String,
    score: f64,
    meta: CandidateMeta,
) {
    let mut incoming = MergedCandidate {
        phrase: phrase.clone(),
        score,
        meta,
    };
    if let Some(dynamic) = out.dynamic.remove(&phrase) {
        merge_into_candidate(&mut incoming, dynamic);
    }
    out.system_phrase_ids.insert(phrase, phrase_id);
    match out.system.entry(phrase_id) {
        std::collections::hash_map::Entry::Occupied(mut slot) => {
            merge_into_candidate(slot.get_mut(), incoming);
        }
        std::collections::hash_map::Entry::Vacant(slot) => {
            slot.insert(incoming);
        }
    }
}

pub(super) fn merge_into_candidate(current: &mut MergedCandidate, incoming: MergedCandidate) {
    const EPSILON: f64 = 1e-9;

    let current_priority = merged_meta_priority(&current.meta);
    let incoming_priority = merged_meta_priority(&incoming.meta);

    if incoming.score > current.score + EPSILON {
        if current_priority > incoming_priority
            && incoming.score - current.score <= META_STICKY_SCORE_MARGIN
        {
            current.score = incoming.score;
        } else {
            *current = incoming;
        }
        return;
    }

    if current.score > incoming.score + EPSILON {
        if incoming_priority > current_priority
            && current.score - incoming.score <= META_STICKY_SCORE_MARGIN
        {
            current.meta = incoming.meta;
        } else if incoming.meta.match_kind == CandidateMatchKind::User && !current.meta.user_signal
        {
            // 同词系统候选分数高出 META_STICKY_SCORE_MARGIN：保留系统的
            // 排序位置，但带上用户学习信号，避免反复学习也无法前置。
            current.meta.user_signal = true;
        }
        return;
    }

    if incoming_priority > current_priority {
        *current = incoming;
    }
}

pub(super) fn merged_meta_priority(meta: &CandidateMeta) -> u8 {
    match meta.match_kind {
        CandidateMatchKind::User => 8,
        CandidateMatchKind::Direct
        | CandidateMatchKind::Shortcut
        | CandidateMatchKind::DateTime => 7,
        CandidateMatchKind::MixedPrefix
        | CandidateMatchKind::FullPinyin
        | CandidateMatchKind::ShortAbbrev
        | CandidateMatchKind::English => 6,
        CandidateMatchKind::Correction => 5,
        CandidateMatchKind::SingleSyllable => 3,
        CandidateMatchKind::Unknown if meta.display_text().is_some() => 2,
        CandidateMatchKind::Unknown => 0,
    }
}
