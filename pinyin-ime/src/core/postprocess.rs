use super::*;

impl PinyinEngine {
    pub fn strip_leading_syllable_from_reading(&self, input: &str) -> Option<String> {
        let normalized = normalize_pinyin_line(input);
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            return None;
        }
        let first = trimmed.chars().next()?;
        let first_ascii = first.to_ascii_lowercase();
        if !first.is_ascii_alphabetic() || first_ascii == 'u' || first_ascii == 'v' {
            return None;
        }
        if let Ok((syls, tail)) =
            split_syllables_with_tail_mode(trimmed, self.syllables.as_ref(), self.parse_options())
        {
            if tail.is_none() && syls.len() >= 2 {
                let bounds =
                    crate::segment::syllable_boundary_char_indices(input, self.syllables.as_ref());
                if bounds.len() < 2 {
                    return None;
                }
                let end = bounds[1];
                let chars: Vec<char> = trimmed.chars().collect();
                if end > chars.len() {
                    return None;
                }
                let rest: String = chars[end..].iter().collect();
                let rest = rest.trim_start();
                if rest.is_empty() {
                    return None;
                }
                return Some(rest.to_string());
            }
        }

        let compact_key = compact_lookup_key(trimmed);
        let parse_result =
            parse_input_variants(trimmed, self.syllables.as_ref(), self.parse_options());
        let parse_intent_char_count = parse_result
            .as_ref()
            .ok()
            .and_then(|variants| infer_uniform_variant_char_count(variants));
        let mixed_limit = self.mixed_pinyin_key_limit(&compact_key);
        let mixed_keys = if mixed_limit == 0 {
            Vec::new()
        } else {
            expand_mixed_pinyin_exact_keys(&compact_key, self.syllables.as_ref(), mixed_limit)
        };
        let mixed_intent_char_count = mixed_keys.first().and_then(|(_, segments)| {
            if (2..=SHORT_HOTWORD_MAX_CHARS).contains(segments) {
                Some(*segments)
            } else {
                None
            }
        });
        if !is_pure_short_abbrev_initial_input(trimmed, &compact_key)
            || parse_intent_char_count.or(mixed_intent_char_count)
                != Some(compact_key.chars().count())
        {
            return None;
        }
        let rest: String = compact_key.chars().skip(1).collect();
        if rest.is_empty() {
            return None;
        }
        Some(rest)
    }

    pub(super) fn first_syllable_single_ranked(
        &self,
        first_syllable: &str,
        full_compact_key: &str,
        now: u64,
    ) -> Vec<RankedCandidate> {
        let mut merged = MergedCandidateMap::with_capacity(64);
        let one = vec![first_syllable.to_string()];
        let out = decode(
            self.dict.as_ref(),
            self.lm.as_ref(),
            &one,
            self.effective_decode_beam(),
            decode_candidate_limit(one.len()),
        );
        merge_scored_candidates(
            &mut merged,
            out.into_iter(),
            0.0,
            &self.user_lexicon,
            now,
            None,
        );
        let wg = self.decode_word_graph(&one, now);
        merge_scored_candidates(
            &mut merged,
            wg.into_iter(),
            0.0,
            &self.user_lexicon,
            now,
            None,
        );
        let ck = compact_lookup_key(first_syllable);
        if !ck.is_empty() {
            if let Some(entries) = self.user_lexicon.lookup_input(&ck) {
                merge_user_entries(
                    &mut merged,
                    entries.iter().filter(|e| phrase_char_count(&e.phrase) == 1),
                    USER_EXACT_INPUT_BONUS,
                    24,
                    now,
                    None,
                    None,
                    Some(1),
                );
            }
        }
        let rerank_prefs = rerank_prefs::get_rerank_prefs();
        let mut ranked: Vec<RankedCandidate> = merged
            .drain_candidates()
            .into_iter()
            .map(|(phrase, merged_candidate)| {
                let compose_boost = if self.mode_flags & MODE_USER_PHRASE_COMPOSE_ACTIVE != 0 {
                    USER_PHRASE_COMPOSE_SINGLE_BOOST
                } else {
                    0.0
                };
                let r = merged_candidate.score
                    + self.blended_rerank_with_prefs(&phrase, now, rerank_prefs)
                    + self.stability_bonus_for_candidate(full_compact_key, &phrase)
                    + compose_boost;
                RankedCandidate {
                    phrase,
                    score: r,
                    meta: merged_candidate.meta,
                }
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
        ranked
    }

    /// 以传入音节范围收集单字池，保证用户可逐字拼出目标词组。

    pub(super) fn syllable_pool_single_ranked(
        &self,
        syllables: &[String],
        full_compact_key: &str,
        now: u64,
    ) -> Vec<RankedCandidate> {
        let mut seen = HashSet::with_capacity(256);
        let mut pool = Vec::with_capacity(512);
        let mut merged = MergedCandidateMap::with_capacity(256);
        for syllable in syllables {
            if let Some(raw) = self.dict.get(syllable) {
                for &ch in raw {
                    if seen.insert(ch) {
                        pool.push(ch);
                    }
                }
            }
            let key = compact_lookup_key(syllable);
            if !key.is_empty() {
                if let Some(entries) = self.user_lexicon.lookup_input(&key) {
                    merge_user_entries(
                        &mut merged,
                        entries.iter().filter(|e| phrase_char_count(&e.phrase) == 1),
                        USER_EXACT_INPUT_BONUS,
                        24,
                        now,
                        None,
                        None,
                        Some(1),
                    );
                }
            }
        }
        for (rank, ch) in self
            .lm
            .rank_chars_by_unigram_with_limit(&pool, TSF_MAX_CANDIDATES)
            .into_iter()
            .enumerate()
        {
            let phrase = ch.to_string();
            let score =
                PHRASE_EXACT_PINYIN_BONUS + 84.0 + self.word_graph_single_char_bonus(ch, now)
                    - rank as f64 * 0.001;
            merge_candidate(&mut merged, phrase, score, None);
        }
        let rerank_prefs = rerank_prefs::get_rerank_prefs();
        let mut ranked: Vec<RankedCandidate> = merged
            .drain_candidates()
            .into_iter()
            .map(|(phrase, merged_candidate)| {
                let r = merged_candidate.score
                    + self.blended_rerank_with_prefs(&phrase, now, rerank_prefs)
                    + self.stability_bonus_for_candidate(full_compact_key, &phrase);
                RankedCandidate {
                    phrase,
                    score: r,
                    meta: merged_candidate.meta,
                }
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
        ranked
    }

    fn single_phrase_matches_syllable(&self, phrase: &str, syllable: &str) -> bool {
        if phrase_char_count(phrase) != 1 {
            return false;
        }
        let Some(ch) = phrase.chars().next() else {
            return false;
        };
        if self
            .dict
            .get(syllable)
            .is_some_and(|chars| chars.contains(&ch))
        {
            return true;
        }
        let key = compact_lookup_key(syllable);
        !key.is_empty()
            && self.user_lexicon.lookup_input(&key).is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| phrase_char_count(&entry.phrase) == 1 && entry.phrase == phrase)
            })
    }

    pub(super) fn merge_exact_single_syllable_chars(
        &self,
        syllable: &str,
        merged: &mut MergedCandidateMap,
        now: u64,
    ) {
        if let Some(raw) = self.dict.get(syllable) {
            for (rank, ch) in self
                .lm
                .rank_chars_by_unigram_with_limit(raw, TSF_MAX_CANDIDATES)
                .into_iter()
                .enumerate()
            {
                let phrase = ch.to_string();
                let score =
                    PHRASE_EXACT_PINYIN_BONUS + 90.0 + self.word_graph_single_char_bonus(ch, now)
                        - rank as f64 * 0.001;
                merge_candidate(merged, phrase, score, None);
            }
        }

        if let Some(entries) = self
            .phrase_lexicon
            .as_ref()
            .and_then(|lex| lex.lookup_pinyin(syllable))
        {
            merge_phrase_entries(
                merged,
                entries
                    .iter()
                    .filter(|entry| phrase_char_count(&entry.phrase) == 1),
                PHRASE_EXACT_PINYIN_BONUS + 150.0,
                TSF_MAX_CANDIDATES,
                &self.user_lexicon,
                now,
                Some(PINYIN_CANDIDATE_META),
                self.phrase_lexicon.as_ref(),
                None,
            );
        }
    }

    pub(super) fn merge_short_abbrev_leading_initial_singles(
        &self,
        raw: &str,
        compact_key: &str,
        merged: &mut MergedCandidateMap,
        now: u64,
    ) {
        if !is_pure_short_abbrev_initial_input(raw, compact_key) {
            return;
        }
        let Some(initial) = compact_key.chars().next() else {
            return;
        };

        let mut seen = HashSet::with_capacity(128);
        let mut pool = Vec::with_capacity(256);
        for (syllable, raw_chars) in self.dict.iter() {
            if !syllable.starts_with(initial) {
                continue;
            }
            for &ch in raw_chars {
                if seen.insert(ch) {
                    pool.push(ch);
                }
            }
        }
        if pool.is_empty() {
            return;
        }

        for (rank, ch) in self
            .lm
            .rank_chars_by_unigram_with_limit(&pool, TSF_MAX_CANDIDATES)
            .into_iter()
            .enumerate()
        {
            let phrase = ch.to_string();
            let score = self.prefix_abbrev_bonus() - 52.0
                + self.word_graph_single_char_bonus(ch, now)
                + self.context_bonus(&phrase, now) * SHORT_ABBREV_SINGLE_CONTEXT_SCALE
                - rank as f64 * 0.001;
            merge_candidate(merged, phrase, score, None);
        }
    }

    fn short_abbrev_initial_single_ranked(
        &self,
        raw: &str,
        compact_key: &str,
        now: u64,
    ) -> Vec<RankedCandidate> {
        let mut merged = MergedCandidateMap::with_capacity(64);
        self.merge_short_abbrev_leading_initial_singles(raw, compact_key, &mut merged, now);
        let rerank_prefs = rerank_prefs::get_rerank_prefs();
        let mut ranked = merged
            .drain_candidates()
            .into_iter()
            .map(|(phrase, merged_candidate)| RankedCandidate {
                score: merged_candidate.score
                    + self.blended_rerank_with_prefs(&phrase, now, rerank_prefs)
                    + self.stability_bonus_for_candidate(compact_key, &phrase),
                phrase,
                meta: merged_candidate.meta,
            })
            .collect::<Vec<_>>();
        sort_ranked_candidates_top_k(&mut ranked, TSF_PAGE_SIZE);
        ranked
    }

    /// 首批快速候选也必须遵守可见页的词长意图，并至少提供一个首字候选。
    pub(super) fn shape_partial_first_page_candidates(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        raw: &str,
        compact_key: &str,
        now: u64,
    ) {
        let mixed_hot_entries = self
            .phrase_lexicon
            .as_ref()
            .filter(|_| self.mixed_pinyin_enabled() && compact_key.chars().count() >= 4)
            .and_then(|lex| lex.lookup_mixed_hot(compact_key));
        let mixed_hot_phrases = mixed_hot_entries
            .as_deref()
            .into_iter()
            .flatten()
            .filter(|entry| {
                (2..=SHORT_HOTWORD_MAX_CHARS).contains(&phrase_char_count(&entry.phrase))
            })
            .take(MIXED_HOT_DIRECT_FRONT_LIMIT)
            .map(|entry| entry.phrase.clone())
            .collect::<Vec<_>>();
        let mixed_hot_char_counts = mixed_hot_entries
            .as_deref()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let chars = phrase_char_count(&entry.phrase);
                (2..=SHORT_HOTWORD_MAX_CHARS)
                    .contains(&chars)
                    .then_some(chars)
            })
            .collect::<HashSet<_>>();
        let mixed_hot_intent = (mixed_hot_char_counts.len() == 1)
            .then(|| mixed_hot_char_counts.iter().next().copied())
            .flatten();
        let mixed_hot_cross_length_collision = mixed_hot_char_counts.len() > 1;
        let mixed_hot_first_syllable = mixed_hot_entries.as_deref().and_then(|entries| {
            entries.iter().find_map(|entry| {
                entry
                    .code
                    .as_deref()
                    .and_then(|code| code.split_whitespace().next())
                    .map(str::to_string)
            })
        });
        let parsed = self.parse_variants_cached(raw).ok();
        let exact_parse_intent = parsed
            .as_deref()
            .and_then(infer_exact_complete_short_parse_intent);
        let parse_intent = parsed
            .as_deref()
            .and_then(infer_uniform_variant_char_count_detailed);
        let abbrev_intent = infer_short_abbrev_phrase_intent(
            raw,
            compact_key,
            ranked.iter().map(|item| &item.phrase),
        );
        let inferred_short_intent = choose_short_phrase_intent([
            (mixed_hot_intent, 94),
            (exact_parse_intent, 90),
            (abbrev_intent, 78),
            (parse_intent, 64),
        ]);
        let short_intent = if mixed_hot_cross_length_collision && exact_parse_intent.is_none() {
            None
        } else {
            inferred_short_intent
        };
        let prefer_abbrev_initial =
            mixed_hot_intent.is_none() && abbrev_intent.is_some() && exact_parse_intent.is_none();
        let first_syllable = mixed_hot_first_syllable.or_else(|| {
            parsed.as_deref().and_then(|variants| {
                variants
                    .iter()
                    .find(|variant| {
                        variant.source == ParseVariantSource::Exact && !variant.syllables.is_empty()
                    })
                    .or_else(|| {
                        variants.iter().find(|variant| {
                            variant.source == ParseVariantSource::AlternateSplit
                                && !variant.syllables.is_empty()
                        })
                    })
                    .or_else(|| {
                        variants
                            .iter()
                            .find(|variant| !variant.syllables.is_empty())
                    })
                    .and_then(|variant| variant.syllables.first().cloned())
            })
        });

        let page_size =
            candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        let mut support_singles = if prefer_abbrev_initial {
            self.short_abbrev_initial_single_ranked(raw, compact_key, now)
        } else if let Some(first_syllable) = first_syllable.as_deref() {
            self.first_syllable_single_ranked(first_syllable, compact_key, now)
        } else {
            Vec::new()
        };
        if support_singles.is_empty() {
            support_singles = self.short_abbrev_initial_single_ranked(raw, compact_key, now);
        }

        let mut seen = ranked
            .iter()
            .map(|item| item.phrase.clone())
            .collect::<HashSet<_>>();
        for item in support_singles.into_iter().take(page_size) {
            if seen.insert(item.phrase.clone()) {
                ranked.push(item);
            }
        }

        if let Some(intent_chars) = short_intent {
            apply_short_intent_final_candidate_density(
                ranked,
                Some(intent_chars),
                self.phrase_lexicon.as_ref(),
            );
            if intent_chars == 2 {
                arrange_two_char_intent_page_density(ranked, page_size);
            }
        }
        if !self.has_selection_feedback_for_reading(compact_key) {
            promote_first_preserved_after_user_priority(ranked, &mixed_hot_phrases);
        }
        ensure_single_char_visible_on_first_page(ranked, page_size);
    }

    /// 多音节完整输入：优先展示若干“等长整词 head”，并保证每页都能看到单字候选用于拼词。

    /// 同时保留其他非等长候选，避免用户翻页时只剩整词路径。

    pub(super) fn apply_multi_syllable_phrase_head_then_first_syllable_singles(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        syllable_count: usize,
        syllables: &[String],
        full_compact_key: &str,
        now: u64,
    ) {
        let all = std::mem::take(ranked);
        let Some(first_syllable) = syllables.first().map(String::as_str) else {
            return;
        };
        let exact_pinyin_phrases: HashSet<String> = self
            .phrase_lexicon
            .as_ref()
            .and_then(|lex| lex.lookup_pinyin(full_compact_key))
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| phrase_char_count(&entry.phrase) == syllable_count)
                    .map(|entry| entry.phrase.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut exact_multi = Vec::new();
        let mut composed_multi = Vec::new();
        let mut others = Vec::new();
        let mut singles_from_ranked = Vec::new();
        for item in all {
            let n = phrase_char_count(&item.phrase);
            if n == 1 {
                if self.single_phrase_matches_syllable(&item.phrase, first_syllable) {
                    singles_from_ranked.push(item);
                }
            } else if n == syllable_count && n >= 2 {
                if exact_pinyin_phrases.contains(&item.phrase) {
                    exact_multi.push(item);
                } else {
                    composed_multi.push(item);
                }
            } else {
                others.push(item);
            }
        }
        let has_exact_full_pinyin = !exact_multi.is_empty();
        let (exact_head_min, exact_head_max) = if syllable_count == 3 {
            (
                THREE_CHAR_CONFIDENT_FRONT_MIN,
                THREE_CHAR_CONFIDENT_FRONT_MAX,
            )
        } else {
            (MULTI_SYLL_PHRASE_HEAD_MIN, MULTI_SYLL_PHRASE_HEAD_MAX)
        };
        let (mut exact_head, mut exact_tail) =
            self.split_multi_syllable_phrase_head(exact_multi, exact_head_min, exact_head_max);
        let (mut composed_head, composed_tail) = self.split_multi_syllable_phrase_head(
            composed_multi,
            MULTI_SYLL_PHRASE_HEAD_MIN,
            MULTI_SYLL_PHRASE_HEAD_MAX,
        );

        // 合并首音节的高频单字、当前排序里已有的首音节单字和首音节单字池，并去重。

        let mut singles = Vec::new();
        let mut seen_single = HashSet::with_capacity(singles_from_ranked.len() * 2 + 16);
        let long_input = full_compact_key.chars().count() > LONG_INPUT_COMPACT_CHAR_LIMIT;
        let composing_phrase = self.mode_flags & MODE_USER_PHRASE_COMPOSE_ACTIVE != 0;
        let take_count = if composing_phrase {
            4
        } else if long_input {
            1
        } else {
            2
        };
        for item in self
            .first_syllable_single_ranked(first_syllable, full_compact_key, now)
            .into_iter()
            .take(take_count)
        {
            if seen_single.insert(item.phrase.clone()) {
                singles.push(item);
            }
        }
        for item in singles_from_ranked {
            if seen_single.insert(item.phrase.clone()) {
                singles.push(item);
            }
        }
        if !long_input {
            for item in self.syllable_pool_single_ranked(&syllables[..1], full_compact_key, now) {
                if seen_single.insert(item.phrase.clone()) {
                    singles.push(item);
                }
            }
        }

        // 先给整词 head，再插入单字；并确保每页有最少单字槽位，便于用户逐字造词。

        let mut prefix_support = if has_exact_full_pinyin {
            self.full_pinyin_prefix_phrase_support_candidates(syllables, full_compact_key, now)
        } else {
            Vec::new()
        };
        let page_size =
            candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
        let reserve_single_slots_per_page = if composing_phrase {
            3usize.min(page_size.saturating_sub(1))
        } else {
            1usize.min(page_size.saturating_sub(1))
        };
        let front_limit = page_size
            .saturating_sub(reserve_single_slots_per_page)
            .max(1)
            .min(if has_exact_full_pinyin {
                exact_head_max
            } else {
                MULTI_SYLL_PHRASE_HEAD_MAX
            });
        ranked.reserve(
            exact_head
                .len()
                .saturating_add(exact_tail.len())
                .saturating_add(composed_head.len())
                .saturating_add(composed_tail.len())
                .saturating_add(prefix_support.len())
                .saturating_add(singles.len())
                .saturating_add(others.len()),
        );
        let mut emitted = HashSet::with_capacity(ranked.capacity().min(512));
        if has_exact_full_pinyin {
            let exact_front = exact_head.len().min(front_limit);
            for item in exact_head.drain(..exact_front) {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            let prefix_front = prefix_support
                .len()
                .min(front_limit.saturating_sub(exact_front));
            for item in prefix_support.drain(..prefix_front) {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            if syllable_count == 2 && !composing_phrase {
                let mut front_fill = page_size
                    .saturating_sub(reserve_single_slots_per_page)
                    .saturating_sub(ranked.len());
                while front_fill > 0 {
                    let item = if !exact_head.is_empty() {
                        Some(exact_head.remove(0))
                    } else if let Some(pos) = exact_tail.iter().position(|item| {
                        candidate_has_user_priority(item)
                            || self
                                .phrase_lexicon
                                .as_ref()
                                .map(|lex| {
                                    matches!(
                                        lex.phrase_layer(&item.phrase),
                                        LexiconLayer::Core | LexiconLayer::Base
                                    ) || lex.phrase_frequency(&item.phrase)
                                        >= SHORT_INPUT_EXACT_FRONT_FREQ_MIN
                                })
                                .unwrap_or(false)
                    }) {
                        Some(exact_tail.remove(pos))
                    } else if !prefix_support.is_empty() {
                        Some(prefix_support.remove(0))
                    } else {
                        None
                    };
                    let Some(item) = item else {
                        break;
                    };
                    push_unique_ranked_candidate(ranked, &mut emitted, item);
                    front_fill -= 1;
                }
            }
            for item in singles.iter().cloned() {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in exact_head {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in exact_tail {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in prefix_support {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in composed_head {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in composed_tail {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
        } else {
            let composed_front = composed_head.len().min(front_limit);
            for item in composed_head.drain(..composed_front) {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in singles.iter().cloned() {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in composed_head {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
            for item in composed_tail {
                push_unique_ranked_candidate(ranked, &mut emitted, item);
            }
        }

        // 逐页补齐单字槽位（从第二页开始也持续保留），避免“翻页看不到单字”。

        let mut single_idx = singles.len();
        for item in others {
            if ranked.len() % page_size == page_size - reserve_single_slots_per_page
                && single_idx < singles.len()
            {
                push_unique_ranked_candidate(ranked, &mut emitted, singles[single_idx].clone());
                single_idx += 1;
            }
            push_unique_ranked_candidate(ranked, &mut emitted, item);
        }
        for item in singles.into_iter().skip(single_idx) {
            push_unique_ranked_candidate(ranked, &mut emitted, item);
        }
    }

    pub(super) fn split_multi_syllable_phrase_head(
        &self,
        items: Vec<RankedCandidate>,
        head_min: usize,
        head_max: usize,
    ) -> (Vec<RankedCandidate>, Vec<RankedCandidate>) {
        let head_min = head_min.max(1).min(head_max.max(1));
        let head_max = head_max.max(1);
        let mut head = Vec::new();
        let mut tail = Vec::new();
        let mut head_closed = false;
        for item in items {
            if head.is_empty() {
                head.push(item);
                continue;
            }
            if head_closed || head.len() >= head_max {
                tail.push(item);
                continue;
            }
            let freq = self
                .phrase_lexicon
                .as_ref()
                .map(|lex| lex.phrase_frequency(&item.phrase))
                .unwrap_or(0);
            let below_freq_floor = head.len() >= head_min && freq < MULTI_SYLL_PHRASE_HEAD_FREQ_MIN;
            if below_freq_floor {
                head_closed = true;
                tail.push(item);
                continue;
            }
            let prev_sc = head.last().map(|h| h.score).unwrap_or(0.0);
            let gap = prev_sc - item.score;
            if gap > MULTI_SYLL_PHRASE_SCORE_GAP && head.len() >= head_min {
                head_closed = true;
                tail.push(item);
                continue;
            }
            head.push(item);
        }
        (head, tail)
    }

    pub(super) fn full_pinyin_prefix_phrase_support_candidates(
        &self,
        syllables: &[String],
        full_compact_key: &str,
        now: u64,
    ) -> Vec<RankedCandidate> {
        if syllables.len() < 3 {
            return Vec::new();
        }
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for end in 2..syllables.len().min(4) {
            let key = syllables[..end].join("");
            let Some(entries) = lex.lookup_pinyin(&key) else {
                continue;
            };
            for entry in entries
                .iter()
                .filter(|entry| phrase_char_count(&entry.phrase) == end)
                .take(FULL_PINYIN_PREFIX_SUPPORT_LIMIT)
            {
                if !seen.insert(entry.phrase.clone()) || entry.phrase == full_compact_key {
                    continue;
                }
                let score = PHRASE_EXACT_PINYIN_BONUS
                    + phrase_span_bonus(
                        end,
                        entry.freq,
                        self.user_lexicon.phrase_signal(&entry.phrase),
                        now,
                    )
                    - out.len() as f64 * 0.001;
                out.push(RankedCandidate {
                    phrase: entry.phrase.clone(),
                    score,
                    meta: CandidateMeta::legacy(PINYIN_CANDIDATE_META),
                });
            }
        }
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.phrase.cmp(&b.phrase))
        });
        out.truncate(FULL_PINYIN_PREFIX_SUPPORT_LIMIT);
        out
    }

    pub(super) fn ensure_short_intent_support_candidates(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        syllables: &[String],
        full_compact_key: &str,
        now: u64,
    ) {
        if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&syllables.len()) {
            return;
        }

        let mut seen = ranked
            .iter()
            .map(|item| item.phrase.clone())
            .collect::<HashSet<_>>();
        let base_score = ranked.first().map(|item| item.score - 0.001).unwrap_or(0.0);
        let mut extras = Vec::new();

        if syllables.len() >= 3 {
            if let Some(lex) = &self.phrase_lexicon {
                let prefix_key = syllables[..2].join("");
                if let Some(entries) = lex.lookup_pinyin(&prefix_key) {
                    for entry in entries
                        .iter()
                        .filter(|entry| phrase_char_count(&entry.phrase) == 2)
                        .take(4)
                    {
                        if seen.insert(entry.phrase.clone()) {
                            extras.push(RankedCandidate {
                                phrase: entry.phrase.clone(),
                                score: base_score - extras.len() as f64 * 0.001,
                                meta: CandidateMeta::legacy(PINYIN_CANDIDATE_META),
                            });
                        }
                    }
                }
            }
        }

        let Some(first_syllable) = syllables.first().map(String::as_str) else {
            return;
        };
        for item in self
            .first_syllable_single_ranked(first_syllable, full_compact_key, now)
            .into_iter()
            .take(2)
        {
            if seen.insert(item.phrase.clone()) {
                extras.push(item);
            }
        }
        for item in self.syllable_pool_single_ranked(&syllables[..1], full_compact_key, now) {
            if seen.insert(item.phrase.clone()) {
                extras.push(item);
            }
        }

        ranked.extend(extras);
    }

    pub(super) fn preferred_exact_complete_multi_variant(
        &self,
        variants: &[ParseVariant],
    ) -> Option<ParseVariant> {
        let mut exact_fallback = None;
        let mut alternate_fallback = None;
        let mut best_exact_lex_variant: Option<(ParseVariant, f64)> = None;
        let mut best_alternate_lex_variant: Option<(ParseVariant, f64)> = None;
        for variant in variants.iter().filter(|variant| {
            matches!(
                variant.source,
                ParseVariantSource::Exact | ParseVariantSource::AlternateSplit
            ) && variant.tail.is_none()
                && variant.syllables.len() >= 2
        }) {
            if variant.source == ParseVariantSource::Exact && exact_fallback.is_none() {
                exact_fallback = Some(variant.clone());
            } else if alternate_fallback.is_none() {
                alternate_fallback = Some(variant.clone());
            }
            let Some(lex) = self.phrase_lexicon.as_ref() else {
                continue;
            };
            let key = variant.syllables.join("");
            if let Some(top_freq) = lex.lookup_pinyin(&key).and_then(|entries| {
                entries
                    .iter()
                    .filter(|entry| phrase_char_count(&entry.phrase) == variant.syllables.len())
                    .map(|entry| entry.freq)
                    .max()
            }) {
                let score = exact_full_pinyin_variant_score(
                    top_freq,
                    variant.syllables.len(),
                    key.len(),
                    variant.source,
                );
                let target = if variant.source == ParseVariantSource::Exact {
                    &mut best_exact_lex_variant
                } else {
                    &mut best_alternate_lex_variant
                };
                let replace = target
                    .as_ref()
                    .map(|(_, best_score)| score > *best_score)
                    .unwrap_or(true);
                if replace {
                    *target = Some((variant.clone(), score));
                }
            }
        }
        best_exact_lex_variant
            .map(|(variant, _)| variant)
            .or(exact_fallback)
            .or_else(|| best_alternate_lex_variant.map(|(variant, _)| variant))
            .or(alternate_fallback)
    }

    pub(super) fn apply_candidate_postprocess(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        ctx: CandidatePostprocessContext<'_>,
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

        let has_direct = !ctx.preserved_direct_phrases.is_empty();
        let has_exact_user = !ctx.preserved_exact_user_phrases.is_empty();
        let has_pinned = !ctx.preserved_pinned_user_phrases.is_empty();
        let has_user_or_direct_lock = has_direct || has_exact_user || has_pinned;

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
            return;
        }

        let preserve_ascii = should_preserve_ascii_input(ctx.raw);
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

        // Final explicit priority table. Broad short-abbrev candidates are not
        // locked here, so density shaping can still limit noisy expansions.
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
        demote_corrections_after_confident_long_exact(ranked);
    }

    pub(super) fn promote_exact_full_pinyin_candidates(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        compact_key: &str,
        syllable_count: usize,
    ) {
        let phrases = self.exact_full_pinyin_phrases(compact_key, syllable_count);
        if phrases.is_empty() {
            return;
        }
        if ranked
            .first()
            .is_some_and(|item| phrases.first() == Some(&item.phrase))
        {
            return;
        }
        if ranked.first().is_some_and(|item| {
            phrase_char_count(&item.phrase) == syllable_count && candidate_has_user_priority(item)
        }) {
            return;
        }
        promote_preserved_candidates(ranked, &phrases);
    }

    pub(super) fn exact_full_pinyin_phrases(
        &self,
        compact_key: &str,
        syllable_count: usize,
    ) -> Vec<String> {
        if syllable_count < 2 {
            return Vec::new();
        }
        let Some(lex) = self.phrase_lexicon.as_ref() else {
            return Vec::new();
        };
        let Some(entries) = lex.lookup_pinyin(compact_key) else {
            return Vec::new();
        };
        let mut phrases = entries
            .iter()
            .filter(|entry| phrase_char_count(&entry.phrase) == syllable_count)
            .map(|entry| entry.phrase.clone())
            .collect::<Vec<_>>();
        phrases.sort_by(|a, b| {
            lex.phrase_frequency(b)
                .cmp(&lex.phrase_frequency(a))
                .then_with(|| a.cmp(b))
        });
        phrases.truncate(full_pinyin_exact_guard_limit(syllable_count));
        phrases
    }

    pub(super) fn promote_exact_full_pinyin_over_overlong_front(
        &self,
        ranked: &mut Vec<RankedCandidate>,
        compact_key: &str,
        syllable_count: usize,
    ) {
        if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&syllable_count) {
            return;
        }
        if ranked
            .first()
            .map(|item| phrase_char_count(&item.phrase) <= syllable_count)
            .unwrap_or(true)
        {
            return;
        }
        self.promote_exact_full_pinyin_candidates(ranked, compact_key, syllable_count);
    }
}

pub(super) fn ensure_single_char_visible_on_first_page(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    if ranked.is_empty() {
        return;
    }
    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    if ranked
        .iter()
        .take(page_size)
        .any(|item| phrase_char_count(&item.phrase) == 1)
    {
        return;
    }
    let Some(pos) = ranked
        .iter()
        .position(|item| phrase_char_count(&item.phrase) == 1)
    else {
        return;
    };
    let item = ranked.remove(pos);
    let insert_at = page_size.saturating_sub(1).min(ranked.len());
    ranked.insert(insert_at, item);
}

/// Reapply the visible-page length contract after late priority or stability
/// reranking. The established full-pinyin order is preserved where possible;
/// if there are not enough short candidates to fill a page, ordinary overlong
/// predictions are removed rather than being allowed to fill the empty slots.
pub(super) fn enforce_short_intent_visible_page(
    ranked: &mut Vec<RankedCandidate>,
    intent_chars: usize,
    page_size: usize,
) {
    if ranked.len() <= 1 || !matches!(intent_chars, 2 | 3) {
        return;
    }

    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    if let Some(exact_pos) = ranked
        .iter()
        .position(|item| phrase_char_count(&item.phrase) == intent_chars)
    {
        if exact_pos > 0 {
            const EXACT_TOP_COMPETITIVE_MARGIN: f64 = 1.0;
            let top = &ranked[0];
            let exact = &ranked[exact_pos];
            let top_is_overlong = phrase_char_count(&top.phrase) > intent_chars;
            let exact_is_competitive = exact.score + EXACT_TOP_COMPETITIVE_MARGIN >= top.score;
            let top_is_correction = correction_style_kind(&top.meta).is_some();
            if top_is_overlong || (exact_is_competitive && !top_is_correction) {
                let exact = ranked.remove(exact_pos);
                ranked.insert(0, exact);
            }
        }
    }

    let short_count = ranked
        .iter()
        .filter(|item| (1..=intent_chars).contains(&phrase_char_count(&item.phrase)))
        .count();
    let exception_phrase = ranked
        .iter()
        .find(|item| item.meta.pinned && phrase_char_count(&item.phrase) > intent_chars)
        .or_else(|| {
            (intent_chars == 3)
                .then(|| {
                    ranked
                        .iter()
                        .find(|item| phrase_char_count(&item.phrase) == 4)
                })
                .flatten()
        })
        .map(|item| item.phrase.clone());
    if short_count + usize::from(exception_phrase.is_some()) < page_size {
        ranked.retain(|item| {
            let phrase_chars = phrase_char_count(&item.phrase);
            (1..=intent_chars).contains(&phrase_chars)
                || exception_phrase
                    .as_ref()
                    .is_some_and(|phrase| phrase == &item.phrase)
        });
    }

    let mut visible_overlong = false;
    let mut index = 0usize;
    while index < page_size.min(ranked.len()) {
        let phrase_chars = phrase_char_count(&ranked[index].phrase);
        let is_short = (1..=intent_chars).contains(&phrase_chars);
        let is_allowed_prediction = !visible_overlong
            && ((intent_chars == 3 && phrase_chars == 4)
                || (ranked[index].meta.pinned && phrase_chars > intent_chars));
        if is_short || is_allowed_prediction {
            visible_overlong |= phrase_chars > intent_chars;
            index += 1;
            continue;
        }

        let replacement = ((index + 1)..ranked.len()).find(|&candidate_index| {
            let candidate = &ranked[candidate_index];
            let candidate_chars = phrase_char_count(&candidate.phrase);
            (1..=intent_chars).contains(&candidate_chars)
                || (!visible_overlong
                    && ((intent_chars == 3 && candidate_chars == 4)
                        || (candidate.meta.pinned && candidate_chars > intent_chars)))
        });
        let Some(replacement) = replacement else {
            ranked.remove(index);
            continue;
        };
        let candidate = ranked.remove(replacement);
        ranked.insert(index, candidate);
    }

    ensure_single_char_visible_on_first_page(ranked, page_size);
}

pub(super) fn demote_corrections_after_confident_long_exact(ranked: &mut Vec<RankedCandidate>) {
    let Some(top) = ranked.first() else {
        return;
    };
    if phrase_char_count(&top.phrase) < 5 || top.meta.match_kind != CandidateMatchKind::FullPinyin {
        return;
    }
    let top_score = top.score;
    let mut normal = Vec::with_capacity(ranked.len());
    let mut corrections = Vec::new();
    for item in std::mem::take(ranked) {
        if correction_style_kind(&item.meta).is_some() && top_score - item.score >= 80.0 {
            corrections.push(item);
        } else {
            normal.push(item);
        }
    }
    let insert_at = normal.len().min(TSF_PAGE_SIZE);
    normal.splice(insert_at..insert_at, corrections);
    *ranked = normal;
}

pub(super) fn promote_mixed_prefix_core_phrase_front(
    ranked: &mut Vec<RankedCandidate>,
    short_intent_chars: Option<usize>,
) {
    if ranked.len() <= 1
        || ranked
            .first()
            .is_some_and(|item| is_core_long_mixed_prefix_candidate(item, short_intent_chars))
    {
        return;
    }
    let top_score = ranked.first().map(|item| item.score).unwrap_or(0.0);
    let Some((idx, _)) = ranked
        .iter()
        .enumerate()
        .skip(1)
        .take(TSF_PAGE_SIZE.saturating_sub(1))
        .filter(|(_, item)| {
            is_core_long_mixed_prefix_candidate(item, short_intent_chars)
                && top_score - item.score <= MIXED_PREFIX_CORE_PHRASE_PROMOTE_MARGIN
        })
        .max_by(|(_, a), (_, b)| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.phrase.cmp(&a.phrase))
        })
    else {
        return;
    };
    let item = ranked.remove(idx);
    ranked.insert(0, item);
}

fn is_core_long_mixed_prefix_candidate(
    item: &RankedCandidate,
    short_intent_chars: Option<usize>,
) -> bool {
    let phrase_chars = phrase_char_count(&item.phrase);
    let exact_short_core_collision = short_intent_chars == Some(2)
        && phrase_chars == 2
        && item.meta.source_layer == LexiconLayer::Core
        && matches!(
            item.meta.match_kind,
            CandidateMatchKind::MixedPrefix | CandidateMatchKind::ShortAbbrev
        );
    exact_short_core_collision
        || (item.meta.match_kind == CandidateMatchKind::MixedPrefix
            && matches!(
                item.meta.source_layer,
                LexiconLayer::Core | LexiconLayer::Base
            )
            && phrase_chars >= 3
            && short_intent_chars.is_none_or(|intent_chars| phrase_chars <= intent_chars))
}

pub(super) fn dedup_preserved_phrase_order(phrases: &mut Vec<String>) {
    let mut seen = HashSet::new();
    phrases.retain(|phrase| seen.insert(phrase.clone()));
}

pub(super) fn sort_preserved_phrases_by_lexicon_frequency(
    phrases: &mut Vec<String>,
    lexicon: Option<&AbbrevLexicon>,
) {
    if phrases.len() <= 1 {
        return;
    }

    let Some(lexicon) = lexicon else {
        return;
    };
    phrases.sort_by(|a, b| {
        lexicon
            .phrase_frequency(b)
            .cmp(&lexicon.phrase_frequency(a))
            .then_with(|| a.cmp(b))
    });
}

/// Enforce the lexicon frequency order at the last presentation boundary.
///
/// Scoring and the many candidate-preservation passes above are intentionally
/// allowed to decide which rows survive, but they must not change the order of
/// ordinary system words that match the exact full-pinyin or jianpin key.
/// User, pinned, direct, shortcut, date/time, and correction rows keep their
/// independent priority and occupy their existing slots.
pub(super) fn enforce_strict_system_lexicon_frequency_order(
    ranked: &mut Vec<RankedCandidate>,
    compact_key: &str,
    intent: InputIntent,
    phrase_len: usize,
    lexicon: Option<&AbbrevLexicon>,
) {
    if ranked.len() <= 1 || phrase_len < 2 {
        return;
    }
    let Some(lexicon) = lexicon else {
        return;
    };
    let Some(entries) = (match intent {
        InputIntent::FullPinyin => lexicon.lookup_pinyin(compact_key),
        InputIntent::ShortAbbrev => lexicon.lookup(compact_key),
        _ => None,
    }) else {
        return;
    };

    let mut frequency_by_phrase = HashMap::with_capacity(entries.len());
    for entry in entries.iter() {
        if phrase_char_count(&entry.phrase) == phrase_len {
            frequency_by_phrase.insert(
                entry.phrase.clone(),
                lexicon.phrase_frequency(&entry.phrase),
            );
        }
    }
    if frequency_by_phrase.len() <= 1 {
        return;
    }

    let mut eligible = ranked
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(
                item.meta.source,
                CandidateSource::System
                    | CandidateSource::Abbrev
                    | CandidateSource::Pinyin
                    | CandidateSource::Mixed
            ) && !candidate_has_user_priority(item)
                && frequency_by_phrase.contains_key(&item.phrase)
        })
        .map(|(index, item)| (index, item.clone()))
        .collect::<Vec<_>>();
    if eligible.len() <= 1 {
        return;
    }

    let eligible_positions = eligible.iter().map(|(index, _)| *index).collect::<Vec<_>>();
    eligible.sort_by(|(_, a), (_, b)| {
        frequency_by_phrase
            .get(&b.phrase)
            .cmp(&frequency_by_phrase.get(&a.phrase))
            .then_with(|| a.phrase.cmp(&b.phrase))
    });
    for (index, (_, item)) in eligible_positions.into_iter().zip(eligible) {
        ranked[index] = item;
    }
}

