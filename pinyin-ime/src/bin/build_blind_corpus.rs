use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct Args {
    lexicon_dirs: Vec<PathBuf>,
    output: PathBuf,
    size: usize,
    seed: u64,
    min_chars: usize,
    max_chars: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    phrase: String,
    input: String,
    freq: u64,
    category: String,
    source: String,
}

fn parse_args() -> Result<Args, String> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let mut lexicon_dirs = Vec::new();
    let mut output = repo
        .join(".cache")
        .join("candidate-benchmark")
        .join("blind_cases.sqlite");
    let mut size = 10_000usize;
    let mut seed = 20_260_717u64;
    let mut min_chars = 2usize;
    let mut max_chars = 8usize;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lexicon-dir" => {
                lexicon_dirs.push(PathBuf::from(
                    args.next().ok_or("missing value for --lexicon-dir")?,
                ));
            }
            "--output" => output = PathBuf::from(args.next().ok_or("missing value for --output")?),
            "--size" => {
                size = args
                    .next()
                    .ok_or("missing value for --size")?
                    .parse()
                    .map_err(|_| "invalid --size")?;
            }
            "--seed" => {
                seed = args
                    .next()
                    .ok_or("missing value for --seed")?
                    .parse()
                    .map_err(|_| "invalid --seed")?;
            }
            "--min-chars" => {
                min_chars = args
                    .next()
                    .ok_or("missing value for --min-chars")?
                    .parse()
                    .map_err(|_| "invalid --min-chars")?;
            }
            "--max-chars" => {
                max_chars = args
                    .next()
                    .ok_or("missing value for --max-chars")?
                    .parse()
                    .map_err(|_| "invalid --max-chars")?;
            }
            "--help" | "-h" => {
                println!(
                    "usage: build_blind_corpus [--lexicon-dir DIR ...] [--output FILE] [--size 10000]\n\
                     [--seed 20260717] [--min-chars 2] [--max-chars 8]\n\
                     default source: <repo>/lexicon/zh"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if lexicon_dirs.is_empty() {
        lexicon_dirs.push(repo.join("lexicon").join("zh"));
    }
    if size == 0 {
        return Err("--size must be positive".to_string());
    }
    if min_chars == 0 || min_chars > max_chars {
        return Err("invalid character length range".to_string());
    }
    Ok(Args {
        lexicon_dirs,
        output,
        size,
        seed,
        min_chars,
        max_chars,
    })
}

fn collect_text_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("read lexicon directory {}: {err}", dir.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|err| format!("read directory entry {}: {err}", dir.display()))?
            .path();
        if path.is_dir() {
            collect_text_files(&path, out)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".txt") && !name.starts_with("README"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0x20000..=0x2ebef | 0x30000..=0x323af
    )
}

fn normalized_reading(code: &str) -> Option<(String, usize)> {
    let tokens = code
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty()
        || tokens
            .iter()
            .any(|token| !token.chars().all(|ch| ch.is_ascii_alphabetic()))
    {
        return None;
    }
    Some((tokens.concat().to_ascii_lowercase(), tokens.len()))
}

fn parse_entry(line: &str, category: &str, source: &str) -> Option<Entry> {
    let mut parts = line.trim_start_matches('\u{feff}').split('\t');
    let phrase = parts.next()?.trim();
    let code = parts.next()?.trim();
    let freq = parts
        .find_map(|field| field.trim().parse::<u64>().ok())
        .unwrap_or(1)
        .max(1);
    let chars = phrase.chars().collect::<Vec<_>>();
    if chars.is_empty() || chars.iter().any(|ch| !is_cjk(*ch)) {
        return None;
    }
    let (input, syllables) = normalized_reading(code)?;
    if syllables != chars.len() {
        return None;
    }
    Some(Entry {
        phrase: phrase.to_string(),
        input,
        freq,
        category: format!("{category}_len{}", chars.len()),
        source: source.to_string(),
    })
}

fn category_for(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .filter(|part| !part.ends_with(".txt"))
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lexicon")
        })
        .to_string()
}

