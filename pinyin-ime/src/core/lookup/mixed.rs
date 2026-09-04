use super::*;

impl PinyinEngine {
    pub(in crate::core) fn rank_syllable_completions(
        &self,
        leading_syllables: &[String],
        tail: &str,
        now: u64,
        limit: usize,
    ) -> Vec<String> {
        let leading_key = leading_syllables.join("");
        predict_syllable_completions_by_score(tail, self.syllables.as_ref(), limit, |pred| {
            self.syllable_completion_probability_score(&leading_key, pred, now, false)
        })
    }

    pub(in crate::core) fn syllable_completion_probability_score(
        &self,
        leading_key: &str,
        predicted_syllable: &str,
        now: u64,
        compositional_suffix: bool,
    ) -> f64 {
        let mut full_key = String::with_capacity(leading_key.len() + predicted_syllable.len());
        full_key.push_str(leading_key);
        full_key.push_str(predicted_syllable);

        let user_hit_score = self
            .user_lexicon
            .lookup_input(&full_key)
            .map(|entries| {
                entries
                    .iter()
                    .take(6)
                    .enumerate()
                    .map(|(rank, entry)| input_entry_bonus(entry, now) - rank as f64 * 0.25)
                    .sum::<f64>()
            })
            .unwrap_or(0.0);

        let lex_freq_score = self
            .phrase_lexicon
            .as_ref()
            .and_then(|lex| lex.lookup_pinyin_hot(&full_key))
            .map(|entries| {
                entries
                    .iter()
                    .take(8)
                    .enumerate()
                    .map(|(rank, entry)| (entry.freq as f64 + 1.0).ln() - rank as f64 * 0.16)
                    .sum::<f64>()
            })
            .unwrap_or(0.0);

        let lex_prefix_score = if leading_key.is_empty() || full_key.chars().count() < 4 {
            0.0
        } else if let Some(lex) = &self.phrase_lexicon {
            lex.pinyin_prefix_probability_score_until(&full_key, || {
                current_lookup_request_superseded()
            })
            .unwrap_or(0.0)
        } else {
            0.0
        };

        // Mixed input often spells a known phrase only at the tail of a
        // longer composition (for example `...w` + `l` -> `wangluo`). A
        // whole-key prefix score cannot see that compositional boundary and
        // used to prune the correct final syllable before word-graph decode.
        // Reward the strongest bounded suffix that is itself a lexicon key.
        let lex_suffix_score = if !compositional_suffix || full_key.len() < 10 {
            0.0
        } else {
            self.phrase_lexicon
                .as_ref()
                .map(|lex| {
                    let min_start = full_key.len().saturating_sub(12);
                    (min_start..full_key.len())
                        .filter(|start| full_key.is_char_boundary(*start))
                        .filter_map(|start| {
                            let suffix = &full_key[start..];
                            (suffix.len() >= 4 && suffix.len() < full_key.len())
                                .then(|| lex.lookup_pinyin_hot(suffix))
                                .flatten()
                        })
                        .map(|entries| {
                            entries
                                .iter()
                                .filter(|entry| phrase_char_count(&entry.phrase) >= 2)
                                .take(4)
                                .map(|entry| (entry.freq as f64 + 1.0).ln())
                                .fold(0.0_f64, f64::max)
                        })
                        .fold(0.0_f64, f64::max)
                })
                .unwrap_or(0.0)
        };

        // Prefer completions that turn the expanded key into two known words,
        // such as `jisuanji` + `wangluo`. Looking only at the last word makes
        // unrelated but frequent tails (`wuli`, `liang`, ...) dominate the
        // global mixed beam.
        let lex_partition_score = if !compositional_suffix || full_key.len() < 10 {
            0.0
        } else {
            self.phrase_lexicon
                .as_ref()
                .map(|lex| {
                    let phrase_key_score = |key: &str| {
                        lex.lookup_pinyin_hot(key)
                            .map(|entries| {
                                entries
                                    .iter()
                                    .filter(|entry| phrase_char_count(&entry.phrase) >= 2)
                                    .take(6)
                                    .map(|entry| (entry.freq as f64 + 1.0).ln())
                                    .fold(0.0_f64, f64::max)
                            })
                            .unwrap_or(0.0)
                    };
                    (4..full_key.len().saturating_sub(3))
                        .filter(|split| full_key.is_char_boundary(*split))
                        .filter_map(|split| {
                            let left = phrase_key_score(&full_key[..split]);
                            let right = phrase_key_score(&full_key[split..]);
                            (left > 0.0 && right > 0.0).then_some(left + right)
                        })
                        .fold(0.0_f64, f64::max)
                })
                .unwrap_or(0.0)
        };

        let dict_prior_score = self
            .dict
            .get(predicted_syllable)
            .and_then(|chars| {
                let ranked = self.lm.rank_chars_by_unigram_with_limit(chars, 6);
                if ranked.is_empty() {
                    None
                } else {
                    Some(
                        ranked
                            .iter()
                            .map(|ch| (self.lm.log_unigram(*ch) + 25.0).clamp(0.0, 25.0))
                            .sum::<f64>()
                            / ranked.len() as f64,
                    )
                }
            })
            .unwrap_or(0.0);

        user_hit_score * SYLLABLE_PREDICT_USER_HIT_SCALE
            + lex_freq_score * SYLLABLE_PREDICT_LEX_FREQ_SCALE
            + lex_prefix_score * SYLLABLE_PREDICT_LEX_PREFIX_SCALE
            + lex_suffix_score * SYLLABLE_PREDICT_LEX_SUFFIX_SCALE
            + lex_partition_score * SYLLABLE_PREDICT_LEX_PARTITION_SCALE
            + dict_prior_score * SYLLABLE_PREDICT_DICT_PRIOR_SCALE
    }

