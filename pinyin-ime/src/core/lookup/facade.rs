use super::*;

impl PinyinEngine {
    pub(in crate::core) fn apply_cached_context_rerank(
        &self,
        compact_key: &str,
        cached: &mut Vec<RankedCandidate>,
    ) {
        let intrinsic_top = cached.first().map(|candidate| candidate.phrase.clone());
        let mut changed = self.apply_stability_bonus_to_candidates(compact_key, cached);
        let feedback_changed = self.apply_selection_feedback(compact_key, cached)
            | self.apply_final_negative_selection_feedback(compact_key, cached);
        changed |= feedback_changed;
        if changed {
            sort_ranked_candidates_top_k(cached, TSF_MAX_CANDIDATES * 2);
        }
        self.promote_confident_top1_over_stability(compact_key, cached);
        if !feedback_changed {
            if let Some(intrinsic_top) = intrinsic_top {
                if let Some(index) = cached
                    .iter()
                    .position(|candidate| candidate.phrase == intrinsic_top)
                {
                    if index > 0 {
                        let candidate = cached.remove(index);
                        cached.insert(0, candidate);
                    }
                }
            }
        }
    }

    pub(in crate::core) fn remove_context_rerank_before_cache(
        &self,
        compact_key: &str,
        cached: &mut Vec<RankedCandidate>,
    ) {
        let mut changed = self.remove_stability_bonus_from_candidates(compact_key, cached);
        changed |= self.remove_selection_feedback(compact_key, cached);
        changed |= self.remove_final_negative_selection_feedback(compact_key, cached);
        let _ = changed;
    }

    pub fn lookup_full(&mut self, pinyin_buffer: &str) -> (Vec<(String, f64)>, Option<String>) {
        let (detailed, error) = self.lookup_full_detailed(pinyin_buffer);
        (
            detailed
                .into_iter()
                .map(|item| (item.phrase, item.score))
                .collect(),
            error,
        )
    }

    /// Runs an interactive lookup with a monotonically increasing request id,
    /// enabling soft-budget and supersession behavior used by the TSF IPC path.
    pub fn lookup_full_with_request_id(
        &mut self,
        pinyin_buffer: &str,
        request_id: u64,
    ) -> (Vec<(String, f64)>, Option<String>) {
        let (detailed, error) =
            self.lookup_full_detailed_with_request_id(pinyin_buffer, request_id);
        (
            detailed
                .into_iter()
                .map(|item| (item.phrase, item.score))
                .collect(),
            error,
        )
    }

    pub fn lookup_full_explain(
        &mut self,
        pinyin_buffer: &str,
    ) -> (Vec<(String, f64, CandidateMeta)>, Option<String>) {
        let (detailed, error) = self.lookup_full_detailed(pinyin_buffer);
        (
            detailed
                .into_iter()
                .map(|item| (item.phrase, item.score, item.meta))
                .collect(),
            error,
        )
    }

    pub fn lookup_with_syllable_predictions(
        &mut self,
        pinyin_buffer: &str,
        request_id: u64,
    ) -> (
        Vec<(String, f64, CandidateMeta)>,
        Vec<String>,
        Option<String>,
    ) {
        let predictions = self.syllable_predictions(pinyin_buffer);
        let (detailed, error) =
            self.lookup_full_detailed_with_request_id(pinyin_buffer, request_id);
        (
            detailed
                .into_iter()
                .map(|item| (item.phrase, item.score, item.meta))
                .collect(),
            predictions,
            error,
        )
    }

    pub fn lookup_tsf_candidates(&mut self, pinyin_buffer: &str) -> Vec<String> {
        let (v, _) = self.lookup_full_detailed(pinyin_buffer);
        v.into_iter()
            .take(LOOKUP_FULL_MAX_CANDIDATES)
            .map(|item| item.phrase)
            .collect()
    }

    pub fn lookup_first_page(&mut self, pinyin_buffer: &str) -> Vec<String> {
        self.lookup_tsf_candidates(pinyin_buffer)
            .into_iter()
            .take(TSF_PAGE_SIZE)
            .collect()
    }

    pub fn syllable_predictions(&self, pinyin_buffer: &str) -> Vec<String> {
        match split_syllables_with_tail_mode(
            pinyin_buffer,
            self.syllables.as_ref(),
            self.parse_options(),
        ) {
            Ok((syllables, Some(tail))) => self.rank_syllable_completions(
                &syllables,
                &tail,
                self.user_lexicon.current_clock(),
                40,
            ),
            _ => Vec::new(),
        }
    }

