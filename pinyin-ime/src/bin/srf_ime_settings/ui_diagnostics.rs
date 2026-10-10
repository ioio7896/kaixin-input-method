use super::*;

// Limit work per file even when old installations contain oversized logs.
fn read_diagnostic_log_tail(path: &Path) -> String {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_BYTES: u64 = 512 * 1024;
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let start = file
        .metadata()
        .map_or(0, |meta| meta.len().saturating_sub(MAX_BYTES));
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    if start > 0 {
        if let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=end);
        } else {
            return String::new();
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn recent_perf_log_lines(limit: usize) -> Vec<String> {
    recent_perf_log_lines_impl(limit, false)
}

fn recent_perf_log_lines_for_export(limit: usize) -> Vec<String> {
    recent_perf_log_lines_impl(limit, true)
}

fn recent_perf_log_lines_impl(limit: usize, include_files_when_full: bool) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let patterns = [
        "[perf]",
        "srf_engine_load",
        "srf_engine_lexicon_mode",
        "srf_engine_ensure_loaded",
        "srf_engine_full_warmup",
        "engine_helper_start",
        "srf_ipc_lookup",
        "srf_lookup_profile",
        "srf_candidate_stage",
        "rapidocr_warmup",
        "rapidocr_idle_release",
        "candidate-refresh",
        "tsf_bridge_failure",
        "shared helper health busy",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit && !include_files_when_full {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = read_diagnostic_log_tail(&path);
        lines.extend(
            text.lines()
                .filter(|line| patterns.iter().any(|pattern| line.contains(pattern)))
                .map(|line| {
                    format!(
                        "{}  {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        line
                    )
                }),
        );
    }
    if include_files_when_full {
        lines.sort_by(|left, right| perf_log_timestamp(left).cmp(&perf_log_timestamp(right)));
    }
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

fn perf_log_timestamp(line: &str) -> Option<&str> {
    let content = line
        .split_once('\t')
        .map(|(_, content)| content)
        .or_else(|| line.split_once("  ").map(|(_, content)| content))?;
    content.get(..23)
}

pub(super) fn recent_compatibility_log_lines(limit: usize) -> Vec<String> {
    let patterns = [
        "compat",
        "fullscreen",
        "fallback",
        "candidateui",
        "candidate ui",
        "game-chat",
        "game-key",
        "game-candidate",
        "commit-transport",
        "sent_unconfirmed",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = read_diagnostic_log_tail(&path);
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        for line in text.lines() {
            let lower = line.to_ascii_lowercase();
            let is_compat_log = file_name.eq_ignore_ascii_case("compatibility.log");
            if is_compat_log || patterns.iter().any(|pattern| lower.contains(pattern)) {
                lines.push(format!("{file_name}  {line}"));
            }
        }
    }
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

pub(super) fn game_input_event_hint(line: &str) -> Option<&'static str> {
    if line.contains("sent_unconfirmed") {
        return Some("文字事件已发送；请确认输入栏实际收到中文，避免重复提交。");
    }
    if line.contains("host_requested_hide") {
        return Some("游戏已接管候选绘制，输入法尊重宿主的隐藏要求。");
    }
    if line.contains("unconfirmed_input_field") {
        return Some("尚未确认可编辑输入栏；可以使用中文聊天切换热键。");
    }
    if line.contains("game-chat.state") && line.contains("phase=editing") {
        return Some("中文聊天已开启。");
    }
    if line.contains("game-chat.state") && line.contains("phase=passthrough") {
        return Some("中文聊天已结束，按键恢复交给游戏。");
    }
    if line.contains("game-chat.open") && line.contains("phase=awaiting") {
        return Some("正在等待游戏输入栏确认，聊天开启键已交给游戏。");
    }
    if line.contains("commit-transport") && line.contains("status=failed") {
        return Some("本次上屏失败。请检查焦点、修饰键和该游戏的提交方式。");
    }
    None
}

fn latest_log_line_matching(patterns: &[&str]) -> Option<String> {
    let mut found = None;
    for path in diagnostic_log_paths() {
        let text = read_diagnostic_log_tail(&path);
        for line in text.lines() {
            if patterns.iter().any(|pattern| line.contains(pattern)) {
                found = Some(format!(
                    "{}  {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    line
                ));
            }
        }
    }
    found
}

fn compact_diagnostic_line(line: &str, max_chars: usize) -> String {
    let compact = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let value = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{value}...")
    } else {
        value
    }
}

fn cold_start_summary_lines() -> Vec<String> {
    let probes: [(&str, &[&str]); 5] = [
        ("tray", &["engine_helper_start"]),
        ("engine", &["srf_engine_lexicon_mode"]),
        ("hot/full", &["srf_engine_full_warmup_finish"]),
        ("first lookup", &["srf_ipc_lookup"][..]),
        ("ensure", &["srf_engine_ensure_loaded"]),
    ];
    probes
        .iter()
        .filter_map(|(label, patterns)| {
            latest_log_line_matching(patterns).map(|line| format!("{label}: {line}"))
        })
        .collect()
}

#[derive(Clone)]
struct LatencyStatsRow {
    label: &'static str,
    count: usize,
    p50_ms: f64,
    p90_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

fn metric_token_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| ch.is_ascii_whitespace() || ch == ',' || ch == ';')
        .unwrap_or(rest.len());
    let token = rest[..end].trim();
    (!token.is_empty()).then_some(token)
}

