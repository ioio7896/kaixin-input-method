use super::*;

pub(in crate::core) fn choose_short_phrase_intent<const N: usize>(
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

pub(in crate::core) fn classify_input_intent(
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

pub(in crate::core) fn is_plain_ascii_word_input(raw: &str, compact_key: &str) -> bool {
    let raw = raw.trim();
    !compact_key.is_empty()
        && raw.chars().count() == compact_key.chars().count()
        && raw.chars().all(|ch| ch.is_ascii_alphabetic())
}

pub(in crate::core) fn intent_layer_score_adjustment(
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
