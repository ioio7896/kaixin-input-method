use crate::text_norm::strip_bom_str;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct Lm {
    vocab: HashSet<char>,
    v_size: f64,
    alpha: f64,
    u_total: f64,
    u_count: HashMap<char, usize>,
    /// Precomputed log-unigram for every observed char.  Avoids `ln()` in the
    /// hot decode path.
    u_logprob: HashMap<char, f64>,
    /// Observed bigram log-probabilities, precomputed at build time.
    bigram_logprob: HashMap<(char, char), f64>,
    /// Fallback log-bigram for unobserved (prev, c) pairs — depends only on
    /// `prev`, so one precomputed value per observed predecessor.
    bigram_unobserved_fallback: HashMap<char, f64>,
}

impl Lm {
    pub fn from_corpus(corpus: &str, vocab: &HashSet<char>) -> Self {
        let mut u_count: HashMap<char, usize> = HashMap::new();
        let mut b_count: HashMap<(char, char), usize> = HashMap::new();
        for line in corpus.lines() {
            let line = strip_bom_str(line);
            let chars: Vec<char> = line.chars().filter(|c| vocab.contains(c)).collect();
            for &c in &chars {
                *u_count.entry(c).or_insert(0) += 1;
            }
            for window in chars.windows(2) {
                *b_count.entry((window[0], window[1])).or_insert(0) += 1;
            }
        }
        Self::from_counts(vocab.clone(), u_count, b_count)
    }

    pub fn from_counts(
        vocab: HashSet<char>,
        u_count: HashMap<char, usize>,
        b_count: HashMap<(char, char), usize>,
    ) -> Self {
        let alpha = 0.01_f64;
        let v_size = vocab.len().max(1) as f64;
        let u_total: f64 = u_count.values().map(|&count| count as f64).sum();

        // Precompute unigram log-probabilities for every observed character.
        let denom = u_total + alpha * v_size;
        let u_logprob: HashMap<char, f64> = u_count
            .iter()
            .map(|(&c, &cnt)| {
                let prob = (cnt as f64 + alpha) / denom;
                (c, prob.ln())
            })
            .collect();

        // Precompute bigram log-probabilities for observed pairs.  Unobserved
        // pairs fall through to a per-prev fallback table.
        let mut bigram_logprob: HashMap<(char, char), f64> = HashMap::with_capacity(b_count.len());
        for (&(prev, c), &joint_cnt) in &b_count {
            let prev_cnt = *u_count.get(&prev).unwrap_or(&0) as f64 + alpha * v_size;
            let prob = (joint_cnt as f64 + alpha) / prev_cnt.max(1e-12);
            bigram_logprob.insert((prev, c), prob.ln());
        }

        // For unobserved (prev, c) pairs: P(c|prev) ≈ alpha / (count(prev) + alpha·V).
        let bigram_unobserved_fallback: HashMap<char, f64> = u_count
            .iter()
            .map(|(&prev, &cnt)| {
                let prev_cnt = cnt as f64 + alpha * v_size;
                (prev, (alpha / prev_cnt.max(1e-12)).ln())
            })
            .collect();

        Self {
            vocab,
            v_size,
            alpha,
            u_total,
            u_count,
            u_logprob,
            bigram_logprob,
            bigram_unobserved_fallback,
        }
    }

    #[inline]
    pub fn log_unigram(&self, c: char) -> f64 {
        if !self.vocab.contains(&c) {
            return -25.0;
        }
        self.u_logprob.get(&c).copied().unwrap_or_else(|| {
            // Character is in vocab but unobserved in corpus — fall back to
            // the Laplace-smoothed minimum.
            let prob = self.alpha / (self.u_total + self.alpha * self.v_size);
            prob.ln()
        })
    }

    #[inline]
    pub fn log_bigram(&self, prev: char, c: char) -> f64 {
        if !self.vocab.contains(&c) {
            return -25.0;
        }
        // Fast path: observed bigram → precomputed log-prob.
        if let Some(&logprob) = self.bigram_logprob.get(&(prev, c)) {
            return logprob;
        }
        // Fallback: unobserved pair → precomputed per-prev floor, or compute
        // once for unseen predecessors.
        if let Some(&fallback) = self.bigram_unobserved_fallback.get(&prev) {
            return fallback;
        }
        let prev_cnt = *self.u_count.get(&prev).unwrap_or(&0) as f64 + self.alpha * self.v_size;
        (self.alpha / prev_cnt.max(1e-12)).ln()
    }

    pub fn rank_chars_by_unigram_with_limit(&self, chars: &[char], limit: usize) -> Vec<char> {
        let mut ranked: Vec<char> = chars.to_vec();
        ranked.sort_by(|a, b| {
            self.log_unigram(*b)
                .partial_cmp(&self.log_unigram(*a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(limit);
        ranked
    }

    pub fn rank_chars_by_unigram(&self, chars: &[char]) -> Vec<char> {
        self.rank_chars_by_unigram_with_limit(chars, 64)
    }
}