pub(super) fn prioritize_preserved_phrase_length(phrases: &mut Vec<String>, preferred_len: usize) {
    if phrases.len() <= 1 {
        return;
    }

    let mut preferred = Vec::new();
    let mut rest = Vec::new();
    for phrase in std::mem::take(phrases) {
        if phrase_char_count(&phrase) == preferred_len {
            preferred.push(phrase);
        } else {
            rest.push(phrase);
        }
    }
    phrases.extend(preferred);
    phrases.extend(rest);
}

#[allow(clippy::ptr_arg)]
pub(super) fn promote_preferred_short_abbrev(compact_key: &str, phrases: &mut Vec<String>) {
    let preferred: &[&str] = match compact_key {
        "bejing" => &["北京"],
        "bj" => &["北京"],
        "chongq" => &["重庆"],
        "cq" => &["重庆"],
        "fenghua" => &["奉化"],
        "fengh" => &["奉化"],
        "jint" => &["今天"],
        "jntian" => &["今天"],
        "migtian" => &["明天"],
        "nj" => &["南京"],
        "sj" => &["世界", "手机", "睡觉"],
        "dt" => &["大厅", "地铁"],
        "js" => &["就是", "结束"],
        "fz" => &["房子"],
        "sb" => &["设备", "上班"],
        "bb" => &["宝宝"],
        "gw" => &["给我", "购物"],
        "zj" => &["再见"],
        "sg" => &["水果"],
        "sc" => &["市场", "蔬菜"],
        "cs" => &["城市", "超市"],
        "pj" => &["普京", "啤酒"],
        "zc" => &["支持", "早餐"],
        "xb" => &["宣布", "下班"],
        "wc" => &["完成", "晚餐"],
        "kd" => &["看到", "快递"],
        "wm" => &["我们", "外卖"],
        "mc" => &["买菜"],
        "qr" => &["确认"],
        "kh" => &["客户"],
        "sh" => &["上海"],
        "shangh" => &["上海"],
        "shur" => &["输入"],
        "yy" => &["医院", "音乐"],
        "zg" => &["中国"],
        "hd" => &["好的"],
        "hl" => &["好了"],
        "sd" => &["是的"],
        "dl" => &["对了"],
        "ygrm" => &["一个人吗"],
        _ => &[],
    };
    for phrase in preferred.iter().rev() {
        phrases.retain(|existing| existing != phrase);
        phrases.insert(0, (*phrase).to_string());
    }
}

