use super::*;

/// Three-letter abbreviations can recall many exact three-character phrases.
/// Keep the confidence layout on the first page, then retain the score order
/// so later pages do not become source-specific candidate blocks.
pub(in crate::core) fn arrange_three_char_intent_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    arrange_three_char_intent_page_density_with_lexicon(ranked, page_size, None);
}

pub(in crate::core) fn arrange_three_char_intent_page_density_with_lexicon(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
    lexicon: Option<&AbbrevLexicon>,
) {
    if ranked.len() <= 1 {
        return;
    }
    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let original = std::mem::take(ranked);
    let mut exact = Vec::new();
    let mut two_char = Vec::new();
    let mut single_char = Vec::new();
    let mut rest = Vec::new();
    for item in &original {
        match phrase_char_count(&item.phrase) {
            3 => exact.push(item.clone()),
            2 => two_char.push(item.clone()),
            1 => single_char.push(item.clone()),
            _ => rest.push(item.clone()),
        }
    }

    let policy = three_char_density_policy(&exact, &two_char, &single_char, &rest, lexicon);

    // Shape only the first page. Rebuilding every later page from separate
    // exact/two-character/single-character queues creates visible blocks of
    // low-frequency words before ordinary candidates. Keep the original score
    // order after the first page instead.
    let mut selected = std::collections::HashSet::new();
    let mut front = Vec::with_capacity(page_size);
    let exact_cap = policy.first_exact_cap.min(page_size);
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 3)
    {
        if front.len() >= exact_cap || !selected.insert(item.phrase.clone()) {
            continue;
        }
        front.push(item.clone());
    }
    let two_quota = policy
        .first_two_char_quota
        .min(page_size.saturating_sub(front.len()));
    let mut selected_two = 0usize;
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
    {
        if selected_two >= two_quota || !selected.insert(item.phrase.clone()) {
            continue;
        }
        selected_two += 1;
        front.push(item.clone());
    }
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 1)
    {
        if front.len() >= page_size || !selected.insert(item.phrase.clone()) {
            continue;
        }
        front.push(item.clone());
    }
    if front.len() < page_size {
        for item in &original {
            if selected.insert(item.phrase.clone()) {
                front.push(item.clone());
                if front.len() >= page_size {
                    break;
                }
            }
        }
    }
    ranked.extend(front);
    ranked.extend(
        original
            .into_iter()
            .filter(|item| !selected.contains(&item.phrase)),
    );
}

pub(in crate::core) fn apply_two_char_intent_page_density(ranked: &mut Vec<RankedCandidate>) {
    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    arrange_two_char_intent_page_density(ranked, page_size);
}

pub(in crate::core) fn apply_two_char_intent_minimum_page_density(
    ranked: &mut Vec<RankedCandidate>,
) {
    let page_size = candidate_prefs::get_effective_candidate_page_size().clamp(3, TSF_PAGE_SIZE);
    arrange_two_char_intent_minimum_page_density(ranked, page_size);
}

pub(in crate::core) fn arrange_two_char_intent_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    if ranked.len() <= 1 {
        return;
    }

    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let original = std::mem::take(ranked);
    let two_char_count = original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
        .count();
    if two_char_count == 0 {
        ranked.extend(original);
        return;
    }
    let front_two_slots = page_size.saturating_sub(1).max(1);
    let mut selected = std::collections::HashSet::new();
    let mut front = Vec::with_capacity(page_size);
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
    {
        if front.len() >= front_two_slots || !selected.insert(item.phrase.clone()) {
            continue;
        }
        front.push(item.clone());
    }
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 1)
    {
        if front.len() >= page_size || !selected.insert(item.phrase.clone()) {
            continue;
        }
        front.push(item.clone());
    }
    ranked.extend(front);
    ranked.extend(
        original
            .into_iter()
            .filter(|item| !selected.contains(&item.phrase)),
    );
}

pub(in crate::core) fn arrange_two_char_intent_minimum_page_density(
    ranked: &mut Vec<RankedCandidate>,
    page_size: usize,
) {
    if ranked.len() <= 1 {
        return;
    }

    let page_size = page_size.clamp(3, TSF_PAGE_SIZE);
    let original = std::mem::take(ranked);
    let two_char_count = original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
        .count();
    if two_char_count == 0 {
        ranked.extend(original);
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

    let mut selected = std::collections::HashSet::new();
    let mut front = Vec::with_capacity(page_size);
    let mut selected_two = 0usize;
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 2)
    {
        if selected_two >= min_two_char_per_page || !selected.insert(item.phrase.clone()) {
            continue;
        }
        selected_two += 1;
        front.push(item.clone());
    }
    for item in original
        .iter()
        .filter(|item| phrase_char_count(&item.phrase) == 1)
    {
        if front.len() >= page_size || !selected.insert(item.phrase.clone()) {
            continue;
        }
        front.push(item.clone());
    }
    ranked.extend(front);
    ranked.extend(
        original
            .into_iter()
            .filter(|item| !selected.contains(&item.phrase)),
    );
}
