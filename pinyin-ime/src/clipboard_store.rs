use crate::runtime_log::{self, RuntimeLogLevel};
use fs2::FileExt;
use rusqlite::{params, Connection, DatabaseName};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STORE_FILE_NAME: &str = "clipboard_store.sqlite";
const ENCRYPTED_MAGIC: &[u8] = b"KXCB-DPAPI-1\n";
const SQLITE_SCHEMA_VERSION: i32 = 1;
const POLL_INTERVAL: Duration = Duration::from_millis(1200);
const BACKGROUND_POLL_TICK: Duration = Duration::from_millis(250);
#[cfg(windows)]
const CLIPBOARD_EVENT_DEBOUNCE: Duration = Duration::from_millis(80);
const SYSTEM_CLIPBOARD_DUPLICATE_WINDOW_SECS: u64 = 2;
const MAX_HISTORY_ITEMS: usize = 60;
const MAX_PINNED_ITEMS: usize = 24;
const MAX_TEXT_UTF16_UNITS: usize = 20_000;
const MAX_AGE_DAYS: usize = 0;

static BACKGROUND_POLLING_ENABLED: AtomicBool = AtomicBool::new(false);
static CLIPBOARD_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static SNAPSHOT_REFRESH_ACTIVE: AtomicBool = AtomicBool::new(false);
static SNAPSHOT_CAPTURE_REQUESTED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static CLIPBOARD_EVENT_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static CLIPBOARD_EVENT_REQUEST_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(windows)]
static CLIPBOARD_EVENT_MODE: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static CLIPBOARD_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct ClipboardPrefs {
    privacy_enabled: bool,
    background_enabled: bool,
    max_history_items: usize,
    max_pinned_items: usize,
    max_text_utf16_units: usize,
    max_age_days: usize,
    candidate_preview_enabled: bool,
    record_source_app: bool,
    pinned_respects_max_age: bool,
    never_clipboard_processes: Vec<String>,
}

impl Default for ClipboardPrefs {
    fn default() -> Self {
        Self {
            privacy_enabled: false,
            background_enabled: false,
            max_history_items: MAX_HISTORY_ITEMS,
            max_pinned_items: MAX_PINNED_ITEMS,
            max_text_utf16_units: MAX_TEXT_UTF16_UNITS,
            max_age_days: MAX_AGE_DAYS,
            candidate_preview_enabled: false,
            record_source_app: false,
            pinned_respects_max_age: true,
            never_clipboard_processes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardEntry {
    pub id: String,
    pub text: String,
    pub captured_at: u64,
    pub first_captured_at: u64,
    pub copy_count: u32,
    pub source_app: Option<String>,
}

impl Drop for ClipboardEntry {
    fn drop(&mut self) {
        zeroize_string(&mut self.id);
        zeroize_string(&mut self.text);
        if let Some(source_app) = &mut self.source_app {
            zeroize_string(source_app);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub pinned: Vec<ClipboardEntry>,
    pub history: Vec<ClipboardEntry>,
}

#[derive(Default)]
struct ClipboardRuntime {
    last_poll: Option<Instant>,
    last_seen_text: Option<String>,
    snapshot_cache: Option<Arc<ClipboardSnapshot>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DuplicateRecordMode {
    Count,
    CoalesceRecentSystemEvent,
}

impl Drop for ClipboardRuntime {
    fn drop(&mut self) {
        if let Some(text) = &mut self.last_seen_text {
            zeroize_string(text);
        }
    }
}

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
}

fn zeroize_vec(bytes: &mut Vec<u8>) {
    zeroize_bytes(bytes.as_mut_slice());
}

fn zeroize_string(text: &mut String) {
    unsafe {
        zeroize_bytes(text.as_mut_vec().as_mut_slice());
    }
}

fn runtime() -> &'static Mutex<ClipboardRuntime> {
    static RUNTIME: OnceLock<Mutex<ClipboardRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(ClipboardRuntime::default()))
}

fn update_snapshot_cache(snapshot: ClipboardSnapshot) {
    if let Ok(mut runtime) = runtime().lock() {
        runtime.snapshot_cache = Some(Arc::new(snapshot));
    }
}

/// Returns the most recently loaded clipboard snapshot without touching the
/// filesystem. Candidate lookup uses this path so a key event never waits on
/// the encrypted SQLite store or its cross-process lock.
pub fn cached_snapshot() -> Option<Arc<ClipboardSnapshot>> {
    if clipboard_prefs().privacy_enabled {
        return None;
    }
    runtime()
        .lock()
        .ok()
        .and_then(|runtime| runtime.snapshot_cache.clone())
}

fn refresh_snapshot_cache(capture_system: bool) {
    if capture_system {
        let _ = capture_system_clipboard(true);
    }
    match load_snapshot_at_path(&store_path()) {
        Ok(snapshot) => update_snapshot_cache(snapshot),
        Err(err) => runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_snapshot_cache",
            format!("status=failed reason={err}"),
        ),
    }
}

fn refresh_snapshot_cache_async(capture_system: bool) {
    if capture_system {
        SNAPSHOT_CAPTURE_REQUESTED.store(true, Ordering::Release);
    }
    if SNAPSHOT_REFRESH_ACTIVE.swap(true, Ordering::AcqRel) {
        return;
    }
    if std::thread::Builder::new()
        .name("kaixin-clipboard-snapshot".to_string())
        .spawn(move || {
            loop {
                let capture_system = SNAPSHOT_CAPTURE_REQUESTED.swap(false, Ordering::AcqRel);
                refresh_snapshot_cache(capture_system);
                if !SNAPSHOT_CAPTURE_REQUESTED.load(Ordering::Acquire) {
                    break;
                }
            }
            SNAPSHOT_REFRESH_ACTIVE.store(false, Ordering::Release);
            // Close the small race between the final pending check and
            // clearing ACTIVE: a new key may have requested a capture there.
            if SNAPSHOT_CAPTURE_REQUESTED.load(Ordering::Acquire) {
                refresh_snapshot_cache_async(false);
            }
        })
        .is_err()
    {
        SNAPSHOT_REFRESH_ACTIVE.store(false, Ordering::Release);
    }
}

/// Starts loading the persisted snapshot before the first clipboard command.
pub fn warmup_snapshot_cache_async() {
    refresh_snapshot_cache_async(false);
}

/// Refreshes both the system clipboard capture and cached snapshot off the
/// interactive lookup thread. Concurrent key events coalesce into one task.
pub fn refresh_system_clipboard_cache_async() {
    refresh_snapshot_cache_async(true);
}

fn next_clipboard_request_id() -> u64 {
    CLIPBOARD_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(windows)]
fn pending_clipboard_event_request_id() -> u64 {
    let request_id = CLIPBOARD_EVENT_REQUEST_ID.swap(0, Ordering::AcqRel);
    if request_id == 0 {
        next_clipboard_request_id()
    } else {
        request_id
    }
}

#[cfg(windows)]
fn clipboard_background_worker_loop() {
    loop {
        if !BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
            CLIPBOARD_EVENT_PENDING.store(false, Ordering::Release);
            std::thread::sleep(BACKGROUND_POLL_TICK);
            continue;
        }

        if CLIPBOARD_EVENT_MODE.load(Ordering::Acquire) {
            if CLIPBOARD_EVENT_PENDING.swap(false, Ordering::AcqRel) {
                let request_id = pending_clipboard_event_request_id();
                std::thread::sleep(CLIPBOARD_EVENT_DEBOUNCE);
                let _ = poll_system_clipboard_changed_event_with_request(request_id);
                continue;
            }
            let _ = poll_system_clipboard_if_due(false);
            std::thread::sleep(BACKGROUND_POLL_TICK);
        } else {
            let _ = poll_system_clipboard_if_due(false);
            std::thread::sleep(BACKGROUND_POLL_TICK);
        }
    }
}

#[cfg(windows)]
fn run_clipboard_listener_or_poll() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_MESSAGE, MSG, WM_CLIPBOARDUPDATE, WNDCLASSW,
    };

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            if BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
                let request_id = next_clipboard_request_id();
                let worker_active = CLIPBOARD_WORKER_ACTIVE.load(Ordering::Acquire);
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Verbose,
                    "clipboard_update",
                    format!(
                        "status=received request_id={} worker_active={}",
                        request_id,
                        if worker_active { 1 } else { 0 }
                    ),
                );
                if worker_active {
                    CLIPBOARD_EVENT_REQUEST_ID.store(request_id, Ordering::Release);
                    CLIPBOARD_EVENT_PENDING.store(true, Ordering::Release);
                } else {
                    let _ = poll_system_clipboard_changed_event_with_request(request_id);
                }
            }
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let worker_started = std::thread::Builder::new()
        .name("srf-clipboard-worker".to_string())
        .spawn(clipboard_background_worker_loop)
        .is_ok();
    CLIPBOARD_WORKER_ACTIVE.store(worker_started, Ordering::Release);

    unsafe {
        let class_name: Vec<u16> = format!(
            "ClipboardListener_{:x}_{:x}",
            std::process::id(),
            current_timestamp_secs()
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..std::mem::zeroed()
        };
        let atom = RegisterClassW(&wc);
        let hwnd = if atom != 0 {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                0,
                instance,
                std::ptr::null(),
            )
        } else {
            0
        };
        if hwnd != 0 && AddClipboardFormatListener(hwnd) != 0 {
            CLIPBOARD_EVENT_MODE.store(true, Ordering::Release);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            CLIPBOARD_EVENT_MODE.store(false, Ordering::Release);
            return;
        }
    }

    if worker_started {
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    loop {
        if BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
            let _ = poll_system_clipboard_if_due(false);
        }
        std::thread::sleep(BACKGROUND_POLL_TICK);
    }
}

