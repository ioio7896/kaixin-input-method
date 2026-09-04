#[path = "input_perf/metrics.rs"]
mod metrics;

use metrics::{duration_us, print_stats};
use pinyin_ime::core::{
    PinyinEngine, MODE_DATE_AUTO_FORMAT, MODE_EMOJI_INPUT, MODE_JIANPIN, MODE_MIXED_PINYIN,
    MODE_SYMBOL_TOOLBOX, MODE_V_ASSIST,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
struct PerfArgs {
    cases_paths: Vec<PathBuf>,
    inline_inputs: Vec<String>,
    workload_size: usize,
    warmup_rounds: usize,
    iterations: usize,
    max_p95_us: Option<u128>,
    max_p99_us: Option<u128>,
    slow_limit: usize,
    startup_warmup: bool,
    incremental: bool,
    mode: BenchMode,
    threads: usize,
    json_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BenchMode {
    /// 现有行为：单线程反复 lookup。
    Plain,
    /// 选词反馈后的冷键：反馈后立刻再查同一输入与相邻输入。
    PostFeedback,
    /// 上屏提交后的下一键：提交候选后测下一输入的 lookup。
    PostCommit,
    /// 同引擎多线程并发 lookup（全局引擎锁争用）。
    Concurrent,
    /// 端到端命名管道往返（进程内引擎服务 + 真实序列化）。
    E2EPipe,
    /// 对照：直接共享引擎 lookup（无管道/无调度器），用于定位 e2e 开销段。
    E2EDirect,
}

fn parse_case_inputs(path: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(path)
        .unwrap_or_else(|err| panic!("open perf cases {}: {err}", path.display()));
    let mut stmt = conn
        .prepare(
            "SELECT input
             FROM cases
             WHERE TRIM(input) <> ''
             ORDER BY sort_order, case_id",
        )
        .expect("prepare perf cases query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query perf cases");
    rows.map(|row| row.expect("read perf case input")).collect()
}

fn default_inline_inputs() -> Vec<String> {
    ["shuru", "nihao", "zhongguo", "bj", "ygd"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn parse_args() -> PerfArgs {
    let mut args = env::args().skip(1);
    let mut cases_paths: Vec<PathBuf> = Vec::new();
    let mut inline_inputs = Vec::new();
    let mut workload_size = 300usize;
    let mut warmup_rounds = 2usize;
    let mut iterations = 3usize;
    let mut max_p95_us = None;
    let mut max_p99_us = None;
    let mut slow_limit = 0usize;
    let mut startup_warmup = false;
    let mut incremental = false;
    let mut mode = BenchMode::Plain;
    let mut threads = 4usize;
    let mut json_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--cases" => {
                if let Some(path) = args.next() {
                    cases_paths.push(PathBuf::from(path));
                }
            }
            "--input" => {
                if let Some(input) = args.next() {
                    if !input.trim().is_empty() {
                        inline_inputs.push(input);
                    }
                }
            }
            "--workload-size" => {
                workload_size = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(workload_size);
            }
            "--warmup" => {
                warmup_rounds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(warmup_rounds);
            }
            "--iterations" => {
                iterations = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(iterations);
            }
            "--max-p95-us" => {
                max_p95_us = args.next().and_then(|value| value.parse().ok());
            }
            "--max-p99-us" => {
                max_p99_us = args.next().and_then(|value| value.parse().ok());
            }
            "--slow" => {
                slow_limit = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(10)
                    .min(100);
            }
            "--startup-warmup" => startup_warmup = true,
            "--incremental" => incremental = true,
            "--mode" => {
                mode = match args.next().as_deref() {
                    Some("plain") => BenchMode::Plain,
                    Some("post-feedback") => BenchMode::PostFeedback,
                    Some("post-commit") => BenchMode::PostCommit,
                    Some("concurrent") => BenchMode::Concurrent,
                    Some("e2e-pipe") => BenchMode::E2EPipe,
                    Some("e2e-direct") => BenchMode::E2EDirect,
                    _ => {
                        eprintln!("unknown --mode, falling back to plain");
                        BenchMode::Plain
                    }
                };
            }
            "--threads" => {
                threads = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(threads)
                    .clamp(1, 16);
            }
            "--json" => json_path = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                eprintln!(
                    "usage: input_perf [--cases cases.sqlite] [--input PINYIN]\n\
                     options: --workload-size N --warmup N --iterations N --incremental\n\
                     modes: --mode plain|post-feedback|post-commit|concurrent|e2e-pipe|e2e-direct\n\
                     concurrent: --threads N\n\
                     diagnostics: --slow N --startup-warmup\n\
                     output: --json PATH (newline-delimited JSON metrics)\n\
                     optional gates: --max-p95-us US --max-p99-us US"
                );
                std::process::exit(0);
            }
            path => cases_paths.push(PathBuf::from(path)),
        }
    }

    if cases_paths.is_empty() && inline_inputs.is_empty() {
        inline_inputs = default_inline_inputs();
    }
    workload_size = workload_size.clamp(1, 2_000);
    warmup_rounds = warmup_rounds.min(20);
    iterations = iterations.clamp(1, 50);

    PerfArgs {
        cases_paths,
        inline_inputs,
        workload_size,
        warmup_rounds,
        iterations,
        max_p95_us,
        max_p99_us,
        slow_limit,
        startup_warmup,
        incremental,
        mode,
        threads,
        json_path,
    }
}

