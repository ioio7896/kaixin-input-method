//! Build-time tool that merges `lexicon/` into `lexicon.bin` (SRFLX002).

use pinyin_ime::thuocl::{
    load_dir_txt_with_profile_default_enabled, save_prebaked_lexicon, LexiconBuildProfile,
};
use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "usage: {} <lexicon-dir> <output lexicon.bin> [--profile hot|standard|full] \
             [--report-duplicates=<path.csv>] [--fail-on-pinyin-conflict]",
            args.first().map(|s| s.as_str()).unwrap_or("bake_lexicon")
        );
        std::process::exit(1);
    }

    let dir = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let extra = &args[3..];
    let profile = parse_profile(extra);
    let report_csv = parse_report_path(extra);
    let fail_on_pinyin = extra.iter().any(|a| a == "--fail-on-pinyin-conflict");

    let (lex, rep) =
        load_dir_txt_with_profile_default_enabled(&dir, profile).expect("load lexicon");
    eprintln!(
        "lexicon files={}, entries={}, profile={:?}",
        rep.files_read, rep.entries_indexed, profile
    );

    if !rep.duplicates.is_empty() {
        eprintln!(
            "duplicates found: {} total, {} pinyin-conflict(s)",
            rep.duplicates.len(),
            rep.pinyin_conflict_count(),
        );
    }
    if let Some(csv_path) = &report_csv {
        match rep.write_duplicate_csv(csv_path) {
            Ok(n) => eprintln!(
                "duplicate report written: {} rows -> {}",
                n,
                csv_path.display()
            ),
            Err(e) => eprintln!("warning: failed to write duplicate report: {}", e),
        }
    }

    save_prebaked_lexicon(&out, &lex).expect("write lexicon.bin");
    if profile != LexiconBuildProfile::Hot {
        lex.save_cold_index(&out.with_file_name("cold_lexicon.sqlite"))
            .expect("write cold exact-pinyin index");
    }
    eprintln!("wrote {}", out.display());

    if fail_on_pinyin && rep.has_pinyin_conflicts() {
        eprintln!(
            "ERROR: {} pinyin conflict(s) found and --fail-on-pinyin-conflict is set",
            rep.pinyin_conflict_count()
        );
        std::process::exit(1);
    }
}

fn parse_report_path(args: &[String]) -> Option<PathBuf> {
    for arg in args {
        if let Some(value) = arg.strip_prefix("--report-duplicates=") {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn parse_profile(args: &[String]) -> LexiconBuildProfile {
    let mut profile = env::var("SRF_LEXICON_PROFILE")
        .ok()
        .map(|value| LexiconBuildProfile::from_name(&value))
        .unwrap_or(LexiconBuildProfile::Standard);

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(value) = arg.strip_prefix("--profile=") {
            profile = LexiconBuildProfile::from_name(value);
        } else if arg == "--profile" {
            if let Some(value) = it.next() {
                profile = LexiconBuildProfile::from_name(value);
            }
        } else if arg == "--full" {
            profile = LexiconBuildProfile::Full;
        } else if arg == "--standard" {
            profile = LexiconBuildProfile::Standard;
        } else if arg == "--hot" {
            profile = LexiconBuildProfile::Hot;
        }
    }
    profile
}