fn metric_number_after(line: &str, key: &str) -> Option<f64> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let mut end = 0usize;
    for (idx, ch) in rest.char_indices() {
        if ch.is_ascii_digit() || ch == '.' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    (end > 0).then(|| rest[..end].parse::<f64>().ok()).flatten()
}

fn push_latency_sample(
    series: &mut BTreeMap<&'static str, Vec<f64>>,
    label: &'static str,
    value_ms: f64,
) {
    if value_ms.is_finite() && value_ms >= 0.0 {
        series.entry(label).or_default().push(value_ms);
    }
}

fn collect_latency_line(series: &mut BTreeMap<&'static str, Vec<f64>>, line: &str) {
    if line.contains("event=srf_ipc_lookup_write ") {
        if let Some(value) = metric_number_after(line, "write=") {
            push_latency_sample(series, "IPC 响应写入", value / 1000.0);
        }
    }
    if line.contains("[perf]") {
        if let (Some(stage), Some(elapsed_ms)) = (
            metric_token_after(line, "stage="),
            metric_number_after(line, "elapsed_ms="),
        ) {
            let label = match stage {
                "Key/WouldEat" => Some("按键预判"),
                "Key/ProcessKey" => Some("按键处理"),
                "CandidateWorker/lookup" => Some("候选查询(worker)"),
                "CandidateWorker/request-to-apply" => Some("按键到候选应用"),
                "CandidateWindow/prepare-resources" => Some("候选窗资源准备"),
                "CandidateWindow/begin-or-update" => Some("候选窗更新"),
                "CandidateWindow/total" => Some("候选窗绘制总计"),
                "CommitCandidate/text-write" => Some("候选上屏写入"),
                _ => None,
            };
            if let Some(label) = label {
                push_latency_sample(series, label, elapsed_ms);
            }
        }
    }

    if line.contains("event=srf_ipc_lookup ") {
        for (key, label) in [
            ("queue_wait=", "IPC 排队"),
            ("lock_wait=", "共享引擎等待"),
            ("init=", "IPC 初始化"),
            ("serialize=", "IPC 序列化"),
        ] {
            if let Some(value) = metric_number_after(line, key) {
                push_latency_sample(series, label, value / 1000.0);
            }
        }
        if let Some(total_us) = metric_number_after(line, "total=") {
            push_latency_sample(series, "IPC 查询总计", total_us / 1000.0);
        }
        if let Some(engine_us) = metric_number_after(line, "engine=") {
            push_latency_sample(series, "IPC 引擎内部", engine_us / 1000.0);
        }
    }
    if line.contains("event=srf_ipc_lookup_waited") || line.contains("event=srf_ipc_lookup_busy") {
        if let Some(waited_us) = metric_number_after(line, "waited_us=") {
            push_latency_sample(series, "共享引擎等待", waited_us / 1000.0);
        }
    }

    if line.contains("event=srf_lookup_profile") {
        if let Some(total_us) = metric_number_after(line, "total=") {
            push_latency_sample(series, "Rust 查询总计", total_us / 1000.0);
        }
        for (key, label) in [
            ("prepare=", "Rust 准备"),
            ("decode=", "Rust 解码"),
            ("correction=", "Rust 纠错"),
            ("rerank_sort=", "Rust 排序后处理"),
            ("finish=", "Rust 收尾"),
        ] {
            if let Some(value_us) = metric_number_after(line, key) {
                push_latency_sample(series, label, value_us / 1000.0);
            }
        }
    }
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((sorted.len() - 1) as f64 * pct).round() as usize;
    sorted[pos.min(sorted.len() - 1)]
}

