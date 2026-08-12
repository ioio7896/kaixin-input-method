use crate::app_paths;
use chrono::{DateTime, Local};
use fs2::FileExt;
use rusqlite::{params, Connection};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const LOG_MAX_BYTES: u64 = 1024 * 1024;
pub const LOG_ROTATE_KEEP: usize = 5;
const EVENT_DB_NAME: &str = "runtime_events.sqlite";
const EVENT_FIELD_MAGIC: &[u8] = b"KXLOG-DPAPI-1\n";
const EVENT_DB_MAX_ROWS: i64 = 10_000;
const EVENT_DB_PRUNE_EVERY_ROWS: usize = 1_024;
const LOG_QUEUE_CAPACITY: usize = 4_096;
const LOG_BATCH_MAX: usize = 512;
const LOG_BATCH_INTERVAL: Duration = Duration::from_millis(100);
const LOG_LEVEL_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeLogLevel {
    Off,
    Error,
    Basic,
    Perf,
    Verbose,
}

impl RuntimeLogLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" => Some(Self::Off),
            "error" | "err" => Some(Self::Error),
            "basic" | "info" | "1" | "true" | "on" => Some(Self::Basic),
            "perf" | "performance" => Some(Self::Perf),
            "verbose" | "debug" | "trace" => Some(Self::Verbose),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Basic => "basic",
            Self::Perf => "perf",
            Self::Verbose => "verbose",
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Error => 1,
            Self::Basic => 2,
            Self::Perf => 3,
            Self::Verbose => 4,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Error,
            2 => Self::Basic,
            3 => Self::Perf,
            4 => Self::Verbose,
            _ => Self::Basic,
        }
    }
}

static CONFIGURED_LEVEL: OnceLock<AtomicU8> = OnceLock::new();
static LOG_LEVEL_WATCHER_STARTED: OnceLock<()> = OnceLock::new();

fn read_configured_level_uncached() -> RuntimeLogLevel {
    if let Some(level) = std::env::var_os("SRF_IME_LOG_LEVEL")
        .and_then(|value| RuntimeLogLevel::parse(&value.to_string_lossy()))
    {
        return level;
    }

    app_paths::config_ini_path()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| read_ini_value(&text, "diagnostics", "log_level"))
        .and_then(|value| RuntimeLogLevel::parse(&value))
        .unwrap_or(RuntimeLogLevel::Error)
}

fn config_file_stamp(path: Option<&Path>) -> Option<(SystemTime, u64)> {
    let metadata = fs::metadata(path?).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

fn configured_level_cache() -> &'static AtomicU8 {
    let cache =
        CONFIGURED_LEVEL.get_or_init(|| AtomicU8::new(read_configured_level_uncached().to_u8()));
    LOG_LEVEL_WATCHER_STARTED.get_or_init(|| {
        let cache: &'static AtomicU8 = cache;
        let _ = thread::Builder::new()
            .name("kaixin-log-level".to_string())
            .spawn(move || {
                let mut last_path = app_paths::config_ini_path();
                let mut last_stamp = config_file_stamp(last_path.as_deref());
                loop {
                    thread::sleep(LOG_LEVEL_REFRESH_INTERVAL);
                    if let Some(level) = std::env::var_os("SRF_IME_LOG_LEVEL")
                        .and_then(|value| RuntimeLogLevel::parse(&value.to_string_lossy()))
                    {
                        cache.store(level.to_u8(), Ordering::Release);
                        continue;
                    }

                    let path = app_paths::config_ini_path();
                    let stamp = config_file_stamp(path.as_deref());
                    if path != last_path || stamp != last_stamp {
                        cache.store(read_configured_level_uncached().to_u8(), Ordering::Release);
                        last_path = path;
                        last_stamp = stamp;
                    }
                }
            });
    });
    cache
}

pub fn configured_level() -> RuntimeLogLevel {
    RuntimeLogLevel::from_u8(configured_level_cache().load(Ordering::Acquire))
}

pub fn enabled(level: RuntimeLogLevel) -> bool {
    let configured = configured_level();
    configured != RuntimeLogLevel::Off && level <= configured
}

pub fn perf_enabled() -> bool {
    enabled(RuntimeLogLevel::Perf)
}

pub fn log_engine(level: RuntimeLogLevel, event: &str, message: impl AsRef<str>) {
    log("engine.log", "engine", level, event, message);
}

pub fn log_tray(level: RuntimeLogLevel, event: &str, message: impl AsRef<str>) {
    log("tray.log", "tray", level, event, message);
}

pub fn log_ocr(level: RuntimeLogLevel, event: &str, message: impl AsRef<str>) {
    log("ocr.log", "ocr", level, event, message);
}

pub fn log_clipboard(level: RuntimeLogLevel, event: &str, message: impl AsRef<str>) {
    log("clipboard.log", "clipboard", level, event, message);
}