fn input_prefixes(input: &str) -> Vec<String> {
    input
        .char_indices()
        .skip(1)
        .map(|(end, _)| input[..end].to_string())
        .chain(std::iter::once(input.to_string()))
        .collect()
}

fn build_workload(inputs: &[String], target_len: usize, incremental: bool) -> Vec<String> {
    if inputs.is_empty() {
        return Vec::new();
    }
    let mut workload = Vec::with_capacity(target_len);
    let mut idx = 0usize;
    while workload.len() < target_len {
        let input = &inputs[idx % inputs.len()];
        if incremental {
            for prefix in input_prefixes(input) {
                workload.push(prefix);
                if workload.len() >= target_len {
                    break;
                }
            }
        } else {
            workload.push(input.clone());
        }
        idx += 1;
    }
    workload
}

fn build_engine(lexicon_dir: &Path) -> PinyinEngine {
    let mut engine = PinyinEngine::with_phrase_dir(Some(lexicon_dir));
    engine.set_mode_flags(
        MODE_V_ASSIST
            | MODE_SYMBOL_TOOLBOX
            | MODE_EMOJI_INPUT
            | MODE_DATE_AUTO_FORMAT
            | MODE_JIANPIN
            | MODE_MIXED_PINYIN,
    );
    engine
}

fn run_plain(args: &PerfArgs, workload: &[String], engine: &mut PinyinEngine) {
    let mut request_id = 1u64;
    for _ in 0..args.warmup_rounds {
        for input in workload {
            if args.incremental {
                let _ = engine.lookup_full_with_request_id(input, request_id);
                request_id = request_id.saturating_add(1);
            } else {
                let _ = engine.lookup_full(input);
            }
        }
    }

    let mut samples = Vec::with_capacity(workload.len() * args.iterations);
    let mut slow_samples: Vec<(Duration, String)> = Vec::new();
    let run_started = Instant::now();
    let mut total_candidates = 0usize;
    for _ in 0..args.iterations {
        for input in workload {
            let started = Instant::now();
            let (candidates, _) = if args.incremental {
                let result = engine.lookup_full_with_request_id(input, request_id);
                request_id = request_id.saturating_add(1);
                result
            } else {
                engine.lookup_full(input)
            };
            let elapsed = started.elapsed();
            if args.slow_limit > 0 {
                slow_samples.push((elapsed, input.clone()));
            }
            samples.push(elapsed);
            total_candidates += candidates.len();
        }
    }

    println!(
        "mode={} inputs={} warmup={} iterations={} candidates={} wall={}ms",
        if args.incremental {
            "incremental"
        } else {
            "plain"
        },
        workload.len(),
        args.warmup_rounds,
        args.iterations,
        total_candidates,
        run_started.elapsed().as_millis()
    );
    let (p95, p99) = print_stats(args.json_path.as_deref(), "plain", &samples);
    print_slow_samples(args, &slow_samples);
    gate_p95_p99(args, p95, p99);
}