pub(super) fn is_high_priority_two_char_entry(entry: &ThuoclEntry) -> bool {
    phrase_char_count(&entry.phrase) == 2 && short_phrase_priority_score(entry.freq, 2) >= 0.82
}

pub(super) fn is_chat_priority_two_char_entry(entry: &ThuoclEntry) -> bool {
    phrase_char_count(&entry.phrase) == 2
        && (0.55..0.82).contains(&short_phrase_priority_score(entry.freq, 2))
}

pub(super) fn is_daily_short_two_char_entry(entry: &ThuoclEntry) -> bool {
    phrase_char_count(&entry.phrase) == 2
        && (0.25..0.55).contains(&short_phrase_priority_score(entry.freq, 2))
}

pub(super) fn is_daily_short_three_char_entry(entry: &ThuoclEntry) -> bool {
    phrase_char_count(&entry.phrase) == 3 && short_phrase_priority_score(entry.freq, 3) >= 0.35
}

pub(super) fn is_protected_two_char_entry(entry: &ThuoclEntry) -> bool {
    is_high_priority_two_char_entry(entry) || is_chat_priority_two_char_entry(entry)
}

/// 计算短词优先级的平滑分数 [0, 1]，基于词频在对数域上的相对位置。
pub(super) fn short_phrase_priority_score(freq: u64, char_count: usize) -> f64 {
    let (low, high) = match char_count {
        1 => (MAX_LEXICON_FREQ * 50 / 100, MAX_LEXICON_FREQ),
        2 => (MAX_LEXICON_FREQ * 20 / 100, HIGH_PRIORITY_TWO_CHAR_FREQ_MIN),
        3 => (MAX_LEXICON_FREQ * 15 / 100, DAILY_SHORT_FREQ_MAX),
        4 => (MAX_LEXICON_FREQ * 10 / 100, DAILY_SHORT_FREQ_MAX),
        _ => return 0.0,
    };
    if freq == 0 || high <= low {
        return 0.0;
    }
    let min_log = (low as f64 + 1.0).ln();
    let max_log = (high as f64 + 1.0).ln();
    let score = ((freq as f64 + 1.0).ln() - min_log) / (max_log - min_log);
    score.clamp(0.0, 1.0)
}

