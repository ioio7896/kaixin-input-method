use super::*;

fn recent_perf_log_lines(limit: usize) -> Vec<String> {
    let patterns = [
        "srf_engine_load",
        "srf_engine_lexicon_mode",
        "srf_engine_ensure_loaded",
        "srf_engine_full_warmup",
        "engine_helper_start",
        "srf_ipc_lookup",
        "srf_lookup_profile",
        "candidate-refresh",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
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
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}

pub(super) fn recent_compatibility_log_lines(limit: usize) -> Vec<String> {
    let patterns = [
        "compat",
        "fullscreen",
        "fallback",
        "candidateui",
        "candidate ui",
    ];
    let mut lines = runtime_log::recent_lines_matching(limit, &patterns);
    if lines.len() >= limit {
        return lines;
    }
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
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

fn latest_log_line_matching(patterns: &[&str]) -> Option<String> {
    let mut found = None;
    for path in diagnostic_log_paths() {
        let text = fs::read_to_string(&path).unwrap_or_default();
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
        let text = fs::read_to_string(&path).unwrap_or_default();
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
    let recent = recent_perf_log_lines(80).join("\n");
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

pub(super) fn build_diagnostics_snapshot(app: &SettingsApp) -> DiagnosticsSnapshot {
    DiagnosticsSnapshot {
        refreshed_at: Instant::now(),
        recovery: engine_recovery_state_summary(),
        cold_lines: cold_start_summary_lines(),
        recent_lines: recent_perf_log_lines(12),
        compat_lines: recent_compatibility_log_lines(12),
        latency_rows: typing_latency_stats(),
        foreground: app.foreground_process.clone(),
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
    quiet_section(ui, "运行状态", |ui| {
        ui.columns(2, |columns| {
            for (idx, (label, value, color)) in status_items.iter().enumerate() {
                diagnostic_status_card(&mut columns[idx % 2], label, value, *color);
                columns[idx % 2].add_space(8.0);
            }
        });
    });

    ui.add_space(10.0);
    section_panel(ui, "打字延迟统计", |ui| {
        let palette = fluent_palette(ui);
        if latency_rows.is_empty() {
            ui.label(
                RichText::new("暂无延迟样本。把日志级别临时切到“性能”，正常打字一小段后再回来看。")
                    .small()
                    .color(palette.muted),
            );
        } else {
            egui::Grid::new("typing_latency_stats_grid")
                .num_columns(6)
                .striped(true)
                .spacing([14.0, 7.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("阶段").strong().color(palette.text));
                    ui.label(RichText::new("样本").strong().color(palette.text));
                    ui.label(RichText::new("P50 ms").strong().color(palette.text));
                    ui.label(RichText::new("P90 ms").strong().color(palette.text));
                    ui.label(RichText::new("P99 ms").strong().color(palette.text));
                    ui.label(RichText::new("Max ms").strong().color(palette.text));
                    ui.end_row();
                    for row in latency_rows {
                        ui.label(RichText::new(row.label).color(palette.text));
                        ui.label(RichText::new(row.count.to_string()).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p50_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p90_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.p99_ms)).monospace());
                        ui.label(RichText::new(format_latency_ms(row.max_ms)).monospace());
                        ui.end_row();
                    }
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

    ui.add_space(10.0);
    section_panel(ui, "诊断控制", |ui| {
        let palette = fluent_palette(ui);
        setting_combo_row(
            ui,
            "日志级别",
            "off/error/basic/perf/verbose；默认 basic，性能日志建议排障时临时开启。",
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
        ui.horizontal(|ui| {
            if outline_button(ui, "打开日志").clicked() {
                app.open_data_location(diagnostic_log_dir());
            }
            if danger_button(ui, "清空日志").clicked() {
                app.clear_tsf_log();
            }
            if outline_button(ui, "导出诊断包").clicked() {
                app.export_diagnostic_package();
            }
        });
        ui.label(
            RichText::new("日志默认脱敏；排障时临时切到“性能”，完成后建议改回“基础”。")
                .small()
                .color(palette.muted),
        );
    });

    ui.add_space(10.0);
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