/// 选词反馈后的冷键，分三个独立 pass，互不污染缓存状态：
/// - pass 1 plain：纯热路径基准（无任何反馈）
/// - pass 2 post_same/post_next：反馈（不计时）后对同一输入与相邻输入
///   立刻再查（定向失效成本 / 缓存幸存路径）
/// - pass 3 full_clear_legacy：反馈后模拟旧版整表清空（切换 mode flags
///   触发 clear_lookup_caches）再查同一输入，量化旧行为的冷启动成本
fn run_post_feedback(args: &PerfArgs, workload: &[String], engine: &mut PinyinEngine) {
    for _ in 0..args.warmup_rounds {
        for input in workload {
            let _ = engine.lookup_full(input);
        }
    }

    let run_started = Instant::now();

    let mut plain_samples = Vec::with_capacity(workload.len() * args.iterations);
    for _ in 0..args.iterations {
        for input in workload {
            let started = Instant::now();
            let _ = engine.lookup_full(input);
            plain_samples.push(started.elapsed());
        }
    }

    let mut post_same_samples = Vec::with_capacity(workload.len() * args.iterations);
    let mut post_next_samples = Vec::with_capacity(workload.len() * args.iterations);
    let mut feedback_applied = 0usize;
    for _ in 0..args.iterations {
        for (idx, input) in workload.iter().enumerate() {
            let (candidates, _) = engine.lookup_full(input);
            if candidates.len() < 2 {
                continue;
            }
            // 模拟"选第 2 个候选"：第 1 个作为被跳过的负反馈。
            let selected = candidates[1].0.clone();
            let skipped = candidates[0].0.clone();
            if engine
                .learn_selection_feedback(input, &selected, 1, 0, &[skipped])
                .is_ok()
            {
                feedback_applied += 1;

                let started = Instant::now();
                let _ = engine.lookup_full(input);
                post_same_samples.push(started.elapsed());

                let next_input = &workload[(idx + 1) % workload.len()];
                let started = Instant::now();
                let _ = engine.lookup_full(next_input);
                post_next_samples.push(started.elapsed());
            }
        }
    }

    let mut full_clear_samples = Vec::with_capacity(workload.len() * args.iterations);
    for _ in 0..args.iterations {
        for input in workload {
            let (candidates, _) = engine.lookup_full(input);
            if candidates.len() < 2 {
                continue;
            }
            let selected = candidates[1].0.clone();
            let skipped = candidates[0].0.clone();
            if engine
                .learn_selection_feedback(input, &selected, 1, 0, &[skipped])
                .is_err()
            {
                continue;
            }
            // 旧行为对照：切换已有模式位触发 clear_lookup_caches 整表清空
            // （比旧实现 user-sensitive 清空还多清了 prefix 缓存，同属
            // "全冷"量级）。
            engine.set_mode_flags(
                MODE_SYMBOL_TOOLBOX
                    | MODE_EMOJI_INPUT
                    | MODE_DATE_AUTO_FORMAT
                    | MODE_JIANPIN
                    | MODE_MIXED_PINYIN,
            );
            engine.set_mode_flags(
                MODE_V_ASSIST
                    | MODE_SYMBOL_TOOLBOX
                    | MODE_EMOJI_INPUT
                    | MODE_DATE_AUTO_FORMAT
                    | MODE_JIANPIN
                    | MODE_MIXED_PINYIN,
            );
            let started = Instant::now();
            let _ = engine.lookup_full(input);
            full_clear_samples.push(started.elapsed());
        }
    }

    println!(
        "mode=post-feedback inputs={} warmup={} iterations={} feedback_applied={} wall={}ms",
        workload.len(),
        args.warmup_rounds,
        args.iterations,
        feedback_applied,
        run_started.elapsed().as_millis()
    );
    let (p95, p99) = print_stats(args.json_path.as_deref(), "plain", &plain_samples);
    print_stats(
        args.json_path.as_deref(),
        "post_feedback_same",
        &post_same_samples,
    );
    print_stats(
        args.json_path.as_deref(),
        "post_feedback_next",
        &post_next_samples,
    );
    print_stats(
        args.json_path.as_deref(),
        "post_feedback_full_clear_legacy",
        &full_clear_samples,
    );
    gate_p95_p99(args, p95, p99);
}

