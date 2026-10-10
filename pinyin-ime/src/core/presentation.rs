use super::*;

impl PinyinEngine {
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