pub(super) fn high_priority_two_char_abbrev_key(
    raw: &str,
    compact_key: &str,
    syllable_set: &HashSet<String>,
) -> Option<String> {
    if compact_key.is_empty()
        || !raw.chars().all(|ch| ch.is_ascii_alphabetic())
        || raw.chars().count() != compact_key.chars().count()
        || !compact_key.chars().all(|ch| ch.is_ascii_alphabetic())
    {
        return None;
    }

    let chars = compact_key.chars().collect::<Vec<_>>();
    if chars.len() == 2 {
        return Some(compact_key.to_string());
    }

    let second_initial = *chars.last()?;
    let first_syllable = chars[..chars.len() - 1].iter().collect::<String>();
    if syllable_set.contains(&first_syllable) {
        let first_initial = first_syllable.chars().next()?;
        return Some(format!("{first_initial}{second_initial}"));
    }

    let first_initial = *chars.first()?;
    let second_syllable = chars[1..].iter().collect::<String>();
    if syllable_set.contains(&second_syllable) {
        let second_initial = second_syllable.chars().next()?;
        return Some(format!("{first_initial}{second_initial}"));
    }

    None
}

pub(super) fn promote_ranked_high_priority_two_char_candidates(
    ranked: &mut Vec<RankedCandidate>,
    raw: &str,
    compact_key: &str,
    syllable_set: &HashSet<String>,
    lexicon: Option<&AbbrevLexicon>,
) {
    let Some(lexicon) = lexicon else {
        return;
    };
    let Some(target_abbrev) = high_priority_two_char_abbrev_key(raw, compact_key, syllable_set)
    else {
        return;
    };

    let mut phrases = ranked
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
        .filter(|item| {
            short_phrase_priority_score(lexicon.phrase_frequency(&item.phrase), 2)
                >= HIGH_PRIORITY_SCORE_THRESHOLD
        })
        .filter(|item| {
            crate::thuocl::abbrev_for_phrase(&item.phrase).as_deref()
                == Some(target_abbrev.as_str())
        })
        .map(|item| item.phrase.clone())
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        return;
    }
    phrases.sort_by(|a, b| {
        lexicon
            .phrase_frequency(b)
            .cmp(&lexicon.phrase_frequency(a))
            .then_with(|| a.cmp(b))
    });
    // A full-syllable-plus-initial input is more specific than its derived
    // two-letter abbreviation. Preserve that intent after frequency sorting.
    promote_preferred_short_abbrev(&target_abbrev, &mut phrases);
    promote_preferred_short_abbrev(compact_key, &mut phrases);
    dedup_preserved_phrase_order(&mut phrases);
    promote_preserved_candidates(ranked, &phrases);
}

pub(super) fn promote_preserved_exact_short_abbrev_top1(
    ranked: &mut Vec<RankedCandidate>,
    raw: &str,
    compact_key: &str,
    groups: &[&[String]],
    lexicon: Option<&AbbrevLexicon>,
) {
    if ranked.len() <= 1 || !is_pure_short_abbrev_initial_input(raw, compact_key) {
        return;
    }
    if ranked.first().is_some_and(|item| {
        item.meta.pinned
            || candidate_has_user_priority(item)
            || matches!(
                item.meta.source,
                CandidateSource::Direct | CandidateSource::Shortcut | CandidateSource::DateTime
            )
    }) {
        return;
    }

    let input_chars = compact_key.chars().count();
    if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&input_chars) {
        return;
    }

    let mut phrases = groups
        .iter()
        .flat_map(|group| group.iter())
        .filter(|phrase| phrase_char_count(phrase) == input_chars)
        .cloned()
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        return;
    }
    dedup_preserved_phrase_order(&mut phrases);
    sort_short_abbrev_top1_phrases(&mut phrases, lexicon);
    promote_preferred_short_abbrev(compact_key, &mut phrases);
    dedup_preserved_phrase_order(&mut phrases);
    promote_preserved_candidates(ranked, &phrases);
}

pub(super) fn sort_short_abbrev_top1_phrases(
    phrases: &mut Vec<String>,
    lexicon: Option<&AbbrevLexicon>,
) {
    if phrases.len() <= 1 {
        return;
    }
    let Some(lexicon) = lexicon else {
        phrases.sort();
        return;
    };
    phrases.sort_by(|a, b| {
        lexicon
            .phrase_frequency(b)
            .cmp(&lexicon.phrase_frequency(a))
            .then_with(|| a.cmp(b))
    });
}

pub(super) fn explicit_separator_phrase_key_from_variants(
    raw: &str,
    variants: &[ParseVariant],
) -> Option<(String, usize)> {
    let has_separator = raw.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '\u{2019}'
                    | '\u{2018}'
                    | '\u{02BC}'
                    | '-'
                    | '_'
                    | '\u{2010}'
                    | '\u{2011}'
                    | '\u{2013}'
                    | '\u{2014}'
            )
    });
    if !has_separator {
        return None;
    }
    let variant = variants
        .iter()
        .find(|variant| variant.tail.is_none() && variant.syllables.len() >= 2)?;
    if variant.tail.is_some() || variant.syllables.len() < 2 {
        return None;
    }
    Some((variant.syllables.join(""), variant.syllables.len()))
}

pub(super) fn single_insert_ascii_variants(key: &str) -> Vec<String> {
    let chars: Vec<char> = key.chars().collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for idx in 0..=chars.len() {
        for alt in [
            'a', 'e', 'i', 'o', 'u', 'n', 'g', 'h', 'r', 'y', 'w', 'm', 'l', 'b', 'p', 'f', 'd',
            't', 'k', 'j', 'q', 'x', 'z', 'c', 's', 'v',
        ] {
            let mut candidate = chars.clone();
            candidate.insert(idx, alt);
            let text: String = candidate.into_iter().collect();
            if seen.insert(text.clone()) {
                out.push(text);
            }
        }
    }
    out
}

pub(super) fn push_unique_ranked_candidate(
    ranked: &mut Vec<RankedCandidate>,
    seen: &mut HashSet<String>,
    item: RankedCandidate,
) {
    if seen.insert(item.phrase.clone()) {
        ranked.push(item);
    }
}

pub(super) fn full_pinyin_exact_guard_limit(syllable_count: usize) -> usize {
    match syllable_count {
        2 => SHORT_INPUT_EXACT_FRONT_LIMIT_TWO,
        3 => SHORT_INPUT_EXACT_FRONT_LIMIT_THREE,
        4 => SHORT_INPUT_EXACT_FRONT_LIMIT_FOUR,
        _ => MULTI_SYLL_PHRASE_HEAD_MIN,
    }
}

pub(super) const SHORT_FULL_PINYIN_TWO_CHAR_BONUS: f64 = 2_000_000_000.0;
pub(super) const SHORT_FULL_PINYIN_THREE_CHAR_BONUS: f64 = 1_000_000_000.0;
pub(super) const EXACT_VARIANT_SOURCE_BONUS: f64 = 1_000.0;
pub(super) const ALTERNATE_SPLIT_VARIANT_SOURCE_BONUS: f64 = 800.0;

pub(super) fn exact_full_pinyin_variant_score(
    top_freq: u64,
    syllable_count: usize,
    compact_len: usize,
    source: ParseVariantSource,
) -> f64 {
    let short_full_pinyin_bonus = if compact_len <= 8 {
        match syllable_count {
            2 => SHORT_FULL_PINYIN_TWO_CHAR_BONUS,
            3 => SHORT_FULL_PINYIN_THREE_CHAR_BONUS,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let source_bonus = match source {
        ParseVariantSource::Exact => EXACT_VARIANT_SOURCE_BONUS,
        ParseVariantSource::AlternateSplit => ALTERNATE_SPLIT_VARIANT_SOURCE_BONUS,
        _ => 0.0,
    };
    top_freq as f64 + short_full_pinyin_bonus + source_bonus
}

pub(super) fn promote_preserved_candidates(ranked: &mut Vec<RankedCandidate>, phrases: &[String]) {
    promote_candidate_priority_groups(ranked, &[phrases]);
}

/// Make the strongest exact mixed-index hit visible without overriding
/// personalized candidates. Unlike the generic preserved-group promoter this
/// never injects missing rows and never moves a system phrase ahead of a
/// pinned or learned user candidate.
pub(super) fn promote_first_preserved_after_user_priority(
    ranked: &mut Vec<RankedCandidate>,
    phrases: &[String],
) {
    let Some(target) = phrases.first() else {
        return;
    };
    let Some(target_index) = ranked.iter().position(|item| item.phrase == *target) else {
        return;
    };
    let item = ranked.remove(target_index);
    let mut user_priority = Vec::new();
    let mut ordinary = Vec::with_capacity(ranked.len());
    for candidate in std::mem::take(ranked) {
        if candidate_has_user_priority(&candidate) {
            user_priority.push(candidate);
        } else {
            ordinary.push(candidate);
        }
    }
    ranked.extend(user_priority);
    ranked.push(item);
    ranked.extend(ordinary);
}

pub(super) fn promote_candidate_priority_groups(
    ranked: &mut Vec<RankedCandidate>,
    groups: &[&[String]],
) {
    if groups.iter().all(|group| group.is_empty()) {
        return;
    }
    let mut desired_order: HashMap<&str, (usize, usize)> = HashMap::new();
    for (group_idx, group) in groups.iter().enumerate() {
        for (phrase_idx, phrase) in group.iter().enumerate() {
            desired_order
                .entry(phrase.as_str())
                .or_insert((group_idx, phrase_idx));
        }
    }
    if desired_order.is_empty() {
        return;
    }

    let existing_phrases: HashSet<String> = ranked.iter().map(|item| item.phrase.clone()).collect();
    let mut inject_score = ranked
        .first()
        .map(|item| item.score + desired_order.len() as f64 + 1.0)
        .unwrap_or(400.0);
    let mut injected_phrases = HashSet::new();
    for group in groups {
        for phrase in *group {
            if existing_phrases.contains(phrase) || !injected_phrases.insert(phrase.as_str()) {
                continue;
            }
            ranked.push(RankedCandidate {
                phrase: phrase.clone(),
                score: inject_score,
                meta: CandidateMeta::default(),
            });
            inject_score -= 1.0;
        }
    }

    let mut preserved = Vec::new();
    let mut rest = Vec::new();
    for item in std::mem::take(ranked) {
        if desired_order.contains_key(item.phrase.as_str()) {
            preserved.push(item);
        } else {
            rest.push(item);
        }
    }
    preserved.sort_by_key(|item| {
        desired_order
            .get(item.phrase.as_str())
            .copied()
            .unwrap_or((usize::MAX, usize::MAX))
    });
    ranked.extend(preserved);
    ranked.extend(rest);
}

pub(super) fn promote_exact_user_hotwords_front(
    ranked: &mut Vec<RankedCandidate>,
    phrases: &[String],
    exact_len_guard: Option<usize>,
    front_limit: usize,
) {
    if ranked.is_empty() || phrases.is_empty() || front_limit == 0 {
        return;
    }

    let user_phrases = ranked
        .iter()
        .filter(|item| candidate_has_user_priority(item))
        .map(|item| item.phrase.clone())
        .collect::<HashSet<_>>();
    let mut desired = Vec::new();
    for phrase in phrases {
        if !user_phrases.contains(phrase) {
            continue;
        }
        let chars = phrase_char_count(phrase);
        if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&chars) {
            continue;
        }
        if exact_len_guard.is_some_and(|guard| guard != chars) {
            continue;
        }
        if !desired.iter().any(|seen| seen == phrase) {
            desired.push(phrase.clone());
        }
        if desired.len() >= front_limit {
            break;
        }
    }
    if desired.is_empty() {
        return;
    }

    let anchor = ranked
        .iter()
        .take_while(|item| is_user_hotword_front_anchor(item))
        .count();
    let desired_order = desired
        .iter()
        .enumerate()
        .map(|(idx, phrase)| (phrase.as_str(), idx))
        .collect::<HashMap<_, _>>();
    let mut leading = Vec::new();
    let mut promoted = Vec::new();
    let mut rest = Vec::new();
    for (idx, item) in std::mem::take(ranked).into_iter().enumerate() {
        if idx < anchor {
            leading.push(item);
        } else if desired_order.contains_key(item.phrase.as_str()) {
            promoted.push(item);
        } else {
            rest.push(item);
        }
    }
    promoted.sort_by_key(|item| {
        desired_order
            .get(item.phrase.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    ranked.extend(leading);
    ranked.extend(promoted);
    ranked.extend(rest);
}

fn is_user_hotword_front_anchor(item: &RankedCandidate) -> bool {
    item.meta.pinned
        || matches!(
            item.meta.source,
            CandidateSource::Direct
                | CandidateSource::Shortcut
                | CandidateSource::DateTime
                | CandidateSource::UMode
        )
}

pub(super) fn infer_uniform_variant_char_count(
    variants: &[(Vec<String>, Option<String>)],
) -> Option<usize> {
    let mut expected = None;
    for (syls, tail) in variants.iter().take(MAX_PARSE_VARIANTS) {
        let count = syls.len() + usize::from(tail.is_some());
        if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&count) {
            return None;
        }
        match expected {
            None => expected = Some(count),
            Some(prev) if prev == count => {}
            Some(_) => return None,
        }
    }
    expected
}

pub(super) fn infer_uniform_variant_char_count_detailed(
    variants: &[crate::segment::ParseVariant],
) -> Option<usize> {
    let mut expected = None;
    for variant in variants.iter().take(MAX_PARSE_VARIANTS) {
        let count = variant.syllables.len() + usize::from(variant.tail.is_some());
        if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&count) {
            return None;
        }
        match expected {
            None => expected = Some(count),
            Some(prev) if prev == count => {}
            Some(_) => return None,
        }
    }
    expected
}

pub(super) fn infer_exact_complete_short_parse_intent(
    variants: &[crate::segment::ParseVariant],
) -> Option<usize> {
    variants
        .iter()
        .find(|variant| {
            variant.source == ParseVariantSource::Exact
                && variant.tail.is_none()
                && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&variant.syllables.len())
        })
        .or_else(|| {
            variants.iter().find(|variant| {
                variant.source == ParseVariantSource::AlternateSplit
                    && variant.tail.is_none()
                    && (2..=SHORT_HOTWORD_MAX_CHARS).contains(&variant.syllables.len())
            })
        })
        .map(|variant| variant.syllables.len())
}

pub(super) fn correction_variant_source_penalty(
    source: ParseVariantSource,
    compact_len: usize,
) -> f64 {
    match source {
        ParseVariantSource::KeyboardTypo => {
            if compact_len <= 4 {
                18.0
            } else {
                0.0
            }
        }
        ParseVariantSource::PhoneticTypo => 24.0,
        ParseVariantSource::SyllableConfusion => 48.0,
        _ => 0.0,
    }
}

pub(super) fn correction_meta_for_parse_variant(
    raw: &str,
    variant: &crate::segment::ParseVariant,
) -> Option<String> {
    let compact_in = compact_lookup_key(raw);
    match variant.source {
        ParseVariantSource::KeyboardTypo => {
            variant.corrected_input.as_ref().and_then(|corrected| {
                if corrected == &compact_in {
                    None
                } else {
                    Some(format!(
                        "\u{7ea0}\u{9519}: {raw}->{corrected}\ttypo=1\tcorrection=keyboard\tcorrected_reading={corrected}"
                    ))
                }
            })
        }
        ParseVariantSource::SyllableConfusion => {
            variant.corrected_input.as_ref().and_then(|corrected| {
                if corrected == &compact_in {
                    None
                } else {
                    Some(format!(
                        "\u{97f3}\u{8fd1}: {raw}->{corrected}\ttypo=1\tcorrection=syllable_confusion\tcorrected_reading={corrected}"
                    ))
                }
            })
        }
        ParseVariantSource::PhoneticTypo => {
            variant.corrected_input.as_ref().and_then(|corrected| {
                if corrected == &compact_in {
                    None
                } else {
                    Some(format!(
                        "\u{7ea0}\u{9519}: {raw}->{corrected}\ttypo=1\tcorrection=phonetic\tcorrected_reading={corrected}"
                    ))
                }
            })
        }
        _ => None,
    }
}

#[inline]
pub(super) fn correction_style_kind(meta: &CandidateMeta) -> Option<&'static str> {
    if meta.match_kind == CandidateMatchKind::Correction
        || meta.source == CandidateSource::Correction
    {
        if meta.display_text().is_some_and(|text| {
            text.split('\t')
                .any(|token| token.trim() == "correction=syllable_confusion")
                || text.starts_with("\u{97f3}\u{8fd1}")
        }) {
            return Some("near");
        }
        return Some("typo");
    }
    let meta = meta.display_text()?;
    if meta
        .split('\t')
        .any(|token| matches!(token.trim(), "typo=1" | "typo"))
    {
        return Some("typo");
    }
    if meta.starts_with(TYPO_CANDIDATE_META) {
        return Some("typo");
    }
    if meta == TYPO_CANDIDATE_META || meta.starts_with("纠错") {
        return Some("typo");
    }
    if meta.starts_with("音近") {
        return Some("near");
    }
    if meta.starts_with("纠错") {
        Some("typo")
    } else if meta.starts_with("音近") {
        Some("near")
    } else {
        None
    }
}