pub fn config_diagnostics_fields() -> String {
    let config_path = app_paths::config_ini_path();
    let config_exists = config_path.as_deref().is_some_and(Path::is_file);
    let log_dir = app_paths::log_dir();
    format!(
        "config_path={} config_exists={} log_level={} log_dir={}",
        config_path
            .as_deref()
            .map(path_for_log)
            .unwrap_or_else(|| "(none)".to_string()),
        if config_exists { 1 } else { 0 },
        configured_level().as_str(),
        log_dir
            .as_deref()
            .map(path_for_log)
            .unwrap_or_else(|| "(none)".to_string())
    )
}

pub fn input_fingerprint(input: &str) -> String {
    format!("input_units={}", input.encode_utf16().count())
}

pub fn known_log_files() -> [&'static str; 5] {
    [
        "tsf.log",
        "engine.log",
        "tray.log",
        "ocr.log",
        "clipboard.log",
    ]
}

pub fn current_log_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = app_paths::log_dir() {
        for name in known_log_files() {
            paths.push(dir.join(name));
        }
    }
    paths
}

pub fn log_dir() -> Option<PathBuf> {
    app_paths::log_dir()
}

pub fn event_store_path() -> Option<PathBuf> {
    app_paths::local_data_dir().map(|dir| dir.join(EVENT_DB_NAME))
}

pub fn recent_event_lines(limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    query_recent_events(limit, |_| true)
        .into_iter()
        .map(|event| format!("{}\t{}", EVENT_DB_NAME, event.line))
        .collect()
}

pub fn recent_lines_matching(limit: usize, patterns: &[&str]) -> Vec<String> {
    if limit == 0 || patterns.is_empty() {
        return Vec::new();
    }
    query_recent_events(limit.saturating_mul(8).max(limit), |event| {
        let lower = event.line.to_ascii_lowercase();
        patterns
            .iter()
            .any(|pattern| lower.contains(&pattern.to_ascii_lowercase()))
    })
    .into_iter()
    .rev()
    .take(limit)
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .map(|event| format!("{}\t{}", EVENT_DB_NAME, event.line))
    .collect()
}