#[cfg(not(windows))]
fn run_clipboard_listener_or_poll() {
    loop {
        if BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
            let _ = poll_system_clipboard_if_due(false);
        }
        std::thread::sleep(BACKGROUND_POLL_TICK);
    }
}

fn ensure_background_poller_started() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(err) = std::thread::Builder::new()
            .name("srf-clipboard-listener".to_string())
            .spawn(run_clipboard_listener_or_poll)
        {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_background_worker",
                format!("status=failed reason={err}"),
            );
        }
    });
}

pub fn set_background_polling_enabled(enabled: bool) {
    let enabled = enabled && !clipboard_prefs().privacy_enabled;
    let previous = BACKGROUND_POLLING_ENABLED.swap(enabled, Ordering::AcqRel);
    if previous == enabled {
        return;
    }
    runtime_log::log_clipboard(
        RuntimeLogLevel::Basic,
        "clipboard_background",
        format!(
            "status=ok enabled={} {}",
            if enabled { 1 } else { 0 },
            diagnostics_fields()
        ),
    );
    if enabled {
        ensure_background_poller_started();
        if let Err(err) = poll_system_clipboard(true, false, true) {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_background_capture",
                format!("status=failed reason={err}"),
            );
        }
    }
}

fn local_app_data_dir() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    crate::app_paths::config_ini_path()
        .unwrap_or_else(|| local_app_data_dir().join(crate::app_paths::CONFIG_FILE_NAME))
}

fn parse_bool(value: &str, default: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "\u{5f00}\u{542f}" | "\u{5f00}" => true,
        "0" | "false" | "no" | "off" | "\u{5173}\u{95ed}" | "\u{5173}" => false,
        _ => default,
    }
}

fn parse_process_list(value: &str) -> Vec<String> {
    value
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn wildcard_match_no_case(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    let pat = pattern.as_bytes();
    let text = value.as_bytes();
    let mut p = 0usize;
    let mut t = 0usize;
    let mut star = None;
    let mut retry = 0usize;
    while t < text.len() {
        if p < pat.len() && (pat[p] == b'?' || pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = t;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            retry += 1;
            t = retry;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

fn process_matches_list(process: &str, patterns: &[String]) -> bool {
    let file_name = Path::new(process)
        .file_name()
        .map(|name| name.to_string_lossy().to_string());
    patterns.iter().any(|pattern| {
        wildcard_match_no_case(pattern, process)
            || file_name
                .as_deref()
                .is_some_and(|name| wildcard_match_no_case(pattern, name))
    })
}

fn parse_clipboard_prefs(text: &str) -> ClipboardPrefs {
    let mut prefs = ClipboardPrefs::default();
    let mut in_clipboard = false;
    let mut in_privacy = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let section = line[1..line.len() - 1].trim();
            in_clipboard = section.eq_ignore_ascii_case("clipboard");
            in_privacy = section.eq_ignore_ascii_case("privacy");
            continue;
        }
        if !in_clipboard && !in_privacy {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let parsed = value.trim().parse::<usize>().ok();
        let key = key.trim().to_ascii_lowercase();
        if in_privacy {
            match key.as_str() {
                "enabled" => prefs.privacy_enabled = parse_bool(value, false),
                "never_clipboard_processes" => {
                    prefs.never_clipboard_processes = parse_process_list(value);
                }
                _ => {}
            }
            continue;
        }
        match key.as_str() {
            "max_history_items" => {
                if let Some(v) = parsed {
                    prefs.max_history_items = v.min(300);
                }
            }
            "max_pinned_items" => {
                if let Some(v) = parsed {
                    prefs.max_pinned_items = v.min(100);
                }
            }
            "max_text_utf16_units" => {
                if let Some(v) = parsed {
                    prefs.max_text_utf16_units = v.clamp(20, 20_000);
                }
            }
            "max_age_days" => {
                if let Some(v) = parsed {
                    prefs.max_age_days = v.min(3650);
                }
            }
            "candidate_preview_enabled" => {
                prefs.candidate_preview_enabled = parse_bool(value, false);
            }
            "record_source_app" => {
                prefs.record_source_app = parse_bool(value, false);
            }
            "pinned_respects_max_age" => {
                prefs.pinned_respects_max_age = parse_bool(value, true);
            }
            "background_enabled" => {
                prefs.background_enabled = parse_bool(value, false);
            }
            _ => {}
        }
    }
    prefs
}

fn clipboard_prefs() -> ClipboardPrefs {
    #[derive(Clone)]
    struct CachedPrefs {
        modified: Option<SystemTime>,
        prefs: ClipboardPrefs,
    }

    static CACHE: OnceLock<Mutex<Option<CachedPrefs>>> = OnceLock::new();

    let path = config_path();
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(cached) = cache.as_ref() {
            if cached.modified == modified {
                return cached.prefs.clone();
            }
        }
        let prefs = std::fs::read_to_string(&path)
            .map(|text| parse_clipboard_prefs(&text))
            .unwrap_or_default();
        *cache = Some(CachedPrefs {
            modified,
            prefs: prefs.clone(),
        });
        prefs
    } else {
        std::fs::read_to_string(path)
            .map(|text| parse_clipboard_prefs(&text))
            .unwrap_or_default()
    }
}

pub fn store_path() -> PathBuf {
    crate::app_paths::local_data_dir()
        .unwrap_or_else(local_app_data_dir)
        .join(STORE_FILE_NAME)
}

pub fn diagnostics_fields() -> String {
    let prefs = clipboard_prefs();
    let store_path = store_path();
    format!(
        "clipboard_background_enabled={} clipboard_store={} clipboard_store_exists={}",
        if prefs.background_enabled { 1 } else { 0 },
        runtime_log::path_for_log(&store_path),
        if store_path.is_file() { 1 } else { 0 }
    )
}

#[cfg(test)]
fn temp_path_for(path: &Path) -> PathBuf {
    path.with_extension("sqlite.tmp")
}

fn write_temp_path_for(path: &Path) -> PathBuf {
    path.with_extension("sqlite.write.tmp")
}

fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension("sqlite.lock")
}

fn encryption_enabled() -> bool {
    cfg!(windows)
}

fn is_encrypted_blob(bytes: &[u8]) -> bool {
    bytes.starts_with(ENCRYPTED_MAGIC)
}

pub fn clipboard_store_encryption_enabled() -> bool {
    encryption_enabled()
}

pub fn clipboard_store_file_is_encrypted(path: &Path) -> io::Result<bool> {
    fs::read(path).map(|bytes| is_encrypted_blob(&bytes))
}

#[cfg(windows)]
fn dpapi_protect(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let protected = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    let mut out = Vec::with_capacity(ENCRYPTED_MAGIC.len() + protected.len());
    out.extend_from_slice(ENCRYPTED_MAGIC);
    out.extend_from_slice(&protected);
    Ok(out)
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let payload = data.strip_prefix(ENCRYPTED_MAGIC).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "missing clipboard store encryption header",
        )
    })?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: payload.len() as u32,
        pbData: payload.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let unprotected = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        zeroize_bytes(slice);
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    Ok(unprotected)
}

