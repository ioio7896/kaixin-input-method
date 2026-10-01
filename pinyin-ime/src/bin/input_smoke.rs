use pinyin_ime::core::{
    PinyinEngine, MODE_DATE_AUTO_FORMAT, MODE_EMOJI_INPUT, MODE_JIANPIN, MODE_MIXED_PINYIN,
    MODE_SYMBOL_TOOLBOX, MODE_V_ASSIST,
};
use regex::Regex;
use std::env;
use std::path::PathBuf;

#[derive(Debug)]
struct Case {
    id: String,
    input: String,
    expected: String,
}

fn parse_cases(path: &PathBuf) -> Vec<Case> {
    let conn = rusqlite::Connection::open(path)
        .unwrap_or_else(|err| panic!("open smoke cases {}: {err}", path.display()));
    let mut stmt = conn
        .prepare(
            "SELECT case_id, input, expected
             FROM cases
             ORDER BY sort_order, case_id",
        )
        .expect("prepare smoke cases query");
    let rows = stmt
        .query_map([], |row| {
            Ok(Case {
                id: row.get(0)?,
                input: row.get(1)?,
                expected: row.get(2)?,
            })
        })
        .expect("query smoke cases");
    rows.map(|row| row.expect("read smoke case")).collect()
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

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let cases_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("tests").join("input_cases.sqlite"));
    let lexicon_dir = repo.join("lexicon");
    let mut engine = PinyinEngine::with_phrase_dir(Some(&lexicon_dir));
    engine.clear_user_lexicon_for_eval();
    engine.set_mode_flags(
        MODE_V_ASSIST
            | MODE_SYMBOL_TOOLBOX
            | MODE_EMOJI_INPUT
            | MODE_DATE_AUTO_FORMAT
            | MODE_JIANPIN
            | MODE_MIXED_PINYIN,
    );
    let cases = parse_cases(&cases_path);
    if cases.is_empty() {
        eprintln!("no smoke cases found: {}", cases_path.display());
        std::process::exit(1);
    }

    let mut failed = 0usize;
    for case in cases {
        let (candidates, err) = engine.lookup_full(&case.input);
        let actual = candidates.first().map(|c| c.0.as_str()).unwrap_or("");
        let matched = if let Some((top_n, _)) = top_n_target(&case.expected) {
            candidates
                .iter()
                .take(top_n)
                .any(|candidate| expected_matches(&case.expected, &candidate.0))
        } else {
            expected_matches(&case.expected, actual)
        };
        if !matched {
            failed += 1;
            eprintln!(
                "FAIL\t{}\tinput={}\texpected={}\tactual={}\terr={:?}",
                case.id, case.input, case.expected, actual, err
            );
            let preview = candidates
                .iter()
                .take(5)
                .map(|item| item.0.as_str())
                .collect::<Vec<_>>()
                .join(" | ");
            eprintln!("TOP\t{}\t{}", case.id, preview);
        } else {
            println!("OK\t{}\t{}\t{}", case.id, case.input, actual);
        }
    }

    if failed > 0 {
        eprintln!("input smoke failed: {failed}");
        std::process::exit(1);
    }
}
