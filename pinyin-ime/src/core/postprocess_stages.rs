use super::*;

impl PinyinEngine {
    pub(super) fn stage_special_layout(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        ctx: &CandidatePostprocessContext<'_>,
    ) {
        if ctx.exact_single_syllable_input {
            keep_only_single_char_candidates(ranked);
            // Pinned single-character user phrases should still be forced to
            // the top even under the single-syllable fast path, otherwise a
            // high-frequency unpinned candidate (e.g. "了" for "le") can
            // outrank a manually pinned candidate (e.g. "乐" for "le").
            let pinned_single_char_phrases: Vec<String> = ctx
                .preserved_pinned_user_phrases
                .iter()
                .filter(|phrase| phrase_char_count(phrase) == 1)
                .cloned()
                .collect();
            if !pinned_single_char_phrases.is_empty() {
                promote_candidate_priority_groups(ranked, &[pinned_single_char_phrases.as_slice()]);
            }
            return;
        }

        let has_pinned = !ctx.preserved_pinned_user_phrases.is_empty();

        if ctx.applied_multi_syllable_phrase_layout {
            limit_typo_corrections_first_page(
                ranked,
                ctx.correction_prefs,
                ctx.suppress_correction,
            );
            drop_single_char_corrections_for_two_char_intent(
                ranked,
                (ctx.final_short_phrase_intent == Some(2)
                    || ctx.exact_guard_syllables == Some(2)
                    || ctx.overlong_pinned_exact_guard_syllables == Some(2))
                .then_some(2),
            );

            let exact_len_guard = ctx
                .exact_guard_syllables
                .or(ctx.overlong_pinned_exact_guard_syllables);
            let filtered_exact_user_phrases;
            let exact_user_priority_group = if let Some(max_len) = exact_len_guard {
                filtered_exact_user_phrases = ctx
                    .preserved_exact_user_phrases
                    .iter()
                    .filter(|phrase| phrase_char_count(phrase) <= max_len)
                    .cloned()
                    .collect::<Vec<_>>();
                filtered_exact_user_phrases.as_slice()
            } else {
                ctx.preserved_exact_user_phrases
            };
            let filtered_pinned_user_phrases;
            let pinned_priority_group = if let Some(max_len) = ctx.exact_guard_syllables {
                filtered_pinned_user_phrases = ctx
                    .preserved_pinned_user_phrases
                    .iter()
                    .filter(|phrase| phrase_char_count(phrase) <= max_len)
                    .cloned()
                    .collect::<Vec<_>>();
                filtered_pinned_user_phrases.as_slice()
            } else {
                ctx.preserved_pinned_user_phrases
            };
            let exact_full_pinyin_priority_phrases;
            let exact_full_pinyin_priority_group = if let Some(syllable_count) = exact_len_guard {
                let mut phrases = self.exact_full_pinyin_phrases(ctx.compact_key, syllable_count);
                phrases.truncate(1);
                exact_full_pinyin_priority_phrases = phrases;
                exact_full_pinyin_priority_phrases.as_slice()
            } else {
                &[]
            };

            promote_candidate_priority_groups(
                ranked,
                &[
                    ctx.preserved_direct_phrases,
                    ctx.final_preferred_phrases,
                    exact_user_priority_group,
                    ctx.preserved_separator_phrases,
                    exact_full_pinyin_priority_group,
                    pinned_priority_group,
                    ctx.preserved_short_abbrev_user_phrases,
                    ctx.preserved_high_priority_two_char_phrases,
                    ctx.preserved_chat_priority_two_char_phrases,
                    ctx.preserved_daily_short_two_char_phrases,
                    ctx.preserved_daily_short_three_char_phrases,
                ],
            );
            promote_exact_user_hotwords_front(
                ranked,
                exact_user_priority_group,
                exact_len_guard,
                ctx.user_hotword_front_limit,
            );
            if !ctx.skip_final_short_density
                && !has_pinned
                && ctx.input_intent == InputIntent::FullPinyin
                && ctx.preserved_daily_short_three_char_phrases.is_empty()
                && (ctx.exact_guard_syllables == Some(2)
                    || ctx.final_short_phrase_intent == Some(2))
            {
                // The multi-syllable layout gathers a large first-syllable
                // character pool.  Always redistribute exact two-character
                // words afterwards; gating this on an overlong prediction
                // left page two and later filled almost entirely with singles.
                apply_two_char_intent_page_density(ranked);
            }
        }
    }