    /// 根据当前模式选择 beam search 宽度：简拼模式下使用较小 beam 以降低延迟。
    #[inline]
    pub(in crate::core) fn effective_decode_beam(&self) -> usize {
        if self.mode_flags & MODE_JIANPIN != 0 {
            ABBREV_DECODE_BEAM
        } else {
            BASE_DECODE_BEAM
        }
    }

    /// 长输入的候选来源更多，动态收窄 beam 可以压住最坏路径延迟。
    #[inline]
    pub(in crate::core) fn effective_decode_beam_for_key(&self, compact_key: &str) -> usize {
        let base = self.effective_decode_beam();
        match compact_key.chars().count() {
            len if len >= 10 => (base / 3).max(8),
            len if len >= 7 => (base / 2).max(12),
            _ => base,
        }
    }

    pub(in crate::core) fn decode_chars_incremental(
        &self,
        syllables: &[String],
        beam_size: usize,
        max_candidates: usize,
    ) -> Vec<(String, f64)> {
        self.incremental_decode_cache.borrow_mut().decode(
            self.dict.as_ref(),
            self.lm.as_ref(),
            syllables,
            beam_size,
            max_candidates,
        )
    }

    pub(in crate::core) fn decode_chars_incremental_until(
        &self,
        syllables: &[String],
        beam_size: usize,
        max_candidates: usize,
        should_stop: impl FnMut() -> bool,
    ) -> (Vec<(String, f64)>, bool) {
        self.incremental_decode_cache.borrow_mut().decode_until(
            self.dict.as_ref(),
            self.lm.as_ref(),
            syllables,
            beam_size,
            max_candidates,
            should_stop,
        )
    }

    #[inline]
    pub(in crate::core) fn exact_abbrev_bonus(&self) -> f64 {
        PHRASE_EXACT_ABBREV_BONUS
            + if self.mode_flags & MODE_JIANPIN != 0 {
                35.0
            } else {
                0.0
            }
    }

    #[inline]
    pub(in crate::core) fn prefix_abbrev_bonus(&self) -> f64 {
        PHRASE_PREFIX_ABBREV_BONUS
            + if self.mode_flags & MODE_JIANPIN != 0 {
                28.0
            } else {
                0.0
            }
    }

