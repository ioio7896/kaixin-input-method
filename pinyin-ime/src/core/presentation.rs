use super::*;

const UNSUPPORTED_SHORT_COMBINATION_PENALTY: f64 = 180.0;

impl PinyinEngine {
    // Decoder output is useful for composing new text, but matching three to
    // five syllables alone is not evidence that the result is a whole word.
    // A real lexicon hit or deliberate user learning is stronger evidence.
    pub(super) fn short_combination_penalty(&self, phrase: &str, meta: &CandidateMeta) -> f64 {
        if self.is_unsupported_short_combination(phrase, meta) {
            UNSUPPORTED_SHORT_COMBINATION_PENALTY
        } else {
            0.0
        }
    }

    fn is_unsupported_short_combination(&self, phrase: &str, meta: &CandidateMeta) -> bool {
        if !(3..=5).contains(&phrase_char_count(phrase))
            || !phrase.chars().all(is_cjk_unified)
            || meta.pinned
            || meta.user_signal
            || correction_style_kind(meta).is_some()
            || !matches!(
                meta.source,
                CandidateSource::System
                    | CandidateSource::Pinyin
                    | CandidateSource::Abbrev
                    | CandidateSource::Mixed
            )
            // The source layer also survives conversion to traditional output.
            || matches!(
                meta.source_layer,
                LexiconLayer::Core | LexiconLayer::Base | LexiconLayer::Ext | LexiconLayer::Large
            )
            || self.user_lexicon.phrase_signal(phrase).freq > 0
        {
            return false;
        }
        self.phrase_lexicon
            .as_ref()
            .is_some_and(|lexicon| lexicon.phrase_frequency(phrase) == 0)
    }

    fn demote_unsupported_short_combinations(&self, ranked: &mut [RankedCandidate]) {
        let positions = ranked
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !candidate_has_user_priority(item)
                    && correction_style_kind(&item.meta).is_none()
                    && matches!(
                        item.meta.source,
                        CandidateSource::System
                            | CandidateSource::Pinyin
                            | CandidateSource::Abbrev
                            | CandidateSource::Mixed
                    )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        // When there is no known whole word to prefer, retain the normal
        // composition layout. Keep all user, shortcut and correction slots.
        if !positions.iter().any(|&index| {
            let item = &ranked[index];
            (2..=5).contains(&phrase_char_count(&item.phrase))
                && !self.is_unsupported_short_combination(&item.phrase, &item.meta)
        }) {
            return;
        }
        let mut rows = positions
            .iter()
            .map(|&index| ranked[index].clone())
            .collect::<Vec<_>>();
        rows.sort_by_key(|item| self.is_unsupported_short_combination(&item.phrase, &item.meta));
        for (index, item) in positions.into_iter().zip(rows) {
            ranked[index] = item;
        }
    }

    // This is the only presentation exit for live and cached lookup routes.
    pub(super) fn finalize_candidate_presentation(
        &self,
        raw: &str,
        key: &str,
        ranked: &mut Vec<RankedCandidate>,
    ) {
        self.finalize_stable_word_candidates(raw, key, ranked);
        if let Some((intent, _)) = self.strict_lexicon_frequency_order_context(raw, key) {
            prefer_short_abbrev_words_over_five_char_predictions(ranked, intent, key.len());
            demote_cold_non_full_lexicon_candidates(ranked, intent, self.phrase_lexicon.as_ref());
        }
        let phrases = self
            .user_lexicon
            .lookup_input(key)
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| entry.phrase.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        promote_exact_user_hotwords_front(
            ranked,
            &phrases,
            None,
            user_hotword_prefs::get_user_hotword_prefs().front_limit,
        );
        // Density, stability and cache replay may promote a decoded combination
        // after scoring. Reapply the preference once at the shared exit without
        // subtracting scores again or discarding new phrases.
        self.demote_unsupported_short_combinations(ranked);
        // An old learned entry or a prebaked lexicon can retain readings that
        // have since been excluded. Apply the same guard to live and cached
        // results, after all user/stable-word promotions.
        ranked.retain(|candidate| {
            let mut chars = candidate.phrase.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => !crate::dict::is_excluded_reading(ch, key),
                _ => true,
            }
        });
        ranked.truncate(LOOKUP_FULL_MAX_CANDIDATES);
    }
}

