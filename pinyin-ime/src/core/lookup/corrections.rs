use super::*;

impl PinyinEngine {
    pub(in crate::core) fn merge_typo_corrections(
        &self,
        compact_key: &str,
        merged: &mut MergedCandidateMap,
        now: u64,
        suppress_correction: bool,
        budget: &mut CorrectionWorkBudget,
    ) {
        if suppress_correction {
            return;
        }
        let char_len = compact_key.chars().count();
        let max_edit = if char_len <= 5 { 1 } else { 2 };
        let correction_lookup_limit = if char_len >= LONG_CORRECTION_INPUT_CHARS {
            LONG_CORRECTION_LOOKUP_LIMIT
        } else {
            TYPO_LOOKUP_LIMIT
        };

        struct Row {
            phrase: String,
            score: f64,
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut push_row = |phrase: String, score: f64| {
            if let Some(existing) = rows.iter_mut().find(|r| r.phrase == phrase) {
                if score > existing.score {
                    existing.score = score;
                }
            } else {
                rows.push(Row { phrase, score });
            }
        };

        // 优先使用个人纠错记忆：raw_input -> corrected_pinyin。
        if let Some(pairs) = self.user_lexicon.lookup_correction_pairs(compact_key) {
            for (rank, entry) in pairs.iter().enumerate() {
                if budget.should_stop(rank) {
                    break;
                }
                let corrected = &entry.phrase;
                if let Ok((syllables, tail)) = split_syllables_with_tail_mode(
                    corrected,
                    self.syllables.as_ref(),
                    crate::segment::ParseOptions::default(),
                ) {
                    if tail.is_none() {
                        let corrected_key = syllables.join("");
                        if let Some(lex) = &self.phrase_lexicon {
                            if let Some(entries) = lex.lookup_pinyin(&corrected_key) {
                                for (entry_rank, phrase_entry) in entries.iter().take(3).enumerate()
                                {
                                    let score = USER_EXACT_INPUT_BONUS
                                        - TYPO_CORRECTION_PENALTY * 0.5
                                        + (phrase_entry.freq as f64 + 1.0).ln()
                                        + signal_bonus(
                                            self.user_lexicon.phrase_signal(&phrase_entry.phrase),
                                            now,
                                            USER_PHRASE_SCORE_SCALE,
                                            USER_RECENCY_SCALE,
                                        )
                                        - rank as f64 * 0.05
                                        - entry_rank as f64 * 0.01;
                                    push_row(phrase_entry.phrase.clone(), score);
                                }
                            }
                        }
                        let word_graph =
                            self.decode_word_graph_until(&syllables, now, || budget.should_stop(0));
                        merge_scored_candidates(
                            merged,
                            word_graph.into_iter(),
                            rank as f64 * 0.05,
                            &self.user_lexicon,
                            now,
                            Some(TYPO_CANDIDATE_META_MARKED),
                        );
                        let decoded = decode_until(
                            self.dict.as_ref(),
                            self.lm.as_ref(),
                            &syllables,
                            self.effective_decode_beam_for_key(compact_key),
                            decode_candidate_limit(syllables.len()),
                            || budget.should_stop(0),
                        );
                        merge_scored_candidates(
                            merged,
                            decoded.into_iter(),
                            rank as f64 * 0.05,
                            &self.user_lexicon,
                            now,
                            Some(TYPO_CANDIDATE_META_MARKED),
                        );
                    }
                }
            }
        }

        for (rank, (entry, dist)) in self
            .user_lexicon
            .approx_lookup_input_scored(compact_key, max_edit, correction_lookup_limit)
            .into_iter()
            .enumerate()
        {
            if budget.should_stop(rank) {
                break;
            }
            let mut score = USER_EXACT_INPUT_BONUS - TYPO_CORRECTION_PENALTY - rank as f64 * 0.02
                + input_entry_bonus(&entry, now);
            if dist >= 2 {
                score -= TYPO_DISTANCE2_EXTRA_PENALTY;
            }
            push_row(entry.phrase, score);
        }

        if let Some(lex) = &self.phrase_lexicon {
            for (rank, (entry, dist)) in lex
                .approx_lookup_scored_until(compact_key, 1, correction_lookup_limit, || {
                    budget.should_stop(0)
                })
                .into_iter()
                .enumerate()
            {
                if budget.should_stop(rank) {
                    break;
                }
                let mut score = self.exact_abbrev_bonus() - TYPO_CORRECTION_PENALTY
                    + (entry.freq as f64 + 1.0).ln()
                    + signal_bonus(
                        self.user_lexicon.phrase_signal(&entry.phrase),
                        now,
                        USER_PHRASE_SCORE_SCALE,
                        USER_RECENCY_SCALE,
                    )
                    - rank as f64 * 0.01;
                if dist >= 2 {
                    score -= TYPO_DISTANCE2_EXTRA_PENALTY;
                }
                push_row(entry.phrase, score);
            }

            for (rank, (entry, dist)) in lex
                .approx_lookup_pinyin_scored_until(compact_key, 1, correction_lookup_limit, || {
                    budget.should_stop(0)
                })
                .into_iter()
                .enumerate()
            {
                if budget.should_stop(rank) {
                    break;
                }
                let mut score = PHRASE_EXACT_PINYIN_BONUS - TYPO_CORRECTION_PENALTY
                    + (entry.freq as f64 + 1.0).ln()
                    + signal_bonus(
                        self.user_lexicon.phrase_signal(&entry.phrase),
                        now,
                        USER_PHRASE_SCORE_SCALE,
                        USER_RECENCY_SCALE,
                    )
                    - rank as f64 * 0.01;
                if dist >= 2 {
                    score -= TYPO_DISTANCE2_EXTRA_PENALTY;
                }
                push_row(entry.phrase, score);
            }

            let variant_limit = if char_len >= LONG_CORRECTION_INPUT_CHARS {
                LONG_MISSING_LETTER_VARIANT_LIMIT
            } else {
                128
            };
            for (correction_rank, corrected_key) in single_insert_ascii_variants(compact_key)
                .into_iter()
                .take(variant_limit)
                .enumerate()
            {
                if budget.should_stop(correction_rank) {
                    break;
                }
                let Some(entries) = lex.lookup_pinyin(&corrected_key) else {
                    continue;
                };
                for (entry_rank, entry) in entries.iter().take(3).enumerate() {
                    let score = PHRASE_EXACT_PINYIN_BONUS - TYPO_CORRECTION_PENALTY
                        + (entry.freq as f64 + 1.0).ln()
                        + signal_bonus(
                            self.user_lexicon.phrase_signal(&entry.phrase),
                            now,
                            USER_PHRASE_SCORE_SCALE,
                            USER_RECENCY_SCALE,
                        )
                        - correction_rank as f64 * 0.02
                        - entry_rank as f64 * 0.01;
                    push_row(entry.phrase.clone(), score);
                }
            }
        }

        rows.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
        rows.truncate(TYPO_RESULT_LIMIT);
        for row in rows {
            merge_candidate(
                merged,
                row.phrase,
                row.score,
                Some(TYPO_CANDIDATE_META_MARKED),
            );
        }
    }

    pub(in crate::core) fn merge_missing_letter_pinyin_corrections(
        &self,
        compact_key: &str,
        merged: &mut MergedCandidateMap,
        now: u64,
        budget: &mut CorrectionWorkBudget,
    ) {
        if compact_key.len() < 4 {
            return;
        }
        let Some(lex) = &self.phrase_lexicon else {
            return;
        };
        let variant_limit = if compact_key.chars().count() >= LONG_CORRECTION_INPUT_CHARS {
            LONG_MISSING_LETTER_VARIANT_LIMIT
        } else {
            128
        };
        for (correction_rank, corrected_key) in single_insert_ascii_variants(compact_key)
            .into_iter()
            .take(variant_limit)
            .enumerate()
        {
            if budget.should_stop(correction_rank) {
                break;
            }
            let Some(entries) = lex.lookup_pinyin(&corrected_key) else {
                continue;
            };
            for (entry_rank, entry) in entries.iter().take(3).enumerate() {
                let common_missing_letter_bonus =
                    common_missing_letter_phrase_bonus(compact_key, &corrected_key, &entry.phrase);
                let score = PHRASE_EXACT_PINYIN_BONUS - TYPO_CORRECTION_PENALTY
                    + 42.0
                    + common_missing_letter_bonus
                    + (entry.freq as f64 + 1.0).ln()
                    + signal_bonus(
                        self.user_lexicon.phrase_signal(&entry.phrase),
                        now,
                        USER_PHRASE_SCORE_SCALE,
                        USER_RECENCY_SCALE,
                    )
                    - correction_rank as f64 * 0.02
                    - entry_rank as f64 * 0.01;
                merge_candidate(
                    merged,
                    entry.phrase.clone(),
                    score,
                    Some(&format!(
                        "{}: {}->{}\ttypo=1\tcorrection=missing_letter\tcorrected_reading={}",
                        TYPO_CANDIDATE_META, compact_key, corrected_key, corrected_key
                    )),
                );
            }
        }
    }

    pub(in crate::core) fn merge_abbrev_chunk_fallback(
        &mut self,
        compact_key: &str,
        merged: &mut MergedCandidateMap,
        now: u64,
    ) {
        let Some(lex) = &self.phrase_lexicon else {
            return;
        };

        let mut start = 0usize;
        let mut chunk_idx = 0usize;
        while start < compact_key.len() && chunk_idx < ABBREV_FALLBACK_MAX_CHUNKS {
            let remaining = &compact_key[start..];
            let max_probe_len = remaining.len().min(ABBREV_FALLBACK_CHUNK);
            if max_probe_len == 0 {
                break;
            }

            let mut matched = false;
            for probe_len in (ABBREV_FALLBACK_MIN_CHUNK..=max_probe_len).rev() {
                let probe = &remaining[..probe_len];
                let bonus = ABBREV_FALLBACK_BASE_BONUS
                    - chunk_idx as f64 * ABBREV_FALLBACK_CHUNK_PENALTY
                    - (max_probe_len - probe_len) as f64 * ABBREV_FALLBACK_SHORTER_CHUNK_PENALTY;
                if bonus <= 0.0 {
                    continue;
                }

                let abbrev_entries = self.cached_prefix_flat(lex, probe);
                let pinyin_entries = self.cached_prefix_pinyin_flat(lex, probe);
                if !abbrev_entries.is_empty() || !pinyin_entries.is_empty() {
                    if !abbrev_entries.is_empty() {
                        merge_phrase_entries(
                            merged,
                            abbrev_entries.iter(),
                            bonus,
                            12,
                            &self.user_lexicon,
                            now,
                            None,
                            Some(lex),
                            None,
                        );
                    }
                    if !pinyin_entries.is_empty() {
                        merge_phrase_entries(
                            merged,
                            pinyin_entries.iter(),
                            bonus - ABBREV_FALLBACK_PINYIN_PENALTY,
                            10,
                            &self.user_lexicon,
                            now,
                            None,
                            Some(lex),
                            None,
                        );
                    }
                    start += probe_len;
                    chunk_idx += 1;
                    matched = true;
                    break;
                }
            }

            if !matched {
                for probe_len in (ABBREV_FALLBACK_MIN_CHUNK..=max_probe_len).rev() {
                    let probe = &remaining[..probe_len];
                    let bonus = ABBREV_FALLBACK_BASE_BONUS
                        - chunk_idx as f64 * ABBREV_FALLBACK_CHUNK_PENALTY
                        - (max_probe_len - probe_len) as f64
                            * ABBREV_FALLBACK_SHORTER_CHUNK_PENALTY;
                    if bonus <= 0.0 {
                        continue;
                    }
                    if probe_len < ABBREV_FALLBACK_APPROX_MIN_CHUNK {
                        continue;
                    }

                    let approx_abbrev = lex.approx_lookup(probe, 1, 8);
                    if !approx_abbrev.is_empty() {
                        merge_phrase_entries(
                            merged,
                            approx_abbrev.iter(),
                            bonus - ABBREV_FALLBACK_APPROX_PENALTY,
                            8,
                            &self.user_lexicon,
                            now,
                            Some(TYPO_CANDIDATE_META_MARKED),
                            Some(lex),
                            None,
                        );
                        start += probe_len;
                        chunk_idx += 1;
                        matched = true;
                        break;
                    }

                    let approx_pinyin = lex.approx_lookup_pinyin(probe, 1, 6);
                    if !approx_pinyin.is_empty() {
                        merge_phrase_entries(
                            merged,
                            approx_pinyin.iter(),
                            bonus - ABBREV_FALLBACK_APPROX_PENALTY - ABBREV_FALLBACK_PINYIN_PENALTY,
                            6,
                            &self.user_lexicon,
                            now,
                            Some(TYPO_CANDIDATE_META_MARKED),
                            Some(lex),
                            None,
                        );
                        start += probe_len;
                        chunk_idx += 1;
                        matched = true;
                        break;
                    }
                }
            }

            if matched {
                continue;
            }

            // 当前段完全无法联想时，至少向前推进 1 个字符，继续尝试后续片段；
            // 避免整串长输入卡死在“无候选”状态。
            start += 1;
            chunk_idx += 1;
        }
    }

    pub(in crate::core) fn cached_prefix_flat(
        &self,
        lex: &AbbrevLexicon,
        prefix_lower: &str,
    ) -> Arc<[ThuoclEntry]> {
        if prefix_lower.is_empty() {
            return Arc::from(Vec::<ThuoclEntry>::new());
        }
        if let Ok(mut cache) = self.prefix_cache_abbrev.lock() {
            if let Some(v) = cache.get(prefix_lower) {
                return v;
            }
        }
        let v = lex.prefix_flat(prefix_lower);
        if let Ok(mut cache) = self.prefix_cache_abbrev.lock() {
            cache.put(prefix_lower.to_string(), Arc::clone(&v));
        }
        v
    }

    pub(in crate::core) fn cached_prefix_pinyin_flat(
        &self,
        lex: &AbbrevLexicon,
        prefix_lower: &str,
    ) -> Arc<[ThuoclEntry]> {
        if prefix_lower.is_empty() {
            return Arc::from(Vec::<ThuoclEntry>::new());
        }
        if let Ok(mut cache) = self.prefix_cache_pinyin.lock() {
            if let Some(v) = cache.get(prefix_lower) {
                return v;
            }
        }
        let v = lex.prefix_pinyin_flat(prefix_lower);
        if let Ok(mut cache) = self.prefix_cache_pinyin.lock() {
            cache.put(prefix_lower.to_string(), Arc::clone(&v));
        }
        v
    }
}
