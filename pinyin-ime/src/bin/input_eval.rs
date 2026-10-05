use pinyin_ime::core::{
    MODE_DATE_AUTO_FORMAT, MODE_EMOJI_INPUT, MODE_JIANPIN, MODE_MIXED_PINYIN, MODE_SYMBOL_TOOLBOX,
    MODE_V_ASSIST,
};
use pinyin_ime::eval_profile::EvaluationProfile;
use regex::Regex;
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Case {
    id: String,
    category: String,
    input: String,
    expected: String,
}

#[derive(Debug)]
struct EvalArgs {
    engine_profile: EvaluationProfile,
    lexicon_dir: PathBuf,
    cases_path: PathBuf,
    explain_case_id: Option<String>,
    explain_limit: usize,
    explain_all: bool,
    min_top1: Option<f64>,
    min_top3: Option<f64>,
    min_top9: Option<f64>,
}

fn parse_cases(path: &Path) -> Vec<Case> {
    let conn = rusqlite::Connection::open(path)
        .unwrap_or_else(|err| panic!("open eval cases {}: {err}", path.display()));
    let mut stmt = conn
        .prepare(
            "SELECT case_id, category, input, expected
             FROM cases
             ORDER BY sort_order, case_id",
        )
        .expect("prepare eval cases query");
    let rows = stmt
        .query_map([], |row| {
            Ok(Case {
                id: row.get(0)?,
                category: row.get(1)?,
                input: row.get(2)?,
                expected: row.get(3)?,
            })
        })
        .expect("query eval cases");
    rows.map(|row| row.expect("read eval case")).collect()
}

fn expected_matches(expected: &str, actual: &str) -> bool {
    match expected {
        "@DATE" => Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap().is_match(actual),
        "@TIME" => Regex::new(r"^\d{2}:\d{2}:\d{2}$").unwrap().is_match(actual),
        s if top_n_target(s).is_some() => actual == top_n_target(s).unwrap().1,
        s if s.starts_with("@REGEX:") => Regex::new(&s[7..])
            .map(|re| re.is_match(actual))
            .unwrap_or(false),
        s => actual == s,
    }
}

fn top_n_target(expected: &str) -> Option<(usize, &str)> {
    let rest = expected.strip_prefix("@TOP")?;
    let (n, target) = rest.split_once(':')?;
    let n = n.parse::<usize>().ok()?;
    if n == 0 || target.is_empty() {
        return None;
    }
    Some((n, target))
}

fn parse_args(repo: &Path) -> EvalArgs {
    let mut args = env::args().skip(1);
    let mut cases_path = None;
    let mut explain_case_id = None;
    let mut explain_limit = 20usize;
    let mut explain_all = false;
    let mut min_top1 = None;
    let mut min_top3 = None;
    let mut min_top9 = None;
    let mut engine_profile = EvaluationProfile::default();
    let mut lexicon_dir = repo.join("lexicon");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine-profile" => {
                engine_profile =
                    EvaluationProfile::parse(&args.next().expect("--engine-profile value"))
                        .unwrap_or_else(|err| panic!("{err}"))
            }
            "--lexicon" => lexicon_dir = PathBuf::from(args.next().expect("--lexicon dir")),
            "--mixed" => {
                cases_path = Some(repo.join("tests").join("mixed_pinyin_cases.sqlite"));
            }
            "--quality" => {
                cases_path = Some(repo.join("tests").join("candidate_quality_cases.sqlite"));
            }
            "--cases" => {
                if let Some(path) = args.next() {
                    cases_path = Some(PathBuf::from(path));
                }
            }
            "--explain" => {
                explain_case_id = args.next();
            }
            "--explain-limit" => {
                explain_limit = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(explain_limit);
            }
            "--explain-all" => {
                explain_all = true;
            }
            "--min-top1" => {
                min_top1 = args.next().and_then(|value| value.parse().ok());
            }
            "--min-top3" => {
                min_top3 = args.next().and_then(|value| value.parse().ok());
            }
            "--min-top9" => {
                min_top9 = args.next().and_then(|value| value.parse().ok());
            }
            "--help" | "-h" => {
                eprintln!(
                    "usage: input_eval [cases.sqlite] [--cases cases.sqlite] [--mixed] [--quality] [--explain case_id] [--explain-limit N] [--explain-all]\n\
                     optional gates: --min-top1 PCT --min-top3 PCT --min-top9 PCT\n\
                     engine: --engine-profile runtime-hot-cold|full --lexicon DIR\n\
                     case table: cases(case_id, category, input, expected, sort_order)\n\
                     expected supports @TOPN:<text>, @DATE, @TIME, @REGEX:<regex>"
                );
                std::process::exit(0);
            }
            path => {
                cases_path = Some(PathBuf::from(path));
            }
        }
    }
    EvalArgs {
        engine_profile,
        lexicon_dir,
        cases_path: cases_path.unwrap_or_else(|| repo.join("tests").join("input_cases.sqlite")),
        explain_case_id,
        explain_limit,
        explain_all,
        min_top1,
        min_top3,
        min_top9,
    }
}