fn typing_latency_stats() -> Vec<LatencyStatsRow> {
    let mut series: BTreeMap<&'static str, Vec<f64>> = BTreeMap::new();
    for path in diagnostic_log_paths() {
        let text = read_diagnostic_log_tail(&path);
        for line in text.lines() {
            collect_latency_line(&mut series, line);
        }
    }

    let order = [
        "按键预判",
        "按键处理",
        "按键到候选应用",
        "候选查询(worker)",
        "IPC 查询总计",
        "IPC 排队",
        "IPC 初始化",
        "IPC 序列化",
        "IPC 响应写入",
        "IPC 引擎内部",
        "Rust 查询总计",
        "Rust 准备",
        "Rust 解码",
        "Rust 纠错",
        "Rust 排序后处理",
        "Rust 收尾",
        "候选窗资源准备",
        "候选窗更新",
        "候选窗绘制总计",
        "候选上屏写入",
        "共享引擎等待",
    ];

    let mut rows = Vec::new();
    for label in order {
        let Some(mut values) = series.remove(label) else {
            continue;
        };
        values.sort_by(|a, b| a.total_cmp(b));
        let count = values.len();
        rows.push(LatencyStatsRow {
            label,
            count,
            p50_ms: percentile(&values, 0.50),
            p90_ms: percentile(&values, 0.90),
            p95_ms: percentile(&values, 0.95),
            p99_ms: percentile(&values, 0.99),
            max_ms: values.last().copied().unwrap_or_default(),
        });
    }
    rows
}

fn format_latency_ms(value: f64) -> String {
    if value < 1.0 {
        format!("{value:.2}")
    } else if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

fn diagnostic_sqlite_io_error(err: rusqlite::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, err)
}

fn write_typing_latency_summary_sqlite(
    path: &Path,
    rows: &[LatencyStatsRow],
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut conn = rusqlite::Connection::open(path).map_err(diagnostic_sqlite_io_error)?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS typing_latency_summary (
           label TEXT PRIMARY KEY,
           count INTEGER NOT NULL,
           p50_ms REAL NOT NULL,
           p90_ms REAL NOT NULL,
           p99_ms REAL NOT NULL,
           max_ms REAL NOT NULL
         );",
    )
    .map_err(diagnostic_sqlite_io_error)?;
    let tx = conn.transaction().map_err(diagnostic_sqlite_io_error)?;
    tx.execute("DELETE FROM typing_latency_summary", [])
        .map_err(diagnostic_sqlite_io_error)?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO typing_latency_summary
                 (label, count, p50_ms, p90_ms, p99_ms, max_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(diagnostic_sqlite_io_error)?;
        for row in rows {
            stmt.execute(rusqlite::params![
                redact_diagnostic_text(row.label),
                row.count as i64,
                row.p50_ms,
                row.p90_ms,
                row.p99_ms,
                row.max_ms
            ])
            .map_err(diagnostic_sqlite_io_error)?;
        }
    }
    tx.commit().map_err(diagnostic_sqlite_io_error)
}

fn file_modified_summary(path: &Path) -> String {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(|modified| {
            let local: chrono::DateTime<chrono::Local> = modified.into();
            local.to_rfc3339()
        })
        .unwrap_or_else(|_| "none".to_string())
}

fn diagnostic_redaction_values() -> Vec<(String, &'static str)> {
    let mut values = Vec::new();
    let mut push_value = |value: String, token: &'static str| {
        let trimmed = value.trim().to_string();
        if !trimmed.is_empty() && !values.iter().any(|(existing, _)| existing == &trimmed) {
            values.push((trimmed, token));
        }
    };
    if let Some(path) = app_paths::local_data_dir() {
        push_value(path.display().to_string(), "<DATA_DIR>");
    }
    for (name, token) in [
        ("USERPROFILE", "<USERPROFILE>"),
        ("LOCALAPPDATA", "<LOCALAPPDATA>"),
        ("APPDATA", "<APPDATA>"),
        ("TEMP", "<TEMP>"),
        ("TMP", "<TEMP>"),
        ("COMPUTERNAME", "<COMPUTERNAME>"),
        ("USERNAME", "<USERNAME>"),
    ] {
        if let Ok(value) = std::env::var(name) {
            push_value(value, token);
        }
    }
    values.sort_by(|left, right| right.0.len().cmp(&left.0.len()));
    values
}

