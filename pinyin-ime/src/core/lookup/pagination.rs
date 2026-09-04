use super::*;

/// Three-letter abbreviations can recall many exact three-character phrases.
/// Interleave shorter candidates page by page so later pages remain useful for
/// composing words, while allowing a confident exact intent to occupy more of
/// the first page.
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
    // 全部分页保持"整词在前、单字填空"的交错，而不是只保证前 3 页：
    // 第 4 页起二字词成块出现会破坏翻页体验的一致性。
    let reserve_pages = two_iter.len().div_ceil(front_two_slots);

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

pub(in crate::core) fn arrange_two_char_intent_minimum_page_density(
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