/// 按纠错策略限制纠错类候选的位置和数量，避免它们干扰确定性输入。
pub(super) fn correction_rank_priority(meta: &CandidateMeta) -> u8 {
    match correction_style_kind(meta) {
        Some("typo") => 2,
        Some("near") => 1,
        _ => 0,
    }
}

pub(super) fn limit_typo_corrections_first_page(
    ranked: &mut Vec<RankedCandidate>,
    prefs: CorrectionPrefs,
    suppress_correction: bool,
) {
    if !prefs.enabled || suppress_correction {
        ranked.retain(|item| correction_style_kind(&item.meta).is_none());
        return;
    }
    let mut out: Vec<RankedCandidate> = Vec::with_capacity(ranked.len());
    let mut corrections: Vec<RankedCandidate> = Vec::new();
    for item in std::mem::take(ranked) {
        if correction_style_kind(&item.meta).is_some() {
            corrections.push(item);
        } else {
            out.push(item);
        }
    }

    corrections.sort_by(|a, b| {
        correction_rank_priority(&b.meta)
            .cmp(&correction_rank_priority(&a.meta))
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.phrase.cmp(&b.phrase))
    });

    match prefs.level {
        CorrectionLevel::Light => {
            if out.is_empty() {
                out.extend(corrections.into_iter().take(1));
            }
        }
        CorrectionLevel::Medium => {
            if let Some(correction) = corrections.into_iter().next() {
                let insert_at = if out.is_empty() { 0 } else { out.len().min(2) };
                out.insert(insert_at, correction);
            }
        }
        CorrectionLevel::Strong => {
            let insert_at = if out.is_empty() { 0 } else { 1.min(out.len()) };
            for (offset, correction) in corrections.into_iter().take(1).enumerate() {
                out.insert((insert_at + offset).min(out.len()), correction);
            }
        }
    }
    *ranked = out;
}

pub(super) fn ensure_strong_typo_correction_in_top_three(
    ranked: &mut Vec<RankedCandidate>,
    prefs: CorrectionPrefs,
    suppress_correction: bool,
) {
    if !prefs.enabled
        || prefs.level == CorrectionLevel::Light
        || ranked.len() <= 3
        || suppress_correction
    {
        return;
    }
    let Some((idx, _)) = ranked
        .iter()
        .enumerate()
        .filter(|(_, item)| correction_style_kind(&item.meta) == Some("typo"))
        .max_by(|(_, a), (_, b)| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.phrase.cmp(&a.phrase))
        })
    else {
        return;
    };
    if idx < 3 {
        return;
    }
    let item = ranked.remove(idx);
    ranked.insert(2.min(ranked.len()), item);
}

pub(super) fn drop_single_char_corrections_for_two_char_intent(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
) {
    if intent_char_count != Some(2) {
        return;
    }
    let has_two_char_non_correction = ranked.iter().any(|item| {
        phrase_char_count(&item.phrase) == 2 && correction_style_kind(&item.meta).is_none()
    });
    if !has_two_char_non_correction {
        return;
    }
    ranked.retain(|item| {
        !(phrase_char_count(&item.phrase) == 1 && correction_style_kind(&item.meta).is_some())
    });
}