    pub fn input_intent(&self, pinyin_buffer: &str) -> InputIntent {
        let normalized = normalize_pinyin_line(pinyin_buffer);
        let raw = normalized.trim();
        if raw.is_empty() {
            return InputIntent::Unknown;
        }
        let compact_key = compact_lookup_key(raw);
        let direct_input_shortcut = self.is_effective_direct_input_shortcut(raw);
        let compact_is_exact_single_syllable =
            is_exact_complete_single_syllable_input(raw, &compact_key, self.syllables.as_ref());
        let parse_result =
            parse_input_variants_detailed(raw, self.syllables.as_ref(), self.parse_options());
        let exact_single_syllable_input = compact_is_exact_single_syllable
            || parse_result
                .as_ref()
                .ok()
                .map(|variants| has_exact_complete_single_syllable_variant(variants, &compact_key))
                .unwrap_or(false);
        let exact_complete_multi_input = parse_result.as_ref().ok().is_some_and(|variants| {
            variants.iter().any(|variant| {
                variant.source == ParseVariantSource::Exact
                    && variant.tail.is_none()
                    && variant.syllables.len() >= 2
            })
        });
        let mixed_limit = self.mixed_pinyin_key_limit(&compact_key);
        let mixed_keys = if mixed_limit == 0 {
            Vec::new()
        } else {
            expand_mixed_pinyin_exact_key_details_by_score(
                &compact_key,
                self.syllables.as_ref(),
                mixed_limit,
                |_, _| 0.0,
            )
        };
        let indexed_mixed_prefix_intent = self.mixed_pinyin_enabled()
            && !direct_input_shortcut
            && compact_key.chars().count() >= 4
            && self
                .phrase_lexicon
                .as_ref()
                .and_then(|lexicon| lexicon.lookup_mixed_hot(&compact_key))
                .is_some_and(|entries| !entries.is_empty());
        let mixed_prefix_intent =
            indexed_mixed_prefix_intent || mixed_keys.iter().any(is_mixed_prefix_input_intent);
        let short_phrase_intent = (is_pure_short_abbrev_initial_input(raw, &compact_key)
            && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&compact_key.chars().count()))
        .then_some(compact_key.chars().count());
        classify_input_intent(
            raw,
            &compact_key,
            direct_input_shortcut,
            self.english_word_input_enabled(),
            exact_single_syllable_input,
            exact_complete_multi_input,
            short_phrase_intent,
            mixed_prefix_intent,
        )
    }

    pub(in crate::core) fn strict_lexicon_frequency_order_context(
        &self,
        raw: &str,
        compact_key: &str,
    ) -> Option<(InputIntent, usize)> {
        match self.input_intent(raw) {
            InputIntent::ShortAbbrev if self.jianpin_enabled() => {
                Some((InputIntent::ShortAbbrev, compact_key.chars().count()))
            }
            InputIntent::FullPinyin => {
                let variants = self.parse_exact_variants_cached(raw).ok()?;
                let syllable_count = variants
                    .iter()
                    .find(|variant| {
                        variant.source == ParseVariantSource::Exact
                            && variant.tail.is_none()
                            && variant.syllables.len() >= 2
                            && variant.syllables.join("") == compact_key
                    })
                    .map(|variant| variant.syllables.len())?;
                Some((InputIntent::FullPinyin, syllable_count))
            }
            InputIntent::MixedPrefix => Some((InputIntent::MixedPrefix, 0)),
            _ => None,
        }
    }

    pub(in crate::core) fn parse_options(&self) -> ParseOptions {
        let fuzzy_enabled = self.mode_flags & MODE_FUZZY_PINYIN != 0;
        ParseOptions {
            fuzzy_enabled,
            double_pinyin_enabled: self.mode_flags & MODE_DOUBLE_PINYIN != 0,
            fuzzy_pairs: if fuzzy_enabled {
                crate::fuzzy_prefs::get_fuzzy_pairs()
            } else {
                crate::fuzzy_prefs::FuzzyPairs::default()
            },
            correction_enabled: crate::correction_prefs::get_correction_prefs().enabled,
        }
    }
}
