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
    node: usize,
    last: char,
    score: f64,
}

struct CachedDecodeState {
    syllables: Vec<String>,
    beam_size: usize,
    nodes: Vec<DecodeNode>,
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
            let mut nodes = Vec::with_capacity(beam_size);
            let mut beam = Vec::with_capacity(beam_size);
            for &ch in ranked.iter().take(beam_size) {
                let node = nodes.len();
                nodes.push(DecodeNode { parent: None, ch });
                beam.push(CachedBeamItem {
                    node,
                    last: ch,
                    score: lm.log_unigram(ch),
                });
            }
            truncate_cached_beam(&mut beam, beam_size, &nodes);
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                nodes,
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
        let (prefix_len, mut nodes, mut beam) = if let Some(index) = reusable {
            let state = self
                .states
                .remove(index)
                .expect("cached decoder state exists");
            (state.syllables.len(), state.nodes, state.beam)
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
            let mut nodes = Vec::new();
            let beam = cold
                .iter()
                .take(beam_size)
                .filter_map(|(text, score)| {
                    let mut parent = None;
                    let mut last = None;
                    for ch in text.chars() {
                        let node = nodes.len();
                        nodes.push(DecodeNode { parent, ch });
                        parent = Some(node);
                        last = Some(ch);
                    }
                    last.map(|last| CachedBeamItem {
                        node: parent.expect("cached decode node exists"),
                        last,
                        score: *score,
                    })
                })
                .collect::<Vec<_>>();
            let result = cold.into_iter().take(max_candidates).collect();
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                nodes,
                beam,
            });
            return (result, true);
        };

        if prefix_len == syllables.len() {
            let result = beam_results(&nodes, &beam, max_candidates);
            self.remember(CachedDecodeState {
                syllables: syllables.to_vec(),
                beam_size,
                nodes,
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
                    let node = nodes.len();
                    nodes.push(DecodeNode {
                        parent: Some(item.node),
                        ch,
                    });
                    next.push(CachedBeamItem {
                        node,
                        last: ch,
                        score: item.score + lm.log_bigram(item.last, ch),
                    });
                }
            }
            truncate_cached_beam(&mut next, beam_size, &nodes);
            beam = next;
        }

        let result = beam_results(&nodes, &beam, max_candidates);
        self.remember(CachedDecodeState {
            syllables: syllables.to_vec(),
            beam_size,
            nodes,
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

fn truncate_cached_beam(items: &mut Vec<CachedBeamItem>, k: usize, nodes: &[DecodeNode]) {
    if items.len() > k {
        let nth = k - 1;
        items.select_nth_unstable_by(nth, |left, right| {
            right.score.total_cmp(&left.score).then_with(|| {
                materialize_cached_node(nodes, left.node)
                    .cmp(&materialize_cached_node(nodes, right.node))
            })
        });
        items.truncate(k);
    }
    items.sort_by(|left, right| {
        right.score.total_cmp(&left.score).then_with(|| {
            materialize_cached_node(nodes, left.node)
                .cmp(&materialize_cached_node(nodes, right.node))
        })
    });
}

fn beam_results(
    nodes: &[DecodeNode],
    beam: &[CachedBeamItem],
    max_candidates: usize,
) -> Vec<(String, f64)> {
    let mut seen = std::collections::HashSet::new();
    beam.iter()
        .map(|item| (materialize_cached_node(nodes, item.node), item.score))
        .filter(|(text, _)| seen.insert(text.clone()))
        .take(max_candidates)
        .collect()
}

fn materialize_cached_node(nodes: &[DecodeNode], node: usize) -> String {
    let mut chars = Vec::new();
    let mut current = Some(node);
    while let Some(index) = current {
        let entry = &nodes[index];
        chars.push(entry.ch);
        current = entry.parent;
    }
    chars.reverse();
    chars.into_iter().collect()
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