pub(super) fn ensure_strong_two_char_correction_before_expansion(
    ranked: &mut Vec<RankedCandidate>,
    prefs: CorrectionPrefs,
    suppress_correction: bool,
) {
    if !prefs.enabled
        || prefs.level != CorrectionLevel::Strong
        || suppress_correction
        || ranked.len() <= 1
    {
        return;
    }
    if ranked
        .first()
        .is_some_and(|item| phrase_char_count(&item.phrase) <= 2)
    {
        return;
    }

    let leading_score = ranked[0].score;
    let search_len = ranked.len().min(TSF_PAGE_SIZE);
    let Some(pos) = ranked.iter().take(search_len).position(|item| {
        phrase_char_count(&item.phrase) == 2
            && item.score >= leading_score
            && correction_style_kind(&item.meta) == Some("typo")
    }) else {
        return;
    };
    if pos == 0 {
        return;
    }

    let item = ranked.remove(pos);
    ranked.insert(0, item);
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SmallInputCandidatePolicy {
    intent_chars: usize,
    expansion_chars: usize,
    expansion_limit: usize,
    page_size: usize,
    reserve_tail_slots: usize,
    reserve_single_slots: usize,
    exact_front_limit: usize,
    shorter_front_slots: usize,
}

pub(super) fn small_input_candidate_policy(
    intent_chars: usize,
    page_size: usize,
) -> Option<SmallInputCandidatePolicy> {
    match intent_chars {
        2 => Some(SmallInputCandidatePolicy {
            intent_chars,
            expansion_chars: intent_chars + 1,
            expansion_limit: SMALL_INPUT_EXPANSION_LIMIT,
            page_size: page_size.clamp(3, TSF_PAGE_SIZE),
            reserve_tail_slots: SMALL_INPUT_EXPANSION_LIMIT,
            reserve_single_slots: 1,
            exact_front_limit: SHORT_INPUT_EXACT_FRONT_LIMIT_TWO,
            shorter_front_slots: 1,
        }),
        3 => Some(SmallInputCandidatePolicy {
            intent_chars,
            expansion_chars: intent_chars + 1,
            expansion_limit: SMALL_INPUT_THREE_CHAR_EXPANSION_LIMIT,
            page_size: page_size.clamp(3, TSF_PAGE_SIZE),
            reserve_tail_slots: SMALL_INPUT_THREE_CHAR_EXPANSION_LIMIT,
            reserve_single_slots: SMALL_INPUT_SINGLE_CHAR_FRONT_SLOTS,
            exact_front_limit: SHORT_INPUT_EXACT_FRONT_LIMIT_THREE,
            shorter_front_slots: 2,
        }),
        4..=8 => Some(SmallInputCandidatePolicy {
            intent_chars,
            expansion_chars: intent_chars + 1,
            expansion_limit: 0,
            page_size: page_size.clamp(3, TSF_PAGE_SIZE),
            reserve_tail_slots: 0,
            reserve_single_slots: SMALL_INPUT_SINGLE_CHAR_FRONT_SLOTS,
            exact_front_limit: SHORT_INPUT_EXACT_FRONT_LIMIT_FOUR,
            shorter_front_slots: 1,
        }),
        _ => None,
    }
}

pub(super) fn apply_small_input_candidate_policy(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    let page_size = candidate_prefs::get_effective_candidate_page_size();
    arrange_small_input_candidates(ranked, intent_chars, page_size);
}

pub(super) fn limit_short_input_first_page_density(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
    lexicon: Option<&AbbrevLexicon>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    if !(2..=4).contains(&intent_chars) || ranked.len() <= 1 {
        return;
    }

    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    let Some(policy) = small_input_candidate_policy(intent_chars, page_size) else {
        return;
    };
    let exact_limit = policy.exact_front_limit;
    let short_front_slots = policy.shorter_front_slots;

    let mut exact_front = Vec::new();
    let mut exact_later = Vec::new();
    let mut short_front = Vec::new();
    let mut rest = Vec::new();
    let mut low_freq_exact_front = 0usize;

    for item in std::mem::take(ranked) {
        let len = phrase_char_count(&item.phrase);
        if len == intent_chars {
            let strong = candidate_has_user_priority(&item)
                || lexicon
                    .map(|lex| {
                        lex.phrase_frequency(&item.phrase) >= SHORT_INPUT_EXACT_FRONT_FREQ_MIN
                    })
                    .unwrap_or(false);
            let can_enter_front = exact_front.len() < exact_limit
                && (strong || low_freq_exact_front < SHORT_INPUT_LOW_FREQ_EXACT_FRONT_LIMIT);
            if can_enter_front {
                if !strong {
                    low_freq_exact_front += 1;
                }
                exact_front.push(item);
            } else {
                exact_later.push(item);
            }
        } else if len > 0 && len < intent_chars && short_front.len() < short_front_slots {
            short_front.push(item);
        } else {
            rest.push(item);
        }
    }

    ranked.reserve(
        exact_front
            .len()
            .saturating_add(exact_later.len())
            .saturating_add(short_front.len())
            .saturating_add(rest.len()),
    );
    ranked.extend(exact_front);
    ranked.extend(short_front);

    let first_page_fill = page_size.saturating_sub(ranked.len()).min(rest.len());
    ranked.extend(rest.drain(..first_page_fill));
    ranked.extend(exact_later);
    ranked.extend(rest);
}

pub(super) fn apply_short_intent_final_candidate_density(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
    lexicon: Option<&AbbrevLexicon>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    if !(2..=4).contains(&intent_chars) || ranked.len() <= 1 {
        return;
    }

    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    let Some(policy) = small_input_candidate_policy(intent_chars, page_size) else {
        return;
    };
    let expansion_chars = policy.expansion_chars;
    let expansion_limit = match intent_chars {
        2 => SHORT_INTENT_TOTAL_EXPANSION_LIMIT,
        3 => SHORT_INTENT_THREE_CHAR_EXPANSION_LIMIT,
        _ => policy.expansion_limit,
    };
    let exact_front_limit = policy.exact_front_limit;

    let mut exact_front = Vec::new();
    let mut exact_later = Vec::new();
    let mut near_shorter = Vec::new();
    let mut far_shorter = Vec::new();
    let mut expansion = Vec::new();
    let mut low_freq_exact_front = 0usize;

    for item in std::mem::take(ranked) {
        let len = phrase_char_count(&item.phrase);
        if len == intent_chars {
            let strong = candidate_has_user_priority(&item)
                || lexicon
                    .map(|lex| {
                        lex.phrase_frequency(&item.phrase) >= SHORT_INPUT_EXACT_FRONT_FREQ_MIN
                    })
                    .unwrap_or(false);
            let can_enter_front = exact_front.len() < exact_front_limit
                && (strong || low_freq_exact_front < SHORT_INPUT_LOW_FREQ_EXACT_FRONT_LIMIT);
            if can_enter_front {
                if !strong {
                    low_freq_exact_front += 1;
                }
                exact_front.push(item);
            } else {
                exact_later.push(item);
            }
        } else if len > 0 && len < intent_chars {
            if len + 1 == intent_chars {
                near_shorter.push(item);
            } else {
                far_shorter.push(item);
            }
        } else if len == expansion_chars && expansion.len() < expansion_limit {
            expansion.push(item);
        }
    }

    let mut shorter = Vec::with_capacity(near_shorter.len() + far_shorter.len());
    shorter.extend(near_shorter);
    shorter.extend(far_shorter);

    ranked.reserve(
        exact_front
            .len()
            .saturating_add(shorter.len())
            .saturating_add(exact_later.len())
            .saturating_add(expansion.len()),
    );
    ranked.extend(exact_front);
    let short_front_cap = page_size
        .saturating_sub(ranked.len())
        .saturating_sub(expansion.len())
        .min(shorter.len());
    ranked.extend(shorter.drain(..short_front_cap));
    ranked.extend(exact_later);
    ranked.extend(expansion);
    ranked.extend(shorter);
    if intent_chars == 3 {
        if lexicon.is_some() {
            arrange_three_char_intent_page_density_with_lexicon(ranked, page_size, lexicon);
        } else {
            arrange_three_char_intent_page_density(ranked, page_size);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThreeCharDensityConfidence {
    Ambiguous,
    Confident,
    Locked,
}

#[derive(Clone, Copy, Debug)]
struct ThreeCharDensityPolicy {
    first_exact_cap: usize,
    later_exact_cap: usize,
    first_two_char_quota: usize,
    later_two_char_quota: usize,
}

const THREE_CHAR_CONFIDENT_SCORE_MARGIN: f64 = 18.0;
const THREE_CHAR_COMPETITIVE_SCORE_MARGIN: f64 = 6.0;
pub(super) const THREE_CHAR_CONFIDENT_FRONT_MIN: usize = 5;
pub(super) const THREE_CHAR_CONFIDENT_FRONT_MAX: usize = 7;

fn three_char_density_policy(
    exact: &[RankedCandidate],
    two_char: &[RankedCandidate],
    single_char: &[RankedCandidate],
    rest: &[RankedCandidate],
    lexicon: Option<&AbbrevLexicon>,
) -> ThreeCharDensityPolicy {
    let locked = exact.iter().any(|item| {
        candidate_has_user_priority(item)
            || (item.meta.match_kind == CandidateMatchKind::FullPinyin
                && !matches!(item.meta.source_layer, LexiconLayer::Ext | LexiconLayer::En))
    });
    let strong_frequency_count = exact
        .iter()
        .take(THREE_CHAR_CONFIDENT_FRONT_MAX)
        .filter(|item| {
            item.meta.source_layer == LexiconLayer::Core
                || lexicon.is_some_and(|lex| {
                    lex.phrase_frequency(&item.phrase) >= SHORT_INPUT_EXACT_FRONT_FREQ_MIN
                })
        })
        .count();

    let best_exact_score = exact
        .iter()
        .filter_map(|item| item.score.is_finite().then_some(item.score))
        .reduce(f64::max);
    let best_alternative_score = two_char
        .iter()
        .chain(single_char)
        .chain(rest)
        .filter_map(|item| item.score.is_finite().then_some(item.score))
        .reduce(f64::max);
    let clear_score_margin = match (best_exact_score, best_alternative_score) {
        (Some(exact_score), Some(alternative_score)) => {
            exact_score - alternative_score >= THREE_CHAR_CONFIDENT_SCORE_MARGIN
        }
        (Some(_), None) => true,
        _ => false,
    };
    let competitive_exact_count = match best_alternative_score {
        Some(alternative_score) => exact
            .iter()
            .take(THREE_CHAR_CONFIDENT_FRONT_MAX)
            .filter(|item| {
                item.score.is_finite()
                    && item.score - alternative_score >= THREE_CHAR_COMPETITIVE_SCORE_MARGIN
            })
            .count(),
        None => exact.len().min(THREE_CHAR_CONFIDENT_FRONT_MAX),
    };

    let confidence = if locked {
        ThreeCharDensityConfidence::Locked
    } else if clear_score_margin || strong_frequency_count > 0 {
        ThreeCharDensityConfidence::Confident
    } else {
        ThreeCharDensityConfidence::Ambiguous
    };
    match confidence {
        ThreeCharDensityConfidence::Locked => ThreeCharDensityPolicy {
            first_exact_cap: THREE_CHAR_CONFIDENT_FRONT_MAX,
            later_exact_cap: THREE_CHAR_CONFIDENT_FRONT_MAX,
            first_two_char_quota: 1,
            later_two_char_quota: 1,
        },
        ThreeCharDensityConfidence::Confident => {
            let evidence_count = competitive_exact_count.max(
                THREE_CHAR_CONFIDENT_FRONT_MIN
                    .saturating_add(strong_frequency_count.saturating_sub(1)),
            );
            let first_exact_cap = evidence_count.clamp(
                THREE_CHAR_CONFIDENT_FRONT_MIN,
                THREE_CHAR_CONFIDENT_FRONT_MAX,
            );
            ThreeCharDensityPolicy {
                first_exact_cap,
                later_exact_cap: first_exact_cap.max(6),
                first_two_char_quota: 1,
                later_two_char_quota: 1,
            }
        }
        ThreeCharDensityConfidence::Ambiguous => ThreeCharDensityPolicy {
            first_exact_cap: 3,
            later_exact_cap: 4,
            first_two_char_quota: 3,
            later_two_char_quota: 2,
        },
    }
}

/// Three-letter abbreviations can recall many exact three-character phrases.
/// Interleave shorter candidates page by page so later pages remain useful for
/// composing words, while allowing a confident exact intent to occupy more of
/// the first page.
pub(super) fn arrange_three_char_intent_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    arrange_three_char_intent_page_density_with_lexicon(ranked, page_size, None);
}

fn arrange_three_char_intent_page_density_with_lexicon(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
    lexicon: Option<&AbbrevLexicon>,
) {
    if ranked.len() <= 1 {
        return;
    }
    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let mut exact = Vec::new();
    let mut two_char = Vec::new();
    let mut single_char = Vec::new();
    let mut rest = Vec::new();
    for item in std::mem::take(ranked) {
        match phrase_char_count(&item.phrase) {
            3 => exact.push(item),
            2 => two_char.push(item),
            1 => single_char.push(item),
            _ => rest.push(item),
        }
    }

    let policy = three_char_density_policy(&exact, &two_char, &single_char, &rest, lexicon);

    let total = exact.len() + two_char.len() + single_char.len() + rest.len();
    ranked.reserve(total);
    let mut page_index = 0usize;
    while ranked.len() < total {
        let before = ranked.len();
        let page_start = page_index.saturating_mul(page_size);
        let page_end = (page_index + 1).saturating_mul(page_size).min(total);
        let page_capacity = page_end.saturating_sub(page_start);
        let single_quota = usize::from(!single_char.is_empty()).min(page_capacity);
        let non_single_capacity = page_capacity.saturating_sub(single_quota);
        let exact_cap = (if page_index == 0 {
            policy.first_exact_cap
        } else {
            policy.later_exact_cap
        })
        .min(non_single_capacity);
        let exact_quota = exact_cap.min(exact.len());
        let two_quota = (if page_index == 0 {
            policy.first_two_char_quota
        } else {
            policy.later_two_char_quota
        })
        .min(non_single_capacity.saturating_sub(exact_quota));

        drain_front(&mut exact, ranked, exact_quota);
        drain_front(&mut two_char, ranked, two_quota);
        drain_front(&mut single_char, ranked, single_quota);

        while ranked.len() < page_end {
            let page_exact_count = ranked[page_start.min(ranked.len())..]
                .iter()
                .filter(|item| phrase_char_count(&item.phrase) == 3)
                .count();
            let future_exact_cap = policy.later_exact_cap.max(1);
            let future_pages = exact.len().div_ceil(future_exact_cap);
            let has_surplus_two =
                two_char.len() > future_pages.saturating_mul(policy.later_two_char_quota);
            let has_surplus_single = single_char.len() > future_pages;
            let added = (page_exact_count < exact_cap && drain_one(&mut exact, ranked))
                || drain_one(&mut rest, ranked)
                || (has_surplus_two && drain_one(&mut two_char, ranked))
                || (has_surplus_single && drain_one(&mut single_char, ranked))
                || drain_one(&mut exact, ranked)
                || drain_one(&mut two_char, ranked)
                || drain_one(&mut single_char, ranked);
            if added {
                continue;
            }
            break;
        }
        if ranked.len() == before {
            break;
        }
        page_index += 1;
    }
}

fn drain_front(source: &mut Vec<RankedCandidate>, target: &mut Vec<RankedCandidate>, count: usize) {
    let take = count.min(source.len());
    target.extend(source.drain(..take));
}

fn drain_one(source: &mut Vec<RankedCandidate>, target: &mut Vec<RankedCandidate>) -> bool {
    if source.is_empty() {
        return false;
    }
    target.push(source.remove(0));
    true
}

pub(super) fn apply_two_char_intent_page_density(ranked: &mut Vec<RankedCandidate>) {
    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    arrange_two_char_intent_page_density(ranked, page_size);
}

pub(super) fn apply_two_char_intent_minimum_page_density(ranked: &mut Vec<RankedCandidate>) {
    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    arrange_two_char_intent_minimum_page_density(ranked, page_size);
}

pub(super) fn arrange_two_char_intent_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    if ranked.len() <= 1 {
        return;
    }

    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let two_char_count = ranked
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
        .count();
    let mut two_char = Vec::new();
    let mut single_char = Vec::new();
    let mut rest = Vec::new();

    for item in std::mem::take(ranked) {
        match phrase_char_count(&item.phrase) {
            2 => two_char.push(item),
            1 => single_char.push(item),
            _ => rest.push(item),
        }
    }

    if two_char_count == 0 {
        ranked.extend(single_char);
        ranked.extend(rest);
        return;
    }

    let total_len = two_char.len() + single_char.len() + rest.len();
    ranked.reserve(total_len);

    let mut two_iter = two_char.into_iter();
    let mut single_iter = single_char.into_iter();
    let front_two_slots = page_size.saturating_sub(1).max(1);
    let reserve_pages = TWO_CHAR_INTENT_SINGLE_RESERVE_PAGES
        .min(single_iter.len())
        .min(two_iter.len().div_ceil(front_two_slots));

    for _ in 0..reserve_pages {
        let page_start = ranked.len();
        for _ in 0..front_two_slots {
            if let Some(item) = two_iter.next() {
                ranked.push(item);
            }
        }
        // When there are fewer than four exact two-character candidates,
        // fill the remaining visible slots with single-character building
        // blocks. Ordinary three-character predictions stay behind the page
        // instead of leaking into it merely because the exact group is short.
        while ranked.len().saturating_sub(page_start) < page_size {
            let Some(item) = single_iter.next() else {
                break;
            };
            ranked.push(item);
        }
    }

    ranked.extend(two_iter);
    ranked.extend(rest);
    ranked.extend(single_iter);
}

pub(super) fn arrange_two_char_intent_minimum_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    if ranked.len() <= 1 {
        return;
    }

    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let two_char_count = ranked
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
        .count();

    let mut two_char = Vec::new();
    let mut single_char = Vec::new();
    let mut rest = Vec::new();

    for item in std::mem::take(ranked) {
        match phrase_char_count(&item.phrase) {
            2 => two_char.push(item),
            1 => single_char.push(item),
            _ => rest.push(item),
        }
    }

    if two_char_count == 0 {
        ranked.extend(single_char);
        ranked.extend(rest);
        return;
    }

    // Jianpin/mixed input is the less certain route, so keep at least two
    // exact two-character candidates. When the lookup has a sufficiently
    // rich exact group, use four in the default five-column page; this keeps
    // high-confidence short-word input from being diluted by singles while
    // preserving the old minimum for ambiguous collisions.
    let min_two_char_per_page = if two_char_count >= 4 {
        4usize.min(page_size).min(two_char_count)
    } else {
        2usize.min(page_size).min(two_char_count)
    };

    let total_len = two_char.len() + single_char.len() + rest.len();
    ranked.reserve(total_len);

    let mut two_iter = two_char.into_iter();
    let mut single_iter = single_char.into_iter();
    let mut pending_two = two_iter.len();

    while pending_two > 0 {
        let page_start = ranked.len();
        let mut take_two = min_two_char_per_page.min(pending_two);
        let leftover_after_min = pending_two.saturating_sub(take_two);
        if leftover_after_min > 0 && leftover_after_min < min_two_char_per_page {
            take_two = (take_two + leftover_after_min)
                .min(page_size)
                .min(pending_two);
        }

        for _ in 0..take_two {
            if let Some(item) = two_iter.next() {
                ranked.push(item);
                pending_two -= 1;
            }
        }

        while ranked.len().saturating_sub(page_start) < page_size {
            let Some(item) = single_iter.next() else {
                break;
            };
            ranked.push(item);
        }
        while ranked.len().saturating_sub(page_start) < page_size && pending_two > 0 {
            let Some(item) = two_iter.next() else {
                break;
            };
            ranked.push(item);
            pending_two -= 1;
        }
    }

    // The first pages are already filled with exact words and composition
    // singles. Keep ordinary predictions immediately after those pages so
    // they remain reachable instead of falling behind a very large single-
    // character tail and being truncated from the result entirely.
    ranked.extend(rest);
    ranked.extend(single_iter);
}

pub(super) fn choose_short_phrase_intent<const N: usize>(
    sources: [(Option<usize>, u8); N],
) -> Option<usize> {
    sources
        .into_iter()
        .filter_map(|(intent, confidence)| {
            intent
                .filter(|chars| (2..=SHORT_HOTWORD_MAX_CHARS).contains(chars))
                .map(|chars| (chars, confidence))
        })
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
        .map(|(chars, _)| chars)
}

pub(super) fn classify_input_intent(
    raw: &str,
    compact_key: &str,
    direct_input_shortcut: bool,
    english_word_input_enabled: bool,
    exact_single_syllable_input: bool,
    exact_complete_multi_input: bool,
    short_phrase_intent: Option<usize>,
    mixed_prefix_intent: bool,
) -> InputIntent {
    if direct_input_shortcut || should_preserve_ascii_input(raw) {
        return InputIntent::Direct;
    }
    if english_word_input_enabled
        && is_plain_ascii_word_input(raw, compact_key)
        && !exact_single_syllable_input
        && !exact_complete_multi_input
    {
        return InputIntent::English;
    }
    if exact_single_syllable_input {
        return InputIntent::SingleSyllable;
    }
    if exact_complete_multi_input {
        return InputIntent::FullPinyin;
    }
    if mixed_prefix_intent {
        return InputIntent::MixedPrefix;
    }
    if short_phrase_intent.is_some() {
        return InputIntent::ShortAbbrev;
    }
    InputIntent::Unknown
}

pub(super) fn is_plain_ascii_word_input(raw: &str, compact_key: &str) -> bool {
    let raw = raw.trim();
    !compact_key.is_empty()
        && raw.chars().count() == compact_key.chars().count()
        && raw.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub(super) fn intent_layer_score_adjustment(
    intent: InputIntent,
    compact_char_count: usize,
    meta: &CandidateMeta,
) -> f64 {
    match intent {
        InputIntent::English => {
            if meta.match_kind == CandidateMatchKind::English
                || meta.source == CandidateSource::English
                || meta.source_layer == LexiconLayer::En
            {
                96.0
            } else {
                0.0
            }
        }
        InputIntent::FullPinyin => {
            let match_adjustment = match meta.match_kind {
                CandidateMatchKind::FullPinyin => 36.0,
                CandidateMatchKind::ShortAbbrev => -20.0,
                CandidateMatchKind::MixedPrefix => -12.0,
                CandidateMatchKind::Correction => -28.0,
                _ => 0.0,
            };
            match_adjustment
                + match meta.source_layer {
                    LexiconLayer::Ext => -FULL_PINYIN_EXT_GATE_PENALTY,
                    LexiconLayer::Large => -6.0,
                    LexiconLayer::En => -220.0,
                    _ => 0.0,
                }
        }
        InputIntent::MixedPrefix => match meta.match_kind {
            CandidateMatchKind::MixedPrefix => {
                if compact_char_count >= 4 {
                    128.0
                } else {
                    32.0
                }
            }
            CandidateMatchKind::FullPinyin if compact_char_count >= 4 => 72.0,
            CandidateMatchKind::ShortAbbrev
                if compact_char_count >= 4 && meta.source_layer == LexiconLayer::Ext =>
            {
                -12.0
            }
            _ => 0.0,
        },
        InputIntent::ShortAbbrev => match meta.match_kind {
            CandidateMatchKind::ShortAbbrev => 18.0,
            CandidateMatchKind::MixedPrefix => 4.0,
            CandidateMatchKind::Correction => -18.0,
            _ => 0.0,
        },
        _ => 0.0,
    }
}

pub(super) fn mixed_prefix_core_phrase_bonus(
    intent: InputIntent,
    phrase: &str,
    meta: &CandidateMeta,
) -> f64 {
    if intent != InputIntent::MixedPrefix
        || meta.match_kind != CandidateMatchKind::MixedPrefix
        || phrase_char_count(phrase) < 3
    {
        return 0.0;
    }
    match meta.source_layer {
        LexiconLayer::Core | LexiconLayer::Base => 48.0,
        _ => 0.0,
    }
}

pub(super) fn exact_full_pinyin_base_bonus(
    phrase: &str,
    exact_full_pinyin_phrases: &HashSet<String>,
    meta: &CandidateMeta,
) -> f64 {
    if !exact_full_pinyin_phrases.contains(phrase) {
        return 0.0;
    }
    let layer_adjustment = match meta.source_layer {
        LexiconLayer::Core | LexiconLayer::Base => PRIMARY_LEXICON_LAYER_BONUS,
        // Exact full-pinyin is explicit intent. Cancel the broad ext-layer
        // gate here; the smaller general ext penalty still applies later.
        LexiconLayer::Ext => FULL_PINYIN_EXT_GATE_PENALTY,
        LexiconLayer::En => -220.0,
        _ => 0.0,
    };
    FULL_PINYIN_EXACT_LENGTH_BONUS + layer_adjustment
}

pub(super) fn primary_lexicon_score_adjustment(
    phrase: &str,
    meta: &CandidateMeta,
    lexicon: Option<&AbbrevLexicon>,
    intent: InputIntent,
) -> f64 {
    let char_count = phrase_char_count(phrase);
    let freq = lexicon.map(|lex| lex.phrase_frequency(phrase)).unwrap_or(0);
    let primary = matches!(meta.source_layer, LexiconLayer::Core | LexiconLayer::Base);
    let mut score = match meta.source_layer {
        LexiconLayer::Core => PRIMARY_LEXICON_LAYER_BONUS,
        LexiconLayer::Base => PRIMARY_LEXICON_LAYER_BONUS * 0.75,
        LexiconLayer::Ext => -EXT_LEXICON_LAYER_PENALTY,
        LexiconLayer::Large => -LARGE_LEXICON_LAYER_PENALTY,
        LexiconLayer::En => -220.0,
        LexiconLayer::Unknown => 0.0,
    };

    if primary && (2..=4).contains(&char_count) {
        score += PRIMARY_SHORT_WORD_FREQ_BONUS * short_phrase_priority_score(freq, char_count);
    }
    if primary && char_count == 1 && intent == InputIntent::SingleSyllable {
        score += PRIMARY_SINGLE_CHAR_FREQ_BONUS * short_phrase_priority_score(freq, 1);
    }
    if meta.source_layer == LexiconLayer::Ext
        && (2..=3).contains(&char_count)
        && matches!(intent, InputIntent::FullPinyin | InputIntent::ShortAbbrev)
    {
        score -= EXT_SHORT_WORD_GATE_PENALTY;
    }
    score
}

pub(super) fn top1_confidence_priority(
    item: &RankedCandidate,
    exact_full_pinyin_phrases: &HashSet<String>,
    exact_syllable_count: Option<usize>,
    input_intent: InputIntent,
) -> i32 {
    if item.meta.pinned {
        return 50;
    }
    if candidate_has_user_priority(item) {
        return 40;
    }
    if input_intent == InputIntent::English
        && (item.meta.match_kind == CandidateMatchKind::English
            || item.meta.source == CandidateSource::English
            || item.meta.source_layer == LexiconLayer::En)
    {
        return 45;
    }
    if exact_full_pinyin_phrases.contains(&item.phrase)
        && exact_syllable_count.is_some_and(|count| phrase_char_count(&item.phrase) == count)
        && item.meta.source_layer != LexiconLayer::Ext
        && item.meta.source_layer != LexiconLayer::En
    {
        return 32;
    }
    match item.meta.match_kind {
        CandidateMatchKind::FullPinyin
            if item.meta.source_layer != LexiconLayer::Ext
                && item.meta.source_layer != LexiconLayer::En =>
        {
            24
        }
        CandidateMatchKind::MixedPrefix => 18,
        CandidateMatchKind::English => 4,
        _ if item.meta.source_layer == LexiconLayer::Ext => 6,
        _ => 12,
    }
}

pub(super) fn stabilize_top1_by_confidence(
    ranked: &mut Vec<RankedCandidate>,
    exact_full_pinyin_phrases: &HashSet<String>,
    exact_syllable_count: Option<usize>,
    input_intent: InputIntent,
) {
    if ranked.len() <= 1 {
        return;
    }
    let top_score = ranked[0].score;
    let top_priority = top1_confidence_priority(
        &ranked[0],
        exact_full_pinyin_phrases,
        exact_syllable_count,
        input_intent,
    );
    let mut best: Option<(usize, i32)> = None;
    for (idx, item) in ranked.iter().enumerate().skip(1).take(5) {
        if top_score - item.score > TOP1_CONFIDENCE_GAP {
            continue;
        }
        let priority = top1_confidence_priority(
            item,
            exact_full_pinyin_phrases,
            exact_syllable_count,
            input_intent,
        );
        if priority <= top_priority {
            continue;
        }
        if best
            .map(|(_, best_priority)| priority > best_priority)
            .unwrap_or(true)
        {
            best = Some((idx, priority));
        }
    }
    if let Some((idx, _)) = best {
        let item = ranked.remove(idx);
        ranked.insert(0, item);
    }
}

pub(super) fn demote_corrections_below_priority_hits(
    ranked: &mut Vec<RankedCandidate>,
    exact_full_pinyin_phrases: &HashSet<String>,
    exact_syllable_count: Option<usize>,
) {
    if ranked.len() <= 1 {
        return;
    }
    if !ranked.iter().any(|item| {
        candidate_blocks_correction_front(item, exact_full_pinyin_phrases, exact_syllable_count)
    }) {
        return;
    }

    let mut non_corrections = Vec::with_capacity(ranked.len());
    let mut corrections = Vec::new();
    for item in std::mem::take(ranked) {
        if correction_style_kind(&item.meta).is_some() {
            corrections.push(item);
        } else {
            non_corrections.push(item);
        }
    }
    if corrections.is_empty() {
        *ranked = non_corrections;
        return;
    }

    let Some(last_priority_idx) = non_corrections
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            candidate_blocks_correction_front(item, exact_full_pinyin_phrases, exact_syllable_count)
                .then_some(idx)
        })
        .max()
    else {
        non_corrections.extend(corrections);
        *ranked = non_corrections;
        return;
    };

    let insert_at = last_priority_idx + 1;
    let mut out = Vec::with_capacity(non_corrections.len() + corrections.len());
    out.extend(non_corrections.drain(..insert_at));
    out.extend(corrections);
    out.extend(non_corrections);
    *ranked = out;
}

