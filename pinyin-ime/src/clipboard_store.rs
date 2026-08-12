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
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(250);
const BACKGROUND_POLL_TICK: Duration = Duration::from_millis(250);
#[cfg(windows)]
const CLIPBOARD_EVENT_DEBOUNCE: Duration = Duration::from_millis(80);
#[cfg(windows)]
const CLIPBOARD_EVENT_QUEUE_CAPACITY: usize = 16;
const SYSTEM_CLIPBOARD_DUPLICATE_WINDOW_SECS: u64 = 2;
const MAX_HISTORY_ITEMS: usize = 60;
const MAX_PINNED_ITEMS: usize = 24;
const MAX_TEXT_UTF16_UNITS: usize = 20_000;
const MAX_AGE_DAYS: usize = 0;

static BACKGROUND_POLLING_ENABLED: AtomicBool = AtomicBool::new(false);
static CLIPBOARD_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static SNAPSHOT_REFRESH_ACTIVE: AtomicBool = AtomicBool::new(false);
static SNAPSHOT_CAPTURE_REQUESTED: AtomicBool = AtomicBool::new(false);
static CLIPBOARD_EVENT_MODE: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static CLIPBOARD_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static COM_WARNING_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static CLIPBOARD_EVENT_QUEUE: ClipboardEventQueue = ClipboardEventQueue::new();

#[cfg(windows)]
struct ClipboardEventQueue {
    slots: [AtomicU64; CLIPBOARD_EVENT_QUEUE_CAPACITY],
    head: AtomicU64,
    tail: AtomicU64,
}

#[cfg(windows)]
impl ClipboardEventQueue {
    const fn new() -> Self {
        Self {
            slots: [const { AtomicU64::new(0) }; CLIPBOARD_EVENT_QUEUE_CAPACITY],
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
        }
    }
}

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
    last_seen_sequence: Option<u32>,
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
fn enqueue_clipboard_event_request(request_id: u64) {
    let head = CLIPBOARD_EVENT_QUEUE.head.load(Ordering::Relaxed);
    let tail = CLIPBOARD_EVENT_QUEUE.tail.load(Ordering::Acquire);
    if head.wrapping_sub(tail) >= CLIPBOARD_EVENT_QUEUE_CAPACITY as u64 {
        CLIPBOARD_EVENT_QUEUE.tail.fetch_add(1, Ordering::AcqRel);
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_update_queue",
            format!("status=dropped reason=queue_full capacity={CLIPBOARD_EVENT_QUEUE_CAPACITY}"),
        );
    }
    let slot = (head as usize) % CLIPBOARD_EVENT_QUEUE_CAPACITY;
    CLIPBOARD_EVENT_QUEUE.slots[slot].store(request_id, Ordering::Release);
    CLIPBOARD_EVENT_QUEUE
        .head
        .store(head.wrapping_add(1), Ordering::Release);
}

#[cfg(windows)]
fn dequeue_clipboard_event_request() -> Option<u64> {
    let tail = CLIPBOARD_EVENT_QUEUE.tail.load(Ordering::Relaxed);
    let head = CLIPBOARD_EVENT_QUEUE.head.load(Ordering::Acquire);
    if tail == head {
        return None;
    }
    let slot = (tail as usize) % CLIPBOARD_EVENT_QUEUE_CAPACITY;
    let request_id = CLIPBOARD_EVENT_QUEUE.slots[slot].swap(0, Ordering::AcqRel);
    CLIPBOARD_EVENT_QUEUE
        .tail
        .store(tail.wrapping_add(1), Ordering::Release);
    (request_id != 0).then_some(request_id)
}

#[cfg(windows)]
fn clear_clipboard_event_queue() {
    let tail = CLIPBOARD_EVENT_QUEUE.tail.load(Ordering::Relaxed);
    let head = CLIPBOARD_EVENT_QUEUE.head.load(Ordering::Acquire);
    for index in tail..head {
        CLIPBOARD_EVENT_QUEUE.slots[(index as usize) % CLIPBOARD_EVENT_QUEUE_CAPACITY]
            .store(0, Ordering::Release);
    }
    CLIPBOARD_EVENT_QUEUE.tail.store(head, Ordering::Release);
}