#[cfg(not(windows))]
fn dpapi_protect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

fn encode_store_contents(contents: &[u8]) -> Result<Vec<u8>, String> {
    if encryption_enabled() {
        dpapi_protect(contents).map_err(|e| format!("encrypt clipboard store: {e}"))
    } else {
        Ok(contents.to_vec())
    }
}

fn decode_store_contents(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if is_encrypted_blob(bytes) {
        dpapi_unprotect(bytes).map_err(|e| format!("decrypt clipboard store: {e}"))
    } else if cfg!(windows) {
        Err("unencrypted clipboard store rejected".to_string())
    } else {
        Ok(bytes.to_vec())
    }
}

#[cfg(windows)]
fn replace_file_atomically(temp_path: &Path, final_path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let final_wide: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            final_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(format!(
            "replace clipboard sqlite store: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file_atomically(temp_path: &Path, final_path: &Path) -> Result<(), String> {
    fs::rename(temp_path, final_path).map_err(|e| format!("replace clipboard sqlite store: {e}"))
}

pub fn store_modified() -> Option<SystemTime> {
    fs::metadata(store_path())
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn current_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_clipboard_entry_id(captured_at: u64) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = nanos ^ ((captured_at as u128) << 64) ^ (pid << 32) ^ counter as u128;
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (mixed >> 96) as u32,
        (mixed >> 80) as u16,
        (mixed >> 64) as u16,
        (mixed >> 48) as u16,
        mixed & 0x0000_ffff_ffff_ffff_ffff
    )
}

fn normalize_text_with_limit(text: &str, max_text_utf16_units: usize) -> Option<String> {
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim_matches('\0')
        .trim_matches('\u{feff}')
        .to_string();
    if normalized.trim().is_empty() {
        return None;
    }

    if max_text_utf16_units == 0 {
        return None;
    }

    let mut truncated = String::new();
    let mut utf16_units = 0usize;
    for ch in normalized.chars() {
        let next = utf16_units + ch.len_utf16();
        if next > max_text_utf16_units {
            break;
        }
        truncated.push(ch);
        utf16_units = next;
    }
    if truncated.trim().is_empty() {
        return None;
    }
    Some(truncated)
}

fn normalize_text(text: &str) -> Option<String> {
    normalize_text_with_limit(text, clipboard_prefs().max_text_utf16_units)
}

fn normalize_existing_text_key(text: &str) -> Option<String> {
    normalize_text_with_limit(text, usize::MAX)
}

fn normalize_source_app(source: Option<String>) -> Option<String> {
    let source = source?.trim().trim_matches('\0').to_string();
    if source.is_empty() {
        return None;
    }
    let file_name = Path::new(&source)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| source.clone());
    let lower_name = file_name.to_ascii_lowercase();
    if lower_name.starts_with("srf_ime_") || lower_name == "pinyin_ime_gui.exe" {
        return None;
    }
    Some(source.chars().take(80).collect())
}

fn new_entry(text: String, captured_at: u64, source_app: Option<String>) -> ClipboardEntry {
    ClipboardEntry {
        id: new_clipboard_entry_id(captured_at),
        text,
        captured_at,
        first_captured_at: captured_at,
        copy_count: 1,
        source_app: normalize_source_app(source_app),
    }
}

fn prune_snapshot(snapshot: &mut ClipboardSnapshot, prefs: &ClipboardPrefs) {
    if snapshot.history.len() > prefs.max_history_items {
        snapshot.history.truncate(prefs.max_history_items);
    }
    if snapshot.pinned.len() > prefs.max_pinned_items {
        snapshot.pinned.truncate(prefs.max_pinned_items);
    }
}

pub fn clipboard_candidate_preview_enabled() -> bool {
    clipboard_prefs().candidate_preview_enabled
}

fn ensure_store_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create clipboard dir: {e}"))?;
    }
    Ok(())
}

fn open_lock_file(path: &Path) -> Result<File, String> {
    let lock_path = lock_path_for(path);
    ensure_store_parent(&lock_path)?;
    OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(lock_path)
        .map_err(|e| format!("open clipboard store lock: {e}"))
}

fn initialize_store_connection(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("configure clipboard sqlite busy timeout: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS clipboard_entries (
           kind TEXT NOT NULL CHECK(kind IN ('P', 'H')),
           sort_order INTEGER NOT NULL,
           id TEXT NOT NULL,
           text TEXT NOT NULL,
           captured_at INTEGER NOT NULL,
           first_captured_at INTEGER NOT NULL,
           copy_count INTEGER NOT NULL,
           source_app TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_clipboard_entries_kind_order
           ON clipboard_entries(kind, sort_order);",
    )
    .map_err(|e| format!("initialize clipboard sqlite schema: {e}"))?;
    conn.execute_batch(&format!("PRAGMA user_version = {SQLITE_SCHEMA_VERSION};"))
        .map_err(|e| format!("stamp clipboard sqlite schema: {e}"))?;
    Ok(())
}

fn open_memory_store_connection() -> Result<Connection, String> {
    let conn = Connection::open_in_memory()
        .map_err(|e| format!("open clipboard sqlite memory store: {e}"))?;
    initialize_store_connection(&conn)?;
    Ok(conn)
}

fn i64_from_u64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("clipboard {field} is too large for sqlite"))
}

fn i64_from_usize(value: usize, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("clipboard {field} is too large for sqlite"))
}

fn u64_from_i64(value: i64, field: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("clipboard {field} is negative in sqlite"))
}