/// 上屏提交后的下一键，分三个独立 pass：
/// - pass 1 plain：纯热路径基准，同时收集每个输入的首候选
/// - pass 2 post_commit_next：提交（不计时）后测下一输入的 lookup
///   （定向失效后的跨输入幸存路径）
/// - pass 3 full_clear_legacy：提交后整表清空（旧行为）再测下一输入
fn run_post_commit(args: &PerfArgs, workload: &[String], engine: &mut PinyinEngine) {
    for _ in 0..args.warmup_rounds {
        for input in workload {
            let _ = engine.lookup_full(input);
        }
    }

    let mut commit_phrases: Vec<Option<String>> = Vec::with_capacity(workload.len());
    for input in workload {
        let (candidates, _) = engine.lookup_full(input);
        commit_phrases.push(candidates.first().map(|c| c.0.clone()));
    }

    let run_started = Instant::now();

    let mut plain_samples = Vec::with_capacity(workload.len() * args.iterations);
    for _ in 0..args.iterations {
        for input in workload {
            let started = Instant::now();
            let _ = engine.lookup_full(input);
            plain_samples.push(started.elapsed());
        }
    }

    let mut post_commit_samples = Vec::with_capacity(workload.len() * args.iterations);
    let mut commits_applied = 0usize;
    for _ in 0..args.iterations {
        for (idx, input) in workload.iter().enumerate() {
            if let Some(Some(phrase)) = commit_phrases.get(idx) {
                if engine.learn_commit(input, phrase).is_ok() {
                    commits_applied += 1;
                    let next_input = &workload[(idx + 1) % workload.len()];
                    let started = Instant::now();
                    let _ = engine.lookup_full(next_input);
                    post_commit_samples.push(started.elapsed());
                }
            }
        }
    }

    let mut full_clear_samples = Vec::with_capacity(workload.len() * args.iterations);
    for _ in 0..args.iterations {
        for (idx, input) in workload.iter().enumerate() {
            if let Some(Some(phrase)) = commit_phrases.get(idx) {
                if engine.learn_commit(input, phrase).is_err() {
                    continue;
                }
                engine.set_mode_flags(
                    MODE_SYMBOL_TOOLBOX
                        | MODE_EMOJI_INPUT
                        | MODE_DATE_AUTO_FORMAT
                        | MODE_JIANPIN
                        | MODE_MIXED_PINYIN,
                );
                engine.set_mode_flags(
                    MODE_V_ASSIST
                        | MODE_SYMBOL_TOOLBOX
                        | MODE_EMOJI_INPUT
                        | MODE_DATE_AUTO_FORMAT
                        | MODE_JIANPIN
                        | MODE_MIXED_PINYIN,
                );
                let next_input = &workload[(idx + 1) % workload.len()];
                let started = Instant::now();
                let _ = engine.lookup_full(next_input);
                full_clear_samples.push(started.elapsed());
            }
        }
    }

    println!(
        "mode=post-commit inputs={} warmup={} iterations={} commits_applied={} wall={}ms",
        workload.len(),
        args.warmup_rounds,
        args.iterations,
        commits_applied,
        run_started.elapsed().as_millis()
    );
    let (p95, p99) = print_stats(args.json_path.as_deref(), "plain", &plain_samples);
    print_stats(
        args.json_path.as_deref(),
        "post_commit_next",
        &post_commit_samples,
    );
    print_stats(
        args.json_path.as_deref(),
        "post_commit_full_clear_legacy",
        &full_clear_samples,
    );
    gate_p95_p99(args, p95, p99);
}

