use pinyin_ime::core::{
    PinyinEngine, LEARN_FLAG_COMPOSED_PHRASE, LEARN_FLAG_WEAK, MODE_JIANPIN, MODE_MIXED_PINYIN,
};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Metrics {
    lookups: usize,
    top1: usize,
    top3: usize,
    top9: usize,
    reciprocal_rank_sum: f64,
    page_turn_sum: usize,
    missing: usize,
}

impl Metrics {
    fn record(&mut self, rank: Option<usize>, page_size: usize) {
        self.lookups += 1;
        if let Some(rank) = rank {
            self.top1 += usize::from(rank == 1);
            self.top3 += usize::from(rank <= 3);
            self.top9 += usize::from(rank <= 9);
            self.reciprocal_rank_sum += 1.0 / rank as f64;
            // A first-page candidate needs no page turn.  This is deliberately
            // based on the configured visible page size rather than Top-9 so
            // vertical/compact candidate UIs are evaluated fairly.
            self.page_turn_sum += (rank - 1) / page_size.max(1);
        } else {
            self.missing += 1;
        }
    }

    fn top1_pct(&self) -> f64 {
        self.top1 as f64 * 100.0 / self.lookups.max(1) as f64
    }

    fn top3_pct(&self) -> f64 {
        self.top3 as f64 * 100.0 / self.lookups.max(1) as f64
    }

    fn mrr(&self) -> f64 {
        self.reciprocal_rank_sum / self.lookups.max(1) as f64
    }

    fn avg_page_turns(&self) -> f64 {
        self.page_turn_sum as f64 / self.lookups.max(1) as f64
    }
}

fn infer_bucket(reading: &str, explicit: Option<&str>) -> String {
    if let Some(bucket) = explicit.map(str::trim).filter(|bucket| !bucket.is_empty()) {
        return bucket.to_string();
    }
    if reading.contains([' ', '\'', '-']) {
        "full_pinyin".to_string()
    } else if reading.len() <= 4 && reading.chars().all(|ch| ch.is_ascii_alphabetic()) {
        "short_abbrev".to_string()
    } else if reading.chars().all(|ch| ch.is_ascii_alphabetic()) {
        "compact_pinyin".to_string()
    } else {
        "other".to_string()
    }
}

fn parse_flags(value: &str) -> u32 {
    value
        .split(['|', ','])
        .fold(0, |flags, part| match part.trim() {
            "weak" => flags | LEARN_FLAG_WEAK,
            "composed" => flags | LEARN_FLAG_COMPOSED_PHRASE,
            _ => flags,
        })
}

fn parse_usize(value: Option<&&str>, field: &str, line_no: usize) -> usize {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| panic!("line {line_no}: invalid {field}"))
}

