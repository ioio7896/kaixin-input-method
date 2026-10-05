use super::*;

// A small direct-hit pool is independent of decoder beams and time budgets.
// These limits control recall, not the number of slots occupied on the page.
const DIRECT_WORDS_PER_ROUTE: usize = 16;
const PERSONAL_WORDS_PER_ROUTE: usize = 8;
const CONTEXT_WORDS_PER_ROUTE: usize = 8;
const DIRECT_WORD_MAX_CHARS: usize = 7;

struct DirectWord {
    entry: ThuoclEntry,
    meta: &'static str,
    quality: u8,
    bonus: f64,
    score: f64,
    protected: bool,
}

fn protect_word_signal(
    protected: &mut [bool],
    signals: &[f64],
    scores: &[RankedCandidate],
    limit: usize,
) {
    let mut positions = (0..signals.len())
        .filter(|&index| !protected[index] && signals[index] > 1e-6)
        .collect::<Vec<_>>();
    positions.sort_by(|&a, &b| {
        signals[b]
            .total_cmp(&signals[a])
            .then_with(|| scores[b].score.total_cmp(&scores[a].score))
            .then_with(|| scores[a].phrase.cmp(&scores[b].phrase))
    });
    for index in positions.into_iter().take(limit) {
        protected[index] = true;
    }
}

impl PinyinEngine {
    // Global phrase preference is shared by valid readings. Deliberate use of
    // this key is stronger than a generated alias, especially for jianpin.
    fn direct_word_preference(&self, key: &str, phrase: &str, meta: &str, now: u64) -> (f64, f64) {
        let (shared_scale, shared_cap, reading_cap, context_scale, context_cap) = match meta {
            ABBREV_CANDIDATE_META => (1.6, 3.0, 6.0, 0.20, 4.0),
            PINYIN_CANDIDATE_META => (1.2, 2.0, 3.0, 0.12, 2.5),
            _ => (1.4, 2.5, 5.0, 0.16, 3.0),
        };
        let stats = self.user_lexicon.phrase_signal(phrase);
        let shared = if stats.freq >= 2 {
            signal_bonus(stats, now, shared_scale, 0.5).min(shared_cap)
        } else {
            0.0
        };
        let reading = self
            .user_lexicon
            .lookup_input(key)
            .and_then(|entries| entries.iter().find(|entry| entry.phrase == phrase))
            .filter(|entry| entry.freq >= 2)
            .map_or(0.0, |entry| {
                (signal_bonus(entry.stats(), now, 2.0, 0.8) * entry.suggestion_score_scale())
                    .min(reading_cap)
            });
        let context = ((self.context_bonus(phrase, now)
            + self.word_level_transition_bonus(phrase, now))
            * context_scale)
            .clamp(0.0, context_cap);
        (shared + reading, context)
    }

    fn direct_word_feedback(&self, key: &str, meta: &str, rows: &mut [RankedCandidate]) {
        let scale = match meta {
            ABBREV_CANDIDATE_META => 1.25,
            PINYIN_CANDIDATE_META => 1.0,
            _ => 1.10,
        };
        self.adjust_selection_feedback(key, rows, scale);
        self.adjust_final_negative_selection_feedback(key, rows, scale);
    }