fn save_snapshot_to_connection(
    conn: &mut Connection,
    snapshot: &ClipboardSnapshot,
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin clipboard sqlite transaction: {e}"))?;
    tx.execute("DELETE FROM clipboard_entries", [])
        .map_err(|e| format!("clear clipboard sqlite store: {e}"))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO clipboard_entries
                 (kind, sort_order, id, text, captured_at, first_captured_at, copy_count, source_app)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(|e| format!("prepare clipboard sqlite insert: {e}"))?;
        for (kind, entries) in [("P", &snapshot.pinned), ("H", &snapshot.history)] {
            for (index, entry) in entries.iter().enumerate() {
                stmt.execute(params![
                    kind,
                    i64_from_usize(index, "sort order")?,
                    entry.id.as_str(),
                    entry.text.as_str(),
                    i64_from_u64(entry.captured_at, "captured_at")?,
                    i64_from_u64(
                        entry.first_captured_at.min(entry.captured_at),
                        "first_captured_at"
                    )?,
                    i64::from(entry.copy_count.max(1)),
                    entry.source_app.as_deref(),
                ])
                .map_err(|e| format!("write clipboard sqlite entry: {e}"))?;
            }
        }
    }
    tx.commit()
        .map_err(|e| format!("commit clipboard sqlite store: {e}"))
}

fn read_snapshot_from_connection(conn: &Connection) -> Result<ClipboardSnapshot, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, id, text, captured_at, first_captured_at, copy_count, source_app
             FROM clipboard_entries
             ORDER BY kind, sort_order, captured_at DESC, text ASC",
        )
        .map_err(|e| format!("prepare clipboard sqlite read: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|e| format!("read clipboard sqlite rows: {e}"))?;

    let mut snapshot = ClipboardSnapshot::default();
    for row in rows {
        let (kind, id, text, captured_at, first_captured_at, copy_count, source_app) =
            row.map_err(|e| format!("read clipboard sqlite entry: {e}"))?;
        let captured_at = u64_from_i64(captured_at, "captured_at")?;
        let first_captured_at = u64_from_i64(first_captured_at, "first_captured_at")?;
        let Some(text) = normalize_text_with_limit(&text, usize::MAX) else {
            continue;
        };
        let entry = ClipboardEntry {
            id: if id.trim().is_empty() {
                new_clipboard_entry_id(captured_at)
            } else {
                id
            },
            text,
            captured_at,
            first_captured_at: first_captured_at.min(captured_at),
            copy_count: u32::try_from(copy_count.max(1)).unwrap_or(u32::MAX),
            source_app: normalize_source_app(source_app),
        };
        match kind.as_str() {
            "P" => snapshot.pinned.push(entry),
            "H" => snapshot.history.push(entry),
            _ => {}
        }
    }
    Ok(snapshot)
}

fn sqlite_owned_data_from_bytes(bytes: &[u8]) -> Result<rusqlite::serialize::OwnedData, String> {
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>();
    let Some(ptr) = NonNull::new(ptr) else {
        return Err("allocate clipboard sqlite memory store".to_string());
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        Ok(rusqlite::serialize::OwnedData::from_raw_nonnull(
            ptr,
            bytes.len(),
        ))
    }
}

fn read_snapshot_from_sqlite_bytes(bytes: &[u8]) -> Result<ClipboardSnapshot, String> {
    let mut conn = Connection::open_in_memory()
        .map_err(|e| format!("open clipboard sqlite memory store: {e}"))?;
    if !bytes.is_empty() {
        let data = sqlite_owned_data_from_bytes(bytes)?;
        conn.deserialize(DatabaseName::Main, data, false)
            .map_err(|e| format!("load clipboard sqlite memory store: {e}"))?;
    }
    initialize_store_connection(&conn)?;
    read_snapshot_from_connection(&conn)
}

fn snapshot_to_sqlite_bytes(snapshot: &ClipboardSnapshot) -> Result<Vec<u8>, String> {
    let mut conn = open_memory_store_connection()?;
    save_snapshot_to_connection(&mut conn, snapshot)?;
    let data = conn
        .serialize(DatabaseName::Main)
        .map_err(|e| format!("serialize clipboard sqlite store: {e}"))?;
    Ok(data.to_vec())
}

fn read_snapshot_from_path(path: &Path) -> Result<ClipboardSnapshot, String> {
    if path.is_file() {
        let raw = fs::read(path).map_err(|e| format!("read clipboard sqlite store: {e}"))?;
        let mut sqlite_bytes = decode_store_contents(&raw)?;
        let snapshot = read_snapshot_from_sqlite_bytes(&sqlite_bytes);
        zeroize_vec(&mut sqlite_bytes);
        return snapshot;
    }

    let snapshot = ClipboardSnapshot::default();
    save_snapshot_atomically(path, &snapshot)?;
    Ok(snapshot)
}

fn restore_backup_if_needed(path: &Path) -> Result<ClipboardSnapshot, String> {
    read_snapshot_from_path(path)
}

fn save_snapshot_atomically(path: &Path, snapshot: &ClipboardSnapshot) -> Result<(), String> {
    ensure_store_parent(path)?;
    let mut sqlite_bytes = snapshot_to_sqlite_bytes(snapshot)?;
    let encoded = encode_store_contents(&sqlite_bytes)?;
    zeroize_vec(&mut sqlite_bytes);
    let temp_path = write_temp_path_for(path);
    let _ = fs::remove_file(&temp_path);
    fs::write(&temp_path, encoded).map_err(|e| format!("write clipboard sqlite store: {e}"))?;
    replace_file_atomically(&temp_path, path)
}

fn with_store_mut_at_path<R>(
    path: &Path,
    f: impl FnOnce(&mut ClipboardSnapshot) -> Result<R, String>,
) -> Result<R, String> {
    let lock_path = lock_path_for(path);
    let lock_file = open_lock_file(path)?;
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("lock clipboard store: {e}"))?;
    let prefs = clipboard_prefs();
    let mut snapshot = restore_backup_if_needed(path)?;
    prune_snapshot(&mut snapshot, &prefs);
    let before = snapshot.clone();
    let result = f(&mut snapshot);
    prune_snapshot(&mut snapshot, &prefs);
    if result.is_ok() && snapshot != before {
        save_snapshot_atomically(path, &snapshot)?;
    }
    let _ = lock_file.unlock();
    drop(lock_file);
    let _ = fs::remove_file(lock_path);
    result
}

fn load_snapshot_at_path(path: &Path) -> Result<ClipboardSnapshot, String> {
    let lock_path = lock_path_for(path);
    let lock_file = open_lock_file(path)?;
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("lock clipboard store: {e}"))?;
    let snapshot = restore_backup_if_needed(path);
    let _ = lock_file.unlock();
    drop(lock_file);
    let _ = fs::remove_file(lock_path);
    snapshot
}

fn remove_text(entries: &mut Vec<ClipboardEntry>, text: &str) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.text != text);
    before != entries.len()
}

fn remove_entry_by_text(entries: &mut Vec<ClipboardEntry>, text: &str) -> Option<ClipboardEntry> {
    let pos = entries.iter().position(|entry| entry.text == text)?;
    Some(entries.remove(pos))
}

fn upsert_entry(entries: &mut Vec<ClipboardEntry>, mut entry: ClipboardEntry, limit: usize) {
    if let Some(pos) = entries
        .iter()
        .position(|existing| existing.text == entry.text)
    {
        let mut existing = entries.remove(pos);
        entry.id = std::mem::take(&mut existing.id);
        entry.captured_at = entry.captured_at.max(existing.captured_at);
        entry.first_captured_at = entry.first_captured_at.min(existing.first_captured_at);
        entry.copy_count = existing.copy_count.saturating_add(entry.copy_count.max(1));
        if entry.source_app.is_none() {
            entry.source_app = std::mem::take(&mut existing.source_app);
        }
    }
    entries.insert(0, entry);
    entries.sort_by(|a, b| {
        b.captured_at
            .cmp(&a.captured_at)
            .then_with(|| a.text.cmp(&b.text))
    });
    if entries.len() > limit {
        entries.truncate(limit);
    }
}