#[cfg(windows)]
fn clipboard_background_worker_loop() {
    // COM must be initialised on this thread so OLE clipboard data
    // (used by Chrome, Edge, and other Chromium-based browsers) can be
    // read via OleGetClipboard / IDataObject::GetData.
    let _com = ComGuard::init();

    loop {
        if !BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
            clear_clipboard_event_queue();
            std::thread::sleep(BACKGROUND_POLL_TICK);
            continue;
        }

        if CLIPBOARD_EVENT_MODE.load(Ordering::Acquire) {
            if let Some(request_id) = dequeue_clipboard_event_request() {
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
struct ComGuard {
    initialized: bool,
}

#[cfg(windows)]
impl ComGuard {
    fn init() -> Self {
        // COINIT_APARTMENTTHREADED is required for the window message pump
        // (clipboard-listener thread) and also enables OLE clipboard access
        // for the background worker thread.
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let result =
            unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
        let initialized = result == 0 || result == 1; // S_OK / S_FALSE
        if !initialized && !COM_WARNING_LOGGED.swap(true, Ordering::AcqRel) {
            let detail = if result == 0x80010106u32 as i32 {
                "RPC_E_CHANGED_MODE; OLE clipboard path disabled on this thread"
            } else {
                "COM initialization failed; OLE clipboard path disabled on this thread"
            };
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_com_init",
                format!(
                    "status=failed hresult=0x{:08X} reason={detail}",
                    result as u32
                ),
            );
        }
        ComGuard { initialized }
    }
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                windows_sys::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

#[cfg(windows)]
fn run_clipboard_listener_or_poll() {
    // The message-pump thread must initialise COM (apartment-threaded) so
    // OLE clipboard data from Chrome, Edge, etc. can be read.
    let _com = ComGuard::init();

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::DataExchange::{
        AddClipboardFormatListener, RemoveClipboardFormatListener,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, IsWindow, PeekMessageW, RegisterClassW,
        TranslateMessage, CS_HREDRAW, CS_VREDRAW, HWND_MESSAGE, MSG, PM_REMOVE, WM_CLIPBOARDUPDATE,
        WNDCLASSW,
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
                    enqueue_clipboard_event_request(request_id);
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

    let class_name: Vec<u16> = format!(
        "ClipboardListener_{:x}_{:x}",
        std::process::id(),
        current_timestamp_secs()
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect();
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    let class_registered = unsafe { RegisterClassW(&wc) != 0 };
    let create_listener = || {
        if !class_registered {
            return 0;
        }
        let hwnd = unsafe {
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
        };
        if hwnd != 0 && unsafe { AddClipboardFormatListener(hwnd) != 0 } {
            hwnd
        } else {
            if hwnd != 0 {
                unsafe { RemoveClipboardFormatListener(hwnd) };
            }
            0
        }
    };

    let mut hwnd = 0;
    loop {
        if hwnd == 0 || unsafe { IsWindow(hwnd) == 0 } {
            if hwnd != 0 {
                unsafe { RemoveClipboardFormatListener(hwnd) };
            }
            hwnd = create_listener();
            CLIPBOARD_EVENT_MODE.store(hwnd != 0, Ordering::Release);
        }

        if hwnd == 0 {
            if !worker_started && BACKGROUND_POLLING_ENABLED.load(Ordering::Acquire) {
                let _ = poll_system_clipboard_if_due(false);
            }
            std::thread::sleep(BACKGROUND_POLL_TICK);
            continue;
        }

        let mut msg: MSG = unsafe { std::mem::zeroed() };
        let mut processed = false;
        while unsafe { PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) } != 0 {
            processed = true;
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if !processed {
            std::thread::sleep(BACKGROUND_POLL_TICK);
        }
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

pub fn configured_max_age_days() -> u64 {
    clipboard_prefs().max_age_days as u64
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

fn with_store_mut_at_path_impl<R>(
    path: &Path,
    publish_runtime_cache: bool,
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
    if result.is_ok() && publish_runtime_cache {
        // Publish the exact committed state while the cross-process store lock
        // is still held. Clipboard candidates can then observe a successful
        // capture immediately instead of waiting for a later async reload.
        update_snapshot_cache(snapshot.clone());
    }
    let _ = lock_file.unlock();
    drop(lock_file);
    let _ = fs::remove_file(lock_path);
    result
}

fn with_store_mut_at_path<R>(
    path: &Path,
    f: impl FnOnce(&mut ClipboardSnapshot) -> Result<R, String>,
) -> Result<R, String> {
    with_store_mut_at_path_impl(path, false, f)
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
    record_text_at_path_with_mode_and_cache(path, text, duplicate_mode, path == store_path())
}

fn record_text_at_path_with_mode_and_cache(
    path: &Path,
    text: &str,
    duplicate_mode: DuplicateRecordMode,
    publish_runtime_cache: bool,
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
    with_store_mut_at_path_impl(path, publish_runtime_cache, |snapshot| {
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
fn clipboard_sequence_number() -> Option<u32> {
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;

    let sequence = unsafe { GetClipboardSequenceNumber() };
    (sequence != 0).then_some(sequence)
}

#[cfg(not(windows))]
fn clipboard_sequence_number() -> Option<u32> {
    None
}

#[cfg(windows)]
struct OpenClipboardGuard;

#[cfg(windows)]
impl OpenClipboardGuard {
    fn open_with_retry() -> Result<Self, String> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::DataExchange::OpenClipboard;

        let mut last_error = 0;
        for attempt in 0..12 {
            if unsafe { OpenClipboard(0) } != 0 {
                return Ok(Self);
            }
            last_error = unsafe { GetLastError() };
            let delay_ms = (12.0 * 1.5_f64.powi(attempt)).round() as u64;
            std::thread::sleep(Duration::from_millis(delay_ms.min(240)));
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

#[cfg(windows)]
fn read_ole_clipboard_text() -> Result<Option<String>, String> {
    // Chrome (and other Chromium-based browsers) uses OLE delayed rendering
    // for clipboard data.  CF_UNICODETEXT may be reported as available by
    // IsClipboardFormatAvailable, but GetClipboardData returns NULL because
    // the OLE data object hasn't rendered the text yet (or the calling thread
    // hasn't initialized COM).  Fall back to ::OleGetClipboard +
    // IDataObject::GetData, which talks to the OLE clipboard directly.
    use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::{OleGetClipboard, CF_UNICODETEXT};

    let data_object: IDataObject =
        unsafe { OleGetClipboard() }.map_err(|e| format!("OleGetClipboard failed: {e}"))?;

    let formatetc = FORMATETC {
        cfFormat: CF_UNICODETEXT.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };

    let medium = match unsafe { data_object.GetData(&formatetc) } {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };

    // Explicit drop of data_object so the IDataObject is released before we
    // inspect the medium — some clipboard owners hold an internal lock on the
    // object that interferes with GlobalLock on the medium.
    drop(data_object);

    let hglobal_raw = unsafe { medium.u.hGlobal.0 };
    if medium.tymed != TYMED_HGLOBAL.0 as u32 || hglobal_raw.is_null() {
        // Release the medium even when we don't consume it.
        let mut m = medium;
        unsafe {
            windows::Win32::System::Ole::ReleaseStgMedium(&mut m);
        }
        return Ok(None);
    }

    let text = unsafe { read_hglobal_unicode_text(hglobal_raw) };
    let mut m = medium;
    unsafe {
        windows::Win32::System::Ole::ReleaseStgMedium(&mut m);
    }
    text
}

#[cfg(windows)]
unsafe fn read_hglobal_unicode_text(
    hglobal: *mut std::ffi::c_void,
) -> Result<Option<String>, String> {
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    let ptr = unsafe { GlobalLock(hglobal) } as *const u16;
    if ptr.is_null() {
        return Err(format!(
            "GlobalLock(OLE CF_UNICODETEXT) failed error={}",
            std::io::Error::last_os_error()
        ));
    }
    let result = (|| {
        let byte_len = unsafe { GlobalSize(hglobal) };
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
        GlobalUnlock(hglobal);
    }
    result
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
    // Synchronous callers may not be running on the background STA thread.
    // Balance this per-call COM initialization and disable only the OLE path
    // when the caller already selected an incompatible apartment model.
    let com = ComGuard::init();
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
    for attempt in 0..9 {
        let sequence_before = clipboard_sequence_number();
        let mut text = None;
        match read_system_clipboard_text_win32() {
            Ok(Some(value)) => text = Some(value),
            Ok(None) => {}
            Err(err) => last_error = Some(err),
        }

        if text.is_none() && clipboard_win::is_format_avail(clipboard_win::formats::CF_UNICODETEXT)
        {
            saw_unicode_format = true;
            match clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode) {
                Ok(value) => text = Some(value),
                Err(err) => last_error = Some(err.to_string()),
            }
        }

        // Chrome and other Chromium-based apps use OLE delayed rendering.
        // After direct Win32 and clipboard_win both come up empty, try the
        // COM OLE clipboard path (OleGetClipboard → IDataObject::GetData).
        if text.is_none() && com.initialized {
            match read_ole_clipboard_text() {
                Ok(Some(value)) => text = Some(value),
                Ok(None) => {}
                Err(err) => last_error = Some(err),
            }
        }

        if let Some(value) = text {
            let sequence_after = clipboard_sequence_number();
            if sequence_before.is_some()
                && sequence_after.is_some()
                && sequence_before != sequence_after
            {
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Verbose,
                    "clipboard_read_retry",
                    format!(
                        "status=retry request_id={} reason=sequence_changed before={} after={}",
                        request_id,
                        sequence_before.unwrap_or_default(),
                        sequence_after.unwrap_or_default()
                    ),
                );
            } else {
                return normalize_read_clipboard_text(&value, request_id);
            }
        }

        if attempt < 8 {
            let delay_ms = match attempt {
                0 => 30,
                1 => 45,
                2 => 60,
                3 => 75,
                4 => 100,
                5 => 200,
                6 => 400,
                _ => 800,
            };
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    if let Some(err) = last_error {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_read_fallback_failed",
            format!(
                "status=failed request_id={} retries=9 reason={err}",
                request_id
            ),
        );
    } else if !saw_unicode_format {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Verbose,
            "clipboard_read_skip",
            format!(
                "status=skipped request_id={} retries=9 reason=no_cf_unicode_text",
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
        let sequence = clipboard_sequence_number();
        let sequence_unchanged = sequence.is_some()
            && runtime.last_seen_sequence.is_some()
            && sequence == runtime.last_seen_sequence;
        let poll_interval = if CLIPBOARD_EVENT_MODE.load(Ordering::Acquire) {
            POLL_INTERVAL
        } else {
            FALLBACK_POLL_INTERVAL
        };
        if !force
            && (sequence_unchanged
                || runtime
                    .last_poll
                    .map(|last| last.elapsed() < poll_interval)
                    .unwrap_or(false))
        {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Verbose,
                "clipboard_capture_skip",
                format!(
                    "status=skipped request_id={} reason={}",
                    request_id,
                    if sequence_unchanged {
                        "sequence_unchanged"
                    } else {
                        "poll_interval"
                    }
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
        let sequence = clipboard_sequence_number();
        let same_sequence = sequence.is_some()
            && runtime.last_seen_sequence.is_some()
            && sequence == runtime.last_seen_sequence;
        if runtime.last_seen_text.as_deref() == Some(text.as_str())
            && !record_duplicate_text
            && (sequence.is_none() || same_sequence)
        {
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
        runtime.last_seen_sequence = sequence;
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
    runtime.last_seen_sequence = clipboard_sequence_number();
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

/// Captures the current system clipboard and returns the committed snapshot.
///
/// Clipboard candidate commands use this synchronous boundary so they never
/// start an async refresh and immediately render an older cached snapshot.
pub fn capture_system_clipboard_snapshot() -> Result<Arc<ClipboardSnapshot>, String> {
    snapshot_after_capture(|| capture_system_clipboard(true).map(|_| ()))
}

fn snapshot_after_capture(
    capture: impl FnOnce() -> Result<(), String>,
) -> Result<Arc<ClipboardSnapshot>, String> {
    capture()?;
    if let Some(snapshot) = cached_snapshot() {
        return Ok(snapshot);
    }
    let _ = snapshot()?;
    Ok(cached_snapshot().unwrap_or_else(|| Arc::new(ClipboardSnapshot::default())))
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
    runtime.last_seen_sequence = clipboard_sequence_number();
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
    runtime.last_seen_sequence = clipboard_sequence_number();
    Ok(Some(text))
}
