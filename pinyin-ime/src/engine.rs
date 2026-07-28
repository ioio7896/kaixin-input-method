use crate::lm::Lm;
use std::collections::{HashMap, VecDeque};

#[derive(Clone)]
struct BeamItem {
    node: usize,
    last: char,
    score: f64,
}

struct DecodeNode {
    parent: Option<usize>,
    ch: char,
}

#[derive(Clone)]
struct CachedBeamItem {
    text: String,
    last: char,
    score: f64,
}

struct CachedDecodeState {
    syllables: Vec<String>,
    beam_size: usize,
    beam: Vec<CachedBeamItem>,
}

/// Small composition-local cache that can extend the beam produced by an
/// earlier syllable prefix. It deliberately stores only final beams, not every
/// expanded edge, keeping memory bounded while still making `nihao` ->
/// `nihaoshijie` incremental at syllable boundaries.
pub(crate) struct IncrementalDecodeCache {
    states: VecDeque<CachedDecodeState>,
    capacity: usize,
}

impl Default for IncrementalDecodeCache {
    fn default() -> Self {
        Self {
            states: VecDeque::new(),
            capacity: 48,
        }
    }
}

impl IncrementalDecodeCache {
    pub(crate) fn clear(&mut self) {
        self.states.clear();
    }

    pub(crate) fn decode(
        &mut self,
        dict: &HashMap<String, Vec<char>>,
        lm: &Lm,
        syllables: &[String],
        beam_size: usize,
        max_candidates: usize,
    ) -> Vec<(String, f64)> {
        self.decode_until(dict, lm, syllables, beam_size, max_candidates, || false)
            .0
    }

    pub(crate) fn decode_until(
        &mut self,
        dict: &HashMap<String, Vec<char>>,
        lm: &Lm,
        syllables: &[String],
        beam_size: usize,
        max_candidates: usize,
        mut should_stop: impl FnMut() -> bool,
    ) -> (Vec<(String, f64)>, bool) {
        if syllables.is_empty() || beam_size == 0 || max_candidates == 0 {
            return (Vec::new(), true);
        }
        if should_stop() {
            return (Vec::new(), false);
        }

        if syllables.len() == 1 {
            let Some(raw) = dict.get(&syllables[0]) else {
                return (Vec::new(), true);
            };
            let ranked = lm.rank_chars_by_unigram_with_limit(raw, max_candidates.max(beam_size));
            if should_stop() {
                return (Vec::new(), false);
            }
            let mut beam = ranked
                .iter()
                .take(beam_size)
                .map(|&ch| CachedBeamItem {
                    text: ch.to_string(),
                    last: ch,
                    score: lm.log_unigram(ch),
                })
                .collect::<Vec<_>>();
            truncate_cached_beam(&mut beam, beam_size);
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                beam,
            });
            return (
                ranked
                    .into_iter()
                    .take(max_candidates)
                    .map(|ch| (ch.to_string(), lm.log_unigram(ch)))
                    .collect(),
                true,
            );
        }

        let reusable = self
            .states
            .iter()
            .enumerate()
            .filter(|(_, state)| {
                state.beam_size == beam_size
                    && state.syllables.len() <= syllables.len()
                    && syllables.starts_with(&state.syllables)
            })
            .max_by_key(|(_, state)| state.syllables.len())
            .map(|(index, _)| index);
        let (prefix_len, mut beam) = if let Some(index) = reusable {
            let state = self
                .states
                .remove(index)
                .expect("cached decoder state exists");
            (state.syllables.len(), state.beam)
        } else {
            let (cold, complete) = decode_until_with_status(
                dict,
                lm,
                syllables,
                beam_size,
                max_candidates.max(beam_size),
                &mut should_stop,
            );
            if !complete {
                return (Vec::new(), false);
            }
            let beam = cold
                .iter()
                .take(beam_size)
                .filter_map(|(text, score)| {
                    text.chars().last().map(|last| CachedBeamItem {
                        text: text.clone(),
                        last,
                        score: *score,
                    })
                })
                .collect::<Vec<_>>();
            let result = cold.into_iter().take(max_candidates).collect();
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                beam,
            });
            return (result, true);
        };

        if prefix_len == syllables.len() {
            let result = beam_results(&beam, max_candidates);
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                beam,
            });
            return (result, true);
        }

        for syllable in syllables.iter().skip(prefix_len) {
            if should_stop() {
                return (Vec::new(), false);
            }
            let Some(raw) = dict.get(syllable) else {
                return (Vec::new(), true);
            };
            let choices = lm.rank_chars_by_unigram(raw);
            if choices.is_empty() {
                return (Vec::new(), true);
            }
            let mut next = Vec::with_capacity(beam.len().saturating_mul(choices.len()));
            for (item_index, item) in beam.iter().enumerate() {
                if item_index % 8 == 0 && should_stop() {
                    return (Vec::new(), false);
                }
                for &ch in &choices {
                    let mut text = item.text.clone();
                    text.push(ch);
                    next.push(CachedBeamItem {
                        text,
                        last: ch,
                        score: item.score + lm.log_bigram(item.last, ch),
                    });
                }
            }
            truncate_cached_beam(&mut next, beam_size);
            beam = next;
        }

        let result = beam_results(&beam, max_candidates);
        self.remember(CachedDecodeState {
            syllables: syllables.to_vec(),
            beam_size,
            beam,
        });
        (result, true)
    }

    fn remember(&mut self, state: CachedDecodeState) {
        self.states.push_front(state);
        while self.states.len() > self.capacity {
            self.states.pop_back();
        }
    }
}