fn expected_rank(
    candidates: &[(String, f64, pinyin_ime::core::CandidateMeta)],
    expected: &str,
) -> String {
    candidates
        .iter()
        .position(|(phrase, _, _)| expected_matches(expected, phrase))
        .map(|idx| (idx + 1).to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn candidate_source_tags(meta: &pinyin_ime::core::CandidateMeta) -> String {
    let mut tags = Vec::new();
    if meta.match_kind == pinyin_ime::core::CandidateMatchKind::Correction
        || meta.source == pinyin_ime::core::CandidateSource::Correction
        || meta
            .display_text()
            .is_some_and(|text| text.contains("typo=1") || text.contains("纠错"))
    {
        tags.push("~");
    }
    if meta.pinned {
        tags.push("⭐");
    } else if meta.source == pinyin_ime::core::CandidateSource::User {
        tags.push("🕐");
    }
    if matches!(
        meta.source_layer,
        pinyin_ime::thuocl::LexiconLayer::Ext | pinyin_ime::thuocl::LexiconLayer::Large
    ) {
        tags.push("📘");
    }
    tags.join("")
}

#[derive(Default)]
struct CategoryStats {
    total: usize,
    top1_hits: usize,
    top3_hits: usize,
    top9_hits: usize,
}

impl CategoryStats {
    fn record(&mut self, top1_hit: bool, top3_hit: bool, top9_hit: bool) {
        self.total += 1;
        self.top1_hits += usize::from(top1_hit);
        self.top3_hits += usize::from(top3_hit);
        self.top9_hits += usize::from(top9_hit);
    }

    fn pct(&self, hits: usize) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            hits as f64 * 100.0 / self.total as f64
        }
    }
}

