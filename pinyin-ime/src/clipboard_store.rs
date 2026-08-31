use crate::runtime_log::{self, RuntimeLogLevel};
use fs2::FileExt;
use rusqlite::{params, Connection, DatabaseName};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
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
#[cfg(windows)]
const CLIPBOARD_LISTENER_IDLE_TICK: Duration = Duration::from_millis(50);
#[cfg(windows)]
const CLIPBOARD_READ_MAX_ATTEMPTS: usize = 3;
const STORE_WRITE_DEBOUNCE: Duration = Duration::from_millis(50);
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
/// When the event queue is full the latest request id lands here so the worker
/// captures the newest state once after draining instead of dropping it.
#[cfg(windows)]
static CLIPBOARD_COALESCED_REQUEST: AtomicU64 = AtomicU64::new(0);

/// A pending store mutation. Records carry the fully built entry so the id and
/// timestamp assigned at enqueue time are identical everywhere the entry is
/// applied (optimistic cache, flush, later disk loads). Clipboard candidates
/// resolve by id, so a second id assignment at flush time would break commits.
#[derive(Clone)]
enum StoreOp {
    Record { entry: ClipboardEntry },
}

fn pending_ops() -> &'static (Mutex<VecDeque<StoreOp>>, Condvar) {
    static OPS: OnceLock<(Mutex<VecDeque<StoreOp>>, Condvar)> = OnceLock::new();
    OPS.get_or_init(|| (Mutex::new(VecDeque::new()), Condvar::new()))
}

fn drain_pending_ops() -> Vec<StoreOp> {
    match pending_ops().0.lock() {
        Ok(mut queue) => queue.drain(..).collect(),
        Err(_) => Vec::new(),
    }
}

fn requeue_store_ops(ops: Vec<StoreOp>) {
    if let Ok(mut queue) = pending_ops().0.lock() {
        for op in ops.into_iter().rev() {
            queue.push_front(op);
        }
    }
}

fn apply_store_ops(snapshot: &mut ClipboardSnapshot, ops: &[StoreOp], prefs: &ClipboardPrefs) {
    for op in ops {
        let StoreOp::Record { entry } = op;
        apply_record_entry(snapshot, entry, prefs);
    }
}

/// Applies pending ops to `snapshot` without removing them from the queue.
/// Used by read paths so returned snapshots include not-yet-flushed records;
/// the queue stays intact for the writer thread.
fn merge_pending_ops_into(snapshot: &mut ClipboardSnapshot, prefs: &ClipboardPrefs) {
    let ops: Vec<StoreOp> = match pending_ops().0.lock() {
        Ok(queue) => queue.iter().cloned().collect(),
        Err(_) => return,
    };
    apply_store_ops(snapshot, &ops, prefs);
}

/// Publishes a committed snapshot and merges records enqueued while it was
/// being built. The merge runs under the runtime mutex so a concurrent
/// optimistic cache update can never be dropped by the replace.
fn publish_snapshot_with_pending(mut snapshot: ClipboardSnapshot) {
    let prefs = clipboard_prefs();
    let Ok(mut runtime) = runtime().lock() else {
        return;
    };
    merge_pending_ops_into(&mut snapshot, &prefs);
    runtime.snapshot_cache = Some(Arc::new(snapshot));
}

static STORE_WRITER_STARTED: OnceLock<()> = OnceLock::new();

fn kick_store_writer() {
    STORE_WRITER_STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("kaixin-clipboard-store-writer".to_string())
            .spawn(store_writer_loop);
    });
    pending_ops().1.notify_one();
}

fn store_writer_loop() {
    let (ops, wake) = pending_ops();
    loop {
        {
            let Ok(queue) = ops.lock() else {
                return;
            };
            if queue.is_empty() {
                // The debounce timeout bounds flush latency for bursts and
                // also recovers any notify lost to a start-up race.
                let _ = wake.wait_timeout(queue, STORE_WRITE_DEBOUNCE);
            }
        }
        if let Err(err) = flush_pending_store_ops() {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_store_flush",
                format!("status=failed reason={err}"),
            );
            std::thread::sleep(STORE_WRITE_DEBOUNCE);
        }
    }
}

