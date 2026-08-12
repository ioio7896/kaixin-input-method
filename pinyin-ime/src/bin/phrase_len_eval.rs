use pinyin_ime::core::{PinyinEngine, MODE_JIANPIN, MODE_MIXED_PINYIN, TSF_PAGE_SIZE};
use pinyin_ime::segment::split_syllables_with_tail;
use pinyin_ime::thuocl::{load_dir_txt_with_profile_default_enabled, LexiconBuildProfile};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Default)]
struct Stats {
    cases: usize,
    top1: usize,
    top3: usize,
    top9: usize,
    missing: usize,
    weighted_cases: u128,
    weighted_top1: u128,
    weighted_top3: u128,
    weighted_top9: u128,
    commit_keys: usize,
    selectable_cases: usize,
    visible_cases: usize,
    first_visible_keys: usize,
    top1_changes: usize,
    stability_cases: usize,
}

#[derive(Clone)]
struct EvalCase {
    source: String,
    mode: &'static str,
    input: String,
    expected: String,
    freq: u64,
    layer: String,
}

#[derive(Clone, Copy, Default)]
struct IncrementalResult {
    first_visible: Option<usize>,
    best_commit_keys: Option<usize>,
    top1_changes: usize,
}

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let lexicon_dir = repo.join("lexicon");
    let mut limit = 500usize;
    let mut popular_limit = usize::MAX;
    let mut popular_three_limit = usize::MAX;
    let mut only_popular = false;
    let mut incremental = false;
    let default_popular_three = repo.join("tests").join("popular_three_char_cases.tsv");
    let default_popular = repo.join("tests").join("popular_four_char_cases.tsv");
    let mut popular_three_path = default_popular_three
        .is_file()
        .then_some(default_popular_three);
    let mut popular_path = default_popular.is_file().then_some(default_popular);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--limit" => {
                limit = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(limit);
            }
            "--popular-four-char" => popular_path = args.next().map(PathBuf::from),
            "--popular-three-char" => popular_three_path = args.next().map(PathBuf::from),
            "--popular-limit" => {
                popular_limit = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(popular_limit);
            }
            "--popular-three-limit" => {
                popular_three_limit = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(popular_three_limit);
            }
            "--only-popular" => {
                only_popular = true;
                popular_three_path = None;
            }
            "--only-popular-three" => {
                only_popular = true;
                popular_path = None;
            }
            "--only-popular-four" => {
                only_popular = true;
                popular_three_path = None;
            }
            "--incremental" => incremental = true,
            "--no-popular-three-char" => popular_three_path = None,
            "--no-popular-four-char" => popular_path = None,
            _ => {}
        }
    }
    let (lexicon, _) =
        load_dir_txt_with_profile_default_enabled(&lexicon_dir, LexiconBuildProfile::Standard)
            .expect("load lexicon");
    let samples = if only_popular {
        Vec::new()
    } else {
        [1usize, 2, 3, 4]
            .into_iter()
            .flat_map(|len| {
                lexicon.evaluation_samples_by_phrase_len(len, limit, if len == 1 { 3 } else { 1 })
            })
            .collect::<Vec<_>>()
    };

    let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(MODE_JIANPIN | MODE_MIXED_PINYIN);
    let mut incremental_engine = incremental.then(|| {
        let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir));
        engine.clear_user_lexicon_for_eval();
        engine.set_mode_flags(MODE_JIANPIN | MODE_MIXED_PINYIN);
        engine
    });

    let mut cases: Vec<EvalCase> = Vec::new();
    for (full, expected, freq, layer) in samples {
        let Ok((syllables, tail)) = split_syllables_with_tail(full.as_str(), engine.syllable_set())
        else {
            continue;
        };
        if tail.is_some() || syllables.len() != expected.chars().count() {
            continue;
        }
        append_cases(
            &mut cases,
            "lexicon",
            &syllables,
            &expected,
            freq,
            format!("{layer:?}"),
        );
    }

    if let Some(path) = popular_three_path {
        match load_popular_phrase_cases(&path, 3, engine.syllable_set()) {
            Ok(mut popular) => {
                popular.truncate(popular_three_limit);
                println!(
                    "POPULAR\tkind=three_char\tpath={}\tphrases={}",
                    path.display(),
                    popular.len()
                );
                for (category, phrase, syllables, freq) in popular {
                    append_cases(
                        &mut cases,
                        "popular_three_char",
                        &syllables,
                        &phrase,
                        freq,
                        category,
                    );
                }
            }
            Err(err) => eprintln!("WARN\tpopular_three_char={}\terror={err}", path.display()),
        }
    }

    if let Some(path) = popular_path {
        match load_popular_phrase_cases(&path, 4, engine.syllable_set()) {
            Ok(mut popular) => {
                popular.truncate(popular_limit);
                println!(
                    "POPULAR\tkind=four_char\tpath={}\tphrases={}",
                    path.display(),
                    popular.len()
                );
                for (category, phrase, syllables, freq) in popular {
                    append_cases(
                        &mut cases,
                        "popular_four_char",
                        &syllables,
                        &phrase,
                        freq,
                        category,
                    );
                }
            }
            Err(err) => eprintln!("WARN\tpopular_four_char={}\terror={err}", path.display()),
        }
    }

    let mut top1 = 0usize;
    let mut top3 = 0usize;
    let mut top9 = 0usize;
    let mut missing = 0usize;
    let mut weighted_cases = 0u128;
    let mut weighted_top1 = 0u128;
    let mut weighted_top3 = 0u128;
    let mut weighted_top9 = 0u128;
    let mut commit_keys = 0usize;
    let mut selectable_cases = 0usize;
    let mut visible_cases = 0usize;
    let mut first_visible_keys = 0usize;
    let mut top1_changes = 0usize;
    let mut stability_cases = 0usize;
    let mut grouped: BTreeMap<(String, String, usize, &'static str), Stats> = BTreeMap::new();
    for case in &cases {
        let incremental_result = incremental_engine
            .as_mut()
            .map(|engine| measure_incremental(engine, &case.input, &case.expected));
        // The interactive path may deliberately return a time-bounded first
        // batch. Repeating the same key consumes its budget-free continuation,
        // which is the stable result this offline ranking audit must score.
        engine.reset_learning_context();
        let _ = engine.lookup_full(&case.input);
        let (candidates, error) = engine.lookup_full(&case.input);
        let rank = candidates
            .iter()
            .position(|candidate| candidate.0 == case.expected)
            .map(|idx| idx + 1);
        let weight = u128::from(case.freq.max(1));
        top1 += usize::from(rank == Some(1));
        top3 += usize::from(rank.is_some_and(|rank| rank <= 3));
        top9 += usize::from(rank.is_some_and(|rank| rank <= 9));
        missing += usize::from(rank.is_none());
        weighted_cases += weight;
        weighted_top1 += weight * u128::from(rank == Some(1));
        weighted_top3 += weight * u128::from(rank.is_some_and(|rank| rank <= 3));
        weighted_top9 += weight * u128::from(rank.is_some_and(|rank| rank <= 9));
        let best_commit_keys = incremental_result
            .and_then(|result| result.best_commit_keys)
            .or_else(|| rank.map(|rank| estimated_commit_keys(&case.input, rank)));
        if let Some(keys) = best_commit_keys {
            commit_keys += keys;
            selectable_cases += 1;
        }
        if let Some(result) = incremental_result {
            if let Some(first_visible) = result.first_visible {
                visible_cases += 1;
                first_visible_keys += first_visible;
            }
            top1_changes += result.top1_changes;
            stability_cases += 1;
        }
        let stats = grouped
            .entry((
                case.source.clone(),
                case.layer.clone(),
                case.expected.chars().count(),
                case.mode,
            ))
            .or_default();
        stats.cases += 1;
        stats.top1 += usize::from(rank == Some(1));
        stats.top3 += usize::from(rank.is_some_and(|rank| rank <= 3));
        stats.top9 += usize::from(rank.is_some_and(|rank| rank <= 9));
        stats.missing += usize::from(rank.is_none());
        stats.weighted_cases += weight;
        stats.weighted_top1 += weight * u128::from(rank == Some(1));
        stats.weighted_top3 += weight * u128::from(rank.is_some_and(|rank| rank <= 3));
        stats.weighted_top9 += weight * u128::from(rank.is_some_and(|rank| rank <= 9));
        if let Some(keys) = best_commit_keys {
            stats.commit_keys += keys;
            stats.selectable_cases += 1;
        }
        if let Some(result) = incremental_result {
            if let Some(first_visible) = result.first_visible {
                stats.visible_cases += 1;
                stats.first_visible_keys += first_visible;
            }
            stats.top1_changes += result.top1_changes;
            stats.stability_cases += 1;
        }
        if let Some(rank) = rank.filter(|rank| *rank > 9) {
            println!(
                "TOP9_MISS\tsource={}\tmode={}\tlen={}\tinput={}\texpected={}\tfreq={}\tlayer={}\trank={}\terror={:?}",
                case.source,
                case.mode,
                case.expected.chars().count(),
                case.input,
                case.expected,
                case.freq,
                case.layer,
                rank,
                error
            );
        } else if rank.is_none() {
            println!(
                "UNRECALLED\tsource={}\tmode={}\tlen={}\tinput={}\texpected={}\tfreq={}\tlayer={}\terror={:?}",
                case.source,
                case.mode,
                case.expected.chars().count(),
                case.input,
                case.expected,
                case.freq,
                case.layer,
                error
            );
        }
    }
    let total = cases.len().max(1) as f64;
    for ((source, category, len, mode), stats) in grouped {
        let total = stats.cases.max(1) as f64;
        let weighted_total = stats.weighted_cases.max(1) as f64;
        let avg_commit_keys = format_optional_average(stats.commit_keys, stats.selectable_cases);
        let avg_first_visible =
            format_optional_average(stats.first_visible_keys, stats.visible_cases);
        let avg_top1_changes = format_optional_average(stats.top1_changes, stats.stability_cases);
        println!(
            "GROUP\tsource={}\tcategory={}\tlen={}\tmode={}\tcases={}\ttop1={:.1}%\ttop3={:.1}%\ttop9={:.1}%\twtop1={:.1}%\twtop3={:.1}%\twtop9={:.1}%\tunrecalled={}\tunrecalled_rate={:.1}%\tselectable={:.1}%\tavg_commit_keys_hit={}\tavg_first_visible_hit={}\tavg_top1_changes={}",
            source,
            category,
            len,
            mode,
            stats.cases,
            stats.top1 as f64 * 100.0 / total,
            stats.top3 as f64 * 100.0 / total,
            stats.top9 as f64 * 100.0 / total,
            stats.weighted_top1 as f64 * 100.0 / weighted_total,
            stats.weighted_top3 as f64 * 100.0 / weighted_total,
            stats.weighted_top9 as f64 * 100.0 / weighted_total,
            stats.missing,
            stats.missing as f64 * 100.0 / total,
            stats.selectable_cases as f64 * 100.0 / total,
            avg_commit_keys,
            avg_first_visible,
            avg_top1_changes,
        );
    }
    let weighted_total = weighted_cases.max(1) as f64;
    let avg_commit_keys = format_optional_average(commit_keys, selectable_cases);
    let avg_first_visible = format_optional_average(first_visible_keys, visible_cases);
    let avg_top1_changes = format_optional_average(top1_changes, stability_cases);
    println!(
        "SUMMARY\tcases={}\ttop1={:.1}%\ttop3={:.1}%\ttop9={:.1}%\twtop1={:.1}%\twtop3={:.1}%\twtop9={:.1}%\tunrecalled={}\tunrecalled_rate={:.1}%\tselectable={:.1}%\tavg_commit_keys_hit={}\tavg_first_visible_hit={}\tavg_top1_changes={}",
        cases.len(),
        top1 as f64 * 100.0 / total,
        top3 as f64 * 100.0 / total,
        top9 as f64 * 100.0 / total,
        weighted_top1 as f64 * 100.0 / weighted_total,
        weighted_top3 as f64 * 100.0 / weighted_total,
        weighted_top9 as f64 * 100.0 / weighted_total,
        missing,
        missing as f64 * 100.0 / total,
        selectable_cases as f64 * 100.0 / total,
        avg_commit_keys,
        avg_first_visible,
        avg_top1_changes,
    );
}