    pub(super) fn stage_recall_density(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        ctx: &CandidatePostprocessContext<'_>,
    ) {
        let has_direct = !ctx.preserved_direct_phrases.is_empty();
        let has_exact_user = !ctx.preserved_exact_user_phrases.is_empty();
        let has_pinned = !ctx.preserved_pinned_user_phrases.is_empty();
        let has_user_or_direct_lock = has_direct || has_exact_user || has_pinned;
        let empty_short_priority_group: &[String] = &[];
        let short_priority_group = if ctx.input_intent == InputIntent::MixedPrefix
            && ctx.compact_key.chars().count() >= 4
        {
            empty_short_priority_group
        } else {
            ctx.preserved_short_phrases
        };

        // Keep protected short candidates available before density shaping can
        // trim noisy overlong expansions.
        promote_candidate_priority_groups(
            ranked,
            &[
                ctx.preserved_exact_lexicon_phrases,
                ctx.preserved_high_priority_two_char_phrases,
                ctx.preserved_chat_priority_two_char_phrases,
                ctx.preserved_daily_short_two_char_phrases,
                ctx.preserved_daily_short_three_char_phrases,
                short_priority_group,
            ],
        );

        if !has_direct {
            apply_small_input_candidate_policy(ranked, ctx.short_phrase_intent);
        }

        limit_typo_corrections_first_page(ranked, ctx.correction_prefs, ctx.suppress_correction);

        if !has_user_or_direct_lock {
            limit_short_input_first_page_density(
                ranked,
                ctx.short_phrase_intent,
                self.phrase_lexicon.as_ref(),
            );
        }

        if !has_user_or_direct_lock {
            ensure_strong_typo_correction_in_top_three(
                ranked,
                ctx.correction_prefs,
                ctx.suppress_correction,
            );
        }
        drop_single_char_corrections_for_two_char_intent(
            ranked,
            (ctx.final_short_phrase_intent == Some(2)
                || ctx.exact_guard_syllables == Some(2)
                || ctx.overlong_pinned_exact_guard_syllables == Some(2))
            .then_some(2),
        );

        if !ctx.skip_final_short_density {
            promote_preserved_short_intent_expansions(
                ranked,
                ctx.final_short_phrase_intent,
                ctx.preserved_short_phrases,
            );
            promote_short_intent_abbrev_prefix_expansions(
                ranked,
                ctx.compact_key,
                ctx.final_short_phrase_intent,
                self.phrase_lexicon.as_ref(),
            );
            apply_short_intent_final_candidate_density(
                ranked,
                ctx.final_short_phrase_intent,
                self.phrase_lexicon.as_ref(),
            );
        }
    }

    pub(super) fn stage_exact_guards(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        ctx: &CandidatePostprocessContext<'_>,
    ) {
        let has_direct = !ctx.preserved_direct_phrases.is_empty();
        let has_exact_user = !ctx.preserved_exact_user_phrases.is_empty();
        let has_pinned = !ctx.preserved_pinned_user_phrases.is_empty();
        let has_user_or_direct_lock = has_direct || has_exact_user || has_pinned;
        let preserve_ascii = should_preserve_ascii_input(ctx.raw);
        if !ctx.direct_input_shortcut {
            promote_ranked_high_priority_two_char_candidates(
                ranked,
                ctx.raw,
                ctx.compact_key,
                self.syllables.as_ref(),
                self.phrase_lexicon.as_ref(),
            );
        }

        if let Some(syllable_count) = ctx.exact_guard_syllables {
            self.promote_exact_full_pinyin_candidates(ranked, ctx.compact_key, syllable_count);
        }
        if let Some(syllable_count) = ctx.overlong_pinned_exact_guard_syllables {
            self.promote_exact_full_pinyin_over_overlong_front(
                ranked,
                ctx.compact_key,
                syllable_count,
            );
        }

        if !ctx.skip_final_short_density && !has_pinned && ctx.final_short_phrase_intent == Some(2)
        {
            if ctx.input_intent == InputIntent::FullPinyin {
                apply_two_char_intent_page_density(ranked);
            } else {
                apply_two_char_intent_minimum_page_density(ranked);
            }
        }
        promote_high_score_exact_short_front(ranked, ctx.final_short_phrase_intent);

        let exact_short_guard_intent = ctx.exact_guard_syllables.or_else(|| {
            (ctx.final_short_phrase_intent == Some(2)
                && !ctx.direct_input_shortcut
                && !preserve_ascii
                && !has_pinned)
                .then_some(2)
        });
        if let Some(intent) = exact_short_guard_intent {
            ensure_exact_short_intent_before_expansion(ranked, Some(intent));
        }

        if !has_user_or_direct_lock && !ctx.direct_input_shortcut && !preserve_ascii {
            ensure_strong_two_char_correction_before_expansion(
                ranked,
                ctx.correction_prefs,
                ctx.suppress_correction,
            );
        }
    }