fn truncate_cached_beam(items: &mut Vec<CachedBeamItem>, k: usize) {
    if items.len() > k {
        let nth = k - 1;
        items.select_nth_unstable_by(nth, |left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.text.cmp(&right.text))
        });
        items.truncate(k);
    }
    items.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.text.cmp(&right.text))
    });
}

fn beam_results(beam: &[CachedBeamItem], max_candidates: usize) -> Vec<(String, f64)> {
    let mut seen = std::collections::HashSet::new();
    beam.iter()
        .filter(|item| seen.insert(item.text.as_str()))
        .take(max_candidates)
        .map(|item| (item.text.clone(), item.score))
        .collect()
}

fn truncate_top_k_by_score(items: &mut Vec<BeamItem>, k: usize) {
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

    // 用 partial 选择替代全量 sort：先把前 k 个元素变成 top-k（无序），再对这 k 个排序。
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

/// 整句 beam search：音节序列 → 得分最高的若干整句。
pub fn decode(
    dict: &HashMap<String, Vec<char>>,
    lm: &Lm,
    syllables: &[String],
    beam_size: usize,
    max_candidates: usize,
) -> Vec<(String, f64)> {
    decode_until(dict, lm, syllables, beam_size, max_candidates, || false)
}

/// Cancellable beam decoder used by latency-bounded interactive stages.
pub fn decode_until(
    dict: &HashMap<String, Vec<char>>,
    lm: &Lm,
    syllables: &[String],
    beam_size: usize,
    max_candidates: usize,
    mut should_stop: impl FnMut() -> bool,
) -> Vec<(String, f64)> {
    decode_until_with_status(
        dict,
        lm,
        syllables,
        beam_size,
        max_candidates,
        &mut should_stop,
    )
    .0
}

fn decode_until_with_status(
    dict: &HashMap<String, Vec<char>>,
    lm: &Lm,
    syllables: &[String],
    beam_size: usize,
    max_candidates: usize,
    should_stop: &mut impl FnMut() -> bool,
) -> (Vec<(String, f64)>, bool) {
    if syllables.is_empty() {
        return (Vec::new(), true);
    }
    if should_stop() {
        return (Vec::new(), false);
    }

    if syllables.len() == 1 {
        let raw = dict.get(&syllables[0]).cloned().unwrap_or_default();
        if raw.is_empty() {
            return (Vec::new(), true);
        }
        let out = lm
            .rank_chars_by_unigram_with_limit(&raw, max_candidates)
            .into_iter()
            .map(|c| (c.to_string(), lm.log_unigram(c)))
            .collect();
        return if should_stop() {
            (Vec::new(), false)
        } else {
            (out, true)
        };
    }

    let mut ranked: Vec<Vec<char>> = Vec::with_capacity(syllables.len());
    for s in syllables {
        if should_stop() {
            return (Vec::new(), false);
        }
        let raw = dict.get(s).cloned().unwrap_or_default();
        if raw.is_empty() {
            return (Vec::new(), true);
        }
        ranked.push(lm.rank_chars_by_unigram(&raw));
    }

    // Keep paths in an arena.  Cloning a full `Vec<char>` for every beam edge is
    // quadratic in sentence length and dominates long mixed-pinyin lookups.
    let mut nodes: Vec<DecodeNode> = Vec::with_capacity(beam_size.saturating_mul(syllables.len()));
    let mut beam: Vec<BeamItem> = Vec::new();
    for &c in &ranked[0] {
        let node = nodes.len();
        nodes.push(DecodeNode {
            parent: None,
            ch: c,
        });
        beam.push(BeamItem {
            node,
            last: c,
            score: lm.log_unigram(c),
        });
    }
    truncate_top_k_by_score(&mut beam, beam_size);

    for choices in ranked.iter().skip(1) {
        if should_stop() {
            return (Vec::new(), false);
        }
        let mut next: Vec<BeamItem> = Vec::with_capacity(beam.len() * choices.len());
        for (item_index, item) in beam.iter().enumerate() {
            if item_index % 8 == 0 && should_stop() {
                return (Vec::new(), false);
            }
            for &c in choices {
                let sc = item.score + lm.log_bigram(item.last, c);
                let node = nodes.len();
                nodes.push(DecodeNode {
                    parent: Some(item.node),
                    ch: c,
                });
                next.push(BeamItem {
                    node,
                    last: c,
                    score: sc,
                });
            }
        }
        truncate_top_k_by_score(&mut next, beam_size);
        beam = next;
    }

    let mut results: Vec<(String, f64)> = beam
        .into_iter()
        .map(|b| {
            let mut chars = Vec::with_capacity(syllables.len());
            let mut node = Some(b.node);
            while let Some(index) = node {
                let entry = &nodes[index];
                chars.push(entry.ch);
                node = entry.parent;
            }
            chars.reverse();
            (chars.into_iter().collect::<String>(), b.score)
        })
        .collect();
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    results.dedup_by(|a, b| a.0 == b.0);
    results.truncate(max_candidates);
    if should_stop() {
        (Vec::new(), false)
    } else {
        (results, true)
    }
}

/// 与 UI 分页一致：最多保留若干整句候选。
pub const DEFAULT_MAX_CANDIDATES: usize = 256;

#[cfg(test)]
mod tests {
    use super::{decode, IncrementalDecodeCache};
    use crate::lm::Lm;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn single_syllable_decode_is_not_capped_by_beam_size() {
        let chars: Vec<char> = (0..80)
            .map(|idx| char::from_u32(0x4E00 + idx).expect("valid CJK char"))
            .collect();
        let vocab: HashSet<char> = chars.iter().copied().collect();
        let mut unigram = HashMap::new();
        for (idx, ch) in chars.iter().copied().enumerate() {
            unigram.insert(ch, 10_000usize - idx);
        }
        let lm = Lm::from_counts(vocab, unigram, HashMap::new());

        let mut dict = HashMap::new();
        dict.insert("shi".to_string(), chars);

        let results = decode(&dict, &lm, &["shi".to_string()], 48, 80);
        assert_eq!(results.len(), 80);
        assert_eq!(results[0].0.chars().count(), 1);
        assert_eq!(results[64].0.chars().count(), 1);
    }

    #[test]
    fn incremental_cache_extends_a_cached_syllable_prefix_without_changing_scores() {
        let chars = ['你', '尼', '好', '号', '世', '事'];
        let vocab: HashSet<char> = chars.into_iter().collect();
        let unigram = chars
            .into_iter()
            .enumerate()
            .map(|(index, ch)| (ch, 100usize - index))
            .collect();
        let lm = Lm::from_counts(vocab, unigram, HashMap::new());
        let dict = HashMap::from([
            ("ni".to_string(), vec!['你', '尼']),
            ("hao".to_string(), vec!['好', '号']),
            ("shi".to_string(), vec!['世', '事']),
        ]);
        let prefix = vec!["ni".to_string(), "hao".to_string()];
        let extended = vec!["ni".to_string(), "hao".to_string(), "shi".to_string()];
        let mut cache = IncrementalDecodeCache::default();
        let _ = cache.decode(&dict, &lm, &prefix, 8, 8);
        let incremental = cache.decode(&dict, &lm, &extended, 8, 8);
        let cold = decode(&dict, &lm, &extended, 8, 8);

        assert_eq!(incremental.len(), cold.len());
        for ((incremental_text, incremental_score), (cold_text, cold_score)) in
            incremental.iter().zip(&cold)
        {
            assert_eq!(incremental_text, cold_text);
            assert!((incremental_score - cold_score).abs() < 1e-9);
        }
        assert_eq!(cache.states.front().unwrap().syllables, extended);
    }
}