    fn direct_words(&self, raw: &str, key: &str, include_unprotected: bool) -> Vec<DirectWord> {
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
        let now = self.user_lexicon.current_clock();
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
                let preferences = route
                    .iter()
                    .map(|entry| self.direct_word_preference(key, &entry.phrase, meta, now))
                    .collect::<Vec<_>>();
                let mut signals = route
                    .iter()
                    .zip(&preferences)
                    .map(|(entry, &(personal, context))| RankedCandidate {
                        phrase: entry.phrase.clone(),
                        score: (lex.phrase_frequency(&entry.phrase) as f64 + 1.0).ln()
                            + personal
                            + context,
                        meta: CandidateMeta::legacy(meta),
                    })
                    .collect::<Vec<_>>();
                self.direct_word_feedback(key, meta, &mut signals);
                let personal = route
                    .iter()
                    .zip(&signals)
                    .zip(&preferences)
                    .map(|((entry, signal), &(_, context))| {
                        signal.score
                            - (lex.phrase_frequency(&entry.phrase) as f64 + 1.0).ln()
                            - context
                    })
                    .collect::<Vec<_>>();
                let contextual = preferences
                    .iter()
                    .map(|&(_, context)| context)
                    .collect::<Vec<_>>();
                let mut protected = (0..route.len())
                    .map(|index| index < DIRECT_WORDS_PER_ROUTE)
                    .collect::<Vec<_>>();
                protect_word_signal(
                    &mut protected,
                    &personal,
                    &signals,
                    PERSONAL_WORDS_PER_ROUTE,
                );
                protect_word_signal(
                    &mut protected,
                    &contextual,
                    &signals,
                    CONTEXT_WORDS_PER_ROUTE,
                );
                for ((entry, signal), protected) in route.into_iter().zip(signals).zip(protected) {
                    if !protected && !include_unprotected {
                        continue;
                    }
                    let mut entry = entry.clone();
                    entry.freq = lex.phrase_frequency(&entry.phrase);
                    words.push(DirectWord {
                        entry,
                        meta,
                        quality,
                        bonus,
                        score: signal.score,
                        protected,
                    });
                }
            }
        };
        if !lengths.is_empty() {
            add(
                lex.lookup_pinyin(key),
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
                        lex.lookup_pinyin(&full_key),
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
                lex.lookup(key),
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
                    let meta = if abbrev {
                        ABBREV_CANDIDATE_META
                    } else {
                        MIXED_HIGH_CANDIDATE_META
                    };
                    let (personal, context) =
                        self.direct_word_preference(key, &entry.phrase, meta, now);
                    let freq = lex.phrase_frequency(&entry.phrase);
                    let mut signal = [RankedCandidate {
                        phrase: entry.phrase.clone(),
                        score: (freq as f64 + 1.0).ln() + personal + context,
                        meta: CandidateMeta::legacy(meta),
                    }];
                    self.direct_word_feedback(key, meta, &mut signal);
                    words.push(DirectWord {
                        entry: ThuoclEntry {
                            phrase: entry.phrase.clone(),
                            freq,
                            code: None,
                            pronunciation_kind: Default::default(),
                        },
                        meta,
                        quality: 2,
                        bonus: USER_MIXED_INPUT_BONUS,
                        score: signal[0].score,
                        protected: true,
                    });
                }
            }
        }
        words.sort_by(|a, b| {
            b.quality
                .cmp(&a.quality)
                .then_with(|| b.score.total_cmp(&a.score))
                .then_with(|| a.entry.phrase.cmp(&b.entry.phrase))
        });
        // Merge protection across duplicate routes without adding their scores.
        let mut deduped: Vec<DirectWord> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();
        for word in words {
            if let Some(&index) = positions.get(&word.entry.phrase) {
                deduped[index].protected |= word.protected;
            } else {
                positions.insert(word.entry.phrase.clone(), deduped.len());
                deduped.push(word);
            }
        }
        deduped
    }

    pub(super) fn merge_direct_word_recall(
        &self,
        raw: &str,
        key: &str,
        now: u64,
        merged: &mut MergedCandidateMap,
    ) -> Vec<String> {
        let words = self.direct_words(raw, key, false);
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
        let words = self.direct_words(raw, key, true);
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
            strongest_quality = strongest_quality.max(word.quality);
            let value = (word.quality, word.score);
            direct
                .entry(phrase.clone())
                .and_modify(|old: &mut (u8, f64)| {
                    if value.0 > old.0 || (value.0 == old.0 && value.1 > old.1) {
                        *old = value;
                    }
                })
                .or_insert(value);
            if word.protected && !ranked.iter().any(|item| item.phrase == phrase) {
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
        if direct.is_empty() {
            if let Some((intent, len)) = context {
                demote_cold_non_full_lexicon_candidates(
                    ranked,
                    intent,
                    self.phrase_lexicon.as_ref(),
                );
                if !self.has_selection_feedback_for_reading(key)
                    && ranked.iter().all(|item| {
                        let (personal, contextual) = self.direct_word_preference(
                            key,
                            &item.phrase,
                            PINYIN_CANDIDATE_META,
                            now,
                        );
                        personal == 0.0 && contextual == 0.0
                    })
                {
                    enforce_strict_system_lexicon_frequency_order(
                        ranked,
                        key,
                        intent,
                        len,
                        self.phrase_lexicon.as_ref(),
                        traditional,
                    );
                }
            }
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
                        .then_with(|| {
                            if len == 3 {
                                std::cmp::Ordering::Equal
                            } else {
                                a.phrase.cmp(&b.phrase)
                            }
                        }),
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
        // Scores above are rebuilt from evidence on every presentation exit;
        // a final frequency-only pass must not erase personalization/context.
    }
}