fn redact_diagnostic_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for (value, token) in diagnostic_redaction_values() {
        redacted = redacted.replace(&value, token);
    }
    redacted
}

fn append_clipboard_store_summary(summary: &mut String) {
    let path = pinyin_ime::clipboard_store::store_path();
    let exists = path.is_file();
    let bytes = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    summary.push_str("clipboard_store_path=<DATA_DIR>\\clipboard_store.sqlite\n");
    summary.push_str(&format!("clipboard_store_exists={}\n", exists));
    summary.push_str(&format!("clipboard_store_bytes={}\n", bytes));
    summary.push_str(&format!(
        "clipboard_store_modified={}\n",
        file_modified_summary(&path)
    ));
    match pinyin_ime::clipboard_store::snapshot() {
        Ok(snapshot) => {
            summary.push_str(&format!(
                "clipboard_history_count={}\nclipboard_pinned_count={}\n",
                snapshot.history.len(),
                snapshot.pinned.len()
            ));
        }
        Err(err) => {
            summary.push_str(&format!("clipboard_snapshot_error={err}\n"));
        }
    }
}

fn process_running_summary(name: &str) -> String {
    #[cfg(windows)]
    {
        let filter = format!("IMAGENAME eq {name}");
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(&filter)
            .arg("/NH")
            .output();
        return match output {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                if text.contains(&name.to_ascii_lowercase()) {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Err(err) => format!("unknown:{err}"),
        };
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        "unknown".to_string()
    }
}

fn recent_runtime_event_lines(limit: usize) -> Vec<String> {
    let sqlite_lines = runtime_log::recent_event_lines(limit);
    if sqlite_lines.len() >= limit {
        return sqlite_lines;
    }
    let mut lines: Vec<(String, String)> = sqlite_lines
        .into_iter()
        .map(|line| (line.clone(), line))
        .collect();
    for path in runtime_log::current_log_paths() {
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "runtime.log".to_string());
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines().rev().take(limit) {
            let clipped: String = line.chars().take(2000).collect();
            lines.push((line.to_string(), format!("{label}\t{clipped}")));
        }
    }
    lines.sort_by(|left, right| left.0.cmp(&right.0));
    let selected = if lines.len() > limit {
        lines.split_off(lines.len() - limit)
    } else {
        lines
    };
    selected.into_iter().map(|(_, line)| line).collect()
}