    #[inline]
    pub(in crate::core) fn mixed_pinyin_bonus(&self) -> f64 {
        PHRASE_EXACT_MIXED_PINYIN_BONUS
            + if self.mode_flags & MODE_MIXED_PINYIN != 0 {
                20.0
            } else {
                0.0
            }
    }

    #[inline]
    pub(in crate::core) fn jianpin_enabled(&self) -> bool {
        self.mode_flags & MODE_JIANPIN != 0
    }

    #[inline]
    pub(in crate::core) fn mixed_pinyin_enabled(&self) -> bool {
        self.mode_flags & MODE_MIXED_PINYIN != 0
    }

    #[inline]
    pub(in crate::core) fn mixed_pinyin_aggressive(&self) -> bool {
        self.mode_flags & MODE_MIXED_PINYIN_AGGRESSIVE != 0
    }

    #[inline]
    pub(in crate::core) fn english_word_input_enabled(&self) -> bool {
        self.mode_flags & MODE_ENGLISH_WORD_INPUT != 0
    }

    pub(in crate::core) fn mixed_pinyin_key_limit(&self, compact_key: &str) -> usize {
        let len = compact_key.chars().count();
        if !self.mixed_pinyin_enabled() || len < 3 {
            0
        } else if len == 3 && !self.mixed_pinyin_aggressive() {
            MIXED_PINYIN_SHORT_KEY_LIMIT
        } else if len == 4 && !self.mixed_pinyin_aggressive() {
            MIXED_PINYIN_FOUR_CHAR_KEY_LIMIT
        } else if len >= 10 {
            MIXED_PINYIN_LONG_KEY_LIMIT
        } else if len >= 6 {
            MIXED_PINYIN_MEDIUM_KEY_LIMIT
        } else {
            MIXED_PINYIN_KEY_LIMIT
        }
    }

    pub(in crate::core) fn mixed_key_expansions_until<S>(
        &self,
        compact_key: &str,
        now: u64,
        limit: usize,
        mut should_stop: S,
    ) -> (Vec<MixedPinyinKeyExpansion>, bool)
    where
        S: FnMut() -> bool,
    {
        if limit == 0 {
            return (Vec::new(), true);
        }
        let cache_key = format!("{compact_key}\t{limit}");
        if let Ok(mut cache) = self.mixed_expansion_cache.lock() {
            if let Some(cached) = cache.get(&cache_key) {
                return (cached.iter().cloned().collect(), true);
            }
        }

        let compositional_suffix = self.long_mixed_composition_intent(compact_key);
        // One mixed beam asks for the same completed key from multiple parser
        // patterns and once again while materializing the next stage. Keep the
        // lexicon-heavy composition score local to this expansion.
        let mut score_cache = HashMap::<String, f64>::new();
        let mut completion_score = |leading: &str, syllable: &str| {
            let mut full_key = String::with_capacity(leading.len() + syllable.len());
            full_key.push_str(leading);
            full_key.push_str(syllable);
            if let Some(score) = score_cache.get(&full_key) {
                return *score;
            }
            let shared_cache_key = mixed_completion_score_cache_key(
                &full_key,
                compositional_suffix,
                now,
                self.cache_epoch,
            );
            if let Ok(mut cache) = self.mixed_completion_score_cache.lock() {
                if let Some(score) = cache.get(&shared_cache_key) {
                    score_cache.insert(full_key, score);
                    return score;
                }
            }
            let score = self.syllable_completion_probability_score(
                leading,
                syllable,
                now,
                compositional_suffix,
            );
            if let Ok(mut cache) = self.mixed_completion_score_cache.lock() {
                cache.insert(shared_cache_key, score);
            }
            score_cache.insert(full_key, score);
            score
        };
        let (expanded, complete) = if let Ok(mut cache) = self.mixed_incremental_cache.lock() {
            cache.expand_by_score_until(
                compact_key,
                self.syllables.as_ref(),
                limit,
                &mut completion_score,
                &mut should_stop,
            )
        } else {
            let expanded = expand_mixed_pinyin_exact_key_details_by_score(
                compact_key,
                self.syllables.as_ref(),
                limit,
                &mut completion_score,
            );
            let complete = !should_stop();
            (expanded, complete)
        };
        if complete {
            if let Ok(mut cache) = self.mixed_expansion_cache.lock() {
                cache.put(cache_key, Arc::from(expanded.clone()));
            }
        }
        (expanded, complete)
    }