fn append_cases(
    out: &mut Vec<EvalCase>,
    source: &str,
    syllables: &[String],
    expected: &str,
    freq: u64,
    layer: String,
) {
    if syllables.is_empty() {
        return;
    }
    let mut variants = vec![("full", syllables.join(""))];
    if syllables.len() >= 2 {
        variants.push((
            "abbrev",
            syllables
                .iter()
                .filter_map(|syllable| syllable.chars().next())
                .collect(),
        ));
        variants.push((
            "mixed_first_full",
            render_mixed(syllables, |idx, _| idx == 0, false),
        ));
    }
    if syllables.len() == 3 {
        for (mode, mask) in [
            ("mixed_middle_full", 0b010usize),
            ("mixed_last_full", 0b100),
            ("mixed_first_two_full", 0b011),
            ("mixed_outer_full", 0b101),
            ("mixed_last_two_full", 0b110),
        ] {
            variants.push((
                mode,
                render_mixed(syllables, |idx, _| mask & (1usize << idx) != 0, false),
            ));
        }
        for (mode, prefix_idx) in [
            ("prefix_first", 0usize),
            ("prefix_middle", 1usize),
            ("prefix_last", 2usize),
        ] {
            variants.push((mode, render_initials_with_one_prefix(syllables, prefix_idx)));
        }
    }
    if syllables.len() == 4 {
        variants.push((
            "mixed_first_two_full",
            render_mixed(syllables, |idx, _| idx < 2, false),
        ));
        variants.push((
            "mixed_alternating",
            render_mixed(syllables, |idx, _| idx % 2 == 0, false),
        ));
        for prefix_idx in 0..syllables.len() {
            if syllables[prefix_idx].len() >= 3 {
                variants.push((
                    "mixed_prefix",
                    render_mixed(syllables, |idx, _| idx != prefix_idx, true),
                ));
            }
        }
    }

    variants.sort_by(|left, right| left.1.cmp(&right.1));
    variants.dedup_by(|left, right| left.1 == right.1);
    out.extend(variants.into_iter().map(|(mode, input)| EvalCase {
        source: source.to_string(),
        mode,
        input,
        expected: expected.to_string(),
        freq,
        layer: layer.clone(),
    }));
}