// Late density/stability promotions can undo the score penalty. Reorder only
// abbreviation/mixed word slots, preserving singles, shortcuts and user intent.
// Stable ordering also makes repeated cached presentation idempotent.
fn prefer_short_abbrev_words_over_five_char_predictions(
    ranked: &mut [RankedCandidate],
    intent: InputIntent,
    input_chars: usize,
) {
    if !matches!(intent, InputIntent::ShortAbbrev | InputIntent::MixedPrefix)
        || !(2..=4).contains(&input_chars)
    {
        return;
    }
    let positions = ranked
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            !candidate_has_user_priority(item)
                && matches!(
                    item.meta.source,
                    CandidateSource::Abbrev | CandidateSource::Mixed
                )
                && correction_style_kind(&item.meta).is_none()
                && (3..=5).contains(&phrase_char_count(&item.phrase))
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if !positions
        .iter()
        .any(|&index| phrase_char_count(&ranked[index].phrase) == 5)
        || !positions
            .iter()
            .any(|&index| phrase_char_count(&ranked[index].phrase) < 5)
    {
        return;
    }
    let mut rows = positions
        .iter()
        .map(|&index| ranked[index].clone())
        .collect::<Vec<_>>();
    rows.sort_by_key(|item| phrase_char_count(&item.phrase) == 5);
    for (index, row) in positions.into_iter().zip(rows) {
        ranked[index] = row;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(phrase: &str, meta: CandidateMeta) -> RankedCandidate {
        RankedCandidate {
            phrase: phrase.to_string(),
            score: 100.0,
            meta,
        }
    }

    fn short_word_engine() -> PinyinEngine {
        let mut engine = PinyinEngine::with_phrase_dir(None);
        engine.clear_user_lexicon_for_eval();
        let mut lexicon = AbbrevLexicon::default();
        for (phrase, reading) in [
            ("计算机", "ji suan ji"),
            ("心平气和", "xin ping qi he"),
            ("我已经到了", "wo yi jing dao le"),
        ] {
            assert!(lexicon.insert_parsed_with_layer(
                ThuoclEntry {
                    phrase: phrase.to_string(),
                    code: Some(reading.to_string()),
                    freq: 100,
                    pronunciation_kind: Default::default(),
                },
                LexiconLayer::Base,
            ));
        }
        engine.phrase_lexicon = Some(lexicon);
        engine
    }

    #[test]
    fn real_short_words_precede_combinations_without_losing_user_or_shortcut_slots() {
        let engine = short_word_engine();
        let meta = CandidateMeta::legacy(PINYIN_CANDIDATE_META);
        let mut rows = vec![
            candidate("鸡蒜鸡", meta.clone()),
            candidate("用户选词", CandidateMeta::user(true)),
            candidate("新瓶弃盒", meta.clone()),
            candidate("计算机", meta.clone()),
            candidate("自定义短语", CandidateMeta::legacy("source=custom")),
            candidate("窝蚁经倒乐", meta.clone()),
            candidate("心平气和", meta.clone()),
            candidate("我已经到了", meta),
        ];
        let expected = [
            "计算机",
            "用户选词",
            "心平气和",
            "我已经到了",
            "自定义短语",
            "鸡蒜鸡",
            "新瓶弃盒",
            "窝蚁经倒乐",
        ];
        for _ in 0..3 {
            engine.demote_unsupported_short_combinations(&mut rows);
            assert_eq!(
                rows.iter()
                    .map(|row| row.phrase.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(rows.iter().all(|row| row.score == 100.0));
        }
    }

    #[test]
    fn short_combination_penalty_preserves_learning_complete_expressions_and_traditional_words() {
        let mut engine = short_word_engine();
        let meta = CandidateMeta::legacy(PINYIN_CANDIDATE_META);
        for phrase in ["鸡蒜鸡", "新瓶弃盒", "窝蚁经倒乐"] {
            assert!(engine.short_combination_penalty(phrase, &meta) > 0.0);
            assert_eq!(
                engine.short_combination_penalty(phrase, &CandidateMeta::user(false)),
                0.0
            );
            assert_eq!(
                engine.short_combination_penalty(phrase, &meta.clone().with_user_signal()),
                0.0
            );
        }
        for phrase in [
            "计算机",
            "心平气和",
            "我已经到了",
            "未知",
            "今天想去看电影",
            "Rust",
        ] {
            assert_eq!(
                engine.short_combination_penalty(phrase, &meta),
                0.0,
                "{phrase}"
            );
        }
        assert_eq!(
            engine.short_combination_penalty(
                "計算機",
                &meta.clone().with_source_layer(LexiconLayer::Base)
            ),
            0.0
        );
        engine
            .user_lexicon
            .learn_with_delta("jisuanji", "鸡蒜鸡", 2)
            .unwrap();
        assert_eq!(engine.short_combination_penalty("鸡蒜鸡", &meta), 0.0);
        // Without a lexical alternative, free composition remains reachable.
        let mut rows = vec![candidate("新瓶弃盒", meta.clone()), candidate("新", meta)];
        engine.demote_unsupported_short_combinations(&mut rows);
        assert_eq!(rows[0].phrase, "新瓶弃盒");
    }

    #[test]
    fn first_full_and_cached_lookup_keep_real_three_to_five_character_words_ahead() {
        let mut engine = short_word_engine();
        for _ in 0..2 {
            for (input, word) in [
                ("jisuanji", "计算机"),
                ("xinpingqihe", "心平气和"),
                ("woyijingdaole", "我已经到了"),
            ] {
                let first = engine.lookup_first_page(input);
                assert_eq!(
                    first.first().map(String::as_str),
                    Some(word),
                    "{input}: {first:?}"
                );
                let full = engine.lookup_full_explain(input).0;
                assert_eq!(
                    full.first().map(|item| item.0.as_str()),
                    Some(word),
                    "{input}: {full:?}"
                );
            }
        }
    }

    #[test]
    fn short_abbrev_favors_three_four_char_words_and_keeps_user_slots() {
        let abbrev = CandidateMeta::legacy(ABBREV_CANDIDATE_META);
        let mut rows = vec![
            candidate("计算机系统", abbrev.clone()),
            candidate("我", CandidateMeta::default()),
            candidate("计算机网络", CandidateMeta::user(true)),
            candidate("计算机系", CandidateMeta::legacy(MIXED_HIGH_CANDIDATE_META)),
            candidate("计算机", abbrev),
        ];
        prefer_short_abbrev_words_over_five_char_predictions(
            &mut rows,
            InputIntent::ShortAbbrev,
            3,
        );
        let expected = ["计算机系", "我", "计算机网络", "计算机", "计算机系统"];
        assert_eq!(
            rows.iter()
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        prefer_short_abbrev_words_over_five_char_predictions(
            &mut rows,
            InputIntent::ShortAbbrev,
            3,
        );
        assert_eq!(
            rows.iter()
                .map(|item| item.phrase.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn full_pinyin_and_complete_five_letter_abbrev_keep_long_words() {
        for (intent, len) in [(InputIntent::FullPinyin, 4), (InputIntent::ShortAbbrev, 5)] {
            let meta = CandidateMeta::legacy(ABBREV_CANDIDATE_META);
            let mut rows = vec![
                candidate("计算机系统", meta.clone()),
                candidate("计算机系", meta),
            ];
            prefer_short_abbrev_words_over_five_char_predictions(&mut rows, intent, len);
            assert_eq!(rows[0].phrase, "计算机系统");
        }
    }

    #[test]
    fn four_letter_five_char_prediction_has_penalty_but_exact_abbrev_does_not() {
        assert!(abbrev_length_penalty(true, 4, "计算机系统") > 28.0);
        assert!(
            abbrev_length_penalty(true, 3, "计算机系统")
                > abbrev_length_penalty(true, 4, "计算机系统")
        );
        assert_eq!(abbrev_length_penalty(true, 5, "计算机系统"), 0.0);
        assert_eq!(abbrev_length_penalty(false, 4, "计算机系统"), 0.0);
        assert_eq!(abbrev_length_penalty(true, 4, "计算机系"), 0.0);
    }
}