/// 同引擎多线程并发 lookup。引擎包在全局 Mutex 里，与引擎服务的
/// shared_engine 锁模式一致（不含 IPC 调度队列）。
fn run_concurrent(args: &PerfArgs, workload: &[String], mut engine: PinyinEngine) {
    for _ in 0..args.warmup_rounds {
        for input in workload {
            let _ = engine.lookup_full(input);
        }
    }

    let engine = Arc::new(Mutex::new(engine));
    let mut per_thread: Vec<Vec<Duration>> = (0..args.threads).map(|_| Vec::new()).collect();
    let run_started = Instant::now();
    let rounds = args.iterations;
    std::thread::scope(|scope| {
        for (thread_idx, samples) in per_thread.iter_mut().enumerate() {
            let engine = Arc::clone(&engine);
            scope.spawn(move || {
                for round in 0..rounds {
                    let offset = (thread_idx + round) % workload.len();
                    let input = &workload[offset];
                    let started = Instant::now();
                    let _ = engine.lock().expect("engine lock").lookup_full(input);
                    samples.push(started.elapsed());
                }
            });
        }
    });

    let mut samples: Vec<Duration> = per_thread.into_iter().flatten().collect();
    let ops = samples.len();
    let wall_us = run_started.elapsed().as_micros().max(1);
    println!(
        "mode=concurrent threads={} inputs={} warmup={} rounds={} ops={} wall={}ms throughput={:.1}/s",
        args.threads,
        workload.len(),
        args.warmup_rounds,
        rounds,
        ops,
        run_started.elapsed().as_millis(),
        ops as f64 / (wall_us as f64 / 1_000_000.0)
    );
    samples.sort_unstable();
    let (p95, p99) = print_stats(args.json_path.as_deref(), "concurrent", &samples);
    gate_p95_p99(args, p95, p99);
}

/// 端到端命名管道往返：进程内启动引擎服务 + 真实管道消息 + 序列化 +
/// 调度队列 + 全局引擎锁。threads > 1 时多客户端并发。
fn run_e2e_pipe(args: &PerfArgs, workload: &[String]) {
    // 隔离管道名，避免连到真实运行中的引擎服务。
    let suffix = std::process::id();
    let pipe_name = format!(r"\\.\pipe\KaixinInput_PerfBench_{suffix}");
    let mutex_name = format!(r"Local\KaixinInput_PerfBench_{suffix}");
    env::set_var("SRF_ENGINE_PIPE_NAME", &pipe_name);
    env::set_var("SRF_ENGINE_MUTEX_NAME", &mutex_name);

    pinyin_ime::ipc_service::start_engine_service();
    pinyin_ime::ipc_service::warmup_shared_engine_sync();

    for _ in 0..args.warmup_rounds {
        for input in workload {
            let _ = pinyin_ime::ipc_service::engine_pipe_roundtrip(input)
                .unwrap_or_else(|err| panic!("e2e warmup roundtrip failed: {err}"));
        }
    }

    let run_started = Instant::now();
    let mut samples: Vec<Duration> = Vec::new();
    let mut total_bytes = 0u64;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..args.threads)
            .map(|thread_idx| {
                scope.spawn(move || {
                    // 每线程一条持久连接 + 递增 request_id，与真实 TSF 客户端
                    // 的交互优先级路径一致。
                    let mut client = pinyin_ime::ipc_service::EnginePipeClient::connect()
                        .unwrap_or_else(|err| panic!("e2e pipe connect failed: {err}"));
                    let mut samples = Vec::with_capacity(args.iterations);
                    let mut total_bytes = 0u64;
                    let mut request_id = 1u64;
                    for round in 0..args.iterations {
                        let input = &workload[(thread_idx + round) % workload.len()];
                        let started = Instant::now();
                        let (count, response_bytes) = client
                            .lookup(input, request_id)
                            .unwrap_or_else(|err| panic!("e2e roundtrip failed: {err}"));
                        request_id = request_id.saturating_add(1);
                        debug_assert!(count > 0);
                        total_bytes += response_bytes as u64;
                        samples.push(started.elapsed());
                    }
                    (samples, total_bytes)
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            let (mut thread_samples, thread_bytes) = handle.join().expect("e2e thread");
            samples.append(&mut thread_samples);
            total_bytes += thread_bytes;
        }
    });
    let ops = samples.len();
    let wall_us = run_started.elapsed().as_micros().max(1);
    println!(
        "mode=e2e-pipe threads={} inputs={} warmup={} rounds={} ops={} wall={}ms throughput={:.1}/s response_bytes_avg={:.1}",
        args.threads,
        workload.len(),
        args.warmup_rounds,
        args.iterations,
        ops,
        run_started.elapsed().as_millis(),
        ops as f64 / (wall_us as f64 / 1_000_000.0),
        total_bytes as f64 / ops.max(1) as f64
    );
    samples.sort_unstable();
    let (p95, p99) = print_stats(args.json_path.as_deref(), "e2e_roundtrip", &samples);
    gate_p95_p99(args, p95, p99);
}