fn next_entry_timestamp(snapshot: &ClipboardSnapshot) -> u64 {
    let newest = snapshot
        .pinned
        .iter()
        .chain(snapshot.history.iter())
        .map(|entry| entry.captured_at)
        .max()
        .unwrap_or(0);
    current_timestamp_secs().max(newest.saturating_add(1))
}

fn is_recent_system_duplicate(snapshot: &ClipboardSnapshot, text: &str, now: u64) -> bool {
    snapshot.history.first().is_some_and(|entry| {
        entry.text == text
            && (entry.captured_at >= now
                || now.saturating_sub(entry.captured_at) <= SYSTEM_CLIPBOARD_DUPLICATE_WINDOW_SECS)
    })
}

fn record_text_at_path_with_mode(
    path: &Path,
    text: &str,
    duplicate_mode: DuplicateRecordMode,
) -> Result<bool, String> {
    let prefs = clipboard_prefs();
    if prefs.privacy_enabled {
        return Ok(false);
    }
    if prefs.max_history_items == 0 {
        return Ok(false);
    }
    let Some(text) = normalize_text_with_limit(text, prefs.max_text_utf16_units) else {
        return Ok(false);
    };
    let source_app = current_foreground_process_name();
    if source_app
        .as_deref()
        .is_some_and(|app| process_matches_list(app, &prefs.never_clipboard_processes))
    {
        return Ok(false);
    }
    with_store_mut_at_path(path, |snapshot| {
        if duplicate_mode == DuplicateRecordMode::CoalesceRecentSystemEvent
            && is_recent_system_duplicate(snapshot, &text, current_timestamp_secs())
        {
            return Ok(false);
        }
        let captured_at = next_entry_timestamp(snapshot);
        let entry = new_entry(
            text.clone(),
            captured_at,
            prefs
                .record_source_app
                .then(|| source_app.clone())
                .flatten(),
        );
        if snapshot.pinned.iter().any(|existing| existing.text == text) {
            upsert_entry(&mut snapshot.pinned, entry.clone(), prefs.max_pinned_items);
        }
        upsert_entry(&mut snapshot.history, entry, prefs.max_history_items);
        Ok(true)
    })
}

fn record_text_at_path(path: &Path, text: &str) -> Result<bool, String> {
    record_text_at_path_with_mode(path, text, DuplicateRecordMode::Count)
}

fn pin_text_at_path(path: &Path, text: &str) -> Result<bool, String> {
    let prefs = clipboard_prefs();
    let Some(match_text) = normalize_existing_text_key(text) else {
        return Ok(false);
    };
    let source_app = if prefs.record_source_app {
        current_foreground_process_name()
    } else {
        None
    };
    with_store_mut_at_path(path, |snapshot| {
        let Some(text) = snapshot
            .pinned
            .iter()
            .chain(snapshot.history.iter())
            .find(|existing| existing.text == match_text)
            .map(|existing| existing.text.clone())
            .or_else(|| normalize_text_with_limit(text, prefs.max_text_utf16_units))
        else {
            return Ok(false);
        };
        let captured_at = next_entry_timestamp(snapshot);
        if !snapshot
            .history
            .iter()
            .any(|existing| existing.text == text)
        {
            upsert_entry(
                &mut snapshot.history,
                new_entry(text.clone(), captured_at, source_app.clone()),
                prefs.max_history_items,
            );
        }
        upsert_entry(
            &mut snapshot.pinned,
            new_entry(text.clone(), captured_at, source_app.clone()),
            prefs.max_pinned_items,
        );
        Ok(true)
    })
}

fn unpin_text_at_path(path: &Path, text: &str) -> Result<bool, String> {
    let prefs = clipboard_prefs();
    let Some(text) = normalize_existing_text_key(text) else {
        return Ok(false);
    };
    let source_app = if prefs.record_source_app {
        current_foreground_process_name()
    } else {
        None
    };
    with_store_mut_at_path(path, |snapshot| {
        let Some(mut removed) = remove_entry_by_text(&mut snapshot.pinned, &text) else {
            return Ok(false);
        };
        if !snapshot
            .history
            .iter()
            .any(|existing| existing.text == removed.text)
        {
            let captured_at = next_entry_timestamp(snapshot);
            removed.captured_at = captured_at;
            removed.first_captured_at = removed.first_captured_at.min(captured_at);
            if removed.source_app.is_none() {
                removed.source_app = source_app
                    .clone()
                    .and_then(|value| normalize_source_app(Some(value)));
            }
            upsert_entry(&mut snapshot.history, removed, prefs.max_history_items);
        }
        Ok(true)
    })
}

fn remove_saved_text_at_path(path: &Path, text: &str) -> Result<bool, String> {
    let Some(text) = normalize_existing_text_key(text) else {
        return Ok(false);
    };
    with_store_mut_at_path(path, |snapshot| {
        let removed_pinned = remove_text(&mut snapshot.pinned, &text);
        let removed_history = remove_text(&mut snapshot.history, &text);
        Ok(removed_pinned || removed_history)
    })
}

fn clear_history_at_path(path: &Path) -> Result<(), String> {
    with_store_mut_at_path(path, |snapshot| {
        snapshot.history.clear();
        Ok(())
    })
}

fn clear_all_at_path(path: &Path) -> Result<(), String> {
    with_store_mut_at_path(path, |snapshot| {
        snapshot.history.clear();
        snapshot.pinned.clear();
        Ok(())
    })
}

fn clear_older_than_days_at_path(path: &Path, days: u64) -> Result<usize, String> {
    let cutoff = current_timestamp_secs().saturating_sub(days.saturating_mul(86_400));
    let prefs = clipboard_prefs();
    with_store_mut_at_path(path, |snapshot| {
        let before = snapshot.history.len() + snapshot.pinned.len();
        snapshot.history.retain(|entry| entry.captured_at >= cutoff);
        if prefs.pinned_respects_max_age {
            snapshot.pinned.retain(|entry| entry.captured_at >= cutoff);
        }
        Ok(before.saturating_sub(snapshot.history.len() + snapshot.pinned.len()))
    })
}

pub fn resolve_entry_text(snapshot: &ClipboardSnapshot, id: &str) -> Option<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    snapshot
        .pinned
        .iter()
        .chain(snapshot.history.iter())
        .find(|entry| entry.id == trimmed)
        .map(|entry| entry.text.clone())
}

#[cfg(windows)]
fn current_foreground_process_name() -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd == 0 {
        return None;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return None;
    }
    let mut buffer = vec![0u16; 32768];
    let mut len = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 || len == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..len as usize]);
    Some(path)
}

#[cfg(not(windows))]
fn current_foreground_process_name() -> Option<String> {
    None
}

#[cfg(windows)]
fn temporary_paste_clipboard_format() -> u32 {
    use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;

    static FORMAT: OnceLock<u32> = OnceLock::new();
    *FORMAT.get_or_init(|| {
        let name: Vec<u16> = "KaixinInput.TemporaryPaste"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    })
}

#[cfg(windows)]
fn clipboard_has_temporary_paste_marker() -> bool {
    use windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable;

    let format = temporary_paste_clipboard_format();
    format != 0 && unsafe { IsClipboardFormatAvailable(format) != 0 }
}