fn render_initials_with_one_prefix(syllables: &[String], prefix_idx: usize) -> String {
    let mut out = String::new();
    for (idx, syllable) in syllables.iter().enumerate() {
        if idx == prefix_idx {
            out.extend(syllable.chars().take(2));
        } else if let Some(initial) = syllable.chars().next() {
            out.push(initial);
        }
    }
    out
}

fn render_mixed(
    syllables: &[String],
    full_segment: impl Fn(usize, &str) -> bool,
    use_prefixes: bool,
) -> String {
    let mut out = String::new();
    for (idx, syllable) in syllables.iter().enumerate() {
        if full_segment(idx, syllable) {
            out.push_str(syllable);
        } else if use_prefixes && syllable.len() >= 3 {
            out.push_str(&syllable[..2]);
        } else if let Some(initial) = syllable.chars().next() {
            out.push(initial);
        }
    }
    out
}

fn load_popular_phrase_cases(
    path: &PathBuf,
    expected_len: usize,
    syllable_set: &std::collections::HashSet<String>,
) -> Result<Vec<(String, String, Vec<String>, u64)>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut out = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 4 {
            return Err(format!(
                "line {}: expected category, phrase, pinyin, freq",
                line_no + 1
            ));
        }
        let category = fields[0].trim().to_string();
        let phrase = fields[1].trim().to_string();
        let syllables = fields[2]
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let freq = fields[3]
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("line {}: invalid positive frequency", line_no + 1))?;
        if freq == 0 {
            return Err(format!("line {}: frequency must be positive", line_no + 1));
        }
        if phrase.chars().count() != expected_len
            || syllables.len() != expected_len
            || syllables.iter().any(|item| !syllable_set.contains(item))
        {
            return Err(format!(
                "line {}: invalid {expected_len}-char phrase or pinyin",
                line_no + 1,
            ));
        }
        out.push((category, phrase, syllables, freq));
    }
    Ok(out)
}