pub(crate) fn export_diagnostic_package_to(
    dest: &Path,
    config_path: &Path,
    model: &SettingsModel,
) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    fs::create_dir_all(dest.join("logs"))?;

    let mut summary = String::new();
    summary.push_str("Kaixin IME diagnostics\n");
    summary.push_str(&format!("created={}\n", chrono::Local::now().to_rfc3339()));
    summary.push_str(&format!("version={}\n", env!("CARGO_PKG_VERSION")));
    summary.push_str("config_path=<DATA_DIR>\\kaixin.ini\n");
    summary.push_str(&format!("config_exists={}\n", config_path.is_file()));
    summary.push_str(&format!(
        "config_modified={}\n",
        file_modified_summary(config_path)
    ));
    summary.push_str("data_dir=<DATA_DIR>\\logs\n");
    summary.push_str(&format!("log_level={}\n", model.log_level.trim()));
    summary.push_str(&format!(
        "clipboard_background_enabled={}\n",
        model.clipboard_background_enabled
    ));
    summary.push_str(&format!(
        "privacy_never_learn_process_count={}\n",
        model.privacy_never_learn_processes.len()
    ));
    summary.push_str(&format!(
        "privacy_never_clipboard_process_count={}\n",
        model.privacy_never_clipboard_processes.len()
    ));
    summary.push_str(&format!(
        "privacy_never_candidate_process_count={}\n",
        model.privacy_never_candidate_processes.len()
    ));
    summary.push_str(&format!("mixed_pinyin={}\n", model.mixed_pinyin));
    append_clipboard_store_summary(&mut summary);
    summary.push_str(&format!(
        "process_srf_ime_tray_running={}\n",
        process_running_summary("srf_ime_tray.exe")
    ));
    summary.push_str(&format!(
        "process_srf_ime_engine_running={}\n",
        process_running_summary("srf_ime_engine.exe")
    ));
    summary.push_str("\nlogs:\n");

    for path in diagnostic_log_paths() {
        let exists = path.is_file();
        let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        summary.push_str(&format!(
            "- {} exists={} bytes={}\n",
            redact_diagnostic_text(&path.display().to_string()),
            exists,
            size
        ));
        if exists {
            let file_name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "log.txt".to_string());
            let mut dest_name = file_name.clone();
            let mut idx = 2usize;
            while dest.join("logs").join(&dest_name).exists() {
                dest_name = format!("{idx}-{file_name}");
                idx += 1;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                let _ = fs::write(
                    dest.join("logs").join(dest_name),
                    redact_diagnostic_text(&text),
                );
            }
        }
    }

    fs::write(dest.join("summary.txt"), redact_diagnostic_text(&summary))?;
    let recent_events = recent_runtime_event_lines(20).join("\n");
    fs::write(
        dest.join("recent-events.log"),
        redact_diagnostic_text(&recent_events),
    )?;
    let recent = recent_perf_log_lines_for_export(80).join("\n");
    fs::write(
        dest.join("recent-perf.log"),
        redact_diagnostic_text(&recent),
    )?;
    let compat = recent_compatibility_log_lines(80).join("\n");
    fs::write(
        dest.join("recent-compatibility.log"),
        redact_diagnostic_text(&compat),
    )?;
    let latency_rows = typing_latency_stats();
    write_typing_latency_summary_sqlite(
        &dest.join("typing-latency-summary.sqlite"),
        &latency_rows,
    )?;
    Ok(())
}

pub(crate) fn export_performance_log_to(
    dest: &Path,
    current_log_level: &str,
) -> std::io::Result<usize> {
    const PERFORMANCE_LOG_LIMIT: usize = 5_000;
    let events = recent_perf_log_lines_for_export(PERFORMANCE_LOG_LIMIT);
    let mut output = String::new();
    output.push_str("Kaixin IME performance log\n");
    output.push_str(&format!("created={}\n", chrono::Local::now().to_rfc3339()));
    output.push_str(&format!("version={}\n", env!("CARGO_PKG_VERSION")));
    output.push_str(&format!(
        "arch={}\nlogical_cpus={}\n",
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, |count| count.get())
    ));
    output.push_str(&format!("log_level={}\n", current_log_level.trim()));
    output.push_str("\nlatency_summary_ms (recent local log samples)\n");
    output.push_str("stage\tsamples\tp50\tp90\tp95\tp99\tmax\n");
    for row in typing_latency_stats()
        .into_iter()
        .filter(|row| row.count > 0)
    {
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.label,
            row.count,
            format_latency_ms(row.p50_ms),
            format_latency_ms(row.p90_ms),
            format_latency_ms(row.p95_ms),
            format_latency_ms(row.p99_ms),
            format_latency_ms(row.max_ms)
        ));
    }
    output.push_str("\nperformance_events\n");
    if events.is_empty() {
        output.push_str(
            "No performance events were found. Set the settings log level to 'perf', reproduce the issue, then export again.\n",
        );
    } else {
        for event in &events {
            output.push_str(event);
            output.push('\n');
        }
    }
    fs::write(dest, redact_diagnostic_text(&output))?;
    Ok(events.len())
}

fn log_level_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" => "关闭",
        "error" => "错误",
        "perf" => "性能",
        "verbose" => "详细",
        _ => "基础",
    }
}

pub(super) fn learning_sensitivity_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "conservative" => "保守",
        "aggressive" => "积极",
        _ => "标准",
    }
}

pub(super) fn user_hotword_boost_label(value: &str) -> &'static str {
    match value.trim().to_ascii_lowercase().as_str() {
        "conservative" => "保守",
        "strong" => "强",
        "aggressive" => "积极",
        _ => "标准",
    }
}