/// Flushes queued records to disk. Failed saves requeue the ops so a transient
/// write error does not lose captures; the loop backs off between attempts.
fn flush_pending_store_ops() -> Result<(), String> {
    let ops = {
        let Ok(mut queue) = pending_ops().0.lock() else {
            return Err("lock clipboard store op queue".to_string());
        };
        if queue.is_empty() {
            return Ok(());
        }
        queue.drain(..).collect::<Vec<_>>()
    };
    if let Err(err) = flush_store_ops_to_path(&store_path(), &ops) {
        requeue_store_ops(ops);
        return Err(err);
    }
    Ok(())
}

/// Persists queued records on the graceful shutdown path so a user quit never
/// loses captures the writer has not flushed yet.
pub fn flush_pending_ops_sync() {
    for _ in 0..8 {
        let empty = pending_ops()
            .0
            .lock()
            .map(|queue| queue.is_empty())
            .unwrap_or(true);
        if empty {
            break;
        }
        if let Err(err) = flush_pending_store_ops() {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_store_flush",
                format!("status=failed reason={err}"),
            );
            break;
        }
    }
}

fn flush_store_ops_to_path(path: &Path, ops: &[StoreOp]) -> Result<(), String> {
    if ops.is_empty() {
        return Ok(());
    }
    let lock_path = lock_path_for(path);
    let lock_file = open_lock_file(path)?;
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("lock clipboard store: {e}"))?;
    let prefs = clipboard_prefs();
    let mut snapshot = restore_backup_if_needed(path)?;
    prune_snapshot(&mut snapshot, &prefs);
    let before = snapshot.clone();
    apply_store_ops(&mut snapshot, ops, &prefs);
    prune_snapshot(&mut snapshot, &prefs);
    if snapshot != before {
        save_snapshot_atomically(path, &snapshot)?;
    }
    publish_snapshot_with_pending(snapshot);
    let _ = lock_file.unlock();
    drop(lock_file);
    let _ = fs::remove_file(lock_path);
    Ok(())
}

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
        Ok(snapshot) => publish_snapshot_with_pending(snapshot),
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
        // Queue full: remember the latest request so the worker captures the
        // newest state once after draining, instead of dropping the event.
        CLIPBOARD_COALESCED_REQUEST.store(request_id, Ordering::Release);
        runtime_log::log_clipboard_lazy(RuntimeLogLevel::Error, "clipboard_update_queue", || {
            format!("status=coalesced reason=queue_full capacity={CLIPBOARD_EVENT_QUEUE_CAPACITY}")
        });
        return;
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
    CLIPBOARD_COALESCED_REQUEST.store(0, Ordering::Release);
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
            // A full queue collapses a burst into this single capture request.
            let coalesced = CLIPBOARD_COALESCED_REQUEST.swap(0, Ordering::AcqRel);
            if coalesced != 0 {
                std::thread::sleep(CLIPBOARD_EVENT_DEBOUNCE);
                let _ = poll_system_clipboard_changed_event_with_request(coalesced);
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
                runtime_log::log_clipboard_lazy(
                    RuntimeLogLevel::Verbose,
                    "clipboard_update",
                    || {
                        format!(
                            "status=received request_id={} worker_active={}",
                            request_id,
                            if worker_active { 1 } else { 0 }
                        )
                    },
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
            // Short idle tick so a queued WM_CLIPBOARDUPDATE is dispatched
            // quickly; the capture itself stays debounced on the worker.
            std::thread::sleep(CLIPBOARD_LISTENER_IDLE_TICK);
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
        stamp: Option<(SystemTime, u64)>,
        checked_at: Instant,
        prefs: ClipboardPrefs,
    }

    static CACHE: OnceLock<Mutex<Option<CachedPrefs>>> = OnceLock::new();
    // Stat at most once per interval: clipboard_prefs() sits on the per-poll,
    // per-read, and per-lookup hot paths, so a metadata syscall per call was
    // measurable. Config edits propagate within one interval.
    const PREFS_STAT_INTERVAL: Duration = Duration::from_millis(250);

    let path = config_path();
    if let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(None)).lock() {
        let stat_due = cache
            .as_ref()
            .map(|cached| cached.checked_at.elapsed() >= PREFS_STAT_INTERVAL)
            .unwrap_or(true);
        if !stat_due {
            return cache
                .as_ref()
                .map(|cached| cached.prefs.clone())
                .unwrap_or_default();
        }
        let stamp = std::fs::metadata(&path).ok().and_then(|metadata| {
            metadata
                .modified()
                .ok()
                .map(|modified| (modified, metadata.len()))
        });
        if let Some(cached) = cache.as_mut() {
            cached.checked_at = Instant::now();
            if cached.stamp == stamp {
                return cached.prefs.clone();
            }
        }
        let prefs = std::fs::read_to_string(&path)
            .map(|text| parse_clipboard_prefs(&text))
            .unwrap_or_default();
        *cache = Some(CachedPrefs {
            stamp,
            checked_at: Instant::now(),
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
    // Queued records must be part of the base state the mutation sees: they
    // are removed from the queue here because this path saves the merged
    // result, and leaving them queued would apply them a second time.
    let pending = if path == store_path() {
        drain_pending_ops()
    } else {
        Vec::new()
    };
    apply_store_ops(&mut snapshot, &pending, &prefs);
    prune_snapshot(&mut snapshot, &prefs);
    let before = snapshot.clone();
    let result = f(&mut snapshot);
    prune_snapshot(&mut snapshot, &prefs);
    if result.is_ok() && snapshot != before {
        if let Err(err) = save_snapshot_atomically(path, &snapshot) {
            requeue_store_ops(pending);
            let _ = lock_file.unlock();
            drop(lock_file);
            let _ = fs::remove_file(lock_path);
            return Err(err);
        }
    }
    if result.is_err() {
        requeue_store_ops(pending);
    }
    if result.is_ok() && publish_runtime_cache {
        // Publish the exact committed state while the cross-process store lock
        // is still held. Clipboard candidates can then observe a successful
        // capture immediately instead of waiting for a later async reload.
        publish_snapshot_with_pending(snapshot.clone());
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

/// Ordering for the kept-sorted entry vectors: newest capture first, text
/// ascending as the tiebreak (mirrors the SQLite read order).
fn entry_sort_cmp(a: &ClipboardEntry, b: &ClipboardEntry) -> std::cmp::Ordering {
    b.captured_at
        .cmp(&a.captured_at)
        .then_with(|| a.text.cmp(&b.text))
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
    // The vector is kept sorted, so a binary search insertion replaces the
    // previous full sort (O(N) text comparisons on strings up to 20k units).
    // binary_search_by wants the probe's ordering relative to the target.
    let insert_at = entries
        .binary_search_by(|existing| entry_sort_cmp(existing, &entry))
        .unwrap_or_else(|pos| pos);
    entries.insert(insert_at, entry);
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

/// Builds the entry for a record. Called once per record so the id and
/// timestamp are stable across the optimistic cache, the flush, and later
/// disk loads — clipboard candidates resolve entries by id.
fn build_record_entry(
    snapshot: &ClipboardSnapshot,
    text: &str,
    prefs: &ClipboardPrefs,
    source_app: Option<&str>,
) -> ClipboardEntry {
    let captured_at = next_entry_timestamp(snapshot);
    new_entry(
        text.to_string(),
        captured_at,
        prefs
            .record_source_app
            .then(|| source_app.map(str::to_string))
            .flatten(),
    )
}

/// Mirrors the old record closure: refresh a pinned copy when the text is
/// pinned, then upsert into history.
fn apply_record_entry(
    snapshot: &mut ClipboardSnapshot,
    entry: &ClipboardEntry,
    prefs: &ClipboardPrefs,
) {
    if snapshot
        .pinned
        .iter()
        .any(|existing| existing.text == entry.text)
    {
        upsert_entry(&mut snapshot.pinned, entry.clone(), prefs.max_pinned_items);
    }
    upsert_entry(
        &mut snapshot.history,
        entry.clone(),
        prefs.max_history_items,
    );
}

/// Makes a freshly recorded entry visible to the next clipboard candidate
/// lookup immediately, without waiting for the writer flush.
fn optimistic_record_into_cache(entry: &ClipboardEntry, prefs: &ClipboardPrefs) {
    let Ok(mut runtime) = runtime().lock() else {
        return;
    };
    let Some(cache) = runtime.snapshot_cache.as_mut() else {
        return;
    };
    let snapshot = Arc::make_mut(cache);
    apply_record_entry(snapshot, entry, prefs);
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
    if path == store_path() {
        // Hot path: queue the record for the background writer and update the
        // runtime cache optimistically. Per-copy events never touch the disk,
        // the store lock, or the DPAPI round trip from the caller's thread.
        let base = cached_snapshot().unwrap_or_else(|| Arc::new(ClipboardSnapshot::default()));
        if duplicate_mode == DuplicateRecordMode::CoalesceRecentSystemEvent
            && is_recent_system_duplicate(&base, &text, current_timestamp_secs())
        {
            return Ok(false);
        }
        let entry = build_record_entry(&base, &text, &prefs, source_app.as_deref());
        {
            let Ok(mut queue) = pending_ops().0.lock() else {
                return Err("lock clipboard store op queue".to_string());
            };
            queue.push_back(StoreOp::Record {
                entry: entry.clone(),
            });
        }
        if publish_runtime_cache {
            optimistic_record_into_cache(&entry, &prefs);
        }
        kick_store_writer();
        return Ok(true);
    }
    // Non-store paths (tests) keep the synchronous implementation.
    with_store_mut_at_path_impl(path, publish_runtime_cache, |snapshot| {
        if duplicate_mode == DuplicateRecordMode::CoalesceRecentSystemEvent
            && is_recent_system_duplicate(snapshot, &text, current_timestamp_secs())
        {
            return Ok(false);
        }
        let entry = build_record_entry(snapshot, &text, &prefs, source_app.as_deref());
        apply_record_entry(snapshot, &entry, &prefs);
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
    with_store_mut_at_path_impl(path, true, |snapshot| {
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
    with_store_mut_at_path_impl(path, true, |snapshot| {
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
    with_store_mut_at_path_impl(path, true, |snapshot| {
        let removed_pinned = remove_text(&mut snapshot.pinned, &text);
        let removed_history = remove_text(&mut snapshot.history, &text);
        Ok(removed_pinned || removed_history)
    })
}

fn clear_history_at_path(path: &Path) -> Result<(), String> {
    with_store_mut_at_path_impl(path, true, |snapshot| {
        snapshot.history.clear();
        Ok(())
    })
}

fn clear_all_at_path(path: &Path) -> Result<(), String> {
    with_store_mut_at_path_impl(path, true, |snapshot| {
        snapshot.history.clear();
        snapshot.pinned.clear();
        Ok(())
    })
}

fn clear_older_than_days_at_path(path: &Path, days: u64) -> Result<usize, String> {
    let cutoff = current_timestamp_secs().saturating_sub(days.saturating_mul(86_400));
    let prefs = clipboard_prefs();
    with_store_mut_at_path_impl(path, true, |snapshot| {
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
        // Bounded backoff: missed opens are recovered by the 250 ms worker
        // tick and the fallback poll, so long exponential waits buy nothing.
        for attempt in 0..8 {
            if unsafe { OpenClipboard(0) } != 0 {
                return Ok(Self);
            }
            last_error = unsafe { GetLastError() };
            let delay_ms = (4 + 2 * attempt).min(40) as u64;
            std::thread::sleep(Duration::from_millis(delay_ms));
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
    runtime_log::log_clipboard_lazy(RuntimeLogLevel::Verbose, "clipboard_read", || {
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
        )
    });
    normalized
}

#[cfg(windows)]
fn read_system_clipboard_text(request_id: u64) -> Option<String> {
    // Synchronous callers may not be running on the background STA thread.
    // Balance this per-call COM initialization and disable only the OLE path
    // when the caller already selected an incompatible apartment model.
    let com = ComGuard::init();
    if clipboard_has_temporary_paste_marker() {
        runtime_log::log_clipboard_lazy(RuntimeLogLevel::Verbose, "clipboard_read_skip", || {
            format!(
                "status=skipped request_id={} reason=temporary_paste_marker",
                request_id
            )
        });
        return None;
    }

    // Clipboard owners are allowed to render CF_UNICODETEXT lazily.  A
    // WM_CLIPBOARDUPDATE can therefore arrive before GetClipboardData has any
    // text to return.  Retry the complete native read rather than treating the
    // first empty result as a missed copy event.  The retry count and delays
    // are bounded: this path runs per keystroke and per manager refresh, and
    // missed opens are recovered by the 250 ms worker tick / fallback poll.
    let mut last_error = None;
    let mut saw_unicode_format = false;
    for attempt in 0..CLIPBOARD_READ_MAX_ATTEMPTS {
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
                runtime_log::log_clipboard_lazy(
                    RuntimeLogLevel::Verbose,
                    "clipboard_read_retry",
                    || {
                        format!(
                            "status=retry request_id={} reason=sequence_changed before={} after={}",
                            request_id,
                            sequence_before.unwrap_or_default(),
                            sequence_after.unwrap_or_default()
                        )
                    },
                );
            } else {
                return normalize_read_clipboard_text(&value, request_id);
            }
        }

        if attempt + 1 < CLIPBOARD_READ_MAX_ATTEMPTS {
            let delay_ms = if attempt == 0 { 30 } else { 50 };
            std::thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    if let Some(err) = last_error {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_read_fallback_failed",
            format!(
                "status=failed request_id={} retries={CLIPBOARD_READ_MAX_ATTEMPTS} reason={err}",
                request_id
            ),
        );
    } else if !saw_unicode_format {
        runtime_log::log_clipboard_lazy(RuntimeLogLevel::Verbose, "clipboard_read_skip", || {
            format!(
                    "status=skipped request_id={} retries={CLIPBOARD_READ_MAX_ATTEMPTS} reason=no_cf_unicode_text",
                    request_id
                )
        });
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
        runtime_log::log_clipboard_lazy(RuntimeLogLevel::Verbose, "clipboard_capture_skip", || {
            format!(
                "status=skipped request_id={} reason=background_disabled",
                request_id
            )
        });
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
            runtime_log::log_clipboard_lazy(
                RuntimeLogLevel::Verbose,
                "clipboard_capture_skip",
                || {
                    format!(
                        "status=skipped request_id={} reason={}",
                        request_id,
                        if sequence_unchanged {
                            "sequence_unchanged"
                        } else {
                            "poll_interval"
                        }
                    )
                },
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
            runtime_log::log_clipboard_lazy(
                RuntimeLogLevel::Verbose,
                "clipboard_capture_skip",
                || {
                    format!(
                        "status=skipped request_id={} reason=unchanged units={} force={} background_required={}",
                        request_id,
                        text.encode_utf16().count(),
                        if force { 1 } else { 0 },
                        if require_background_enabled { 1 } else { 0 }
                    )
                },
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
    runtime_log::log_clipboard_lazy(RuntimeLogLevel::Basic, "clipboard_capture", || {
        format!(
                "status=ok request_id={} units={} recorded={} force={} duplicate_event={} background_required={}",
                request_id,
                text.encode_utf16().count(),
                if recorded { 1 } else { 0 },
                if force { 1 } else { 0 },
                if record_duplicate_text { 1 } else { 0 },
                if require_background_enabled { 1 } else { 0 }
            )
    });
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

/// Snapshot for clipboard candidate commands (vvu / vv cb).
///
/// Compares the system clipboard sequence number with the last capture
/// without opening the clipboard, so the common unchanged case never touches
/// the clipboard API, the store lock, or disk on the lookup thread. When the
/// sequence moved — a copy the background monitor has not recorded yet — one
/// synchronous capture commits the newest text before the first page renders;
/// an async refresh started at the same keystroke would complete after the
/// candidate list is already drawn, leaving the newest copy off the page.
pub fn clipboard_candidate_snapshot() -> Arc<ClipboardSnapshot> {
    if clipboard_prefs().privacy_enabled {
        return Arc::new(ClipboardSnapshot::default());
    }
    let sequence_changed = runtime()
        .lock()
        .ok()
        .map(
            |runtime| match (clipboard_sequence_number(), runtime.last_seen_sequence) {
                (Some(current), Some(seen)) => current != seen,
                // Never captured or the sequence is unavailable: capture once so
                // the first render is grounded in the real system clipboard.
                _ => true,
            },
        )
        .unwrap_or(true);
    if sequence_changed {
        return match capture_system_clipboard_snapshot() {
            Ok(snapshot) => snapshot,
            Err(_) => {
                // Clipboard busy: fall back to the cache and retry in the
                // background so the next keystroke still sees the newest copy.
                refresh_snapshot_cache_async(true);
                cached_snapshot().unwrap_or_else(|| Arc::new(ClipboardSnapshot::default()))
            }
        };
    }
    if let Some(snapshot) = cached_snapshot() {
        // The cache matches the system clipboard; reload the store off the
        // lookup thread to pick up cross-process changes (manager GUI, flush).
        refresh_snapshot_cache_async(false);
        return snapshot;
    }
    Arc::new(snapshot().unwrap_or_default())
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
    let mut snapshot = load_snapshot_at_path(&store_path())?;
    // Include records the writer has not flushed yet so callers (manager GUI,
    // resolve fallback) see the newest captures immediately.
    merge_pending_ops_into(&mut snapshot, &clipboard_prefs());
    update_snapshot_cache(snapshot.clone());
    Ok(snapshot)
}

/// Snapshot for clipboard id resolution. Serves the runtime cache when warm
/// (entry ids are immutable and never reused, so a cached snapshot is always
/// correct for any id it contains) and falls back to a full load on cold
/// start. Keeps the 750 ms pipe budget off the disk path entirely.
pub fn resolve_snapshot() -> Result<ClipboardSnapshot, String> {
    if let Some(snapshot) = cached_snapshot() {
        return Ok(snapshot.as_ref().clone());
    }
    snapshot()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs_with(history: usize, pinned: usize) -> ClipboardPrefs {
        ClipboardPrefs {
            max_history_items: history,
            max_pinned_items: pinned,
            ..ClipboardPrefs::default()
        }
    }

    fn make_entry(text: &str, captured_at: u64) -> ClipboardEntry {
        new_entry(text.to_string(), captured_at, None)
    }

    fn assert_sorted(entries: &[ClipboardEntry]) {
        for pair in entries.windows(2) {
            assert_eq!(
                entry_sort_cmp(&pair[0], &pair[1]),
                std::cmp::Ordering::Less,
                "entries not sorted: {:?}",
                entries
            );
        }
    }

    #[test]
    fn record_into_snapshot_appends_to_history() {
        let prefs = prefs_with(60, 24);
        let mut snapshot = ClipboardSnapshot::default();
        let entry = build_record_entry(&snapshot, "hello", &prefs, None);
        apply_record_entry(&mut snapshot, &entry, &prefs);
        assert_eq!(snapshot.history.len(), 1);
        assert_eq!(snapshot.history[0].text, "hello");
        assert!(snapshot.pinned.is_empty());
    }

    #[test]
    fn record_into_snapshot_refreshes_pinned_copy() {
        let prefs = prefs_with(60, 24);
        let mut snapshot = ClipboardSnapshot::default();
        let pinned = make_entry("keep me", 10);
        upsert_entry(&mut snapshot.pinned, pinned, prefs.max_pinned_items);
        let entry = build_record_entry(&snapshot, "keep me", &prefs, None);
        apply_record_entry(&mut snapshot, &entry, &prefs);
        assert_eq!(snapshot.pinned.len(), 1);
        assert_eq!(snapshot.pinned[0].copy_count, 2);
        assert_eq!(snapshot.history.len(), 1);
    }

    #[test]
    fn upsert_entry_merges_duplicate_text() {
        let prefs = prefs_with(60, 24);
        let mut entries = Vec::new();
        let first = make_entry("same text", 100);
        upsert_entry(&mut entries, first.clone(), prefs.max_history_items);
        let second = make_entry("same text", 101);
        upsert_entry(&mut entries, second, prefs.max_history_items);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, first.id, "existing id must be kept");
        assert_eq!(entries[0].captured_at, 101);
        assert_eq!(entries[0].copy_count, 2);
    }

    #[test]
    fn upsert_entry_keeps_sort_invariant_under_churn() {
        let prefs = prefs_with(300, 24);
        let mut entries = Vec::new();
        // Deterministic pseudo-random captured_at/text mix, including
        // duplicates and out-of-order timestamps.
        let mut seed = 0x5eedu64;
        for index in 0..200u64 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let captured_at = seed % 500;
            let text = if seed % 7 == 0 {
                "dup text".to_string()
            } else {
                format!("text {seed}")
            };
            let entry = make_entry(&text, captured_at);
            upsert_entry(&mut entries, entry, prefs.max_history_items);
            assert_sorted(&entries);
            let _ = index;
        }
        assert!(entries.len() <= prefs.max_history_items);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.text == "dup text")
                .count(),
            1,
            "duplicate texts must merge into one entry"
        );
    }

    #[test]
    fn upsert_entry_truncates_to_limit() {
        let prefs = prefs_with(5, 24);
        let mut entries = Vec::new();
        for captured_at in 0..10u64 {
            let entry = make_entry(&format!("text {captured_at}"), captured_at);
            upsert_entry(&mut entries, entry, prefs.max_history_items);
        }
        assert_eq!(entries.len(), 5);
        assert_sorted(&entries);
    }

    #[test]
    fn snapshot_sqlite_bytes_round_trip() {
        let prefs = prefs_with(60, 24);
        let mut snapshot = ClipboardSnapshot::default();
        for index in 0..5u64 {
            let entry = make_entry(&format!("history {index}"), index);
            upsert_entry(&mut snapshot.history, entry, prefs.max_history_items);
        }
        for index in 0..2u64 {
            let entry = make_entry(&format!("pinned {index}"), index);
            upsert_entry(&mut snapshot.pinned, entry, prefs.max_pinned_items);
        }
        let bytes = snapshot_to_sqlite_bytes(&snapshot).expect("serialize snapshot");
        let loaded = read_snapshot_from_sqlite_bytes(&bytes).expect("deserialize snapshot");
        assert_eq!(loaded, snapshot);
    }

    #[test]
    fn resolve_entry_text_finds_by_id() {
        let prefs = prefs_with(60, 24);
        let mut snapshot = ClipboardSnapshot::default();
        let entry = make_entry("resolve target", 1);
        let id = entry.id.clone();
        upsert_entry(&mut snapshot.history, entry, prefs.max_history_items);
        assert_eq!(
            resolve_entry_text(&snapshot, &id).as_deref(),
            Some("resolve target")
        );
        assert_eq!(resolve_entry_text(&snapshot, "no-such-id"), None);
        assert_eq!(resolve_entry_text(&snapshot, "  "), None);
    }

    #[test]
    fn apply_store_ops_preserves_queue_order() {
        let prefs = prefs_with(60, 24);
        let mut base = ClipboardSnapshot::default();
        let ops = vec![
            StoreOp::Record {
                entry: make_entry("first", 1),
            },
            StoreOp::Record {
                entry: make_entry("second", 2),
            },
            StoreOp::Record {
                entry: make_entry("first", 3),
            },
        ];
        apply_store_ops(&mut base, &ops, &prefs);
        assert_eq!(base.history.len(), 2);
        // "first" merged into one entry with count 2, "second" behind it.
        assert_eq!(base.history[0].text, "first");
        assert_eq!(base.history[0].copy_count, 2);
        assert_eq!(base.history[1].text, "second");
    }

    #[test]
    fn apply_record_entry_respects_history_limit() {
        let prefs = prefs_with(2, 24);
        let mut snapshot = ClipboardSnapshot::default();
        for index in 0..5u64 {
            let entry = make_entry(&format!("text {index}"), index);
            apply_record_entry(&mut snapshot, &entry, &prefs);
        }
        assert_eq!(snapshot.history.len(), 2);
        assert_sorted(&snapshot.history);
        // Newest survives truncation.
        assert_eq!(snapshot.history[0].text, "text 4");
    }
}