fn usage() -> ! {
    eprintln!(
        "usage: learning_replay_eval [events.tsv] [--events path] [--lexicon dir] \
         [--min-top1 percent] [--min-top3 percent] [--min-mrr value] \\
         [--max-missing count] [--max-avg-page-turns value] [--page-size count] [--show-top count]\n\
         events: commit<TAB>reading<TAB>phrase<TAB>flags\n\
                 select<TAB>reading<TAB>phrase<TAB>index<TAB>page<TAB>skipped1|skipped2\n\
                 lookup<TAB>case_id<TAB>reading<TAB>expected<TAB>optional_bucket\n\
                 age<TAB>learning_event_ticks\n\
                 reset"
    );
    std::process::exit(0);
}

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let mut events_path = repo.join("tests").join("user_learning_replay.tsv");
    let mut lexicon_dir = repo.join("lexicon");
    let mut min_top1 = None;
    let mut min_top3 = None;
    let mut min_mrr = None;
    let mut max_missing = None;
    let mut max_avg_page_turns = None;
    let mut page_size = 9usize;
    let mut show_top = 0usize;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--events" => events_path = PathBuf::from(args.next().expect("--events path")),
            "--lexicon" => lexicon_dir = PathBuf::from(args.next().expect("--lexicon dir")),
            "--min-top1" => min_top1 = args.next().and_then(|value| value.parse::<f64>().ok()),
            "--min-top3" => min_top3 = args.next().and_then(|value| value.parse::<f64>().ok()),
            "--min-mrr" => min_mrr = args.next().and_then(|value| value.parse::<f64>().ok()),
            "--max-missing" => {
                max_missing = args.next().and_then(|value| value.parse::<usize>().ok())
            }
            "--max-avg-page-turns" => {
                max_avg_page_turns = args.next().and_then(|value| value.parse::<f64>().ok())
            }
            "--page-size" => {
                page_size = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                    .expect("--page-size must be positive")
            }
            "--show-top" => {
                show_top = args
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .expect("--show-top must be a non-negative integer")
            }
            "--help" | "-h" => usage(),
            path if !path.starts_with('-') => events_path = PathBuf::from(path),
            unknown => panic!("unknown argument: {unknown}"),
        }
    }

    run_replay(
        &events_path,
        &lexicon_dir,
        min_top1,
        min_top3,
        min_mrr,
        max_missing,
        max_avg_page_turns,
        page_size,
        show_top,
    );
}