pub fn recent_process_names(limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for event in query_recent_events(limit.saturating_mul(16).max(limit), |_| true)
        .into_iter()
        .rev()
    {
        for key in ["process=", "app="] {
            if let Some(name) = extract_event_field(&event.line, key) {
                if !out.iter().any(|existing| existing == &name) {
                    out.push(name);
                }
            }
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

#[derive(Clone)]
struct PendingLog {
    sequence: u64,
    created_at: SystemTime,
    thread_id: thread::ThreadId,
    file_name: &'static str,
    component: &'static str,
    level: RuntimeLogLevel,
    event: String,
    message: String,
}

#[derive(Default)]
struct LogQueueState {
    queue: VecDeque<PendingLog>,
    next_sequence: u64,
    flushed_sequence: u64,
    dropped_perf: u64,
}

struct AsyncLogQueue {
    state: Mutex<LogQueueState>,
    wake: Condvar,
}

static ASYNC_LOG_QUEUE: OnceLock<AsyncLogQueue> = OnceLock::new();
static ASYNC_LOG_WORKER_STARTED: OnceLock<()> = OnceLock::new();

fn async_log_queue() -> &'static AsyncLogQueue {
    let queue = ASYNC_LOG_QUEUE.get_or_init(|| AsyncLogQueue {
        state: Mutex::new(LogQueueState::default()),
        wake: Condvar::new(),
    });
    ASYNC_LOG_WORKER_STARTED.get_or_init(|| {
        let queue: &'static AsyncLogQueue = queue;
        let _ = thread::Builder::new()
            .name("kaixin-runtime-log".to_string())
            .spawn(move || async_log_worker(queue));
    });
    queue
}

fn log(
    file_name: &'static str,
    component: &'static str,
    level: RuntimeLogLevel,
    event: &str,
    message: impl AsRef<str>,
) {
    if !enabled(level) {
        return;
    }

    let queue = async_log_queue();
    let Ok(mut state) = queue.state.lock() else {
        return;
    };
    if state.queue.len() >= LOG_QUEUE_CAPACITY {
        if level >= RuntimeLogLevel::Perf {
            state.dropped_perf = state.dropped_perf.saturating_add(1);
            return;
        }
        if let Some(index) = state
            .queue
            .iter()
            .position(|pending| pending.level >= RuntimeLogLevel::Perf)
        {
            state.queue.remove(index);
            state.dropped_perf = state.dropped_perf.saturating_add(1);
        } else {
            state.queue.pop_front();
        }
    }
    state.next_sequence = state.next_sequence.saturating_add(1);
    let sequence = state.next_sequence;
    state.queue.push_back(PendingLog {
        sequence,
        created_at: SystemTime::now(),
        thread_id: thread::current().id(),
        file_name,
        component,
        level,
        event: event.to_string(),
        message: message.as_ref().trim().to_string(),
    });
    if state.queue.len() >= LOG_BATCH_MAX || level <= RuntimeLogLevel::Error {
        queue.wake.notify_one();
    }
}

pub fn flush_pending_logs(timeout: Duration) -> bool {
    let Some(queue) = ASYNC_LOG_QUEUE.get() else {
        return true;
    };
    let Ok(state) = queue.state.lock() else {
        return false;
    };
    let target = state.next_sequence;
    if state.flushed_sequence >= target {
        return true;
    }
    queue.wake.notify_one();
    let deadline = Instant::now() + timeout;
    let mut state = state;
    while state.flushed_sequence < target {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok((next, wait_result)) = queue.wake.wait_timeout(state, remaining) else {
            return false;
        };
        state = next;
        if wait_result.timed_out() {
            break;
        }
    }
    state.flushed_sequence >= target
}

fn async_log_worker(queue: &'static AsyncLogQueue) {
    let mut event_connection: Option<Connection> = None;
    let mut rows_since_prune = 0usize;
    loop {
        let batch = {
            let Ok(state) = queue.state.lock() else {
                thread::sleep(LOG_BATCH_INTERVAL);
                continue;
            };
            let Ok((mut state, _)) = queue.wake.wait_timeout(state, LOG_BATCH_INTERVAL) else {
                continue;
            };
            if state.queue.is_empty() {
                continue;
            }
            let count = state.queue.len().min(LOG_BATCH_MAX);
            state.queue.drain(..count).collect::<Vec<_>>()
        };

        let last_sequence = batch.last().map(|pending| pending.sequence).unwrap_or(0);
        flush_log_batch(&batch, &mut event_connection, &mut rows_since_prune);

        if let Ok(mut state) = queue.state.lock() {
            state.flushed_sequence = state.flushed_sequence.max(last_sequence);
            queue.wake.notify_all();
        }
    }
}

struct PreparedLog<'a> {
    pending: &'a PendingLog,
    created_at: DateTime<Local>,
    line: String,
}

fn flush_log_batch(
    batch: &[PendingLog],
    event_connection: &mut Option<Connection>,
    rows_since_prune: &mut usize,
) {
    if batch.is_empty() {
        return;
    }

    let prepared = batch
        .iter()
        .map(|pending| {
            let created_at = DateTime::<Local>::from(pending.created_at);
            let line = format_log_line(pending, &created_at);
            PreparedLog {
                pending,
                created_at,
                line,
            }
        })
        .collect::<Vec<_>>();

    // On Windows the queryable event database stores protected fields below.
    // Avoid creating a second plaintext copy in component .log files.
    if !cfg!(windows) {
        let mut by_file: BTreeMap<&'static str, Vec<&str>> = BTreeMap::new();
        for item in &prepared {
            by_file
                .entry(item.pending.file_name)
                .or_default()
                .push(item.line.as_str());
        }
        for (file_name, lines) in by_file {
            if let Some(path) = app_paths::log_file(file_name) {
                let _ = append_lines(&path, &lines);
            }
        }
    }

    let Some(path) = event_store_path() else {
        return;
    };
    if event_connection.is_none() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        *event_connection = open_event_connection(&path).ok();
        if event_connection.is_some() {
            *rows_since_prune = EVENT_DB_PRUNE_EVERY_ROWS;
        }
    }
    let Some(connection) = event_connection.as_mut() else {
        return;
    };
    let should_prune = rows_since_prune.saturating_add(prepared.len()) >= EVENT_DB_PRUNE_EVERY_ROWS;
    match append_events_batch(connection, &prepared, should_prune) {
        Ok(()) => {
            if should_prune {
                *rows_since_prune = 0;
            } else {
                *rows_since_prune = rows_since_prune.saturating_add(prepared.len());
            }
        }
        Err(_) => {
            *event_connection = None;
        }
    }
}

fn format_log_line(pending: &PendingLog, created_at: &DateTime<Local>) -> String {
    format!(
        "{} [{}] component={} event={} pid={} tid={:?} version={} {}",
        created_at.format("%Y-%m-%d %H:%M:%S%.3f"),
        pending.level.as_str(),
        pending.component,
        pending.event,
        std::process::id(),
        pending.thread_id,
        env!("CARGO_PKG_VERSION"),
        pending.message
    )
}

fn append_lines(path: &Path, lines: &[&str]) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = log_lock_path(path);
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    lock_file.lock_exclusive()?;
    rotate_if_needed(path);
    let mut buffer = String::new();
    for line in lines {
        buffer.push_str(line);
        buffer.push('\n');
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(buffer.as_bytes())?;
    Ok(())
}