fn engine_recovery_state_summary() -> Option<String> {
    let reason = read_state_string("LastEngineRecoveryReason")?;
    let time =
        read_state_string("LastEngineRecoveryTime").unwrap_or_else(|| "时间未知".to_string());
    Some(format!("{time}  {reason}"))
}

#[cfg(windows)]
fn read_state_string(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let subkey: Vec<u16> = r"Software\kaixin\State"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0 || bytes <= 2 {
        return None;
    }

    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    if let Some(pos) = buffer.iter().position(|unit| *unit == 0) {
        buffer.truncate(pos);
    }
    let text = String::from_utf16_lossy(&buffer).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(not(windows))]
fn read_state_string(_name: &str) -> Option<String> {
    None
}

#[derive(Clone)]
pub(crate) struct DiagnosticsSnapshot {
    pub(super) refreshed_at: Instant,
    recovery: Option<String>,
    cold_lines: Vec<String>,
    recent_lines: Vec<String>,
    compat_lines: Vec<String>,
    latency_rows: Vec<LatencyStatsRow>,
    foreground: Option<ProcessSuggestion>,
    latest_candidate_refresh: Option<String>,
}

pub(super) fn build_diagnostics_snapshot(
    foreground: Option<ProcessSuggestion>,
) -> DiagnosticsSnapshot {
    DiagnosticsSnapshot {
        refreshed_at: Instant::now(),
        recovery: engine_recovery_state_summary(),
        cold_lines: cold_start_summary_lines(),
        recent_lines: recent_perf_log_lines(12),
        compat_lines: recent_compatibility_log_lines(12),
        latency_rows: typing_latency_stats(),
        foreground,
        latest_candidate_refresh: latest_log_line_matching(&["candidate-refresh"]),
    }
}