#[cfg(windows)]
struct OpenClipboardGuard;

#[cfg(windows)]
impl OpenClipboardGuard {
    fn open_with_retry() -> Result<Self, String> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::DataExchange::OpenClipboard;

        let mut last_error = 0;
        for attempt in 0..6 {
            if unsafe { OpenClipboard(0) } != 0 {
                return Ok(Self);
            }
            last_error = unsafe { GetLastError() };
            std::thread::sleep(Duration::from_millis(12 + attempt * 8));
        }
        Err(format!(
            "OpenClipboard failed error={}",
            if last_error == 0 { 5 } else { last_error }
        ))
    }
}

#[cfg(windows)]
impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
    }
}

#[cfg(windows)]
fn read_unicode_clipboard_handle(handle: isize) -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::{GetLastError, HGLOBAL};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    if handle == 0 {
        return Err(format!(
            "GetClipboardData(CF_UNICODETEXT) failed error={}",
            unsafe { GetLastError() }
        ));
    }

    let global_handle = handle as HGLOBAL;
    let ptr = unsafe { GlobalLock(global_handle) } as *const u16;
    if ptr.is_null() {
        return Err(format!(
            "GlobalLock(CF_UNICODETEXT) failed error={}",
            unsafe { GetLastError() }
        ));
    }

    let result = (|| {
        let byte_len = unsafe { GlobalSize(global_handle) };
        if byte_len < std::mem::size_of::<u16>() {
            return Ok(None);
        }
        let unit_cap = byte_len / std::mem::size_of::<u16>();
        let scan_units = unit_cap.min(clipboard_prefs().max_text_utf16_units.saturating_add(1));
        if scan_units == 0 {
            return Ok(None);
        }
        let units = unsafe { std::slice::from_raw_parts(ptr, scan_units) };
        let len = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(scan_units);
        if len == 0 {
            return Ok(None);
        }
        Ok(Some(String::from_utf16_lossy(&units[..len])))
    })();

    unsafe {
        GlobalUnlock(global_handle);
    }
    result
}

#[cfg(windows)]
fn read_system_clipboard_text_win32() -> Result<Option<String>, String> {
    use clipboard_win::formats::CF_UNICODETEXT;
    use windows_sys::Win32::System::DataExchange::{GetClipboardData, IsClipboardFormatAvailable};

    if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
        return Ok(None);
    }
    let _guard = OpenClipboardGuard::open_with_retry()?;
    read_unicode_clipboard_handle(unsafe { GetClipboardData(CF_UNICODETEXT) })
}

fn normalize_read_clipboard_text(text: &str, request_id: u64) -> Option<String> {
    let units = text.encode_utf16().count();
    let normalized = normalize_text(text);
    runtime_log::log_clipboard(
        RuntimeLogLevel::Verbose,
        "clipboard_read",
        format!(
            "status={} request_id={} units={}{}",
            if normalized.is_some() {
                "ok"
            } else {
                "skipped"
            },
            request_id,
            units,
            if normalized.is_some() {
                ""
            } else {
                " reason=normalize_empty_or_over_limit"
            }
        ),
    );
    normalized
}

#[cfg(windows)]
fn read_system_clipboard_text(request_id: u64) -> Option<String> {
    if clipboard_has_temporary_paste_marker() {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Verbose,
            "clipboard_read_skip",
            format!(
                "status=skipped request_id={} reason=temporary_paste_marker",
                request_id
            ),
        );
        return None;
    }

    // Clipboard owners are allowed to render CF_UNICODETEXT lazily.  A
    // WM_CLIPBOARDUPDATE can therefore arrive before GetClipboardData has any
    // text to return.  Retry the complete native read rather than treating the
    // first empty result as a missed copy event.
    let mut last_error = None;
    let mut saw_unicode_format = false;
    for attempt in 0..8 {
        match read_system_clipboard_text_win32() {
            Ok(Some(text)) => return normalize_read_clipboard_text(&text, request_id),
            Ok(None) => {}
            Err(err) => last_error = Some(err),
        }

        if clipboard_win::is_format_avail(clipboard_win::formats::CF_UNICODETEXT) {
            saw_unicode_format = true;
            match clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode) {
                Ok(text) => return normalize_read_clipboard_text(&text, request_id),
                Err(err) => last_error = Some(err.to_string()),
            }
        }
        if attempt < 7 {
            std::thread::sleep(Duration::from_millis(30 + attempt * 15));
        }
    }
    if let Some(err) = last_error {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_read_fallback_failed",
            format!(
                "status=failed request_id={} retries=8 reason={err}",
                request_id
            ),
        );
    } else if !saw_unicode_format {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Verbose,
            "clipboard_read_skip",
            format!(
                "status=skipped request_id={} retries=8 reason=no_cf_unicode_text",
                request_id
            ),
        );
    }
    None
}

#[cfg(not(windows))]
fn read_system_clipboard_text(_request_id: u64) -> Option<String> {
    None
}

fn poll_system_clipboard_with_request(
    force: bool,
    record_duplicate_text: bool,
    require_background_enabled: bool,
    request_id: u64,
) -> Result<Option<String>, String> {
    let prefs = clipboard_prefs();
    if prefs.privacy_enabled {
        return Ok(None);
    }
    if require_background_enabled && !prefs.background_enabled {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Verbose,
            "clipboard_capture_skip",
            format!(
                "status=skipped request_id={} reason=background_disabled",
                request_id
            ),
        );
        return Ok(None);
    }
    {
        let mut runtime = runtime()
            .lock()
            .map_err(|_| "lock clipboard runtime".to_string())?;
        if !force
            && runtime
                .last_poll
                .map(|last| last.elapsed() < POLL_INTERVAL)
                .unwrap_or(false)
        {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Verbose,
                "clipboard_capture_skip",
                format!(
                    "status=skipped request_id={} reason=poll_interval",
                    request_id
                ),
            );
            return Ok(None);
        }
        runtime.last_poll = Some(Instant::now());
    }

    let Some(text) = read_system_clipboard_text(request_id) else {
        return Ok(None);
    };

    {
        let mut runtime = runtime()
            .lock()
            .map_err(|_| "lock clipboard runtime".to_string())?;
        if runtime.last_seen_text.as_deref() == Some(text.as_str()) && !record_duplicate_text {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Verbose,
                "clipboard_capture_skip",
                format!(
                    "status=skipped request_id={} reason=unchanged units={} force={} background_required={}",
                    request_id,
                    text.encode_utf16().count(),
                    if force { 1 } else { 0 },
                    if require_background_enabled { 1 } else { 0 }
                ),
            );
            return Ok(Some(text));
        }
        runtime.last_seen_text = Some(text.clone());
    }

    let recorded = match record_text_at_path_with_mode(
        &store_path(),
        &text,
        DuplicateRecordMode::CoalesceRecentSystemEvent,
    ) {
        Ok(recorded) => recorded,
        Err(err) => {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_capture",
                format!("status=failed request_id={} reason={err}", request_id),
            );
            return Err(err);
        }
    };
    runtime_log::log_clipboard(
        RuntimeLogLevel::Basic,
        "clipboard_capture",
        format!(
            "status=ok request_id={} units={} recorded={} force={} duplicate_event={} background_required={}",
            request_id,
            text.encode_utf16().count(),
            if recorded { 1 } else { 0 },
            if force { 1 } else { 0 },
            if record_duplicate_text { 1 } else { 0 },
            if require_background_enabled { 1 } else { 0 }
        ),
    );
    Ok(Some(text))
}