fn load_entries(args: &Args) -> Result<Vec<Entry>, String> {
    let mut best_by_input = HashMap::<String, Entry>::new();
    for root in &args.lexicon_dirs {
        let mut files = Vec::new();
        collect_text_files(root, &mut files)?;
        files.sort();
        for path in files {
            let text = fs::read_to_string(&path)
                .map_err(|err| format!("read lexicon {}: {err}", path.display()))?;
            let category = category_for(root, &path);
            let source = path.to_string_lossy();
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let Some(entry) = parse_entry(line, &category, &source) else {
                    continue;
                };
                let chars = entry.phrase.chars().count();
                if !(args.min_chars..=args.max_chars).contains(&chars) {
                    continue;
                }
                match best_by_input.get_mut(&entry.input) {
                    Some(current)
                        if entry.freq > current.freq
                            || (entry.freq == current.freq && entry.phrase < current.phrase) =>
                    {
                        *current = entry;
                    }
                    None => {
                        best_by_input.insert(entry.input.clone(), entry);
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(best_by_input.into_values().collect())
}

fn stable_hash(seed: u64, value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64 ^ seed;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn select_entries(mut entries: Vec<Entry>, size: usize, seed: u64) -> Vec<Entry> {
    entries.sort_by(|left, right| {
        stable_hash(seed, &left.input)
            .cmp(&stable_hash(seed, &right.input))
            .then_with(|| left.input.cmp(&right.input))
    });
    entries.truncate(size.min(entries.len()));
    entries
}

fn write_sqlite(args: &Args, entries: &[Entry]) -> Result<(), String> {
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create output directory {}: {err}", parent.display()))?;
    }
    let mut conn = Connection::open(&args.output)
        .map_err(|err| format!("open output {}: {err}", args.output.display()))?;
    conn.execute_batch(
        "DROP TABLE IF EXISTS cases;
         DROP TABLE IF EXISTS case_metadata;
         CREATE TABLE cases (
             sort_order INTEGER NOT NULL,
             case_id TEXT PRIMARY KEY,
             category TEXT NOT NULL,
             input TEXT NOT NULL,
             expected TEXT NOT NULL
         );
         CREATE TABLE case_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )
    .map_err(|err| format!("initialize output database: {err}"))?;
    let tx = conn
        .transaction()
        .map_err(|err| format!("start output transaction: {err}"))?;
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO cases(sort_order, case_id, category, input, expected)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|err| format!("prepare case insert: {err}"))?;
        for (index, entry) in entries.iter().enumerate() {
            insert
                .execute(params![
                    index as i64 + 1,
                    format!("blind_{:05}", index + 1),
                    entry.category,
                    entry.input,
                    entry.phrase,
                ])
                .map_err(|err| format!("insert case {}: {err}", index + 1))?;
        }
    }
    for (key, value) in [
        ("generator", "build_blind_corpus_v1".to_string()),
        ("seed", args.seed.to_string()),
        ("requested_size", args.size.to_string()),
        ("actual_size", entries.len().to_string()),
        (
            "lexicon_dirs",
            args.lexicon_dirs
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(";"),
        ),
        (
            "schema",
            "cases(sort_order, case_id, category, input, expected)".to_string(),
        ),
    ] {
        tx.execute(
            "INSERT INTO case_metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|err| format!("insert metadata {key}: {err}"))?;
    }
    tx.commit()
        .map_err(|err| format!("commit output database: {err}"))
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let available = load_entries(&args)?;
    if available.len() < args.size {
        eprintln!(
            "warning: requested {} cases but only {} unique readings are available",
            args.size,
            available.len()
        );
    }
    let entries = select_entries(available, args.size, args.seed);
    write_sqlite(&args, &entries)?;
    println!(
        "blind corpus written: path={} cases={} seed={} chars={}..{}",
        args.output.display(),
        entries.len(),
        args.seed,
        args.min_chars,
        args.max_chars
    );
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("build_blind_corpus: {err}");
        std::process::exit(1);
    }
}