pub(super) fn diagnostics_ui(ui: &mut egui::Ui, app: &mut SettingsApp) {
    let Some(snapshot) = app.diagnostics_cache.as_ref().cloned() else {
        ui.label("正在读取诊断数据…");
        return;
    };
    let recovery = snapshot.recovery.as_ref();
    let cold_lines = &snapshot.cold_lines;
    let recent_lines = &snapshot.recent_lines;
    let compat_lines = &snapshot.compat_lines;
    let latency_rows = &snapshot.latency_rows;
    let foreground = snapshot.foreground.as_ref();
    let foreground_policy = foreground.and_then(|process| {
        matching_compat_rule(&app.model.compat_rules, &process.name).map(|rule| {
            if rule.enabled {
                rule.policy.label().to_string()
            } else {
                "规则已停用".to_string()
            }
        })
    });
    let latest_candidate_refresh = snapshot.latest_candidate_refresh.as_ref();
    let palette = fluent_palette(ui);
    let engine_value = recovery
        .map(|value| compact_diagnostic_line(value, 52))
        .unwrap_or_else(|| "正常".to_string());
    let foreground_value = foreground
        .map(|process| process.name.clone())
        .unwrap_or_else(|| "未获取".to_string());
    let policy_value = foreground_policy
        .clone()
        .unwrap_or_else(|| "未命中自定义规则".to_string());
    let performance_value = if recent_lines.is_empty() {
        "暂无性能事件".to_string()
    } else {
        format!("最近 {} 条", recent_lines.len())
    };
    let latency_value = if latency_rows.is_empty() {
        "暂无样本".to_string()
    } else {
        format!("{} 项指标", latency_rows.len())
    };
    let refresh_value = latest_candidate_refresh
        .map(String::as_str)
        .map(|line| compact_diagnostic_line(line, 52))
        .unwrap_or_else(|| "暂无刷新耗时".to_string());
    let compat_value = if compat_lines.is_empty() {
        "暂无记录".to_string()
    } else {
        format!("最近 {} 条", compat_lines.len())
    };
    let cold_value = if cold_lines.is_empty() {
        "暂无摘要".to_string()
    } else {
        format!("{} 条摘要", cold_lines.len())
    };
    let status_items = [
        (
            "引擎",
            engine_value.as_str(),
            if recovery.is_some() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "前台进程",
            foreground_value.as_str(),
            if foreground.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "兼容策略",
            policy_value.as_str(),
            if foreground_policy.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "性能日志",
            performance_value.as_str(),
            if recent_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "延迟统计",
            latency_value.as_str(),
            if latency_rows.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "候选刷新",
            refresh_value.as_str(),
            if latest_candidate_refresh.is_some() {
                palette.success
            } else {
                palette.warning
            },
        ),
        (
            "兼容降级",
            compat_value.as_str(),
            if compat_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
        (
            "冷启动",
            cold_value.as_str(),
            if cold_lines.is_empty() {
                palette.warning
            } else {
                palette.success
            },
        ),
    ];
    section_panel(ui, "运行状态", |ui| {
        diagnostic_status_table(ui, &status_items)
    });

    ui.add_space(2.0);
    section_panel(ui, "打字延迟统计", |ui| {
        let palette = fluent_palette(ui);
        if latency_rows.is_empty() {
            ui.label(
                RichText::new("暂无延迟样本。把日志级别临时切到“性能”，正常打字一小段后再回来看。")
                    .small()
                    .color(palette.muted),
            );
        } else {
            egui::ScrollArea::horizontal()
                .id_salt("latency_table_scroll")
                .show(ui, |ui| {
                    egui::Grid::new("typing_latency_stats_grid")
                        .num_columns(7)
                        .striped(true)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("阶段").strong().color(palette.text));
                            ui.label(RichText::new("样本").strong().color(palette.text));
                            ui.label(RichText::new("P50 ms").strong().color(palette.text));
                            ui.label(RichText::new("P90 ms").strong().color(palette.text));
                            ui.label(RichText::new("P95 ms").strong().color(palette.text));
                            ui.label(RichText::new("P99 ms").strong().color(palette.text));
                            ui.label(RichText::new("Max ms").strong().color(palette.text));
                            ui.end_row();
                            for row in latency_rows {
                                ui.label(RichText::new(row.label).color(palette.text));
                                ui.label(RichText::new(row.count.to_string()).monospace());
                                ui.label(RichText::new(format_latency_ms(row.p50_ms)).monospace());
                                ui.label(RichText::new(format_latency_ms(row.p90_ms)).monospace());
                                ui.label(RichText::new(format_latency_ms(row.p95_ms)).monospace());
                                ui.label(RichText::new(format_latency_ms(row.p99_ms)).monospace());
                                ui.label(RichText::new(format_latency_ms(row.max_ms)).monospace());
                                ui.end_row();
                            }
                        });
                });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "统计来自最近的 TSF / engine 日志；输入内容已脱敏，只保留耗时和样本数量。",
                )
                .small()
                .color(palette.muted),
            );
        }
    });

    ui.add_space(2.0);
    section_panel(ui, "诊断控制", |ui| {
        let palette = fluent_palette(ui);
        setting_combo_row(
            ui,
            "日志级别",
            "off/error/basic/perf/verbose；性能日志建议排障时临时开启。",
            log_level_label(&app.model.log_level).to_string(),
            "diagnostic_log_level",
            |ui| {
                selectable_string(ui, &mut app.model.log_level, "off", "关闭");
                selectable_string(ui, &mut app.model.log_level, "error", "错误");
                selectable_string(ui, &mut app.model.log_level, "basic", "基础");
                selectable_string(ui, &mut app.model.log_level, "perf", "性能");
                selectable_string(ui, &mut app.model.log_level, "verbose", "详细");
            },
        );
        ui.horizontal_wrapped(|ui| {
            if outline_button(ui, "打开日志").clicked() {
                app.open_data_location(diagnostic_log_dir());
            }
            if danger_button(ui, "清空日志").clicked() {
                app.clear_tsf_log();
            }
            if outline_button(ui, "导出诊断包").clicked() {
                app.export_diagnostic_package();
            }
            if outline_button(ui, "导出性能日志").clicked() {
                app.export_performance_log();
            }
        });
        ui.label(
            RichText::new(
                "日志默认脱敏；导出仅收集性能事件和延迟统计。排障时切到“性能”并保存设置，复现后导出，完成后建议改回“基础”。",
            )
                .small()
                .color(palette.muted),
        );
    });

    ui.add_space(2.0);
    section_panel(ui, "最近事件", |ui| {
        let palette = fluent_palette(ui);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("最近 12 条诊断事件")
                    .size(SETTINGS_FONT_SMALL)
                    .color(palette.muted),
            );
            if recent_lines.is_empty() && compat_lines.is_empty() && cold_lines.is_empty() {
                ui.label(RichText::new("暂无记录").small().color(palette.muted));
            }
        });
        ui.add_space(8.0);
        egui::Frame::none()
            .fill(palette.surface_alt)
            .rounding(6.0)
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("recent_diagnostic_events")
                    .max_height(260.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if !cold_lines.is_empty() {
                            diagnostic_log_group(ui, "冷启动摘要", &cold_lines);
                        }
                        if !compat_lines.is_empty() {
                            diagnostic_log_group(ui, "兼容 / 降级", &compat_lines);
                        }
                        if !recent_lines.is_empty() {
                            diagnostic_log_group(ui, "性能事件", &recent_lines);
                        }
                        if recent_lines.is_empty()
                            && compat_lines.is_empty()
                            && cold_lines.is_empty()
                        {
                            ui.label(RichText::new("暂无性能或兼容事件。").color(palette.muted));
                        }
                    });
            });
    });
}