fn poll_system_clipboard(
    force: bool,
    record_duplicate_text: bool,
    require_background_enabled: bool,
) -> Result<Option<String>, String> {
    poll_system_clipboard_with_request(
        force,
        record_duplicate_text,
        require_background_enabled,
        next_clipboard_request_id(),
    )
}

fn mark_runtime_seen_text(text: &str) -> Result<(), String> {
    let Some(text) = normalize_text(text) else {
        return Ok(());
    };
    let mut runtime = runtime()
        .lock()
        .map_err(|_| "lock clipboard runtime".to_string())?;
    runtime.last_poll = Some(Instant::now());
    runtime.last_seen_text = Some(text);
    Ok(())
}

fn poll_system_clipboard_changed_event_with_request(
    request_id: u64,
) -> Result<Option<String>, String> {
    poll_system_clipboard_with_request(true, true, true, request_id)
}

pub fn poll_system_clipboard_if_due(force: bool) -> Result<Option<String>, String> {
    poll_system_clipboard(force, false, true)
}

pub fn capture_system_clipboard(force: bool) -> Result<Option<String>, String> {
    poll_system_clipboard(force, false, false)
}

pub fn record_text(text: &str) -> Result<bool, String> {
    record_text_at_path(&store_path(), text)
}

pub fn record_text_and_mark_seen(text: &str) -> Result<bool, String> {
    let recorded = record_text(text)?;
    mark_runtime_seen_text(text)?;
    Ok(recorded)
}

pub fn pin_text(text: &str) -> Result<bool, String> {
    pin_text_at_path(&store_path(), text)
}

pub fn unpin_text(text: &str) -> Result<bool, String> {
    unpin_text_at_path(&store_path(), text)
}

pub fn remove_saved_text(text: &str) -> Result<bool, String> {
    remove_saved_text_at_path(&store_path(), text)
}

pub fn clear_history() -> Result<(), String> {
    clear_history_at_path(&store_path())
}

pub fn clear_older_than_days(days: u64) -> Result<usize, String> {
    clear_older_than_days_at_path(&store_path(), days)
}

pub fn clear_all() -> Result<(), String> {
    clear_all_at_path(&store_path())
}

pub fn snapshot() -> Result<ClipboardSnapshot, String> {
    if clipboard_prefs().privacy_enabled {
        return Ok(ClipboardSnapshot::default());
    }
    let snapshot = load_snapshot_at_path(&store_path())?;
    update_snapshot_cache(snapshot.clone());
    Ok(snapshot)
}

pub fn pin_current_clipboard() -> Result<Option<String>, String> {
    let request_id = next_clipboard_request_id();
    let Some(text) = read_system_clipboard_text(request_id) else {
        return Ok(None);
    };
    pin_text(&text)?;
    let mut runtime = runtime()
        .lock()
        .map_err(|_| "lock clipboard runtime".to_string())?;
    runtime.last_poll = Some(Instant::now());
    runtime.last_seen_text = Some(text.clone());
    Ok(Some(text))
}