#[derive(Clone)]
struct RuntimeEventRow {
    line: String,
}

fn append_events_batch(
    connection: &mut Connection,
    rows: &[PreparedLog<'_>],
    prune: bool,
) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    {
        let mut insert = transaction.prepare_cached(
            "INSERT INTO runtime_events
             (created_at, component, level, event, message, line, pid, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for row in rows {
            let protected_message = protect_event_field(&row.pending.message)?;
            let protected_line = protect_event_field(&row.line)?;
            insert.execute(params![
                row.created_at.to_rfc3339(),
                row.pending.component,
                row.pending.level.as_str(),
                row.pending.event,
                protected_message,
                protected_line,
                i64::from(std::process::id()),
                env!("CARGO_PKG_VERSION"),
            ])?;
        }
    }
    if prune {
        transaction.execute(
            "DELETE FROM runtime_events
             WHERE id NOT IN (
               SELECT id FROM runtime_events ORDER BY id DESC LIMIT ?1
             )",
            params![EVENT_DB_MAX_ROWS],
        )?;
    }
    transaction.commit()
}

fn open_event_connection(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(2))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS runtime_events (
           id INTEGER PRIMARY KEY,
           created_at TEXT NOT NULL,
           component TEXT NOT NULL,
           level TEXT NOT NULL,
           event TEXT NOT NULL,
           message TEXT NOT NULL,
           line TEXT NOT NULL,
           pid INTEGER NOT NULL,
           version TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_runtime_events_created
           ON runtime_events(created_at);
         CREATE INDEX IF NOT EXISTS idx_runtime_events_component_event
           ON runtime_events(component, event);",
    )?;
    Ok(conn)
}

fn query_recent_events(
    limit: usize,
    keep: impl Fn(&RuntimeEventRow) -> bool,
) -> Vec<RuntimeEventRow> {
    let _ = flush_pending_logs(Duration::from_millis(250));
    let Some(path) = event_store_path() else {
        return Vec::new();
    };
    let Ok(conn) = open_event_connection(&path) else {
        return Vec::new();
    };
    let mut stmt = match conn.prepare(
        "SELECT line
         FROM runtime_events
         ORDER BY id DESC
         LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map([limit as i64], |row| {
        let protected: Vec<u8> = row.get(0)?;
        let line = unprotect_event_field(&protected).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                protected.len(),
                rusqlite::types::Type::Blob,
                Box::new(err),
            )
        })?;
        Ok(RuntimeEventRow { line })
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    let mut events = Vec::new();
    for row in rows.flatten() {
        if keep(&row) {
            events.push(row);
        }
    }
    events.reverse();
    events
}

fn protect_event_field(value: &str) -> rusqlite::Result<Vec<u8>> {
    crate::windows_security::dpapi_protect_with_magic(EVENT_FIELD_MAGIC, value.as_bytes())
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))
}

fn unprotect_event_field(value: &[u8]) -> std::io::Result<String> {
    if cfg!(windows) && !crate::windows_security::dpapi_blob_has_magic(EVENT_FIELD_MAGIC, value) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "unencrypted runtime event rejected",
        ));
    }
    let mut bytes = crate::windows_security::dpapi_unprotect_with_magic(EVENT_FIELD_MAGIC, value)?;
    let text = String::from_utf8(std::mem::take(&mut bytes))
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    Ok(text)
}

fn extract_event_field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let value = line[start..]
        .split(',')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(|ch| ch == '"' || ch == '\'' || ch == ';')
        .trim();
    (!value.is_empty()).then(|| value.chars().take(120).collect())
}

fn log_lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

pub fn path_for_log(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|ch| if ch.is_whitespace() { '_' } else { ch })
        .collect()
}

fn rotate_if_needed(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < LOG_MAX_BYTES {
        return;
    }
    let oldest = rotated_log_path(path, LOG_ROTATE_KEEP);
    let _ = fs::remove_file(&oldest);
    for index in (2..=LOG_ROTATE_KEEP).rev() {
        let from = rotated_log_path(path, index - 1);
        let to = rotated_log_path(path, index);
        if from.is_file() {
            let _ = fs::rename(from, to);
        }
    }
    let backup = rotated_log_path(path, 1);
    if fs::rename(path, &backup).is_err() {
        let _ = fs::remove_file(path);
    }
}

pub fn rotated_log_path(path: &Path, index: usize) -> PathBuf {
    if index <= 1 {
        return path.with_extension("previous.log");
    }
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "log".into());
    let ext = path
        .extension()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "log".into());
    path.with_file_name(format!("{stem}.previous.{index}.{ext}"))
}

fn read_ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{FEFF}');
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if current != section {
            continue;
        }
        let (name, value) = line.split_once('=')?;
        if name.trim().eq_ignore_ascii_case(key) {
            return Some(value.trim().to_string());
        }
    }
    None
}