fn estimated_commit_keys(input: &str, rank: usize) -> usize {
    let page_downs = rank.saturating_sub(1) / TSF_PAGE_SIZE;
    input.chars().count() + page_downs + 1
}

fn format_optional_average(total: usize, cases: usize) -> String {
    if cases == 0 {
        "-".to_string()
    } else {
        format!("{:.2}", total as f64 / cases as f64)
    }
}

fn measure_incremental(
    engine: &mut PinyinEngine,
    input: &str,
    expected: &str,
) -> IncrementalResult {
    engine.reset_learning_context();
    let chars = input.chars().collect::<Vec<_>>();
    let mut result = IncrementalResult::default();
    let mut previous_top1: Option<String> = None;
    for prefix_len in 1..=chars.len() {
        let prefix = chars[..prefix_len].iter().collect::<String>();
        let (candidates, _) = engine.lookup_full(&prefix);
        if let Some(rank) = candidates
            .iter()
            .position(|candidate| candidate.0 == expected)
            .map(|index| index + 1)
        {
            let keys = estimated_commit_keys(&prefix, rank);
            result.best_commit_keys = Some(
                result
                    .best_commit_keys
                    .map_or(keys, |current| current.min(keys)),
            );
            if result.first_visible.is_none() && rank <= TSF_PAGE_SIZE {
                result.first_visible = Some(prefix_len);
            }
        }
        let top1 = candidates.first().map(|candidate| candidate.0.clone());
        if prefix_len > 1 && top1 != previous_top1 {
            result.top1_changes += 1;
        }
        previous_top1 = top1;
    }
    result
}