fn diagnostic_log_group(ui: &mut egui::Ui, title: &str, lines: &[String]) {
    let palette = fluent_palette(ui);
    ui.label(
        RichText::new(title)
            .strong()
            .size(SETTINGS_FONT_SMALL)
            .color(palette.text),
    );
    ui.add_space(4.0);
    for line in lines {
        ui.add(
            egui::Label::new(
                RichText::new(line)
                    .monospace()
                    .size(SETTINGS_FONT_LOG)
                    .color(palette.text),
            )
            .wrap(),
        );
        ui.add_space(2.0);
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn compact_diagnostics_fit_narrow_windows_with_long_events_and_latency_rows() {
        use super::super::layout_tests::{footer_test_app, with_layout};
        for width in [320.0, 480.0, 920.0] {
            for scale in [1.0, 1.5, 2.0] {
                let mut app = footer_test_app();
                app.diagnostics_cache = Some(DiagnosticsSnapshot {
                    refreshed_at: Instant::now(),
                    recovery: Some(
                        "示例：正在等待候选引擎恢复；这一行包含较长状态说明。".repeat(3),
                    ),
                    cold_lines: vec!["示例冷启动摘要".repeat(8)],
                    recent_lines: vec!["示例性能事件 elapsed_ms=2.0".repeat(8)],
                    compat_lines: vec!["示例兼容事件 unconfirmed_input_field".repeat(8)],
                    latency_rows: vec![LatencyStatsRow {
                        label: "按键到候选应用",
                        count: 100,
                        p50_ms: 1.0,
                        p90_ms: 2.0,
                        p95_ms: 3.0,
                        p99_ms: 4.0,
                        max_ms: 5.0,
                    }],
                    foreground: None,
                    latest_candidate_refresh: None,
                });
                let model = app.model.clone();
                with_layout(width, 4.0, scale, |ui| {
                    let bounds = ui.available_rect_before_wrap();
                    let rect = ui.scope(|ui| diagnostics_ui(ui, &mut app)).response.rect;
                    assert!(rect.left() >= bounds.left() - 1.0 && rect.right() <= bounds.right() + 1.0,
                        "diagnostics overflow at width {width}, scale {scale}: {rect:?} / {bounds:?}");
                    assert!(model == app.model);
                    assert!(app.performance_export_rx.is_none());
                });
            }
        }
    }

    #[test]
    fn ipc_timings_keep_microseconds_and_write_stage_separate() {
        let mut series = BTreeMap::new();
        collect_latency_line(&mut series,
            "event=srf_ipc_lookup request_id=1 queue_wait=2000us lock_wait=3000us init=0us serialize=500us engine=4000us total=9500us");
        collect_latency_line(&mut series, "event=srf_ipc_lookup_write write=1500us");
        assert_eq!(series["IPC 排队"], vec![2.0]);
        assert_eq!(series["共享引擎等待"], vec![3.0]);
        assert_eq!(series["IPC 序列化"], vec![0.5]);
        assert_eq!(series["IPC 查询总计"], vec![9.5]);
        assert_eq!(series["IPC 响应写入"], vec![1.5]);
    }

    #[test]
    fn invalid_latency_samples_are_excluded() {
        let mut series = BTreeMap::new();
        push_latency_sample(&mut series, "stage", f64::NAN);
        push_latency_sample(&mut series, "stage", -1.0);
        push_latency_sample(&mut series, "stage", f64::INFINITY);
        assert!(series.is_empty());
    }
}