pub fn unpin_current_clipboard() -> Result<Option<String>, String> {
    let request_id = next_clipboard_request_id();
    let Some(text) = read_system_clipboard_text(request_id) else {
        return Ok(None);
    };
    let _ = unpin_text(&text)?;
    let mut runtime = runtime()
        .lock()
        .map_err(|_| "lock clipboard runtime".to_string())?;
    runtime.last_poll = Some(Instant::now());
    runtime.last_seen_text = Some(text.clone());
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store_path(name: &str) -> PathBuf {
        let unique = format!(
            "srf_clipboard_test_{}_{}_{}.sqlite",
            name,
            std::process::id(),
            current_timestamp_secs()
        );
        std::env::temp_dir().join(unique)
    }

    fn test_entry(text: &str, captured_at: u64) -> ClipboardEntry {
        new_entry(text.to_string(), captured_at, None)
    }

    fn remove_test_store(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(temp_path_for(path));
        let _ = fs::remove_file(write_temp_path_for(path));
        let _ = fs::remove_file(lock_path_for(path));
        let _ = fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn clipboard_entry_id_persists_and_resolves() {
        let path = temp_store_path("entry_id_persists");
        remove_test_store(&path);

        let entry = ClipboardEntry {
            id: "clip-id-1".to_string(),
            text: "alpha".to_string(),
            captured_at: 42,
            first_captured_at: 40,
            copy_count: 2,
            source_app: None,
        };
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![entry],
        };
        save_snapshot_atomically(&path, &snapshot).unwrap();

        let parsed = load_snapshot_at_path(&path).unwrap();
        assert_eq!(
            resolve_entry_text(&parsed, "clip-id-1").as_deref(),
            Some("alpha")
        );

        remove_test_store(&path);
    }

    #[test]
    fn record_pin_and_clear_roundtrip() {
        let path = temp_store_path("roundtrip");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        record_text_at_path(&path, "第一条").unwrap();
        record_text_at_path(&path, "第二条").unwrap();
        pin_text_at_path(&path, "第二条").unwrap();

        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.pinned.len(), 1);
        assert_eq!(snapshot.pinned[0].text, "第二条");
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].text, "第二条");
        assert_eq!(snapshot.history[1].text, "第一条");

        clear_history_at_path(&path).unwrap();
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.pinned.len(), 1);
        assert!(snapshot.history.is_empty());

        clear_all_at_path(&path).unwrap();
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert!(snapshot.pinned.is_empty());
        assert!(snapshot.history.is_empty());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn pin_and_unpin_preserve_history_position() {
        let path = temp_store_path("pin_preserves_history_position");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        record_text_at_path(&path, "第一条").unwrap();
        record_text_at_path(&path, "第二条").unwrap();
        record_text_at_path(&path, "第三条").unwrap();

        pin_text_at_path(&path, "第二条").unwrap();
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.pinned.len(), 1);
        assert_eq!(snapshot.pinned[0].text, "第二条");
        assert_eq!(
            snapshot
                .history
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["第三条", "第二条", "第一条"]
        );

        unpin_text_at_path(&path, "第二条").unwrap();
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert!(snapshot.pinned.is_empty());
        assert_eq!(
            snapshot
                .history
                .iter()
                .map(|entry| entry.text.as_str())
                .collect::<Vec<_>>(),
            vec!["第三条", "第二条", "第一条"]
        );

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn record_existing_history_text_moves_it_to_newest() {
        let path = temp_store_path("record_existing_moves_top");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        let now = current_timestamp_secs();
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![
                test_entry("latest", now),
                test_entry("used", now.saturating_sub(1)),
            ],
        };
        save_snapshot_atomically(&path, &snapshot).unwrap();

        assert!(record_text_at_path(&path, "used").unwrap());
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.history[0].text, "used");
        assert!(snapshot.history[0].captured_at > snapshot.history[1].captured_at);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn overlong_text_is_truncated() {
        let long = "甲".repeat(MAX_TEXT_UTF16_UNITS + 1);
        let normalized =
            normalize_text_with_limit(&long, MAX_TEXT_UTF16_UNITS).expect("truncate long text");
        assert_eq!(normalized.chars().count(), MAX_TEXT_UTF16_UNITS);
    }

    #[test]
    fn default_limit_accepts_twenty_thousand_chars() {
        let prefs = ClipboardPrefs::default();
        let text = "a".repeat(20_000);
        let normalized =
            normalize_text_with_limit(&text, prefs.max_text_utf16_units).expect("normalize text");
        assert_eq!(normalized.chars().count(), 20_000);
    }

    #[test]
    fn config_limit_allows_twenty_thousand_chars() {
        let prefs = parse_clipboard_prefs("[clipboard]\nmax_text_utf16_units=20000\n");
        assert_eq!(prefs.max_text_utf16_units, 20_000);
    }

    #[test]
    fn privacy_never_clipboard_processes_parse_and_match() {
        let prefs = parse_clipboard_prefs(
            "[clipboard]\nbackground_enabled=1\n[privacy]\nnever_clipboard_processes=secret*.exe, KeePass.exe\n",
        );
        assert!(prefs.background_enabled);
        assert!(process_matches_list(
            "secret-notes.exe",
            &prefs.never_clipboard_processes
        ));
        assert!(process_matches_list(
            "keepass.exe",
            &prefs.never_clipboard_processes
        ));
        assert!(!process_matches_list(
            "notepad.exe",
            &prefs.never_clipboard_processes
        ));
    }

    #[test]
    fn global_privacy_mode_disables_clipboard_capture_preferences() {
        let prefs =
            parse_clipboard_prefs("[clipboard]\nbackground_enabled=1\n[privacy]\nenabled=1\n");
        assert!(prefs.privacy_enabled);
        assert!(prefs.background_enabled);
    }

    #[test]
    fn store_roundtrip_preserves_twenty_thousand_chars() {
        let path = temp_store_path("long_text_roundtrip");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        let text = "a".repeat(20_000);
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![test_entry(&text, 10)],
        };
        save_snapshot_atomically(&path, &snapshot).unwrap();
        let raw = fs::read(&path).unwrap();
        if encryption_enabled() {
            assert!(is_encrypted_blob(&raw));
            assert!(!raw.starts_with(b"SQLite format 3\0"));
        } else {
            assert!(raw.starts_with(b"SQLite format 3\0"));
            assert!(!is_encrypted_blob(&raw));
        }
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].text, text);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn duplicate_records_merge_and_count() {
        let path = temp_store_path("duplicate_count");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        record_text_at_path(&path, "same").unwrap();
        record_text_at_path(&path, "other").unwrap();
        record_text_at_path(&path, "same").unwrap();
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].text, "same");
        assert_eq!(snapshot.history[0].copy_count, 2);
        assert!(snapshot.history[0].first_captured_at < snapshot.history[0].captured_at);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn recent_system_duplicate_is_coalesced() {
        let path = temp_store_path("system_duplicate_coalesce");
        remove_test_store(&path);

        assert!(record_text_at_path(&path, "same").unwrap());
        assert!(!record_text_at_path_with_mode(
            &path,
            "same",
            DuplicateRecordMode::CoalesceRecentSystemEvent
        )
        .unwrap());

        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].text, "same");
        assert_eq!(snapshot.history[0].copy_count, 1);

        remove_test_store(&path);
    }

    #[test]
    fn older_system_duplicate_still_counts() {
        let path = temp_store_path("system_duplicate_old_counts");
        remove_test_store(&path);

        let captured_at = current_timestamp_secs()
            .saturating_sub(SYSTEM_CLIPBOARD_DUPLICATE_WINDOW_SECS)
            .saturating_sub(1);
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![test_entry("same", captured_at)],
        };
        save_snapshot_atomically(&path, &snapshot).unwrap();

        assert!(record_text_at_path_with_mode(
            &path,
            "same",
            DuplicateRecordMode::CoalesceRecentSystemEvent
        )
        .unwrap());

        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].text, "same");
        assert_eq!(snapshot.history[0].copy_count, 2);

        remove_test_store(&path);
    }

    #[test]
    fn mark_runtime_seen_text_normalizes_clipboard_text() {
        mark_runtime_seen_text("alpha\r\n").unwrap();
        let runtime = runtime().lock().unwrap();
        assert!(runtime.last_poll.is_some());
        assert_eq!(runtime.last_seen_text.as_deref(), Some("alpha\n"));
    }

    #[test]
    fn cached_snapshot_is_an_in_memory_arc_snapshot() {
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![test_entry("cached", 42)],
        };
        update_snapshot_cache(snapshot);
        let first = cached_snapshot().expect("cached snapshot");
        let second = cached_snapshot().expect("cached snapshot clone");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.history[0].text, "cached");
    }

    #[test]
    fn existing_over_limit_entries_can_still_be_managed() {
        let path = temp_store_path("manage_existing_over_limit");
        remove_test_store(&path);

        let long = "旧".repeat(MAX_TEXT_UTF16_UNITS + 1);
        let snapshot = ClipboardSnapshot {
            pinned: Vec::new(),
            history: vec![test_entry(&long, 10)],
        };
        save_snapshot_atomically(&path, &snapshot).unwrap();

        assert!(pin_text_at_path(&path, &long).unwrap());
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert_eq!(snapshot.pinned.len(), 1);
        assert_eq!(snapshot.pinned[0].text, long);

        assert!(unpin_text_at_path(&path, &long).unwrap());
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert!(snapshot.pinned.is_empty());
        assert_eq!(snapshot.history[0].text, long);

        assert!(remove_saved_text_at_path(&path, &long).unwrap());
        let snapshot = load_snapshot_at_path(&path).unwrap();
        assert!(snapshot.pinned.is_empty());
        assert!(snapshot.history.is_empty());

        remove_test_store(&path);
    }

    #[test]
    fn store_encoding_roundtrips_current_format() {
        let plain = b"SQLite format 3\0opaque sqlite bytes";
        let encoded = encode_store_contents(plain).unwrap();
        assert_eq!(decode_store_contents(&encoded).unwrap(), plain);
        if encryption_enabled() {
            assert!(is_encrypted_blob(&encoded));
            assert_ne!(encoded, plain);
        } else {
            assert_eq!(encoded, plain);
        }
    }

    #[cfg(windows)]
    #[test]
    fn clipboard_store_rejects_plaintext_sqlite() {
        assert!(decode_store_contents(b"SQLite format 3\0plaintext").is_err());
    }

    #[test]
    fn record_preserves_clipboard_text_verbatim() {
        let path = temp_store_path("preserve_verbatim_record");
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));

        assert!(record_text_at_path(&path, "name@example.com").unwrap());
        assert!(record_text_at_path(&path, "123456789012").unwrap());
        assert!(record_text_at_path(&path, "hello").unwrap());

        let snapshot = load_snapshot_at_path(&path).unwrap();
        let texts: Vec<&str> = snapshot
            .history
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(texts.len(), 3);
        assert!(texts.contains(&"name@example.com"));
        assert!(texts.contains(&"123456789012"));
        assert!(texts.contains(&"hello"));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(temp_path_for(&path));
        let _ = fs::remove_file(lock_path_for(&path));
    }

    #[test]
    fn prune_snapshot_does_not_expire_entries_by_age() {
        let now = current_timestamp_secs();
        let mut snapshot = ClipboardSnapshot {
            pinned: vec![test_entry("pinned-old", now.saturating_sub(10 * 86_400))],
            history: vec![
                test_entry("old", now.saturating_sub(10 * 86_400)),
                test_entry("new", now),
            ],
        };
        prune_snapshot(
            &mut snapshot,
            &ClipboardPrefs {
                max_age_days: 7,
                ..ClipboardPrefs::default()
            },
        );
        assert_eq!(snapshot.pinned.len(), 1);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].text, "old");
        assert_eq!(snapshot.history[1].text, "new");
    }
}
