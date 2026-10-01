use super::*;

impl PinyinEngine {
    // This is the only presentation exit for live and cached lookup routes.
    pub(super) fn finalize_candidate_presentation(&self, raw: &str, key: &str, ranked: &mut Vec<RankedCandidate>) {
        self.finalize_stable_word_candidates(raw, key, ranked);
        if let Some((intent, _)) = self.strict_lexicon_frequency_order_context(raw, key) {
            demote_cold_non_full_lexicon_candidates(ranked, intent, self.phrase_lexicon.as_ref());
        }
        let phrases = self.user_lexicon.lookup_input(key)
            .map(|entries| entries.iter().map(|entry| entry.phrase.clone()).collect::<Vec<_>>())
            .unwrap_or_default();
        promote_exact_user_hotwords_front(ranked, &phrases, None,
            user_hotword_prefs::get_user_hotword_prefs().front_limit);
        ranked.truncate(LOOKUP_FULL_MAX_CANDIDATES);
    }
}
