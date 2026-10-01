//! 词库调试工具用法：
//!   cargo run -p pinyin-ime --bin thuocl_tool -- <词库目录>
//!   cargo run -p pinyin-ime --bin thuocl_tool -- <词库目录> --lookup twyt
//!   cargo run -p pinyin-ime --bin thuocl_tool -- <词库目录> --prefix tw

use pinyin_ime::thuocl::{abbrev_for_phrase, load_dir_txt};
use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "用法:\n  {} <词库目录> [--lookup 缩写] [--prefix 前缀]\n示例:\n  {} ..\\\\lexicon --lookup xg\n  {} ..\\\\lexicon --prefix tw",
            args.first().map(|s| s.as_str()).unwrap_or("thuocl_tool"),
            args.first().map(|s| s.as_str()).unwrap_or("thuocl_tool"),
            args.first().map(|s| s.as_str()).unwrap_or("thuocl_tool"),
        );
        std::process::exit(1);
    }

    let mut dir: Option<PathBuf> = None;
    let mut lookup: Option<String> = None;
    let mut prefix: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--lookup" => {
                i += 1;
                lookup = args.get(i).cloned();
                i += 1;
            }
            "--prefix" => {
                i += 1;
                prefix = args.get(i).cloned();
                i += 1;
            }
            s if !s.starts_with('-') => {
                dir = Some(PathBuf::from(s));
                i += 1;
            }
            _ => {
                eprintln!("未知参数: {}", args[i]);
                std::process::exit(1);
            }
        }
    }

    let dir = dir.expect("请指定词库目录");
    let (lex, rep) = load_dir_txt(&dir).expect("加载失败");

    println!("目录: {}", dir.display());
    println!(
        "已读 txt 文件: {} 个, 总行: {}, 已索引词条: {}, 跳过(无缩写): {}",
        rep.files_read, rep.lines_total, rep.entries_indexed, rep.lines_skipped_no_abbrev
    );
    println!("不同缩写键数量: {}", lex.len_keys());
    for (p, err) in &rep.files_failed {
        eprintln!("读取失败: {} — {}", p.display(), err);
    }
    for (p, line, sample) in &rep.sample_skipped_lines {
        eprintln!("跳过示例: {}:{} … {}", p.display(), line, sample);
    }

    if let Some(ref q) = lookup {
        let q = q.to_lowercase();
        let list = lex.lookup(&q);
        println!("--lookup {:?}:", q);
        match list {
            Some(entries) => {
                for e in entries.iter().take(20) {
                    println!("  {}  {}", e.phrase, e.freq);
                }
            }
            None => println!("  (无)"),
        }
    }

    if let Some(ref p) = prefix {
        let p = p.to_lowercase();
        let flat = lex.prefix_flat(&p);
        println!("--prefix {:?} (合并去重按频次, 前 30 条):", p);
        for e in flat.iter().take(30) {
            println!("  {}  {}", e.phrase, e.freq);
        }
    }

    if lookup.is_none() && prefix.is_none() {
        println!("示例缩写: 天外有天 -> {:?}", abbrev_for_phrase("天外有天"));
    }
}