pub(super) fn ensure_typo_correction_visible_after_priority_hits(
    ranked: &mut Vec<RankedCandidate>,
    prefs: CorrectionPrefs,
    suppress_correction: bool,
    exact_full_pinyin_phrases: &HashSet<String>,
    exact_syllable_count: Option<usize>,
) {
    if !prefs.enabled
        || prefs.level == CorrectionLevel::Light
        || ranked.len() <= 3
        || suppress_correction
    {
        return;
    }

    let Some((idx, _)) = ranked
        .iter()
        .enumerate()
        .filter(|(_, item)| correction_style_kind(&item.meta) == Some("typo"))
        .max_by(|(_, a), (_, b)| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.phrase.cmp(&a.phrase))
        })
    else {
        return;
    };
    if idx < 3 {
        return;
    }

    let last_priority_idx = ranked
        .iter()
        .take(3)
        .enumerate()
        .filter_map(|(front_idx, item)| {
            (correction_style_kind(&item.meta).is_none()
                && candidate_blocks_correction_front(
                    item,
                    exact_full_pinyin_phrases,
                    exact_syllable_count,
                ))
            .then_some(front_idx)
        })
        .max();
    if last_priority_idx.is_some_and(|front_idx| front_idx >= 2) {
        return;
    }

    let item = ranked.remove(idx);
    let insert_at = last_priority_idx
        .map(|front_idx| (front_idx + 1).max(2))
        .unwrap_or(2)
        .min(ranked.len());
    ranked.insert(insert_at, item);
}

pub(super) fn limit_english_candidates_for_non_english_intent(
    ranked: &mut Vec<RankedCandidate>,
    input_intent: InputIntent,
) {
    if input_intent == InputIntent::English || ranked.len() <= 2 {
        return;
    }
    let mut non_english = Vec::with_capacity(ranked.len());
    let mut english = Vec::new();
    for item in std::mem::take(ranked) {
        if is_english_word_candidate(&item) {
            english.push(item);
        } else {
            non_english.push(item);
        }
    }
    non_english.extend(english.into_iter().take(2));
    *ranked = non_english;
}

pub(super) fn drop_disabled_english_word_candidates(
    ranked: &mut Vec<RankedCandidate>,
    english_word_input_enabled: bool,
    phrase_lexicon: Option<&AbbrevLexicon>,
) {
    if english_word_input_enabled {
        return;
    }
    ranked.retain(|item| {
        !is_english_word_candidate(item)
            && !is_english_lexicon_phrase_candidate(item, phrase_lexicon)
    });
}

pub(super) fn is_english_word_candidate(item: &RankedCandidate) -> bool {
    item.meta.match_kind == CandidateMatchKind::English
        || item.meta.source == CandidateSource::English
        || item.meta.source_layer == LexiconLayer::En
}

pub(super) fn is_english_lexicon_phrase_candidate(
    item: &RankedCandidate,
    phrase_lexicon: Option<&AbbrevLexicon>,
) -> bool {
    if item.meta.pinned
        || matches!(
            item.meta.source,
            CandidateSource::Direct | CandidateSource::Shortcut | CandidateSource::User
        )
    {
        return false;
    }
    phrase_lexicon.is_some_and(|lex| lex.phrase_layer(&item.phrase) == LexiconLayer::En)
}

pub(super) fn candidate_blocks_correction_front(
    item: &RankedCandidate,
    exact_full_pinyin_phrases: &HashSet<String>,
    exact_syllable_count: Option<usize>,
) -> bool {
    if item.meta.pinned || candidate_has_user_priority(item) {
        return true;
    }
    if item.meta.match_kind != CandidateMatchKind::FullPinyin
        && item.meta.source != CandidateSource::Pinyin
    {
        return false;
    }
    exact_full_pinyin_phrases.contains(&item.phrase)
        && exact_syllable_count.is_some_and(|count| phrase_char_count(&item.phrase) == count)
        && item.meta.source_layer != LexiconLayer::Ext
        && item.meta.source_layer != LexiconLayer::En
}

pub(super) fn promote_high_score_exact_short_front(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&intent_chars) || ranked.len() <= 3 {
        return;
    }
    let front_limit = 2.min(ranked.len());
    for target_idx in 0..front_limit {
        let target_score = ranked[target_idx].score;
        let search_limit = ranked.len().min(TSF_PAGE_SIZE + 3);
        let Some(best_idx) = (target_idx + 1..search_limit)
            .filter(|idx| {
                phrase_char_count(&ranked[*idx].phrase) == intent_chars
                    && correction_style_kind(&ranked[*idx].meta).is_none()
                    && ranked[*idx].score > target_score
            })
            .max_by(|a, b| {
                ranked[*a]
                    .score
                    .partial_cmp(&ranked[*b].score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        else {
            continue;
        };
        let item = ranked.remove(best_idx);
        ranked.insert(target_idx, item);
    }
}

pub(super) fn ensure_exact_short_intent_before_expansion(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    if !(2..=SHORT_HOTWORD_MAX_CHARS).contains(&intent_chars) || ranked.len() <= 1 {
        return;
    }
    if ranked
        .first()
        .is_some_and(|item| phrase_char_count(&item.phrase) <= intent_chars)
    {
        return;
    }
    let Some(pos) = ranked.iter().position(|item| {
        phrase_char_count(&item.phrase) == intent_chars
            && correction_style_kind(&item.meta).is_none()
    }) else {
        return;
    };
    let item = ranked.remove(pos);
    ranked.insert(0, item);
}

pub(super) fn promote_preserved_short_intent_expansions(
    ranked: &mut Vec<RankedCandidate>,
    intent_char_count: Option<usize>,
    preserved_phrases: &[String],
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    if !(2..=3).contains(&intent_chars) || preserved_phrases.is_empty() {
        return;
    }

    let expansion_chars = intent_chars + 1;
    let mut phrases = preserved_phrases
        .iter()
        .filter(|phrase| phrase_char_count(phrase) == expansion_chars)
        .cloned()
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        return;
    }
    dedup_preserved_phrase_order(&mut phrases);
    promote_preserved_candidates(ranked, &phrases);
}

pub(super) fn promote_short_intent_abbrev_prefix_expansions(
    ranked: &mut Vec<RankedCandidate>,
    compact_key: &str,
    intent_char_count: Option<usize>,
    lexicon: Option<&AbbrevLexicon>,
) {
    let Some(intent_chars) = intent_char_count else {
        return;
    };
    let Some(lexicon) = lexicon else {
        return;
    };
    if !(2..=3).contains(&intent_chars) || compact_key.len() <= intent_chars {
        return;
    }
    if !compact_key.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return;
    }

    let expansion_chars = intent_chars + 1;
    let mut phrases = Vec::new();
    for prefix_len in (expansion_chars..compact_key.len()).rev() {
        let prefix = &compact_key[..prefix_len];
        let Some(entries) = lexicon.lookup(prefix) else {
            continue;
        };
        phrases.extend(
            entries
                .iter()
                .filter(|entry| phrase_char_count(&entry.phrase) == expansion_chars)
                .take(SHORT_INTENT_TOTAL_EXPANSION_LIMIT)
                .map(|entry| entry.phrase.clone()),
        );
        if !phrases.is_empty() {
            break;
        }
    }
    if phrases.is_empty() {
        return;
    }
    dedup_preserved_phrase_order(&mut phrases);
    promote_preserved_candidates(ranked, &phrases);
}

pub(super) fn candidate_has_user_priority(item: &RankedCandidate) -> bool {
    item.meta.source == CandidateSource::User
        || item.meta.pinned
        || item.meta.display_text().is_some_and(|meta| {
            meta.split('\t')
                .any(|token| token == USER_CANDIDATE_META || token == MIXED_USER_CANDIDATE_META)
        })
}

/// Put common system single characters in the exact order of the dedicated
/// 8,105-character frequency list while keeping pinned/learned candidates in
/// front. Missing rows are injected from the independent index so the general
/// lexicon's per-key Top-K limit cannot remove a valid hot single character.
pub(super) fn promote_common_single_chars_after_user_priority(
    ranked: &mut Vec<RankedCandidate>,
    common_order: &[char],
    exact_full_pinyin: bool,
) {
    if common_order.is_empty() {
        return;
    }

    let mut user_priority = Vec::new();
    let mut ordinary_by_phrase = HashMap::with_capacity(ranked.len());
    let mut ordinary_order = Vec::with_capacity(ranked.len());
    let mut user_phrases = HashSet::new();
    for item in std::mem::take(ranked) {
        if candidate_has_user_priority(&item) {
            user_phrases.insert(item.phrase.clone());
            user_priority.push(item);
        } else {
            ordinary_order.push(item.phrase.clone());
            ordinary_by_phrase.insert(item.phrase.clone(), item);
        }
    }

    let common_capacity = TSF_MAX_CANDIDATES.saturating_sub(user_priority.len());
    let top_score = user_priority
        .first()
        .map(|item| item.score - 1.0)
        .or_else(|| {
            ordinary_by_phrase
                .values()
                .map(|item| item.score)
                .reduce(f64::max)
        })
        .unwrap_or(400.0);
    let meta_label = if exact_full_pinyin {
        PINYIN_CANDIDATE_META
    } else {
        ABBREV_CANDIDATE_META
    };
    let mut common = Vec::with_capacity(common_capacity.min(common_order.len()));
    for (rank, ch) in common_order.iter().take(common_capacity).enumerate() {
        let phrase = ch.to_string();
        if user_phrases.contains(&phrase) {
            continue;
        }
        let item = ordinary_by_phrase
            .remove(&phrase)
            .unwrap_or_else(|| RankedCandidate {
                phrase,
                score: top_score - rank as f64 * 0.001,
                meta: CandidateMeta::legacy(meta_label).with_source_layer(LexiconLayer::Base),
            });
        common.push(item);
    }

    let mut rest = Vec::with_capacity(ordinary_by_phrase.len());
    for phrase in ordinary_order {
        if let Some(item) = ordinary_by_phrase.remove(&phrase) {
            rest.push(item);
        }
    }
    ranked.extend(user_priority);
    ranked.extend(common);
    ranked.extend(rest);
}

pub(super) fn radical_pinyin_candidates(input: &str) -> Vec<RankedCandidate> {
    let key = input
        .trim_start_matches('u')
        .replace(['\'', ' '], "")
        .to_ascii_lowercase();
    if key.is_empty() {
        return Vec::new();
    }
    let rows = RADICAL_PINYIN_CACHE.get_or_init(load_radical_pinyin_rows);
    let mut out = Vec::new();
    for (phrase, code) in rows {
        if code.replace('\'', "") == key {
            out.push(RankedCandidate {
                phrase: phrase.clone(),
                score: 145.0 - out.len() as f64 * 0.01,
                meta: CandidateMeta::legacy("U 拆字"),
            });
            if out.len() >= TSF_PAGE_SIZE {
                break;
            }
        }
    }
    out
}