fn check_gate(name: &str, actual: f64, minimum: Option<f64>) -> bool {
    let Some(minimum) = minimum else {
        return true;
    };
    if actual + f64::EPSILON >= minimum {
        true
    } else {
        eprintln!("FAIL {name} below gate: actual={actual:.1}% minimum={minimum:.1}%");
        false
    }
}

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let args = parse_args(&repo);
    args.engine_profile.report(&args.lexicon_dir);
    let mut engine = args.engine_profile.create_engine(&args.lexicon_dir);
    engine.set_mode_flags(
        MODE_V_ASSIST
            | MODE_SYMBOL_TOOLBOX
            | MODE_EMOJI_INPUT
            | MODE_DATE_AUTO_FORMAT
            | MODE_JIANPIN
            | MODE_MIXED_PINYIN,
    );
    let cases = parse_cases(&args.cases_path);
    if cases.is_empty() {
        eprintln!("no eval cases found: {}", args.cases_path.display());
        std::process::exit(1);
    }

    if let Some(case_id) = args.explain_case_id.as_deref() {
        let Some(case) = cases.iter().find(|case| case.id == case_id) else {
            eprintln!(
                "case_id not found in {}: {}",
                args.cases_path.display(),
                case_id
            );
            std::process::exit(2);
        };
        let (candidates, err) = engine.lookup_full_explain(&case.input);
        let intent = engine.input_intent(&case.input);
        println!(
            "case_id\tcategory\tinput\tintent\texpected\texpected_rank\terror\n{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id,
            case.category,
            case.input,
            intent.label(),
            case.expected,
            expected_rank(&candidates, &case.expected),
            err.as_deref().unwrap_or("")
        );
        println!("rank\tcandidate\tscore\ttags\tlayer\tmatch_kind\tsource\tmeta\tmatch");
        for (idx, (phrase, score, meta)) in candidates.iter().take(args.explain_limit).enumerate() {
            println!(
                "{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{}",
                idx + 1,
                phrase,
                score,
                candidate_source_tags(meta),
                meta.source_layer.label(),
                meta.match_kind.label(),
                meta.source.label(),
                meta.display_text().unwrap_or(""),
                expected_matches(&case.expected, phrase)
            );
        }
        return;
    }

    if args.explain_all {
        println!("case_id\tcategory\tinput\tintent\texpected\texpected_rank\ttop1\ttop1_tags\ttop1_layer\ttop1_match_kind\ttop1_source\ttop1_meta\tpreview");
        for case in &cases {
            let (candidates, _err) = engine.lookup_full_explain(&case.input);
            let intent = engine.input_intent(&case.input);
            let (top1, top1_meta) = candidates
                .first()
                .map(|(phrase, _, meta)| (phrase.as_str(), Some(meta)))
                .unwrap_or(("", None));
            let preview = candidates
                .iter()
                .take(9)
                .map(|(phrase, _, meta)| {
                    format!(
                        "{}{}[{}/{}]",
                        candidate_source_tags(meta),
                        phrase,
                        meta.source_layer.label(),
                        meta.match_kind.label()
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                case.id,
                case.category,
                case.input,
                intent.label(),
                case.expected,
                expected_rank(&candidates, &case.expected),
                top1,
                top1_meta.map(candidate_source_tags).unwrap_or_default(),
                top1_meta
                    .map(|meta| meta.source_layer.label())
                    .unwrap_or(""),
                top1_meta.map(|meta| meta.match_kind.label()).unwrap_or(""),
                top1_meta.map(|meta| meta.source.label()).unwrap_or(""),
                top1_meta.and_then(|meta| meta.display_text()).unwrap_or(""),
                preview
            );
        }
        return;
    }

    let mut top1_hits = 0usize;
    let mut top3_hits = 0usize;
    let mut top9_hits = 0usize;
    let mut category_stats = std::collections::BTreeMap::<String, CategoryStats>::new();
    println!("case_id\tcategory\tinput\texpected\ttop1\ttop3_hit\ttop9_hit\tpreview");
    for case in &cases {
        let (candidates, _err) = engine.lookup_full(&case.input);
        let top1 = candidates.first().map(|c| c.0.as_str()).unwrap_or("");
        let top1_hit = expected_matches(&case.expected, top1);
        let top3_hit = candidates
            .iter()
            .take(3)
            .any(|candidate| expected_matches(&case.expected, &candidate.0));
        let top9_hit = candidates
            .iter()
            .take(9)
            .any(|candidate| expected_matches(&case.expected, &candidate.0));
        if top1_hit {
            top1_hits += 1;
        }
        if top3_hit {
            top3_hits += 1;
        }
        if top9_hit {
            top9_hits += 1;
        }
        category_stats
            .entry(case.category.clone())
            .or_default()
            .record(top1_hit, top3_hit, top9_hit);
        let preview = candidates
            .iter()
            .take(9)
            .map(|item| item.0.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id, case.category, case.input, case.expected, top1, top3_hit, top9_hit, preview
        );
    }

    let total = cases.len() as f64;
    let top1_pct = top1_hits as f64 * 100.0 / total;
    let top3_pct = top3_hits as f64 * 100.0 / total;
    let top9_pct = top9_hits as f64 * 100.0 / total;
    println!(
        "SUMMARY cases={} top1={:.1}% ({}) top3={:.1}% ({}) top9={:.1}% ({})",
        cases.len(),
        top1_pct,
        top1_hits,
        top3_pct,
        top3_hits,
        top9_pct,
        top9_hits
    );
    println!("CATEGORY\tcases\ttop1\ttop3\ttop9");
    for (category, stats) in category_stats {
        println!(
            "{}\t{}\t{:.1}% ({})\t{:.1}% ({})\t{:.1}% ({})",
            category,
            stats.total,
            stats.pct(stats.top1_hits),
            stats.top1_hits,
            stats.pct(stats.top3_hits),
            stats.top3_hits,
            stats.pct(stats.top9_hits),
            stats.top9_hits
        );
    }

    let gates_ok = check_gate("top1", top1_pct, args.min_top1)
        & check_gate("top3", top3_pct, args.min_top3)
        & check_gate("top9", top9_pct, args.min_top9);
    if !gates_ok {
        std::process::exit(1);
    }
}