    pub(in crate::core) fn long_mixed_composition_intent(&self, compact_key: &str) -> bool {
        if compact_key.len() < 9 || !compact_key.is_ascii() {
            return false;
        }
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return false;
        };
        // A known full-pinyin prefix followed by two to four initials is the
        // ambiguous shape that needs suffix-aware completion ranking. Keep it
        // off ordinary long full-pinyin paths.
        (2..=4).any(|suffix_len| {
            let Some(split) = compact_key.len().checked_sub(suffix_len) else {
                return false;
            };
            lex.lookup_pinyin_hot(&compact_key[..split])
                .is_some_and(|entries| {
                    entries
                        .iter()
                        .any(|entry| phrase_char_count(&entry.phrase) >= 2)
                })
        })
    }

    pub(in crate::core) fn mixed_expansion_known_word_partition_score(
        &self,
        expansion: &MixedPinyinKeyExpansion,
    ) -> Option<KnownMixedWordPartition> {
        let lex = self.phrase_lexicon.as_ref()?;
        let mut best: Option<KnownMixedWordPartition> = None;
        let mut split = 0usize;
        for syllable_index in 1..expansion.syllables.len() {
            split += expansion.syllables[syllable_index - 1].len();
            let left_chars = syllable_index;
            let right_chars = expansion.syllables.len() - syllable_index;
            if left_chars < 2 || right_chars < 2 {
                continue;
            }
            let left_freq = lex
                .lookup_pinyin_hot(&expansion.key[..split])
                .and_then(|entries| {
                    entries
                        .iter()
                        .filter(|entry| phrase_char_count(&entry.phrase) == left_chars)
                        .map(|entry| entry.freq)
                        .max()
                });
            let right_freq = lex
                .lookup_pinyin_hot(&expansion.key[split..])
                .and_then(|entries| {
                    entries
                        .iter()
                        .filter(|entry| phrase_char_count(&entry.phrase) == right_chars)
                        .map(|entry| entry.freq)
                        .max()
                });
            let (Some(left_freq), Some(right_freq)) = (left_freq, right_freq) else {
                continue;
            };
            let score = (left_freq as f64 + 1.0).ln() + (right_freq as f64 + 1.0).ln();
            let candidate = KnownMixedWordPartition {
                score,
                split_syllable_index: syllable_index,
                split_key_byte: split,
                left_freq,
                right_freq,
            };
            if best
                .as_ref()
                .is_none_or(|current| candidate.score > current.score)
            {
                best = Some(candidate);
            }
        }
        best
    }

    pub(in crate::core) fn mixed_known_boundary_candidate_adjustment(
        &self,
        expansion: &MixedPinyinKeyExpansion,
        partition: &KnownMixedWordPartition,
        phrase: &str,
    ) -> f64 {
        if phrase_char_count(phrase) != expansion.syllables.len() {
            return 0.0;
        }
        let split_char = partition.split_syllable_index;
        let split_byte = phrase
            .char_indices()
            .nth(split_char)
            .map(|(index, _)| index)
            .unwrap_or(phrase.len());
        let (left_phrase, right_phrase) = phrase.split_at(split_byte);
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return 0.0;
        };
        let left_matches = lex
            .lookup_pinyin_hot(&expansion.key[..partition.split_key_byte])
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == left_phrase));
        let right_matches = lex
            .lookup_pinyin_hot(&expansion.key[partition.split_key_byte..])
            .is_some_and(|entries| entries.iter().any(|entry| entry.phrase == right_phrase));
        if left_matches && right_matches {
            let balance = (partition.left_freq.min(partition.right_freq) as f64 + 1.0)
                .ln()
                .min(12.0)
                * 0.5;
            MIXED_PINYIN_KNOWN_BOUNDARY_MATCH_BONUS + balance
        } else {
            -MIXED_PINYIN_KNOWN_BOUNDARY_MISMATCH_PENALTY
        }
    }
}
