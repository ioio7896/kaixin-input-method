use super::*;

// A small direct-hit pool is independent of decoder beams and time budgets.
// These limits control recall, not the number of slots occupied on the page.
const DIRECT_WORDS_PER_ROUTE: usize = 16;
const DIRECT_WORD_MAX_CHARS: usize = 7;

struct DirectWord {
    entry: ThuoclEntry,
    meta: &'static str,
    quality: u8,
    bonus: f64,
}

impl PinyinEngine {
    fn direct_words(&self, raw: &str, key: &str) -> Vec<DirectWord> {
        if key.len() < 2 || self.is_effective_direct_input_shortcut(raw) {
            return Vec::new();
        }
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return Vec::new();
        };
        let variants = self.parse_exact_variants_cached(raw).unwrap_or_default();
        // Respect explicit separators and every exact split, including
        // ambiguous readings. A primary split must not erase an alternative.
        let lengths = variants
            .iter()
            .filter(|variant| {
                matches!(
                    variant.source,
                    ParseVariantSource::Exact | ParseVariantSource::AlternateSplit
                ) && variant.tail.is_none()
                    && variant.syllables.join("") == key
            })
            .map(|variant| variant.syllables.len())
            .collect::<HashSet<_>>();
        if lengths.contains(&1) {
            return Vec::new();
        }
        let mut words = Vec::new();
        let mut add = |entries: Option<Arc<[ThuoclEntry]>>,
                       meta,
                       quality,
                       bonus,
                       allowed: &dyn Fn(usize) -> bool| {
            if let Some(entries) = entries {
                let mut route = entries
                    .iter()
                    .filter(|entry| {
                        let len = phrase_char_count(&entry.phrase);
                        (2..=DIRECT_WORD_MAX_CHARS).contains(&len)
                            && allowed(len)
                            && !self.user_lexicon.phrase_is_blocked(&entry.phrase)
                    })
                    .collect::<Vec<_>>();
                route.sort_by(|a, b| {
                    lex.phrase_frequency(&b.phrase)
                        .cmp(&lex.phrase_frequency(&a.phrase))
                        .then_with(|| a.phrase.cmp(&b.phrase))
                });
                for entry in route.into_iter().take(DIRECT_WORDS_PER_ROUTE) {
                    let mut entry = entry.clone();
                    entry.freq = lex.phrase_frequency(&entry.phrase);
                    words.push(DirectWord {
                        entry,
                        meta,
                        quality,
                        bonus,
                    });
                }
            }
        };
        if !lengths.is_empty() {
            add(
                lex.lookup_pinyin_hot(key),
                PINYIN_CANDIDATE_META,
                3,
                PHRASE_DIRECT_PINYIN_LOOKUP_BONUS,
                &|len| lengths.contains(&len),
            );
        }
        if self.mode_flags & MODE_DOUBLE_PINYIN != 0 {
            for variant in variants
                .iter()
                .filter(|variant| {
                    variant.source == ParseVariantSource::Exact
                        && variant.tail.is_none()
                        && variant.syllables.len() >= 2
                })
                .take(8)
            {
                let full_key = variant.syllables.join("");
                if full_key != key {
                    add(
                        lex.lookup_pinyin_hot(&full_key),
                        PINYIN_CANDIDATE_META,
                        3,
                        PHRASE_DIRECT_PINYIN_LOOKUP_BONUS,
                        &|len| len == variant.syllables.len(),
                    );
                }
            }
        }
        let compact_input = raw == key && raw.bytes().all(|ch| ch.is_ascii_alphabetic());
        if compact_input && self.jianpin_enabled() && is_pure_short_abbrev_initial_input(raw, key) {
            add(
                if key.len() == 3 {
                    lex.lookup(key)
                } else {
                    lex.lookup_hot(key)
                },
                ABBREV_CANDIDATE_META,
                2,
                self.exact_abbrev_bonus(),
                &|len| len == key.len(),
            );
        }
        if compact_input && self.mixed_pinyin_enabled() && key.len() >= 3 {
            add(
                lex.lookup_mixed_hot(key),
                MIXED_HIGH_CANDIDATE_META,
                2,
                self.mixed_pinyin_bonus() + MIXED_PINYIN_HIGH_CONFIDENCE_BONUS,
                &|_| true,
            );
        }
        if compact_input {
            if let Some(entries) = self.user_lexicon.lookup_mixed_input(key) {
                for entry in entries.iter().take(TSF_PAGE_SIZE) {
                    let len = phrase_char_count(&entry.phrase);
                    let abbrev = len == key.len();
                    if !(2..=DIRECT_WORD_MAX_CHARS).contains(&len)
                        || self.user_lexicon.phrase_is_blocked(&entry.phrase)
                        || !(if abbrev {
                            self.jianpin_enabled()
                        } else {
                            self.mixed_pinyin_enabled()
                        })
                    {
                        continue;
                    }
                    words.push(DirectWord {
                        entry: ThuoclEntry {
                            phrase: entry.phrase.clone(),
                            freq: lex.phrase_frequency(&entry.phrase),
                            code: None,
                            pronunciation_kind: Default::default(),
                        },
                        meta: if abbrev {
                            ABBREV_CANDIDATE_META
                        } else {
                            MIXED_HIGH_CANDIDATE_META
                        },
                        quality: 2,
                        bonus: USER_MIXED_INPUT_BONUS,
                    });
                }
            }
        }
        words.sort_by(|a, b| {
            b.quality
                .cmp(&a.quality)
                .then_with(|| b.entry.freq.cmp(&a.entry.freq))
                .then_with(|| a.entry.phrase.cmp(&b.entry.phrase))
        });
        let mut seen = HashSet::new();
        words.retain(|word| seen.insert(word.entry.phrase.clone()));
        words
    }

    pub(super) fn merge_direct_word_recall(
        &self,
        raw: &str,
        key: &str,
        now: u64,
        merged: &mut MergedCandidateMap,
    ) -> Vec<String> {
        let words = self.direct_words(raw, key);
        for word in &words {
            merge_phrase_entries(
                merged,
                std::iter::once(&word.entry),
                word.bonus,
                1,
                &self.user_lexicon,
                now,
                Some(word.meta),
                self.phrase_lexicon.as_ref(),
                (word.meta == ABBREV_CANDIDATE_META).then_some(key.len()),
            );
        }
        let mut protected = Vec::new();
        if !self.is_effective_direct_input_shortcut(raw) {
            if let Some(entries) = self.user_lexicon.lookup_input(key) {
                protected.extend(
                    entries
                        .iter()
                        .filter(|entry| !self.user_lexicon.phrase_is_blocked(&entry.phrase))
                        .take(TSF_PAGE_SIZE)
                        .map(|entry| entry.phrase.clone()),
                );
            }
        }
        // If a partial pool exceeds its cap, explicit user choices get the
        // first protected slots, before system hits and generated aliases.
        protected.extend(words.into_iter().map(|word| word.entry.phrase));
        if !self.is_effective_direct_input_shortcut(raw) && self.mixed_pinyin_enabled() {
            if let Some(entries) = self.user_lexicon.lookup_mixed_input(key) {
                protected.extend(
                    entries
                        .iter()
                        .take(TSF_PAGE_SIZE)
                        .map(|entry| entry.phrase.clone()),
                );
            }
        }
        dedup_preserved_phrase_order(&mut protected);
        protected
    }

    // The same boundary runs after partial, complete and cache-specific
    // shaping. It can restore direct hits lost by intermediate truncation.
    // It does not add deltas to existing scores, so cache replay is idempotent.
    pub(super) fn finalize_stable_word_candidates(
        &self,
        raw: &str,
        key: &str,
        ranked: &mut Vec<RankedCandidate>,
    ) {
        let words = self.direct_words(raw, key);
        // The direct route already establishes intent. Avoid running mixed
        // beam expansion again on the common hot-word presentation boundary.
        let context = words
            .first()
            .map(|word| {
                if word.quality == 3 {
                    (
                        InputIntent::FullPinyin,
                        phrase_char_count(&word.entry.phrase),
                    )
                } else if word.meta == ABBREV_CANDIDATE_META {
                    (InputIntent::ShortAbbrev, key.len())
                } else {
                    (InputIntent::MixedPrefix, 0)
                }
            })
            .or_else(|| self.strict_lexicon_frequency_order_context(raw, key));
        if !self.has_selection_feedback_for_reading(key) {
            if let Some((intent, len)) = context {
                enforce_strict_system_lexicon_frequency_order(
                    ranked,
                    key,
                    intent,
                    len,
                    self.phrase_lexicon.as_ref(),
                    self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0,
                );
            }
        }
        let now = self.user_lexicon.current_clock();
        let traditional = self.mode_flags & MODE_TRADITIONAL_OUTPUT != 0;
        let display = |phrase: &str| {
            if traditional {
                crate::traditional::to_traditional(phrase)
            } else {
                phrase.to_string()
            }
        };
        let mut direct = HashMap::new();
        let mut strongest_quality = 0;
        for word in words {
            let phrase = display(&word.entry.phrase);
            if self.user_lexicon.phrase_is_blocked(&phrase) {
                continue;
            }
            // Phrase preference travels between valid routes; per-reading
            // feedback remains stronger. Neither is multiplied by path count.
            let stats = self.user_lexicon.phrase_signal(&word.entry.phrase);
            let shared_bonus = if stats.freq >= 2 {
                signal_bonus(stats, now, 1.2, 0.5).min(2.0)
            } else {
                0.0
            };
            let mut signal = vec![RankedCandidate {
                phrase: word.entry.phrase.clone(),
                score: (word.entry.freq as f64 + 1.0).ln() + shared_bonus,
                meta: CandidateMeta::legacy(word.meta),
            }];
            self.apply_selection_feedback(key, &mut signal);
            self.apply_final_negative_selection_feedback(key, &mut signal);
            strongest_quality = strongest_quality.max(word.quality);
            let value = (word.quality, signal[0].score);
            direct
                .entry(phrase.clone())
                .and_modify(|old: &mut (u8, f64)| {
                    if value.0 > old.0 || (value.0 == old.0 && value.1 > old.1) {
                        *old = value;
                    }
                })
                .or_insert(value);
            if !ranked.iter().any(|item| item.phrase == phrase) {
                let mut merged = MergedCandidateMap::default();
                merge_phrase_entries(
                    &mut merged,
                    std::iter::once(&word.entry),
                    word.bonus,
                    1,
                    &self.user_lexicon,
                    now,
                    Some(word.meta),
                    self.phrase_lexicon.as_ref(),
                    (word.meta == ABBREV_CANDIDATE_META).then_some(key.len()),
                );
                if let Some((_, candidate)) = merged.drain_candidates().pop() {
                    let mut restored = vec![RankedCandidate {
                        score: candidate.score
                            + self.blended_rerank_with_prefs(
                                &word.entry.phrase,
                                now,
                                rerank_prefs::get_rerank_prefs(),
                            )
                            + self.stability_bonus_for_candidate(key, &word.entry.phrase),
                        phrase: word.entry.phrase,
                        meta: candidate.meta,
                    }];
                    self.apply_selection_feedback(key, &mut restored);
                    self.apply_final_negative_selection_feedback(key, &mut restored);
                    restored[0].phrase = phrase;
                    ranked.extend(restored);
                }
            }
        }
        let mut seen = HashSet::new();
        ranked.retain(|item| {
            !item.meta.blocked
                && !self.user_lexicon.phrase_is_blocked(&item.phrase)
                && seen.insert(item.phrase.clone())
        });
        let finish_three_char_order = |rows: &mut Vec<RankedCandidate>| {
            if !self.has_selection_feedback_for_reading(key) {
                if let Some((intent, _)) = context {
                    enforce_strict_system_lexicon_frequency_order(
                        rows,
                        key,
                        intent,
                        3,
                        self.phrase_lexicon.as_ref(),
                        traditional,
                    );
                }
            }
        };
        if direct.is_empty() {
            if let Some((intent, _)) = context {
                demote_cold_non_full_lexicon_candidates(
                    ranked,
                    intent,
                    self.phrase_lexicon.as_ref(),
                );
            }
            finish_three_char_order(ranked);
            return;
        }
        let page_size =
            candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        let strongest_lengths = ranked
            .iter()
            .filter(|item| {
                direct
                    .get(&item.phrase)
                    .is_some_and(|value| value.0 == strongest_quality)
            })
            .map(|item| phrase_char_count(&item.phrase))
            .collect::<HashSet<_>>();
        // Replay the same density policy regardless of which pipeline produced
        // this batch. Mixed-length ambiguity keeps its existing layout slots.
        if strongest_lengths.len() == 1 && !ranked.iter().any(|item| item.meta.pinned) {
            let len = *strongest_lengths.iter().next().unwrap();
            if matches!(len, 2 | 3) {
                apply_short_intent_final_candidate_density(
                    ranked,
                    Some(len),
                    self.phrase_lexicon.as_ref(),
                );
                if len == 2 {
                    if strongest_quality == 3 {
                        arrange_two_char_intent_page_density(ranked, page_size);
                    } else {
                        arrange_two_char_intent_minimum_page_density(ranked, page_size);
                    }
                }
                enforce_short_intent_visible_page(ranked, len, page_size);
            }
        }
        let ordinary = |item: &RankedCandidate| {
            !candidate_has_user_priority(item)
                && matches!(
                    item.meta.source,
                    CandidateSource::System
                        | CandidateSource::Pinyin
                        | CandidateSource::Abbrev
                        | CandidateSource::Mixed
                        | CandidateSource::Correction
                )
        };
        // Preserve length/layout slots and explicit user/shortcut positions.
        // Exact whole words precede weaker matches within the same length.
        for len in 2..=DIRECT_WORD_MAX_CHARS {
            let positions = ranked
                .iter()
                .enumerate()
                .filter(|(_, item)| ordinary(item) && phrase_char_count(&item.phrase) == len)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let mut rows = positions
                .iter()
                .map(|&index| ranked[index].clone())
                .collect::<Vec<_>>();
            rows.sort_by(
                |a, b| match (direct.get(&a.phrase), direct.get(&b.phrase)) {
                    (Some(a_score), Some(b_score)) => b_score
                        .0
                        .cmp(&a_score.0)
                        .then_with(|| b_score.1.total_cmp(&a_score.1))
                        .then_with(|| a.phrase.cmp(&b.phrase)),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                },
            );
            for (index, row) in positions.into_iter().zip(rows) {
                ranked[index] = row;
            }
        }
        if let Some((intent, _)) = context {
            demote_cold_non_full_lexicon_candidates(ranked, intent, self.phrase_lexicon.as_ref());
        }
        // A page made entirely of fallback singles must not hide every exact
        // hot word. Reserve one ordinary slot, leaving explicit priority intact.
        if !ranked.iter().take(page_size).any(|item| {
            direct
                .get(&item.phrase)
                .is_some_and(|value| value.0 == strongest_quality)
        }) {
            let best = ranked
                .iter()
                .enumerate()
                .filter(|(_, item)| ordinary(item))
                .filter_map(|(index, item)| direct.get(&item.phrase).map(|value| (index, value)))
                .max_by(|(ai, a), (bi, b)| {
                    a.0.cmp(&b.0)
                        .then_with(|| a.1.total_cmp(&b.1))
                        .then_with(|| bi.cmp(ai))
                })
                .map(|(index, _)| index);
            let slot = ranked.iter().take(page_size).position(ordinary);
            if let (Some(best), Some(slot)) = (best, slot) {
                let item = ranked.remove(best);
                ranked.insert(slot, item);
            }
        }
        // This must run after restoration, density, direct-hit sorting and
        // first-page rescue, before truncation can remove a high-frequency row.
        // Only ordinary exact three-character slots are reordered; explicit
        // user choices, corrections, shortcuts and other lengths keep theirs.
        finish_three_char_order(ranked);

    }
}