    pub(super) fn stage_explicit_priority(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        ctx: &CandidatePostprocessContext<'_>,
    ) {
        let exact_len_guard = ctx
            .exact_guard_syllables
            .or(ctx.overlong_pinned_exact_guard_syllables);
        let filtered_exact_user_phrases;
        let exact_user_priority_group = if let Some(max_len) = exact_len_guard {
            filtered_exact_user_phrases = ctx
                .preserved_exact_user_phrases
                .iter()
                .filter(|phrase| phrase_char_count(phrase) <= max_len)
                .cloned()
                .collect::<Vec<_>>();
            filtered_exact_user_phrases.as_slice()
        } else {
            ctx.preserved_exact_user_phrases
        };
        let filtered_pinned_user_phrases;
        let pinned_priority_group = if let Some(max_len) = ctx.exact_guard_syllables {
            filtered_pinned_user_phrases = ctx
                .preserved_pinned_user_phrases
                .iter()
                .filter(|phrase| phrase_char_count(phrase) <= max_len)
                .cloned()
                .collect::<Vec<_>>();
            filtered_pinned_user_phrases.as_slice()
        } else {
            ctx.preserved_pinned_user_phrases
        };
        let exact_full_pinyin_priority_phrases;
        let exact_full_pinyin_priority_group = if let Some(syllable_count) = exact_len_guard {
            exact_full_pinyin_priority_phrases =
                self.exact_full_pinyin_phrases(ctx.compact_key, syllable_count);
            exact_full_pinyin_priority_phrases.as_slice()
        } else {
            &[]
        };
        // An exact user mixed-input key is explicit user intent even when the
        // parser classifies its short form as an abbreviation rather than a
        // mixed-prefix input.
        let mixed_user_priority_group = ctx.preserved_mixed_user_phrases;

        // Final explicit priority table. Broad system short-abbrev candidates
        // are not locked here, so density shaping can still limit noisy expansions.
        if ctx.overlong_pinned_exact_guard_syllables.is_some() {
            promote_candidate_priority_groups(
                ranked,
                &[
                    ctx.preserved_direct_phrases,
                    ctx.final_preferred_phrases,
                    mixed_user_priority_group,
                    exact_user_priority_group,
                    ctx.preserved_separator_phrases,
                    exact_full_pinyin_priority_group,
                    ctx.preserved_exact_lexicon_phrases,
                    pinned_priority_group,
                    ctx.preserved_short_abbrev_user_phrases,
                    ctx.preserved_high_priority_two_char_phrases,
                    ctx.preserved_chat_priority_two_char_phrases,
                    ctx.preserved_daily_short_two_char_phrases,
                    ctx.preserved_daily_short_three_char_phrases,
                ],
            );
        } else if ctx.exact_guard_syllables.is_some() {
            promote_candidate_priority_groups(
                ranked,
                &[
                    pinned_priority_group,
                    ctx.preserved_direct_phrases,
                    ctx.final_preferred_phrases,
                    mixed_user_priority_group,
                    exact_user_priority_group,
                    ctx.preserved_short_abbrev_user_phrases,
                    ctx.preserved_separator_phrases,
                    exact_full_pinyin_priority_group,
                    ctx.preserved_exact_lexicon_phrases,
                    ctx.preserved_high_priority_two_char_phrases,
                    ctx.preserved_chat_priority_two_char_phrases,
                    ctx.preserved_daily_short_two_char_phrases,
                    ctx.preserved_daily_short_three_char_phrases,
                ],
            );
        } else {
            promote_candidate_priority_groups(
                ranked,
                &[
                    pinned_priority_group,
                    ctx.preserved_direct_phrases,
                    ctx.final_preferred_phrases,
                    mixed_user_priority_group,
                    ctx.preserved_separator_phrases,
                    ctx.preserved_exact_lexicon_phrases,
                    ctx.preserved_high_priority_two_char_phrases,
                    ctx.preserved_chat_priority_two_char_phrases,
                    ctx.preserved_daily_short_two_char_phrases,
                    ctx.preserved_daily_short_three_char_phrases,
                    exact_user_priority_group,
                    ctx.preserved_short_abbrev_user_phrases,
                ],
            );
        }
        promote_preserved_exact_short_abbrev_top1(
            ranked,
            ctx.raw,
            ctx.compact_key,
            &[
                ctx.final_preferred_phrases,
                ctx.preserved_high_priority_two_char_phrases,
                ctx.preserved_chat_priority_two_char_phrases,
                ctx.preserved_daily_short_two_char_phrases,
                ctx.preserved_daily_short_three_char_phrases,
            ],
            self.phrase_lexicon.as_ref(),
        );
        promote_learned_short_abbrev_hotwords_front(
            ranked,
            ctx.preserved_short_abbrev_user_phrases,
            Some(ctx.compact_key.chars().count()),
            ctx.user_hotword_front_limit,
        );
        promote_exact_user_hotwords_front(
            ranked,
            exact_user_priority_group,
            exact_len_guard,
            ctx.user_hotword_front_limit,
        );
        if ctx.input_intent == InputIntent::MixedPrefix {
            promote_mixed_prefix_core_phrase_front(
                ranked,
                ctx.final_short_phrase_intent.or(ctx.short_phrase_intent),
            );
        }
    }
}