fn print_slow_samples(args: &PerfArgs, slow_samples: &[(Duration, String)]) {
    if args.slow_limit == 0 || slow_samples.is_empty() {
        return;
    }
    let mut sorted = slow_samples.to_vec();
    sorted.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (idx, (duration, input)) in sorted.iter().take(args.slow_limit).enumerate() {
        println!("SLOW\t{}\t{}\t{}", idx + 1, duration_us(*duration), input);
    }
}

fn gate_p95_p99(args: &PerfArgs, p95: u128, p99: u128) {
    let mut failed = false;
    if let Some(limit) = args.max_p95_us {
        if p95 > limit {
            eprintln!("FAIL p95 lookup exceeded gate: actual={p95}us limit={limit}us");
            failed = true;
        }
    }
    if let Some(limit) = args.max_p99_us {
        if p99 > limit {
            eprintln!("FAIL p99 lookup exceeded gate: actual={p99}us limit={limit}us");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// 对照模式：直接走共享引擎（无管道/无调度器），用于定位 e2e 开销段。
fn run_e2e_direct(args: &PerfArgs, workload: &[String]) {
    let suffix = std::process::id();
    let pipe_name = format!(r"\\.\pipe\KaixinInput_PerfBench_{suffix}");
    let mutex_name = format!(r"Local\KaixinInput_PerfBench_{suffix}");
    env::set_var("SRF_ENGINE_PIPE_NAME", &pipe_name);
    env::set_var("SRF_ENGINE_MUTEX_NAME", &mutex_name);
    pinyin_ime::ipc_service::start_engine_service();
    pinyin_ime::ipc_service::warmup_shared_engine_sync();
    for _ in 0..args.warmup_rounds {
        for input in workload {
            let _ = pinyin_ime::ipc_service::probe_shared_engine(input)
                .unwrap_or_else(|err| panic!("direct warmup failed: {err}"));
        }
    }
    let mut samples = Vec::with_capacity(workload.len() * args.iterations);
    let run_started = Instant::now();
    for _ in 0..args.iterations {
        for input in workload {
            let started = Instant::now();
            let _ = pinyin_ime::ipc_service::probe_shared_engine(input)
                .unwrap_or_else(|err| panic!("direct lookup failed: {err}"));
            samples.push(started.elapsed());
        }
    }
    println!(
        "mode=e2e-direct inputs={} warmup={} iterations={} wall={}ms",
        workload.len(),
        args.warmup_rounds,
        args.iterations,
        run_started.elapsed().as_millis()
    );
    let (p95, p99) = print_stats(args.json_path.as_deref(), "direct", &samples);
    gate_p95_p99(args, p95, p99);
}

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    let args = parse_args();
    if let Some(path) = &args.json_path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).unwrap_or_else(|err| {
                panic!(
                    "create performance JSON directory {}: {err}",
                    parent.display()
                )
            });
        }
        fs::write(path, [])
            .unwrap_or_else(|err| panic!("initialize performance JSON {}: {err}", path.display()));
    }
    let mut inputs = args.inline_inputs.clone();
    for path in &args.cases_paths {
        inputs.extend(parse_case_inputs(path));
    }
    inputs.sort();
    inputs.dedup();

    let workload = build_workload(&inputs, args.workload_size, args.incremental);
    if workload.is_empty() {
        eprintln!("no perf inputs found");
        std::process::exit(1);
    }

    let lexicon_dir = repo.join("lexicon");
    match args.mode {
        BenchMode::Plain => {
            let mut engine = build_engine(&lexicon_dir);
            if args.startup_warmup {
                engine.warmup_interactive_paths();
            }
            run_plain(&args, &workload, &mut engine);
        }
        BenchMode::PostFeedback => {
            let mut engine = build_engine(&lexicon_dir);
            run_post_feedback(&args, &workload, &mut engine);
        }
        BenchMode::PostCommit => {
            let mut engine = build_engine(&lexicon_dir);
            run_post_commit(&args, &workload, &mut engine);
        }
        BenchMode::Concurrent => {
            let engine = build_engine(&lexicon_dir);
            run_concurrent(&args, &workload, engine);
        }
        BenchMode::E2EPipe => run_e2e_pipe(&args, &workload),
        BenchMode::E2EDirect => run_e2e_direct(&args, &workload),
    }
}