fn run_replay(
    events_path: &Path,
    lexicon_dir: &Path,
    min_top1: Option<f64>,
    min_top3: Option<f64>,
    min_mrr: Option<f64>,
    max_missing: Option<usize>,
    max_avg_page_turns: Option<f64>,
    page_size: usize,
    show_top: usize,
) {
    let text = fs::read_to_string(events_path)
        .unwrap_or_else(|err| panic!("read replay {}: {err}", events_path.display()));
    let mut engine = PinyinEngine::with_phrase_dir(Some(lexicon_dir));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(MODE_JIANPIN | MODE_MIXED_PINYIN);

    let mut metrics = Metrics::default();
    let mut bucket_metrics = BTreeMap::<String, Metrics>::new();
    let mut commit_counts: HashMap<(String, String), usize> = HashMap::new();
    let mut first_top1_after: HashMap<(String, String), usize> = HashMap::new();
    println!("case_id\tbucket\treading\texpected\trank\tpage_turns\tcommits\ttop1");

    for (index, raw_line) in text.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = raw_line.split('\t').collect();
        match fields.first().copied().unwrap_or_default().trim() {
            "commit" => {
                let reading = fields.get(1).expect("commit reading").trim();
                let phrase = fields.get(2).expect("commit phrase").trim();
                let flags = fields.get(3).map_or(0, |value| parse_flags(value));
                engine
                    .learn_commit_with_flags(reading, phrase, flags)
                    .unwrap_or_else(|err| panic!("line {line_no}: commit: {err}"));
                *commit_counts
                    .entry((reading.to_string(), phrase.to_string()))
                    .or_default() += 1;
            }
            "select" => {
                let reading = fields.get(1).expect("select reading").trim();
                let phrase = fields.get(2).expect("select phrase").trim();
                let selected_index = parse_usize(fields.get(3), "selected index", line_no);
                let page = parse_usize(fields.get(4), "page", line_no);
                let skipped: Vec<String> = fields
                    .get(5)
                    .into_iter()
                    .flat_map(|value| value.split('|'))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect();
                engine
                    .learn_selection_feedback(reading, phrase, selected_index, page, &skipped)
                    .unwrap_or_else(|err| panic!("line {line_no}: selection: {err}"));
            }
            "lookup" => {
                let case_id = fields.get(1).expect("lookup case_id").trim();
                let reading = fields.get(2).expect("lookup reading").trim();
                let expected = fields.get(3).expect("lookup expected").trim();
                let bucket = infer_bucket(reading, fields.get(4).copied());
                let (candidates, error) = engine.lookup_full_explain(reading);
                if let Some(error) = error {
                    panic!("line {line_no}: lookup {case_id}: {error}");
                }
                let rank = candidates
                    .iter()
                    .position(|(phrase, _, _)| phrase == expected)
                    .map(|rank| rank + 1);
                let commits = commit_counts
                    .get(&(reading.to_string(), expected.to_string()))
                    .copied()
                    .unwrap_or(0);
                metrics.record(rank, page_size);
                bucket_metrics
                    .entry(bucket.clone())
                    .or_default()
                    .record(rank, page_size);
                if let Some(rank) = rank {
                    if rank == 1 {
                        first_top1_after
                            .entry((reading.to_string(), expected.to_string()))
                            .or_insert(commits);
                    }
                } else {
                    metrics.missing += 1;
                }
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    case_id,
                    bucket,
                    reading,
                    expected,
                    rank.map_or_else(|| "-".to_string(), |rank| rank.to_string()),
                    rank.map_or(0, |rank| (rank - 1) / page_size),
                    commits,
                    rank == Some(1)
                );
                // This is deliberately opt-in: candidates can contain private
                // user words, so the normal evaluator only emits aggregate
                // metrics.  A developer can request a bounded, local snapshot
                // while investigating a ranking regression.
                if show_top > 0 {
                    for (candidate_rank, (phrase, score, meta)) in
                        candidates.iter().take(show_top).enumerate()
                    {
                        println!(
                            "candidate\t{}\t{}\t{}\t{:.2}\tsource={}\tmatch={}\tlayer={}\tpinned={}\tpartial={}\tuser_signal={}",
                            case_id,
                            candidate_rank + 1,
                            phrase,
                            score,
                            meta.source.label(),
                            meta.match_kind.label(),
                            meta.source_layer.label(),
                            usize::from(meta.pinned),
                            usize::from(meta.partial),
                            usize::from(meta.user_signal),
                        );
                    }
                }
            }
            "age" => {
                let ticks = fields
                    .get(1)
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .unwrap_or_else(|| panic!("line {line_no}: invalid age ticks"));
                engine.advance_learning_clock_for_eval(ticks);
            }
            "reset" => {
                engine.clear_user_lexicon_for_eval();
                commit_counts.clear();
            }
            event => panic!("line {line_no}: unknown event {event}"),
        }
    }

    if metrics.lookups == 0 {
        panic!("no lookup events in {}", events_path.display());
    }
    let top1_pct = metrics.top1_pct();
    println!(
        "summary\tlookups={}\ttop1={:.1}%\ttop3={:.1}%\ttop9={:.1}%\tmrr={:.3}\tavg_page_turns={:.3}\tmissing={}",
        metrics.lookups,
        top1_pct,
        metrics.top3_pct(),
        metrics.top9 as f64 * 100.0 / metrics.lookups as f64,
        metrics.mrr(),
        metrics.avg_page_turns(),
        metrics.missing
    );
    for (bucket_name, bucket) in bucket_metrics {
        println!(
            "bucket\t{}\tlookups={}\ttop1={:.1}%\ttop3={:.1}%\tmrr={:.3}\tavg_page_turns={:.3}\tmissing={}",
            bucket_name,
            bucket.lookups,
            bucket.top1_pct(),
            bucket.top3_pct(),
            bucket.mrr(),
            bucket.avg_page_turns(),
            bucket.missing,
        );
    }
    for ((reading, phrase), commits) in first_top1_after {
        println!("first_top1_after\t{reading}\t{phrase}\t{commits}");
    }

    let gate_failed = min_top1.is_some_and(|minimum| top1_pct < minimum)
        || min_top3.is_some_and(|minimum| metrics.top3_pct() < minimum)
        || min_mrr.is_some_and(|minimum| metrics.mrr() < minimum)
        || max_missing.is_some_and(|maximum| metrics.missing > maximum)
        || max_avg_page_turns.is_some_and(|maximum| metrics.avg_page_turns() > maximum);
    if gate_failed {
        std::process::exit(2);
    }
}