pub(super) fn load_radical_pinyin_rows() -> Vec<(String, String)> {
    let Some(lex_dir) = default_phrase_lexicon_dir() else {
        return Vec::new();
    };
    let candidates = [
        lex_dir.join("radical_pinyin.txt"),
        lex_dir
            .parent()
            .map(|p| p.join("full").join("radical_pinyin.txt"))
            .unwrap_or_default(),
    ];
    for path in candidates {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let mut rows = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with("---")
                || line.contains(':')
            {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(phrase) = parts.next() else {
                continue;
            };
            let Some(code) = parts.next() else {
                continue;
            };
            if phrase.chars().count() == 1
                && code.chars().all(|c| c.is_ascii_alphabetic() || c == '\'')
            {
                rows.push((phrase.to_string(), code.to_ascii_lowercase()));
            }
        }
        if !rows.is_empty() {
            return rows;
        }
    }
    Vec::new()
}

pub(super) fn keep_only_single_char_candidates(ranked: &mut Vec<RankedCandidate>) {
    ranked.retain(|item| phrase_char_count(&item.phrase) == 1);
}

pub(super) fn apply_traditional_output(ranked: &mut Vec<RankedCandidate>) {
    let mut seen = HashSet::new();
    let mut converted = Vec::with_capacity(ranked.len());
    for mut item in ranked.drain(..) {
        if should_convert_candidate_to_traditional(&item) {
            item.phrase = crate::traditional::to_traditional(&item.phrase);
        }
        if seen.insert(item.phrase.clone()) {
            converted.push(item);
        }
    }
    *ranked = converted;
}

pub(super) fn should_convert_candidate_to_traditional(item: &RankedCandidate) -> bool {
    let text = item.phrase.trim();
    if text.is_empty() {
        return false;
    }
    if item
        .meta
        .display_text()
        .is_some_and(candidate_meta_requests_traditional_keep)
    {
        return false;
    }
    !looks_like_literal_candidate_that_should_not_be_converted(text)
}

pub(super) fn candidate_meta_requests_traditional_keep(meta: &str) -> bool {
    meta.split('\t').map(str::trim).any(|token| {
        matches!(
            token,
            "traditional=keep"
                | "traditional_keep=1"
                | "no_traditional=1"
                | "clipboard_quick=1"
                | "clipboard_quick"
                | "直输"
                | "英文"
                | "功能键"
        )
    })
}

pub(super) fn looks_like_literal_candidate_that_should_not_be_converted(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("clipboard://")
        || lower.starts_with("mailto:")
        || lower.contains("://")
        || lower.starts_with("www.")
    {
        return true;
    }
    if looks_like_email_literal(text)
        || looks_like_path_literal(text)
        || looks_like_code_literal(text)
    {
        return true;
    }
    false
}

pub(super) fn looks_like_email_literal(text: &str) -> bool {
    let trimmed = text.trim();
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.chars().any(char::is_whitespace)
}

pub(super) fn looks_like_path_literal(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.starts_with("\\\\") || trimmed.starts_with("~/") || trimmed.starts_with("~\\") {
        return true;
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    trimmed.contains('\\') && (trimmed.contains(':') || trimmed.contains('.'))
}

pub(super) fn looks_like_code_literal(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("```")
        || trimmed.contains("\n```")
        || (trimmed.contains("://") && trimmed.contains('[') && trimmed.contains(']'))
}

pub(super) fn short_intent_exact_bonus(intent_char_count: Option<usize>, phrase: &str) -> f64 {
    match intent_char_count {
        Some(3) if phrase_char_count(phrase) == 3 => THREE_CHAR_INTENT_EXACT_BOOST,
        Some(4) if phrase_char_count(phrase) == 4 => FOUR_CHAR_INTENT_EXACT_BOOST,
        _ => 0.0,
    }
}

pub(super) fn abbrev_length_penalty(
    short_ascii_input: bool,
    input_chars: usize,
    phrase: &str,
) -> f64 {
    if !short_ascii_input || input_chars == 0 {
        return 0.0;
    }
    let phrase_chars = phrase_char_count(phrase);
    if phrase_chars <= input_chars + 1 {
        return 0.0;
    }
    // Very short abbreviations are ambiguous; keep long expansions available,
    // but make them prove themselves via frequency, learning, or context.
    if input_chars <= 3 {
        ABBREV_OVERLONG_BASE_PENALTY
            + phrase_chars.saturating_sub(input_chars + 1) as f64 * ABBREV_OVERLONG_STEP_PENALTY
    } else {
        phrase_chars.saturating_sub(input_chars + 2) as f64 * (ABBREV_OVERLONG_STEP_PENALTY * 0.5)
    }
}

pub(super) fn explicit_separator_phrase_bonus(
    phrase: &str,
    separator_phrases: &HashSet<String>,
) -> f64 {
    if separator_phrases.contains(phrase) {
        EXPLICIT_SEPARATOR_PHRASE_BONUS
    } else {
        0.0
    }
}

pub(super) fn short_ascii_multi_char_rerank_discount(
    short_ascii_input: bool,
    input_chars: usize,
    phrase: &str,
) -> f64 {
    if !short_ascii_input || input_chars > 3 {
        return 0.0;
    }
    let phrase_chars = phrase_char_count(phrase);
    if phrase_chars < 4 {
        return 0.0;
    }
    SHORT_ASCII_MULTI_CHAR_RERANK_DISCOUNT
        + phrase_chars.saturating_sub(4) as f64 * (SHORT_ASCII_MULTI_CHAR_RERANK_DISCOUNT * 0.5)
}

pub(super) fn is_high_confidence_mixed_key(expansion: &MixedPinyinKeyExpansion) -> bool {
    (expansion.prefix_segments == 0 && expansion.initial_segments <= 1)
        || expansion.full_segments >= 2
        || (expansion.prefix_segments >= 2 && expansion.substantive_full_segments >= 1)
        || (expansion.prefix_segments >= 1
            && expansion.initial_segments >= 1
            && expansion.substantive_full_segments >= 1)
        || (expansion.segment_count == 4
            && expansion.leading_full
            && expansion.substantive_full_segments >= 1)
        || (expansion.segment_count >= 5 && expansion.full_segments >= 1)
}

pub(super) fn is_mixed_prefix_input_intent(expansion: &MixedPinyinKeyExpansion) -> bool {
    (expansion.leading_full || expansion.substantive_full_segments >= 2)
        && expansion.substantive_full_segments > 0
        && is_high_confidence_mixed_key(expansion)
}

pub(super) fn mixed_pinyin_path_adjustment(expansion: &MixedPinyinKeyExpansion) -> f64 {
    let strict_initials = if expansion.segment_count >= 5 {
        expansion.initial_segments.min(2)
    } else {
        expansion.initial_segments
    };
    let relaxed_initials = expansion.initial_segments.saturating_sub(strict_initials);
    let mut score = -((strict_initials as f64 * MIXED_PINYIN_INITIAL_SEGMENT_PENALTY)
        + (relaxed_initials as f64 * MIXED_PINYIN_LONG_INITIAL_SEGMENT_PENALTY)
        + (expansion.prefix_segments as f64 * MIXED_PINYIN_PREFIX_SEGMENT_PENALTY));
    if is_high_confidence_mixed_key(expansion) {
        score += MIXED_PINYIN_HIGH_CONFIDENCE_BONUS;
    } else {
        score -= MIXED_PINYIN_LOW_CONFIDENCE_PENALTY;
    }
    score
}

pub(super) fn mixed_pinyin_candidate_meta(expansion: &MixedPinyinKeyExpansion) -> &'static str {
    if is_high_confidence_mixed_key(expansion) {
        MIXED_HIGH_CANDIDATE_META
    } else {
        MIXED_LOW_CANDIDATE_META
    }
}

pub(super) fn arrange_small_input_candidates(
    ranked: &mut Vec<RankedCandidate>,
    intent_chars: usize,
    page_size: usize,
) {
    let Some(policy) = small_input_candidate_policy(intent_chars, page_size) else {
        return;
    };

    let mut exact = Vec::new();
    let mut single_chars = Vec::new();
    let mut near_shorter = Vec::new();
    let mut far_shorter = Vec::new();
    let mut expansion = Vec::new();

    for item in std::mem::take(ranked) {
        let len = phrase_char_count(&item.phrase);
        if len == policy.intent_chars {
            exact.push(item);
        } else if len == 1 && policy.reserve_single_slots > 0 {
            single_chars.push(item);
        } else if len + 1 == policy.intent_chars {
            near_shorter.push(item);
        } else if len < policy.intent_chars {
            far_shorter.push(item);
        } else if len == policy.expansion_chars && expansion.len() < policy.expansion_limit {
            expansion.push(item);
        }
    }

    let reserve = expansion
        .len()
        .min(policy.reserve_tail_slots)
        .min(policy.page_size.saturating_sub(1));
    let reserve_single = single_chars
        .len()
        .min(policy.reserve_single_slots)
        .min(policy.page_size.saturating_sub(reserve));
    let front_cap = policy.page_size.saturating_sub(reserve + reserve_single);

    let mut primary = Vec::with_capacity(exact.len() + near_shorter.len() + far_shorter.len());
    primary.extend(exact);
    primary.extend(near_shorter);
    primary.extend(far_shorter);

    let front_len = primary.len().min(front_cap);
    let mut primary_tail = primary.split_off(front_len);
    let mut single_tail = single_chars.split_off(reserve_single);

    ranked.extend(primary);
    ranked.extend(single_chars);
    ranked.extend(expansion);
    ranked.append(&mut primary_tail);
    ranked.append(&mut single_tail);
}

pub(super) fn is_exact_complete_single_syllable_input(
    raw: &str,
    compact_key: &str,
    syllable_set: &HashSet<String>,
) -> bool {
    compact_key.len() >= 2
        && raw.chars().all(|ch| ch.is_ascii_alphabetic())
        && raw.chars().count() == compact_key.chars().count()
        && syllable_set.contains(compact_key)
}

pub(super) fn has_exact_complete_single_syllable_variant(
    variants: &[ParseVariant],
    compact_key: &str,
) -> bool {
    compact_key.len() >= 2
        && variants.iter().any(|variant| {
            variant.source == ParseVariantSource::Exact
                && variant.tail.is_none()
                && variant.syllables.len() == 1
                && variant.syllables[0] == compact_key
        })
}

pub(super) fn is_pure_short_abbrev_initial_input(raw: &str, compact_key: &str) -> bool {
    (2..=8).contains(&compact_key.chars().count())
        && raw.chars().all(|ch| ch.is_ascii_alphabetic())
        && raw.chars().count() == compact_key.chars().count()
        && compact_key.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub(super) fn infer_short_abbrev_phrase_intent<'a, I>(
    raw: &str,
    compact_key: &str,
    phrases: I,
) -> Option<usize>
where
    I: IntoIterator<Item = &'a String>,
{
    if !is_pure_short_abbrev_initial_input(raw, compact_key) {
        return None;
    }
    let intent_chars = compact_key.chars().count();
    phrases
        .into_iter()
        .any(|phrase| phrase_char_count(phrase) == intent_chars)
        .then_some(intent_chars)
}

pub(super) fn should_preserve_ascii_input(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 512 || raw.chars().any(char::is_control) {
        return false;
    }
    let has_ascii_alnum = raw.chars().any(|ch| ch.is_ascii_alphanumeric());
    if !has_ascii_alnum {
        return false;
    }
    if !raw.is_ascii() {
        return true;
    }

    let lower = raw.to_ascii_lowercase();
    if lower.contains("://")
        || lower.starts_with("www.")
        || lower.starts_with("localhost:")
        || lower.starts_with('-')
    {
        return true;
    }
    if raw.contains('@') && raw.contains('.') {
        return true;
    }
    if raw.contains('\\') || raw.contains('/') {
        return true;
    }
    if raw.contains('_')
        || raw.contains("::")
        || raw.contains("->")
        || raw.contains("=>")
        || raw.contains("==")
        || raw.contains("!=")
        || raw.contains("<=")
        || raw.contains(">=")
        || raw.contains("&&")
        || raw.contains("||")
    {
        return true;
    }
    if raw.contains('.') && raw.chars().any(|ch| ch.is_ascii_alphabetic()) {
        return true;
    }
    if raw.chars().any(|ch| {
        matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | ';' | '`' | '"' | '$' | '=' | '+'
        )
    }) {
        return true;
    }
    if looks_like_camel_or_code_case(raw) {
        return true;
    }

    let first = raw.split_whitespace().next().unwrap_or_default();
    raw.split_whitespace().nth(1).is_some() && is_common_command_name(first)
}

pub(super) fn looks_like_camel_or_code_case(raw: &str) -> bool {
    let mut has_lower = false;
    let mut uppercase_count = 0usize;
    let mut previous_was_lower = false;
    for ch in raw.chars() {
        let is_lower = ch.is_ascii_lowercase();
        if ch.is_ascii_uppercase() {
            uppercase_count += 1;
            if previous_was_lower {
                return true;
            }
        }
        has_lower |= is_lower;
        previous_was_lower = is_lower;
    }
    has_lower && uppercase_count >= 2
}

pub(super) fn is_common_command_name(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "cargo"
            | "cd"
            | "cmd"
            | "curl"
            | "dir"
            | "docker"
            | "git"
            | "go"
            | "grep"
            | "java"
            | "kubectl"
            | "ls"
            | "make"
            | "node"
            | "npm"
            | "pip"
            | "pnpm"
            | "powershell"
            | "python"
            | "rg"
            | "rustc"
            | "ssh"
            | "sudo"
            | "uv"
            | "yarn"
    )
}

pub(super) fn merge_ascii_completion_candidates(
    raw: &str,
    compact_key: &str,
    merged: &mut MergedCandidateMap,
    english_word_input_enabled: bool,
) -> Vec<String> {
    let mut preserved = Vec::new();
    if !english_word_input_enabled {
        return preserved;
    }
    let raw = raw.trim();
    if raw.len() > 64 || compact_key.len() < 2 || !raw.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return preserved;
    }

    merge_candidate(merged, raw.to_string(), 58.0, Some(ASCII_COMPLETION_META));

    let mut seen = HashSet::new();
    seen.insert(compact_key.to_string());
    seen.insert(raw.to_ascii_lowercase());
    let mut added = 0usize;
    for word in crate::english_words::lookup_prefix(compact_key, 16) {
        let lower = word.to_ascii_lowercase();
        if !seen.insert(lower) {
            continue;
        }
        merge_candidate(
            merged,
            word.clone(),
            62.0 - added as f64 * 0.25,
            Some(ASCII_COMPLETION_META),
        );
        preserved.push(word);
        added += 1;
        if added >= 14 {
            break;
        }
    }
    // Keep the currently typed word itself as a first-page candidate even
    // when it is the only exact match in the English lexicon.  Previously
    // `python` was merged into the candidate map but not added to this
    // priority list because the prefix lookup returned only `python`, so
    // ordinary Chinese candidates could push it off the first page.
    if !preserved.iter().any(|item| item.eq_ignore_ascii_case(raw)) {
        preserved.push(raw.to_string());
    }
    preserved
}
