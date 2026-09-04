use crate::clipboard_store;
use crate::core::{
    default_phrase_lexicon_dir, validate_trusted_phrase_dir, CandidateMeta,
    LookupCancellationToken, LookupSession, PinyinEngine, LOOKUP_FULL_MAX_CANDIDATES,
    TSF_MAX_CANDIDATES,
};
use crate::runtime_log::{self, RuntimeLogLevel};
use crate::segment::syllable_boundary_offsets_utf16;
use crate::shared_rules::strip_windows_verbatim_prefix;
use crate::win_handle::OwnedWinHandle;
use crate::ENGINE_PANIC_RC;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_MORE_DATA, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, WriteFile, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    SetNamedPipeHandleState, PIPE_NOWAIT, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_MESSAGE, PIPE_WAIT,
};

const PIPE_NAME: &str = r"\\.\pipe\KaixinInput_Engine_V5";
const PIPE_NAME_ENV: &str = "SRF_ENGINE_PIPE_NAME";
const MUTEX_NAME_ENV: &str = "SRF_ENGINE_MUTEX_NAME";
const CAPABILITY_FILE_NAME: &str = "engine_capability.dat";
const CAPABILITY_MAGIC: &[u8] = b"KXIPC-DPAPI-1\n";
const PROTOCOL_MAGIC: u32 = 0x31504653; // "SFP1"
const PROTOCOL_VERSION: u16 = 5;
const ENGINE_APP_VERSION: &str = env!("SRF_APP_VERSION");
const ENGINE_GIT_COMMIT: &str = env!("SRF_ENGINE_GIT_COMMIT");
const ENGINE_GIT_DIRTY: &str = env!("SRF_ENGINE_GIT_DIRTY");
const ENGINE_MODEL_HASH: &str = env!("SRF_ENGINE_MODEL_HASH");
const PIPE_CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(30);
const PIPE_HEADER_IO_TIMEOUT: Duration = Duration::from_secs(5 * 60);
// A TIP process keeps its engine pipe open for the lifetime of the host process.
// Modern Windows sessions can therefore have far more than 16 clients (browser
// renderers, the shell, editors, and input hosts all load the TIP independently).
// Keep one named-pipe instance in reserve for the listener: if active clients
// are allowed to consume every instance, CreateNamedPipeW repeatedly fails and
// the listener eventually treats normal saturation as a fatal service failure.
const PIPE_MAX_INSTANCES: u32 = 128;
const PIPE_MAX_ACTIVE_CLIENTS: usize = PIPE_MAX_INSTANCES as usize - 1;
const MAX_BRIDGE_INPUT_UNITS: usize = 256;
const MAX_LEARN_PHRASE_UNITS: usize = 512;
const MAX_SELECTION_FEEDBACK_SKIPPED: usize = 64;
const MAX_CLIPBOARD_TEXT_UNITS: usize = 20_000;
const MAX_LEXICON_PATH_UNITS: usize = 4096;
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const ROW_TEXT_UNITS: usize = 512;
const ROW_META_UNITS: usize = 512;
// The first candidate page should be cheap to transfer and paint.  Additional
// rows are requested only when the user reaches the end of the visible batch.
const LOOKUP_FIRST_BATCH_CANDIDATES: usize = 18;
const LOOKUP_RESPONSE_BYTES: usize = 4
    + LOOKUP_FULL_MAX_CANDIDATES * (2 + (ROW_TEXT_UNITS - 1) * 2 + 2 + (ROW_META_UNITS - 1) * 2)
    + 4;
const LOOKUP_SLOW_THRESHOLD_US: u128 = 20_000;
const LOOKUP_FAST_SAMPLE_MASK: usize = 0x3f;
const LOOKUP_INTERACTIVE_BUSY_WAIT: Duration = Duration::from_millis(20);
const LOOKUP_BACKGROUND_BUSY_WAIT: Duration = Duration::from_millis(3);
const FULL_LOAD_INSTALL_IDLE: Duration = Duration::from_millis(500);
const FULL_LOAD_INSTALL_RETRY_SLEEP: Duration = Duration::from_millis(25);
// The full prebaked lexicon is deliberately not promoted into the interactive
// shared engine.  It is much larger than the hot lexicon and loading it in the
// background can briefly create two complete lexicon graphs, while the later
// install holds the same mutex used by key lookups.  Interactive candidates
// stay on the bounded hot index; offline/evaluation callers can still use the
// full PinyinEngine constructor directly.
const BACKGROUND_FULL_LEXICON_LOAD_ENABLED: bool = false;
const PIPE_LISTENER_MAX_CONSECUTIVE_FAILURES: usize = 120;
const PIPE_BUFFER_BYTES: u32 = if LOOKUP_RESPONSE_BYTES as u32 + 4096 > 64 * 1024 {
    LOOKUP_RESPONSE_BYTES as u32 + 4096
} else {
    64 * 1024
};
static ACTIVE_PIPE_CLIENTS: AtomicUsize = AtomicUsize::new(0);
static LOOKUP_FAST_SAMPLE_COUNTER: AtomicUsize = AtomicUsize::new(0);
static DEFAULT_ENGINE_READY: AtomicBool = AtomicBool::new(false);
static NEXT_LOOKUP_SESSION_ID: AtomicU64 = AtomicU64::new(1);

const SCHEDULER_WAIT_BUCKET_US: [u64; 12] = [
    100,
    250,
    500,
    1_000,
    2_000,
    4_000,
    8_000,
    16_000,
    32_000,
    64_000,
    128_000,
    u64::MAX,
];

fn engine_build_id() -> String {
    format!(
        "kaixin-{}+git.{}{}",
        ENGINE_APP_VERSION,
        ENGINE_GIT_COMMIT,
        if ENGINE_GIT_DIRTY == "1" {
            ".dirty"
        } else {
            ""
        }
    )
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

fn metadata_signature(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return "missing".to_string();
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| format!("{}.{}", duration.as_secs(), duration.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("len={} mtime={}", meta.len(), modified)
}

fn lexicon_signature(dir: Option<&Path>) -> String {
    let Some(dir) = dir else {
        return "none".to_string();
    };
    let lexicon_bin = dir.join("lexicon.bin");
    if lexicon_bin.is_file() {
        return format!("lexicon.bin {}", metadata_signature(&lexicon_bin));
    }
    format!("lexicon-dir {}", metadata_signature(dir))
}

fn engine_cache_signature(
    lexicon_state: &str,
    _full_in_flight: bool,
    target_dir: Option<&Path>,
) -> String {
    format!(
        "build={};model={};lexicon_state={};lexicon={}",
        engine_build_id(),
        ENGINE_MODEL_HASH,
        lexicon_state,
        lexicon_signature(target_dir)
    )
}

struct ActivePipeClient;

impl ActivePipeClient {
    fn try_acquire() -> Option<Self> {
        let mut current = ACTIVE_PIPE_CLIENTS.load(Ordering::Acquire);
        loop {
            if current >= PIPE_MAX_ACTIVE_CLIENTS {
                return None;
            }
            match ACTIVE_PIPE_CLIENTS.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self),
                Err(next) => current = next,
            }
        }
    }
}

impl Drop for ActivePipeClient {
    fn drop(&mut self) {
        ACTIVE_PIPE_CLIENTS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[repr(u16)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum EngineCommand {
    Init = 1,
    Lookup = 2,
    Learn = 3,
    SyllableBounds = 4,
    RecordClipboard = 5,
    SetCandidatePin = 6,
    Health = 7,
    ResolveClipboard = 8,
    Shutdown = 9,
    LearnCorrection = 10,
    LearnSelectionFeedback = 11,
    CandidateAction = 12,
    ResetLearningContext = 13,
    CancelLookup = 14,
    LearnStatus = 15,
}

impl EngineCommand {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::Init),
            2 => Some(Self::Lookup),
            3 => Some(Self::Learn),
            4 => Some(Self::SyllableBounds),
            5 => Some(Self::RecordClipboard),
            6 => Some(Self::SetCandidatePin),
            7 => Some(Self::Health),
            8 => Some(Self::ResolveClipboard),
            9 => Some(Self::Shutdown),
            10 => Some(Self::LearnCorrection),
            11 => Some(Self::LearnSelectionFeedback),
            12 => Some(Self::CandidateAction),
            13 => Some(Self::ResetLearningContext),
            14 => Some(Self::CancelLookup),
            15 => Some(Self::LearnStatus),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LexiconRuntimeState {
    #[default]
    Core,
    Hot,
    Full,
}

impl LexiconRuntimeState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Hot => "hot",
            Self::Full => "full",
        }
    }
}

#[derive(Default)]
struct SharedEngine {
    engine: Option<PinyinEngine>,
    loaded_dir: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    lexicon_state: LexiconRuntimeState,
    full_load_in_flight: bool,
    last_lookup_at: Option<Instant>,
}

fn perf_log_enabled() -> bool {
    runtime_log::perf_enabled()
}

fn should_log_lookup_summary(first: bool, total_us: u128, candidate_count: usize) -> bool {
    first
        || total_us > LOOKUP_SLOW_THRESHOLD_US
        || candidate_count == 0
        || (LOOKUP_FAST_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed) & LOOKUP_FAST_SAMPLE_MASK)
            == 0
}

impl SharedEngine {
    fn trusted_dir(requested_dir: Option<&Path>) -> Result<Option<PathBuf>, String> {
        match requested_dir {
            Some(path) => validate_trusted_phrase_dir(path)
                .ok_or_else(|| format!("untrusted lexicon path: {}", path.display()))
                .map(strip_windows_verbatim_prefix)
                .map(Some),
            None => Ok(default_phrase_lexicon_dir().map(strip_windows_verbatim_prefix)),
        }
    }

    fn install_fast_engine(&mut self, engine: PinyinEngine, trusted_dir: Option<PathBuf>) {
        let lexicon_state = if engine.has_phrase_lexicon() {
            LexiconRuntimeState::Hot
        } else {
            LexiconRuntimeState::Core
        };
        self.engine = Some(engine);
        self.loaded_dir = None;
        self.target_dir = trusted_dir;
        self.lexicon_state = lexicon_state;
        update_default_engine_ready(self.target_dir.as_deref());
    }

    fn schedule_full_load_if_needed(&mut self) {
        if !BACKGROUND_FULL_LEXICON_LOAD_ENABLED {
            return;
        }
        if self.full_load_in_flight {
            return;
        }
        let Some(dir) = self.target_dir.clone() else {
            return;
        };
        if self.lexicon_state == LexiconRuntimeState::Full && self.loaded_dir.as_ref() == Some(&dir)
        {
            return;
        }
        self.full_load_in_flight = true;
        if !spawn_full_lexicon_load(dir) {
            self.full_load_in_flight = false;
            DEFAULT_ENGINE_READY.store(false, Ordering::Release);
        }
    }

    fn engine_mut(&mut self) -> Result<&mut PinyinEngine, String> {
        self.engine
            .as_mut()
            .ok_or_else(|| "shared engine is not initialized".to_string())
    }
}

/// Learning is feedback, not part of the request/response critical path.
///
/// The TSF side already coalesces duplicate learn events, but the old IPC
/// handlers still performed the SQLite-backed mutation before sending the
/// response. A slow user-dictionary write could therefore make the TSF bridge
/// hit its 750 ms learn timeout and tear down an otherwise healthy pipe. Keep
/// one bounded, FIFO worker in the helper so mutations remain serialized with
/// the shared engine. The IPC response contains a completion id; the TSF polls
/// that id before treating the learning operation as successful.
enum AsyncLearningRequest {
    Commit {
        reading: String,
        committed: String,
        flags: u32,
    },
    Correction {
        raw: String,
        corrected: String,
        committed: String,
    },
    SelectionFeedback {
        reading: String,
        committed: String,
        selected_index: usize,
        page: usize,
        skipped_candidates: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AsyncLearningOutcome {
    Pending,
    Succeeded,
    Failed(String),
}

#[derive(Default)]
struct AsyncLearningStatusRegistry {
    statuses: HashMap<u64, AsyncLearningOutcome>,
}

impl AsyncLearningStatusRegistry {
    fn reserve(&mut self, id: u64, capacity: usize) -> Result<(), &'static str> {
        self.statuses
            .retain(|_, outcome| matches!(outcome, AsyncLearningOutcome::Pending));
        if self.statuses.len() >= capacity {
            return Err("learning status busy");
        }
        self.statuses.insert(id, AsyncLearningOutcome::Pending);
        Ok(())
    }

    fn cancel(&mut self, id: u64) {
        self.statuses.remove(&id);
    }

    fn complete(&mut self, id: u64, outcome: AsyncLearningOutcome) {
        if self.statuses.contains_key(&id) {
            self.statuses.insert(id, outcome);
        }
    }

    fn poll(&mut self, id: u64) -> Option<AsyncLearningOutcome> {
        let outcome = self.statuses.get(&id)?.clone();
        if !matches!(outcome, AsyncLearningOutcome::Pending) {
            self.statuses.remove(&id);
        }
        Some(outcome)
    }
}

struct AsyncLearningTask {
    id: u64,
    request: AsyncLearningRequest,
}

const ASYNC_LEARNING_QUEUE_CAPACITY: usize = 256;
const ASYNC_LEARNING_STATUS_CAPACITY: usize = ASYNC_LEARNING_QUEUE_CAPACITY * 4;
const ASYNC_LEARNING_PENDING_RC: i32 = -7;

fn async_learning_statuses() -> &'static Mutex<AsyncLearningStatusRegistry> {
    static STATUSES: OnceLock<Mutex<AsyncLearningStatusRegistry>> = OnceLock::new();
    STATUSES.get_or_init(|| Mutex::new(AsyncLearningStatusRegistry::default()))
}

fn async_learning_lifecycle_gate() -> &'static Mutex<()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(()))
}

fn async_learning_drain_event() -> &'static (Mutex<()>, Condvar) {
    static EVENT: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
    EVENT.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

static NEXT_ASYNC_LEARNING_ID: AtomicU64 = AtomicU64::new(1);
static ASYNC_LEARNING_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ASYNC_LEARNING_SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn notify_async_learning_drain() {
    let (_, event) = async_learning_drain_event();
    event.notify_all();
}

fn finish_async_learning_task(id: u64, outcome: AsyncLearningOutcome) {
    if let Ok(mut statuses) = async_learning_statuses().lock() {
        statuses.complete(id, outcome);
    }
    ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    notify_async_learning_drain();
}

fn async_learning_sender() -> Option<&'static SyncSender<AsyncLearningTask>> {
    static SENDER: OnceLock<Option<SyncSender<AsyncLearningTask>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel(ASYNC_LEARNING_QUEUE_CAPACITY);
            match std::thread::Builder::new()
                .name("srf-engine-learning".to_string())
                .spawn(move || async_learning_worker_loop(receiver))
            {
                Ok(_) => Some(sender),
                Err(err) => {
                    runtime_log::log_engine(
                        RuntimeLogLevel::Error,
                        "srf_async_learning_worker_start_failed",
                        format!("reason={err}"),
                    );
                    None
                }
            }
        })
        .as_ref()
}

fn enqueue_async_learning(request: AsyncLearningRequest) -> Result<u64, &'static str> {
    let _gate = async_learning_lifecycle_gate()
        .lock()
        .map_err(|_| "learning lifecycle unavailable")?;
    if ASYNC_LEARNING_SHUTDOWN.load(Ordering::Acquire) {
        return Err("learning worker shutting down");
    }
    let Some(sender) = async_learning_sender() else {
        return Err("learning worker unavailable");
    };

    let id = NEXT_ASYNC_LEARNING_ID
        .fetch_add(1, Ordering::Relaxed)
        .max(1);
    {
        let mut statuses = async_learning_statuses()
            .lock()
            .map_err(|_| "learning status unavailable")?;
        statuses.reserve(id, ASYNC_LEARNING_STATUS_CAPACITY)?;
    }
    ASYNC_LEARNING_ACTIVE.fetch_add(1, Ordering::AcqRel);
    match sender.try_send(AsyncLearningTask { id, request }) {
        Ok(()) => Ok(id),
        Err(TrySendError::Full(task)) => {
            if let Ok(mut statuses) = async_learning_statuses().lock() {
                statuses.cancel(task.id);
            }
            ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
            notify_async_learning_drain();
            Err("shared engine busy")
        }
        Err(TrySendError::Disconnected(task)) => {
            if let Ok(mut statuses) = async_learning_statuses().lock() {
                statuses.cancel(task.id);
            }
            ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
            notify_async_learning_drain();
            Err("learning worker disconnected")
        }
    }
}

fn async_learning_outcome_from_result(
    result: Result<(), (&'static str, String)>,
) -> AsyncLearningOutcome {
    match result {
        Ok(()) => AsyncLearningOutcome::Succeeded,
        Err((_, error)) => AsyncLearningOutcome::Failed(error),
    }
}

fn async_learning_worker_loop(receiver: Receiver<AsyncLearningTask>) {
    while let Ok(task) = receiver.recv() {
        let id = task.id;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_async_learning_request(task.request)
        }));
        let outcome = match result {
            Ok(result) => {
                if let Err((event, error)) = &result {
                    runtime_log::log_engine(RuntimeLogLevel::Error, event, error);
                }
                async_learning_outcome_from_result(result)
            }
            Err(_) => {
                let error = "learning worker panicked".to_string();
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_async_learning_failed",
                    &error,
                );
                AsyncLearningOutcome::Failed(error)
            }
        };
        finish_async_learning_task(id, outcome);
    }
}

fn begin_async_learning_shutdown() {
    if let Ok(_gate) = async_learning_lifecycle_gate().lock() {
        ASYNC_LEARNING_SHUTDOWN.store(true, Ordering::Release);
    } else {
        ASYNC_LEARNING_SHUTDOWN.store(true, Ordering::Release);
    }
}

fn wait_for_async_learning_drain() {
    wait_for_async_learning_drain_state(&ASYNC_LEARNING_ACTIVE, async_learning_drain_event());
}

fn wait_for_async_learning_drain_state(active: &AtomicUsize, event: &(Mutex<()>, Condvar)) {
    let (lock, event) = event;
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    };
    while active.load(Ordering::Acquire) != 0 {
        guard = match event.wait(guard) {
            Ok(guard) => guard,
            Err(poison) => poison.into_inner(),
        };
    }
}

fn process_async_learning_request(
    request: AsyncLearningRequest,
) -> Result<(), (&'static str, String)> {
    if let Err(err) = ensure_shared_engine_loaded(None) {
        return Err((
            "srf_async_learning_failed",
            format!("operation=ensure_engine error={err}"),
        ));
    }

    let mut state = lock_shared_engine_recover();
    let engine = state.engine_mut().map_err(|err| {
        (
            "srf_async_learning_failed",
            format!("operation=engine_mut error={err}"),
        )
    })?;

    match request {
        AsyncLearningRequest::Commit {
            reading,
            committed,
            flags,
        } => engine
            .learn_commit_with_flags(reading.trim(), committed.trim(), flags)
            .map_err(|err| {
                (
                    "srf_async_learning_failed",
                    format!("operation=commit error={err}"),
                )
            }),
        AsyncLearningRequest::Correction {
            raw,
            corrected,
            committed,
        } => {
            engine
                .learn_correction(raw.trim(), corrected.trim())
                .map_err(|err| {
                    (
                        "srf_ipc_learn_correction_failed",
                        format!("operation=correction error={err}"),
                    )
                })?;
            engine
                .learn_commit_with_flags(corrected.trim(), committed.trim(), 0)
                .map_err(|err| {
                    (
                        "srf_async_learning_failed",
                        format!("operation=correction_commit error={err}"),
                    )
                })
        }
        AsyncLearningRequest::SelectionFeedback {
            reading,
            committed,
            selected_index,
            page,
            skipped_candidates,
        } => engine
            .learn_selection_feedback(
                reading.trim(),
                committed.trim(),
                selected_index,
                page,
                &skipped_candidates,
            )
            .map_err(|err| {
                (
                    "srf_ipc_learn_selection_feedback_failed",
                    format!("operation=selection_feedback error={err}"),
                )
            }),
    }
}

fn shared_engine() -> &'static Mutex<SharedEngine> {
    static SHARED: OnceLock<Mutex<SharedEngine>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(SharedEngine::default()))
}

// A panic while a request owns SharedEngine poisons the standard-library
// mutex.  The pipe worker catches request panics so the helper stays alive,
// but treating the poisoned mutex as a permanent engine failure makes every
// subsequent health check return rc=-2 and leaves the TIP unable to compose.
// SharedEngine is disposable state: rebuild it from the trusted lexicon on
// the next request and clear the poison bit immediately.
fn recover_shared_engine_poison(
    poison: std::sync::PoisonError<MutexGuard<'static, SharedEngine>>,
    context: &'static str,
) -> MutexGuard<'static, SharedEngine> {
    let mut guard = poison.into_inner();
    *guard = SharedEngine::default();
    shared_engine().clear_poison();
    DEFAULT_ENGINE_READY.store(false, Ordering::Release);
    runtime_log::log_engine(
        RuntimeLogLevel::Error,
        "srf_shared_engine_lock_recovered",
        format!("context={context} state=reset"),
    );
    guard
}

fn lock_shared_engine_recover() -> MutexGuard<'static, SharedEngine> {
    match shared_engine().lock() {
        Ok(guard) => guard,
        Err(poison) => recover_shared_engine_poison(poison, "lock"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LookupPriority {
    Interactive,
    Background,
}

struct ScheduledLookup {
    ticket: u64,
    session_id: u64,
    generation: u64,
    priority: LookupPriority,
}

#[derive(Default)]
struct LookupSchedulerState {
    next_ticket: u64,
    running: Option<LookupPriority>,
    queue: VecDeque<ScheduledLookup>,
}

struct LookupSchedulerMetrics {
    wait_buckets: [AtomicU64; SCHEDULER_WAIT_BUCKET_US.len()],
    admitted: AtomicU64,
    coalesced: AtomicU64,
    superseded: AtomicU64,
    busy: AtomicU64,
    background_yields: AtomicU64,
    active_workers: AtomicUsize,
}

impl Default for LookupSchedulerMetrics {
    fn default() -> Self {
        Self {
            wait_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            admitted: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
            superseded: AtomicU64::new(0),
            busy: AtomicU64::new(0),
            background_yields: AtomicU64::new(0),
            active_workers: AtomicUsize::new(0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct LookupSchedulerSnapshot {
    admitted: u64,
    queue_p50_us: u64,
    queue_p95_us: u64,
    queue_p99_us: u64,
    coalesced: u64,
    superseded: u64,
    busy: u64,
    background_yields: u64,
    active_workers: usize,
}

impl LookupSchedulerMetrics {
    fn record_wait(&self, waited_us: u64) {
        let bucket = SCHEDULER_WAIT_BUCKET_US
            .iter()
            .position(|upper| waited_us <= *upper)
            .unwrap_or(SCHEDULER_WAIT_BUCKET_US.len() - 1);
        self.wait_buckets[bucket].fetch_add(1, Ordering::Relaxed);
    }

    fn percentile(&self, percentile: u64) -> u64 {
        let counts: [u64; SCHEDULER_WAIT_BUCKET_US.len()] =
            std::array::from_fn(|index| self.wait_buckets[index].load(Ordering::Relaxed));
        let total: u64 = counts.iter().sum();
        if total == 0 {
            return 0;
        }
        let target = total.saturating_mul(percentile).div_ceil(100).max(1);
        let mut seen = 0u64;
        for (index, count) in counts.into_iter().enumerate() {
            seen = seen.saturating_add(count);
            if seen >= target {
                return SCHEDULER_WAIT_BUCKET_US[index];
            }
        }
        u64::MAX
    }

    fn snapshot(&self) -> LookupSchedulerSnapshot {
        LookupSchedulerSnapshot {
            admitted: self.admitted.load(Ordering::Relaxed),
            queue_p50_us: self.percentile(50),
            queue_p95_us: self.percentile(95),
            queue_p99_us: self.percentile(99),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            superseded: self.superseded.load(Ordering::Relaxed),
            busy: self.busy.load(Ordering::Relaxed),
            background_yields: self.background_yields.load(Ordering::Relaxed),
            active_workers: self.active_workers.load(Ordering::Acquire),
        }
    }
}

struct LookupScheduler {
    state: Mutex<LookupSchedulerState>,
    changed: Condvar,
    metrics: LookupSchedulerMetrics,
}

impl Default for LookupScheduler {
    fn default() -> Self {
        Self {
            state: Mutex::new(LookupSchedulerState::default()),
            changed: Condvar::new(),
            metrics: LookupSchedulerMetrics::default(),
        }
    }
}

#[derive(Debug)]
enum SchedulerAcquireError {
    Superseded,
    Busy,
    Poisoned,
}

struct LookupPermit {
    scheduler: &'static LookupScheduler,
    queue_wait_us: u64,
}

impl LookupPermit {
    fn queue_wait_us(&self) -> u64 {
        self.queue_wait_us
    }
}

impl Drop for LookupPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scheduler.state.lock() {
            state.running = None;
            self.scheduler
                .metrics
                .active_workers
                .fetch_sub(1, Ordering::AcqRel);
            self.scheduler.changed.notify_all();
        }
    }
}

impl LookupScheduler {
    fn selected_ticket(state: &LookupSchedulerState) -> Option<u64> {
        state
            .queue
            .iter()
            .find(|item| item.priority == LookupPriority::Interactive)
            .or_else(|| state.queue.front())
            .map(|item| item.ticket)
    }

    fn acquire(
        &'static self,
        session_id: u64,
        generation: u64,
        cancellation: Option<&LookupCancellationToken>,
        priority: LookupPriority,
        timeout: Duration,
    ) -> Result<LookupPermit, SchedulerAcquireError> {
        let started = Instant::now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| SchedulerAcquireError::Poisoned)?;
        state.next_ticket = state.next_ticket.wrapping_add(1).max(1);
        let ticket = state.next_ticket;

        if session_id != 0 {
            let before = state.queue.len();
            state.queue.retain(|item| item.session_id != session_id);
            let removed = before.saturating_sub(state.queue.len()) as u64;
            if removed != 0 {
                self.metrics.coalesced.fetch_add(removed, Ordering::Relaxed);
            }
        }
        state.queue.push_back(ScheduledLookup {
            ticket,
            session_id,
            generation,
            priority,
        });
        self.changed.notify_all();

        loop {
            let superseded = cancellation.is_some_and(|token| token.is_superseded(generation));
            if superseded {
                state.queue.retain(|item| item.ticket != ticket);
                self.metrics.superseded.fetch_add(1, Ordering::Relaxed);
                self.changed.notify_all();
                return Err(SchedulerAcquireError::Superseded);
            }

            if state.running.is_none() && Self::selected_ticket(&state) == Some(ticket) {
                let position = state
                    .queue
                    .iter()
                    .position(|item| item.ticket == ticket)
                    .expect("selected scheduler ticket must still be queued");
                let admitted = state
                    .queue
                    .remove(position)
                    .expect("selected scheduler ticket must be removable");
                debug_assert_eq!(admitted.generation, generation);
                state.running = Some(priority);
                let waited_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                self.metrics.record_wait(waited_us);
                self.metrics.admitted.fetch_add(1, Ordering::Relaxed);
                self.metrics.active_workers.fetch_add(1, Ordering::AcqRel);
                return Ok(LookupPermit {
                    scheduler: self,
                    queue_wait_us: waited_us,
                });
            }

            let elapsed = started.elapsed();
            if elapsed >= timeout {
                state.queue.retain(|item| item.ticket != ticket);
                self.metrics
                    .record_wait(elapsed.as_micros().min(u64::MAX as u128) as u64);
                self.metrics.busy.fetch_add(1, Ordering::Relaxed);
                self.changed.notify_all();
                return Err(SchedulerAcquireError::Busy);
            }
            let remaining = timeout.saturating_sub(elapsed);
            let (next_state, _) = self
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(2)))
                .map_err(|_| SchedulerAcquireError::Poisoned)?;
            state = next_state;
        }
    }

    fn try_acquire_background(&'static self) -> Option<LookupPermit> {
        let mut state = self.state.lock().ok()?;
        if state.running.is_some() || !state.queue.is_empty() {
            self.metrics
                .background_yields
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }
        state.running = Some(LookupPriority::Background);
        self.metrics.active_workers.fetch_add(1, Ordering::AcqRel);
        Some(LookupPermit {
            scheduler: self,
            queue_wait_us: 0,
        })
    }

    fn has_interactive_pressure(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.running == Some(LookupPriority::Interactive)
                || state
                    .queue
                    .iter()
                    .any(|item| item.priority == LookupPriority::Interactive)
        })
    }

    fn snapshot(&self) -> LookupSchedulerSnapshot {
        self.metrics.snapshot()
    }

    fn note_superseded(&self) {
        self.metrics.superseded.fetch_add(1, Ordering::Relaxed);
    }
}

fn lookup_scheduler() -> &'static LookupScheduler {
    static SCHEDULER: OnceLock<LookupScheduler> = OnceLock::new();
    SCHEDULER.get_or_init(LookupScheduler::default)
}

struct ClientLookupSession {
    id: u64,
    client_process_id: u32,
    lookup: LookupSession,
}

impl Default for ClientLookupSession {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ClientLookupSession {
    fn new(client_process_id: u32) -> Self {
        Self {
            id: NEXT_LOOKUP_SESSION_ID
                .fetch_add(1, Ordering::Relaxed)
                .max(1),
            client_process_id,
            lookup: LookupSession::default(),
        }
    }
}

impl Drop for ClientLookupSession {
    fn drop(&mut self) {
        if self.client_process_id == 0 {
            return;
        }
        if let Ok(mut registry) = lookup_cancellation_registry().lock() {
            if registry
                .sessions
                .get(&self.client_process_id)
                .is_some_and(|entry| entry.owner_id == self.id)
            {
                registry.sessions.remove(&self.client_process_id);
                registry
                    .latest_superseding_request_ids
                    .remove(&self.client_process_id);
            }
        }
    }
}

struct RegisteredLookupCancellation {
    owner_id: u64,
    active_request_id: u64,
    token: LookupCancellationToken,
}

#[derive(Default)]
struct LookupCancellationRegistry {
    sessions: HashMap<u32, RegisteredLookupCancellation>,
    latest_superseding_request_ids: HashMap<u32, u64>,
}

fn lookup_cancellation_registry() -> &'static Mutex<LookupCancellationRegistry> {
    static REGISTRY: OnceLock<Mutex<LookupCancellationRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(LookupCancellationRegistry::default()))
}

struct ActiveLookupRegistration {
    client_process_id: u32,
    owner_id: u64,
    request_id: u64,
}

impl Drop for ActiveLookupRegistration {
    fn drop(&mut self) {
        if let Ok(mut registry) = lookup_cancellation_registry().lock() {
            if let Some(entry) = registry.sessions.get_mut(&self.client_process_id) {
                if entry.owner_id == self.owner_id && entry.active_request_id == self.request_id {
                    entry.active_request_id = 0;
                }
            }
        }
    }
}

fn register_active_lookup(
    session: &ClientLookupSession,
    request_id: u64,
) -> Option<ActiveLookupRegistration> {
    if session.client_process_id == 0 || request_id == 0 {
        return None;
    }
    let token = session.lookup.cancellation_token();
    let mut registry = lookup_cancellation_registry().lock().ok()?;
    let cancelled_before_registration = registry
        .latest_superseding_request_ids
        .get(&session.client_process_id)
        .is_some_and(|superseding| request_id < *superseding);
    registry.sessions.insert(
        session.client_process_id,
        RegisteredLookupCancellation {
            owner_id: session.id,
            active_request_id: request_id,
            token: token.clone(),
        },
    );
    drop(registry);
    if cancelled_before_registration {
        token.next_generation();
        lookup_scheduler().changed.notify_all();
    }
    Some(ActiveLookupRegistration {
        client_process_id: session.client_process_id,
        owner_id: session.id,
        request_id,
    })
}

fn cancel_registered_lookup(client_process_id: u32, superseding_request_id: u64) -> bool {
    if client_process_id == 0 || superseding_request_id == 0 {
        return false;
    }
    let token = {
        let Ok(mut registry) = lookup_cancellation_registry().lock() else {
            return false;
        };
        registry
            .latest_superseding_request_ids
            .entry(client_process_id)
            .and_modify(|current| *current = (*current).max(superseding_request_id))
            .or_insert(superseding_request_id);
        let Some(entry) = registry.sessions.get(&client_process_id) else {
            return false;
        };
        if entry.active_request_id == 0 || entry.active_request_id >= superseding_request_id {
            return false;
        }
        entry.token.clone()
    };
    token.next_generation();
    lookup_scheduler().changed.notify_all();
    true
}

fn lock_shared_engine_for_lookup(
    reading: &str,
    request_id: u64,
    lookup_generation: u64,
    cancellation: Option<&LookupCancellationToken>,
    after_ensure: bool,
) -> Result<MutexGuard<'static, SharedEngine>, (i32, Vec<u8>)> {
    let started = Instant::now();
    let busy_wait = if request_id == 0 {
        LOOKUP_BACKGROUND_BUSY_WAIT
    } else {
        LOOKUP_INTERACTIVE_BUSY_WAIT
    };
    let mut waited = false;
    loop {
        match shared_engine().try_lock() {
            Ok(guard) => {
                if waited && perf_log_enabled() {
                    runtime_log::log_engine(
                        RuntimeLogLevel::Perf,
                        "srf_ipc_lookup_waited",
                        format!(
                            "request_id={} {} waited_us={} after_ensure={}",
                            request_id,
                            runtime_log::input_fingerprint(reading),
                            started.elapsed().as_micros(),
                            if after_ensure { 1 } else { 0 }
                        ),
                    );
                }
                return Ok(guard);
            }
            Err(TryLockError::WouldBlock) if started.elapsed() < busy_wait => {
                waited = true;
                let superseded = cancellation.map_or_else(
                    || crate::core::lookup_request_superseded(lookup_generation),
                    |token| token.is_superseded(lookup_generation),
                );
                if superseded {
                    return Err((-8, error_payload("lookup superseded")));
                }
                std::thread::yield_now();
            }
            Err(TryLockError::WouldBlock) => {
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_ipc_lookup_busy",
                    format!(
                        "request_id={} {} status=busy waited_us={} after_ensure={}",
                        request_id,
                        runtime_log::input_fingerprint(reading),
                        started.elapsed().as_micros(),
                        if after_ensure { 1 } else { 0 }
                    ),
                );
                return Err((-5, error_payload("shared engine busy")));
            }
            Err(TryLockError::Poisoned(poison)) => {
                return Ok(recover_shared_engine_poison(poison, "lookup"));
            }
        }
    }
}

fn ensure_shared_engine_loaded(requested_dir: Option<&Path>) -> Result<(), String> {
    if requested_dir.is_none() && DEFAULT_ENGINE_READY.load(Ordering::Acquire) {
        return Ok(());
    }
    let started = Instant::now();
    let trusted_dir = match requested_dir {
        Some(path) => SharedEngine::trusted_dir(Some(path))?,
        None => default_trusted_dir()?,
    };
    let already_loaded = {
        let state = lock_shared_engine_recover();
        state.engine.is_some() && state.target_dir == trusted_dir
    };

    if !already_loaded {
        let engine = PinyinEngine::with_hot_phrase_dir(trusted_dir.as_deref());
        let mut state = lock_shared_engine_recover();
        let reused = state.engine.is_some() && state.target_dir == trusted_dir;
        if !reused {
            state.install_fast_engine(engine, trusted_dir);
        }
        state.schedule_full_load_if_needed();

        if perf_log_enabled() {
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_engine_ensure_loaded",
                format!(
                    "reused={} state={} full_in_flight={} elapsed={}us",
                    if reused { 1 } else { 0 },
                    state.lexicon_state.as_str(),
                    if state.full_load_in_flight { 1 } else { 0 },
                    started.elapsed().as_micros()
                ),
            );
        }
        return Ok(());
    }

    let mut state = lock_shared_engine_recover();
    state.schedule_full_load_if_needed();

    if perf_log_enabled() {
        runtime_log::log_engine(
            RuntimeLogLevel::Perf,
            "srf_engine_ensure_loaded",
            format!(
                "reused=1 state={} full_in_flight={} elapsed={}us",
                state.lexicon_state.as_str(),
                if state.full_load_in_flight { 1 } else { 0 },
                started.elapsed().as_micros()
            ),
        );
    }
    Ok(())
}

fn default_trusted_dir() -> Result<Option<PathBuf>, String> {
    static DEFAULT_DIR: OnceLock<Result<Option<PathBuf>, String>> = OnceLock::new();
    DEFAULT_DIR
        .get_or_init(|| SharedEngine::trusted_dir(None))
        .clone()
}

fn update_default_engine_ready(target_dir: Option<&Path>) {
    let ready = default_trusted_dir()
        .ok()
        .is_some_and(|default_dir| default_dir.as_deref() == target_dir);
    DEFAULT_ENGINE_READY.store(ready, Ordering::Release);
}

fn spawn_full_lexicon_load(dir: PathBuf) -> bool {
    std::thread::Builder::new()
        .name("srf-engine-full-lexicon".to_string())
        .spawn(move || {
            let started = Instant::now();
            runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "srf_engine_full_warmup_start",
                format!("dir={}", dir.display()),
            );

            let mut full_engine = PinyinEngine::with_phrase_dir(Some(&dir));
            full_engine.warmup_correction_indexes();
            let loaded = full_engine.has_phrase_lexicon();
            let mut phrase_lexicon = full_engine.take_phrase_lexicon();
            let mut full_engine = Some(full_engine);
            let elapsed_us = started.elapsed().as_micros();

            let install_started = Instant::now();
            let mut installed = false;
            loop {
                let Some(permit) = lookup_scheduler().try_acquire_background() else {
                    std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                    continue;
                };
                match shared_engine().try_lock() {
                    Ok(mut state) => {
                        let recently_used = match state.last_lookup_at {
                            Some(last) => last.elapsed() < FULL_LOAD_INSTALL_IDLE,
                            None => false,
                        };
                        if recently_used {
                            drop(state);
                            drop(permit);
                            std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                            continue;
                        }

                        if state.target_dir.as_ref() == Some(&dir) && loaded {
                            // Installing the optional phrase lexicon is a
                            // performance enhancement.  Keep a panic here
                            // from unwinding through the mutex and poisoning
                            // every later health request.
                            let install_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    if let Some(engine) = state.engine.as_mut() {
                                        engine.install_phrase_lexicon(phrase_lexicon.take());
                                    } else if let Some(mut engine) = full_engine.take() {
                                        engine.install_phrase_lexicon(phrase_lexicon.take());
                                        state.engine = Some(engine);
                                    }
                                }));
                            if install_result.is_ok() {
                                state.loaded_dir = Some(dir.clone());
                                state.lexicon_state = LexiconRuntimeState::Full;
                                installed = true;
                            } else {
                                state.engine = None;
                                state.loaded_dir = None;
                                state.lexicon_state = LexiconRuntimeState::Core;
                                DEFAULT_ENGINE_READY.store(false, Ordering::Release);
                                runtime_log::log_engine(
                                    RuntimeLogLevel::Error,
                                    "srf_engine_full_warmup_install_panic",
                                    format!("dir={}", dir.display()),
                                );
                            }
                        }
                        state.full_load_in_flight = false;
                        break;
                    }
                    Err(TryLockError::WouldBlock) => {
                        drop(permit);
                        std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                    }
                    Err(TryLockError::Poisoned(poison)) => {
                        let mut state = recover_shared_engine_poison(poison, "full-load-install");
                        state.full_load_in_flight = false;
                        break;
                    }
                }
            }

            runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "srf_engine_full_warmup_finish",
                format!(
                    "loaded={} installed={} elapsed={}us install_delay={}us dir={}",
                    if loaded { 1 } else { 0 },
                    if installed { 1 } else { 0 },
                    elapsed_us,
                    install_started.elapsed().as_micros(),
                    dir.display()
                ),
            );

            if installed {
                warmup_installed_full_engine();
            }
        })
        .is_ok()
}

fn warmup_installed_full_engine() {
    const READINGS: &[&str] = &[
        "ni",
        "nihao",
        "jintian",
        "mingtian",
        "shur",
        "shurufa",
        "bejing",
        "beijing",
        "anguan",
        "angui",
        "hangzhou",
        "nihaoshijie",
        "zhonghuarenmingongheguo",
    ];

    for reading in READINGS {
        loop {
            let Some(permit) = lookup_scheduler().try_acquire_background() else {
                std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                continue;
            };
            match shared_engine().try_lock() {
                Ok(mut state) => {
                    let recently_used = state
                        .last_lookup_at
                        .is_some_and(|last| last.elapsed() < FULL_LOAD_INSTALL_IDLE);
                    if recently_used {
                        drop(state);
                        drop(permit);
                        std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                        continue;
                    }
                    if state.lexicon_state != LexiconRuntimeState::Full {
                        return;
                    }
                    if let Ok(engine) = state.engine_mut() {
                        engine.warmup_interactive_prefixes_until(reading, || {
                            !lookup_scheduler().has_interactive_pressure()
                        });
                    }
                    let preempted = lookup_scheduler().has_interactive_pressure();
                    drop(state);
                    if preempted {
                        drop(permit);
                        std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                        continue;
                    }
                    break;
                }
                Err(TryLockError::WouldBlock) => {
                    drop(permit);
                    std::thread::sleep(FULL_LOAD_INSTALL_RETRY_SLEEP);
                }
                Err(TryLockError::Poisoned(poison)) => {
                    drop(recover_shared_engine_poison(poison, "full-load-warmup"));
                    return;
                }
            }
        }
    }
}

pub fn engine_mutex_name_from_env() -> String {
    std::env::var(MUTEX_NAME_ENV)
        .unwrap_or_else(|_| "Local\\KaixinInput_Engine_Mutex_V5".to_string())
}

fn engine_pipe_name_from_env() -> String {
    std::env::var(PIPE_NAME_ENV).unwrap_or_else(|_| PIPE_NAME.to_string())
}

pub fn start_engine_service() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        // Start this before the first client request so the first learn event
        // cannot pay thread-creation latency on the named-pipe handler.
        let _ = async_learning_sender();
        clipboard_store::warmup_snapshot_cache_async();
        let pipe_name = engine_pipe_name_from_env();
        let mutex_name = engine_mutex_name_from_env();
        runtime_log::log_engine(
            RuntimeLogLevel::Basic,
            "engine_service_start",
            format!(
                "status=starting {} {} pipe={} mutex={}",
                runtime_log::config_diagnostics_fields(),
                clipboard_store::diagnostics_fields(),
                pipe_name,
                mutex_name
            ),
        );
        match std::thread::Builder::new()
            .name("srf-engine-pipe-listener".to_string())
            .spawn(listen_loop)
        {
            Ok(_) => runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "engine_service_start",
                "status=ok listener=spawned",
            ),
            Err(err) => runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "engine_service_start",
                format!("status=failed listener=spawn reason={err}"),
            ),
        }
    });
}

pub fn warmup_shared_engine_async() {
    let _ = std::thread::Builder::new()
        .name("srf-engine-warmup".to_string())
        .spawn(|| {
            let dir = SharedEngine::trusted_dir(None).ok().flatten();
            let mut engine = PinyinEngine::with_hot_phrase_dir(dir.as_deref());
            engine
                .warmup_interactive_paths_until(|| !lookup_scheduler().has_interactive_pressure());
            let mut state = lock_shared_engine_recover();
            if state.engine.is_none() {
                state.install_fast_engine(engine, dir);
            }
            state.schedule_full_load_if_needed();
        });
}

/// Synchronously warm up the engine after the tray/helper starts so first input
/// does not pay the initialization cost.
pub fn warmup_shared_engine_sync() {
    // Build the engine outside the shared mutex. Otherwise TSF can send Init
    // while startup warmup is still parsing/mapping the lexicon, causing the
    // client to wait on this lock and making the first key feel stuck.
    let dir = SharedEngine::trusted_dir(None).ok().flatten();
    let mut engine = PinyinEngine::with_hot_phrase_dir(dir.as_deref());
    engine.warmup_interactive_paths_until(|| !lookup_scheduler().has_interactive_pressure());
    let mut state = lock_shared_engine_recover();
    if state.engine.is_none() {
        state.install_fast_engine(engine, dir);
    }
    state.schedule_full_load_if_needed();
}

pub fn probe_shared_engine(input: &str) -> Result<usize, String> {
    let reading = input.trim();
    if reading.is_empty() {
        return Err("probe input is empty".to_string());
    }
    if reading.encode_utf16().count() > MAX_BRIDGE_INPUT_UNITS {
        return Err("probe input is too long".to_string());
    }

    ensure_shared_engine_loaded(None)?;
    let mut state = lock_shared_engine_recover();
    state.schedule_full_load_if_needed();
    let lexicon_state = state.lexicon_state.as_str().to_string();
    let full_in_flight = state.full_load_in_flight;
    let engine = state.engine_mut()?;
    let started = Instant::now();
    let (ranked, warning) = engine.lookup_full_detailed_with_request_id(reading, 0);
    let elapsed_us = started.elapsed().as_micros();
    let count = ranked.len().min(TSF_MAX_CANDIDATES);
    runtime_log::log_engine(
        RuntimeLogLevel::Basic,
        "install_health_check_probe",
        format!(
            "{} candidates={} lexicon_state={} full_in_flight={} engine={}us status={}",
            runtime_log::input_fingerprint(reading),
            count,
            lexicon_state,
            if full_in_flight { 1 } else { 0 },
            elapsed_us,
            if count > 0 { "ok" } else { "failed" }
        ),
    );
    if count == 0 {
        return Err(warning.unwrap_or_else(|| "probe returned no candidates".to_string()));
    }
    Ok(count)
}

pub fn probe_shared_engine_via_pipe(input: &str) -> Result<usize, String> {
    let reading = input.trim();
    if reading.is_empty() {
        return Err("probe input is empty".to_string());
    }
    if reading.encode_utf16().count() > MAX_BRIDGE_INPUT_UNITS {
        return Err("probe input is too long".to_string());
    }

    start_engine_service();
    warmup_shared_engine_sync();

    let started = Instant::now();
    let (count, response_bytes) = engine_pipe_roundtrip(reading)?;
    runtime_log::log_engine(
        RuntimeLogLevel::Basic,
        "install_health_check_pipe_probe",
        format!(
            "{} candidates={} response_bytes={} elapsed={}us status={}",
            runtime_log::input_fingerprint(reading),
            count,
            response_bytes,
            started.elapsed().as_micros(),
            if count > 0 { "ok" } else { "failed" }
        ),
    );
    if count == 0 {
        return Err("pipe lookup returned no candidates".to_string());
    }
    Ok(count)
}

/// 单次命名管道 lookup 往返（打开新连接 + 读取令牌），要求进程内引擎服务
/// 已启动（start_engine_service + warmup_shared_engine_sync）。返回
/// (候选数, 响应字节数)。供健康检查与一次性探测使用；性能基准请用
/// EnginePipeClient 复用连接，避免每键支付连接建立与令牌解密的开销。
pub fn engine_pipe_roundtrip(input: &str) -> Result<(usize, usize), String> {
    let mut client = EnginePipeClient::connect()?;
    client.lookup(input, 0)
}

/// 持久化管道客户端：与真实 TSF 客户端一致——能力令牌只读取一次、
/// 连接保持复用，避免每键支付文件读取 + DPAPI 解密的固定开销。
pub struct EnginePipeClient {
    file: std::fs::File,
    capability: String,
}

impl Drop for EnginePipeClient {
    fn drop(&mut self) {
        zeroize_string(&mut self.capability);
    }
}

impl EnginePipeClient {
    pub fn connect() -> Result<Self, String> {
        let pipe_name = engine_pipe_name_from_env();
        let file = open_pipe_client(&pipe_name)?;
        let capability = read_engine_capability_token()
            .map_err(|err| format!("engine capability unavailable: {err}"))?;
        Ok(Self { file, capability })
    }

    /// 单次 lookup 往返。request_id 非 0 走交互优先级（真实 TSF 客户端），
    /// 0 为后台优先级（健康探测）。返回 (候选数, 响应字节数)。
    pub fn lookup(&mut self, input: &str, request_id: u64) -> Result<(usize, usize), String> {
        let reading = input.trim();
        if reading.is_empty() {
            return Err("roundtrip input is empty".to_string());
        }
        if reading.encode_utf16().count() > MAX_BRIDGE_INPUT_UNITS {
            return Err("roundtrip input is too long".to_string());
        }

        let mut payload = Vec::with_capacity(24 + self.capability.len() * 2 + reading.len() * 2);
        append_utf16_string(&mut payload, &self.capability);
        append_u32(&mut payload, 0);
        append_utf16_string(&mut payload, reading);
        payload.extend_from_slice(&request_id.to_le_bytes());

        let mut request = Vec::with_capacity(12 + payload.len());
        request.extend_from_slice(&PROTOCOL_MAGIC.to_le_bytes());
        request.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        request.extend_from_slice(&(EngineCommand::Lookup as u16).to_le_bytes());
        request.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        request.extend_from_slice(&payload);

        self.file
            .write_all(&request)
            .map_err(|err| format!("pipe write failed: {err}"))?;
        self.file
            .flush()
            .map_err(|err| format!("pipe flush failed: {err}"))?;

        let mut header = [0u8; 16];
        self.file
            .read_exact(&mut header)
            .map_err(|err| format!("pipe response header read failed: {err}"))?;
        let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let version = u16::from_le_bytes([header[4], header[5]]);
        let command = u16::from_le_bytes([header[6], header[7]]);
        let status = i32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let payload_len =
            u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as usize;
        if magic != PROTOCOL_MAGIC
            || version != PROTOCOL_VERSION
            || command != EngineCommand::Lookup as u16
        {
            return Err(format!(
                "pipe response header mismatch magic={magic:#x} version={version} command={command}"
            ));
        }
        if payload_len > LOOKUP_RESPONSE_BYTES {
            return Err(format!("pipe response too large: {payload_len}"));
        }

        let mut response = vec![0u8; payload_len];
        if payload_len > 0 {
            self.file
                .read_exact(&mut response)
                .map_err(|err| format!("pipe response payload read failed: {err}"))?;
        }
        if status != 0 {
            return Err(format!("pipe lookup failed status={status}"));
        }
        if response.len() < 4 {
            return Err("pipe lookup response was truncated".to_string());
        }
        let count =
            u32::from_le_bytes([response[0], response[1], response[2], response[3]]) as usize;
        Ok((count, response.len()))
    }
}

fn open_pipe_client(pipe_name: &str) -> Result<std::fs::File, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe_name) {
            Ok(file) => return Ok(file),
            Err(err) if Instant::now() < deadline => {
                let last = err;
                std::thread::sleep(Duration::from_millis(25));
                if Instant::now() >= deadline {
                    return Err(format!("pipe open failed: {last}"));
                }
            }
            Err(err) => return Err(format!("pipe open failed: {err}")),
        }
    }
}

fn listen_loop() {
    if std::panic::catch_unwind(listen_loop_inner).is_err() {
        runtime_log::log_engine(
            RuntimeLogLevel::Error,
            "srf_ipc_listener_panic",
            "listener loop panicked",
        );
        std::process::exit(1);
    }
}

fn listen_loop_inner() {
    let pipe_name_text = engine_pipe_name_from_env();
    let pipe_name = wide(&pipe_name_text);
    let security = match crate::windows_security::LocalLogonPipeSecurity::new() {
        Ok(security) => security,
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_security_init_failed",
                err.to_string(),
            );
            std::process::exit(1);
        }
    };
    let mut consecutive_create_failures = 0usize;
    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_MAX_INSTANCES,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                security.as_ptr(),
            )
        };
        if pipe == INVALID_HANDLE_VALUE {
            let last_error = unsafe { GetLastError() };
            // All pipe instances being occupied is transient load, not a
            // broken listener. In particular, never let a connection burst
            // turn into a process exit after the failure counter expires.
            if last_error == ERROR_PIPE_BUSY {
                consecutive_create_failures = 0;
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            consecutive_create_failures = consecutive_create_failures.saturating_add(1);
            if consecutive_create_failures >= PIPE_LISTENER_MAX_CONSECUTIVE_FAILURES {
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_ipc_create_pipe_failed",
                    format!(
                        "pipe={} consecutive_failures={} last_error={}",
                        pipe_name_text, consecutive_create_failures, last_error
                    ),
                );
                std::process::exit(1);
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        consecutive_create_failures = 0;
        // SAFETY: CreateNamedPipeW returned a new handle owned by this loop.
        let pipe = unsafe { OwnedWinHandle::from_raw(pipe) }.expect("validated engine pipe");

        let connected = unsafe { ConnectNamedPipe(pipe.as_raw(), null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if connected {
            let Some(client_guard) = ActivePipeClient::try_acquire() else {
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_ipc_client_limit",
                    format!(
                        "active={} limit={}",
                        ACTIVE_PIPE_CLIENTS.load(Ordering::Acquire),
                        PIPE_MAX_ACTIVE_CLIENTS
                    ),
                );
                unsafe {
                    let _ = DisconnectNamedPipe(pipe.as_raw());
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            let mode = PIPE_READMODE_MESSAGE | PIPE_NOWAIT;
            unsafe {
                let _ = SetNamedPipeHandleState(pipe.as_raw(), &mode, null_mut(), null_mut());
            }
            if std::thread::Builder::new()
                .name("srf-engine-pipe-client".to_string())
                .spawn(move || handle_client(pipe, client_guard))
                .is_err()
            {
                std::thread::sleep(Duration::from_millis(50));
            }
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn capability_path() -> Option<PathBuf> {
    crate::app_paths::local_data_dir().map(|dir| dir.join(CAPABILITY_FILE_NAME))
}

#[cfg(windows)]
fn dpapi_unprotect_capability(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let payload = data.strip_prefix(CAPABILITY_MAGIC).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "missing engine capability token header",
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
fn dpapi_unprotect_capability(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

fn read_engine_capability_token() -> io::Result<String> {
    let path = capability_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "engine capability token path is unavailable",
        )
    })?;
    let bytes = std::fs::read(path)?;
    let mut decoded = if bytes.starts_with(CAPABILITY_MAGIC) {
        dpapi_unprotect_capability(&bytes)?
    } else {
        bytes
    };
    let mut token = match String::from_utf8(std::mem::take(&mut decoded)) {
        Ok(token) => token,
        Err(err) => {
            let mut bytes = err.into_bytes();
            zeroize_vec(&mut bytes);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid capability token",
            ));
        }
    };
    zeroize_vec(&mut decoded);
    let trimmed = token.trim().to_string();
    zeroize_string(&mut token);
    let token = trimmed;
    if token.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty capability token",
        ))
    } else {
        Ok(token)
    }
}

fn verify_capability_token(cursor: &mut PayloadCursor<'_>) -> Result<(), i32> {
    let len = cursor.read_u32().ok_or(-1)? as usize;
    let mut supplied = cursor.read_utf16_string(len, 128).ok_or(-1)?;
    let mut expected = match cached_engine_capability_token() {
        Ok(token) => token,
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_capability_unavailable",
                err.to_string(),
            );
            return Err(-6);
        }
    };
    let ok = crate::windows_security::constant_time_eq(supplied.trim(), expected.trim());
    zeroize_string(&mut supplied);
    zeroize_string(&mut expected);
    if ok {
        Ok(())
    } else {
        runtime_log::log_engine(
            RuntimeLogLevel::Error,
            "srf_ipc_capability_denied",
            "status=denied",
        );
        Err(-6)
    }
}

/// 服务端能力令牌缓存。令牌文件由 tray 启动时写入、静态不变；每个请求都
/// 读文件 + DPAPI 解密会把毫秒级开销加在每键热路径上。用 (mtime, len)
/// 校验后复用内存明文，文件变化（令牌轮换）时重新解密。
static CAPABILITY_CACHE: Mutex<Option<(SystemTime, u64, String)>> = Mutex::new(None);

fn cached_engine_capability_token() -> io::Result<String> {
    let path = capability_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "engine capability token path is unavailable",
        )
    })?;
    let meta = std::fs::metadata(&path)?;
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let len = meta.len();
    {
        let cache = CAPABILITY_CACHE
            .lock()
            .map_err(|_| io::Error::other("capability cache poisoned"))?;
        if let Some((cached_mtime, cached_len, token)) = cache.as_ref() {
            if *cached_mtime == mtime && *cached_len == len {
                return Ok(token.clone());
            }
        }
    }
    let token = read_engine_capability_token()?;
    let mut cache = CAPABILITY_CACHE
        .lock()
        .map_err(|_| io::Error::other("capability cache poisoned"))?;
    if let Some((_, _, mut old)) = cache.replace((mtime, len, token.clone())) {
        zeroize_string(&mut old);
    }
    Ok(token)
}

fn handle_client(pipe_guard: OwnedWinHandle, _client_guard: ActivePipeClient) {
    let pipe = pipe_guard.as_raw();
    // A persistent pipe is owned by one TIP process. Keep composition-local
    // lookup state on that connection without changing the V5 wire format.
    let mut client_process_id = 0u32;
    unsafe {
        let _ = GetNamedPipeClientProcessId(pipe, &mut client_process_id);
    }
    let mut lookup_session = ClientLookupSession::new(client_process_id);
    while let Some((command, payload)) = read_request(pipe) {
        let (status, response) =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch_request_with_session(command, &payload, Some(&mut lookup_session))
            })) {
                Ok(result) => result,
                Err(_) => (ENGINE_PANIC_RC, Vec::new()),
            };
        let shutdown_requested = command == EngineCommand::Shutdown && status == 0;
        if !write_response(pipe, command, status, &response) {
            break;
        }
        if shutdown_requested {
            runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "srf_engine_helper_shutdown",
                "requested_by_tsf_bridge",
            );
            // Persist clipboard records the store writer has not flushed yet
            // so a user quit never loses recent captures.
            clipboard_store::flush_pending_ops_sync();
            // Learning requests are acknowledged only after the worker has
            // completed them.  Still drain here because a request may have
            // been accepted by the queue while the shutdown command was in
            // flight from another pipe client.
            wait_for_async_learning_drain();
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
            }
            std::process::exit(0);
        }
    }

    unsafe {
        let _ = FlushFileBuffers(pipe);
        let _ = DisconnectNamedPipe(pipe);
    }
}

fn read_request(pipe: HANDLE) -> Option<(EngineCommand, Vec<u8>)> {
    let mut header = [0u8; 12];
    if !read_exact_with_timeout(pipe, &mut header, PIPE_HEADER_IO_TIMEOUT) {
        return None;
    }

    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let version = u16::from_le_bytes([header[4], header[5]]);
    let command_raw = u16::from_le_bytes([header[6], header[7]]);
    let payload_len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if magic != PROTOCOL_MAGIC || version != PROTOCOL_VERSION {
        return None;
    }

    let command = EngineCommand::from_u16(command_raw)?;
    if payload_len > MAX_REQUEST_BYTES {
        return None;
    }
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 && !read_exact(pipe, &mut payload) {
        return None;
    }
    Some((command, payload))
}

fn write_response(pipe: HANDLE, command: EngineCommand, status: i32, payload: &[u8]) -> bool {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(&PROTOCOL_MAGIC.to_le_bytes());
    header.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    header.extend_from_slice(&(command as u16).to_le_bytes());
    header.extend_from_slice(&status.to_le_bytes());
    header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    write_all(pipe, &header) && (payload.is_empty() || write_all(pipe, payload))
}

fn error_payload(message: &str) -> Vec<u8> {
    let units: Vec<u16> = message
        .encode_utf16()
        .take(MAX_LEXICON_PATH_UNITS)
        .collect();
    let mut out = Vec::with_capacity(4 + units.len() * 2);
    out.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn dispatch_request_with_session(
    command: EngineCommand,
    payload: &[u8],
    mut lookup_session: Option<&mut ClientLookupSession>,
) -> (i32, Vec<u8>) {
    match command {
        EngineCommand::Init => handle_init(payload),
        EngineCommand::Lookup => handle_lookup(payload, lookup_session.as_deref_mut()),
        EngineCommand::Learn => handle_learn(payload),
        EngineCommand::SyllableBounds => handle_syllable_bounds(payload),
        EngineCommand::RecordClipboard => handle_record_clipboard(payload),
        EngineCommand::SetCandidatePin => handle_set_candidate_pin(payload),
        EngineCommand::Health => handle_health(),
        EngineCommand::ResolveClipboard => handle_resolve_clipboard(payload),
        EngineCommand::Shutdown => handle_shutdown(payload),
        EngineCommand::LearnCorrection => handle_learn_correction(payload),
        EngineCommand::LearnSelectionFeedback => handle_learn_selection_feedback(payload),
        EngineCommand::CandidateAction => handle_candidate_action(payload),
        EngineCommand::ResetLearningContext => handle_reset_learning_context(payload),
        EngineCommand::CancelLookup => handle_cancel_lookup(payload, lookup_session.as_deref()),
        EngineCommand::LearnStatus => handle_learn_status(payload),
    }
}

fn handle_cancel_lookup(
    payload: &[u8],
    lookup_session: Option<&ClientLookupSession>,
) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    let superseding_request_id = match cursor.read_u64() {
        Some(value) if value != 0 => value,
        _ => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }
    let Some(session) = lookup_session else {
        return (-4, Vec::new());
    };
    let cancelled = cancel_registered_lookup(session.client_process_id, superseding_request_id);
    if perf_log_enabled() {
        runtime_log::log_engine(
            RuntimeLogLevel::Perf,
            "srf_ipc_lookup_cancel",
            format!(
                "client_pid={} superseding_request_id={} cancelled={}",
                session.client_process_id,
                superseding_request_id,
                if cancelled { 1 } else { 0 }
            ),
        );
    }
    // Cancellation is intentionally idempotent. There may be no active
    // lookup by the time this best-effort control request reaches the helper.
    (0, Vec::new())
}

fn append_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn append_utf16_string(out: &mut Vec<u8>, value: &str) {
    let units: Vec<u16> = value.encode_utf16().take(MAX_LEXICON_PATH_UNITS).collect();
    append_u32(out, units.len() as u32);
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

fn utf16_string_payload(value: &str, max_units: usize) -> Vec<u8> {
    let units: Vec<u16> = value.encode_utf16().take(max_units).collect();
    let mut out = Vec::with_capacity(4 + units.len() * 2);
    append_u32(&mut out, units.len() as u32);
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn handle_shutdown(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }
    begin_async_learning_shutdown();
    (0, Vec::new())
}

fn append_utf16_field_u16(out: &mut Vec<u8>, value: &str, max_units: usize) {
    let units: Vec<u16> = value
        .encode_utf16()
        .take(max_units.min(u16::MAX as usize))
        .collect();
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

fn handle_health() -> (i32, Vec<u8>) {
    let (loaded, target_dir, lexicon_state, full_in_flight) = match shared_engine().try_lock() {
        Ok(state) => {
            let lexicon_state = state.lexicon_state.as_str().to_string();
            (
                state.engine.is_some(),
                state.target_dir.clone(),
                lexicon_state,
                state.full_load_in_flight,
            )
        }
        Err(std::sync::TryLockError::WouldBlock) => (false, None, "busy".to_string(), true),
        Err(std::sync::TryLockError::Poisoned(poison)) => {
            let state = recover_shared_engine_poison(poison, "health");
            (
                false,
                state.target_dir.clone(),
                "poison-recovered".to_string(),
                false,
            )
        }
    };
    let cache_signature =
        engine_cache_signature(&lexicon_state, full_in_flight, target_dir.as_deref());

    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_default();

    let mut out = Vec::new();
    append_u32(&mut out, PROTOCOL_VERSION as u32);
    append_utf16_string(&mut out, &engine_build_id());
    append_utf16_string(&mut out, &exe);
    append_utf16_string(&mut out, "");
    append_utf16_string(&mut out, "");
    append_utf16_string(&mut out, &engine_pipe_name_from_env());
    append_utf16_string(&mut out, &engine_mutex_name_from_env());
    append_u32(&mut out, if loaded { 1 } else { 0 });
    append_utf16_string(&mut out, &lexicon_state);
    append_utf16_string(&mut out, "");
    append_u32(&mut out, if full_in_flight { 1 } else { 0 });
    append_utf16_string(&mut out, ENGINE_MODEL_HASH);
    append_utf16_string(&mut out, &cache_signature);
    (0, out)
}

fn handle_init(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    let len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let path = match cursor.read_utf16_string(len, MAX_LEXICON_PATH_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    let requested_dir = if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    };

    match ensure_shared_engine_loaded(requested_dir.as_deref()) {
        Ok(()) => (0, Vec::new()),
        Err(err) => (-3, error_payload(&err)),
    }
}

fn handle_lookup(
    payload: &[u8],
    lookup_session: Option<&mut ClientLookupSession>,
) -> (i32, Vec<u8>) {
    static FIRST_LOOKUP_LOGGED: OnceLock<()> = OnceLock::new();
    let lookup_started = Instant::now();
    let mut cursor = PayloadCursor::new(payload);
    // Lookup can enter clipboard utility modes and return clipboard previews.
    // Authenticate every lookup rather than trying to classify readings here:
    // classification after parsing has repeatedly left alternate query paths
    // capable of exposing the same protected store.
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let mode_flags = match cursor.read_u32() {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let reading = match cursor.read_utf16_string(len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let request_id = if cursor.is_at_end() {
        0
    } else {
        match cursor.read_u64() {
            Some(v) => v,
            None => return (-1, Vec::new()),
        }
    };
    let full_result = if cursor.is_at_end() {
        false
    } else {
        match cursor.read_u32() {
            Some(v) => v != 0,
            None => return (-1, Vec::new()),
        }
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }
    // Each pipe owns an independent cancellation domain. Client request ids
    // are compared only inside the calling process, whose PID is obtained from
    // the pipe rather than trusted from the payload.
    let (session_id, lookup_generation, cancellation) =
        if let Some(session) = lookup_session.as_deref() {
            let generation = session.lookup.next_lookup_generation();
            (
                session.id,
                generation,
                Some(session.lookup.cancellation_token()),
            )
        } else {
            (0, crate::core::next_lookup_request_generation(), None)
        };
    // Register only after allocating the engine generation. A delayed cancel
    // can now advance the token past this generation; it cannot be erased by
    // lookup startup racing immediately after registration.
    let _active_lookup_registration = lookup_session
        .as_deref()
        .and_then(|session| register_active_lookup(session, request_id));
    let priority = if request_id == 0 {
        LookupPriority::Background
    } else {
        LookupPriority::Interactive
    };
    // Background probes must not initialize or claim the shared engine while
    // an interactive request is already waiting. They can retry from the
    // TIP-side cache during the next quiet window.
    if priority == LookupPriority::Background && lookup_scheduler().has_interactive_pressure() {
        return (-5, error_payload("shared engine busy"));
    }
    // Do first-time model construction outside the scheduler permit. Building
    // the fast engine can take much longer than a lookup and must not make a
    // background probe the head-of-line owner for all interactive clients.
    match shared_engine().try_lock() {
        Ok(state) if state.engine.is_none() => {
            drop(state);
            if priority == LookupPriority::Background {
                return (-5, error_payload("shared engine busy"));
            }
            if let Err(err) = ensure_shared_engine_loaded(None) {
                return (-3, error_payload(&err));
            }
        }
        Ok(_) | Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Poisoned(poison)) => {
            drop(recover_shared_engine_poison(poison, "lookup-ensure"));
            if priority == LookupPriority::Background {
                return (-5, error_payload("shared engine busy"));
            }
            if let Err(err) = ensure_shared_engine_loaded(None) {
                return (-3, error_payload(&err));
            }
        }
    }
    let scheduler_wait = if priority == LookupPriority::Interactive {
        LOOKUP_INTERACTIVE_BUSY_WAIT
    } else {
        LOOKUP_BACKGROUND_BUSY_WAIT
    };
    let permit_result = if priority == LookupPriority::Background {
        if lookup_scheduler().has_interactive_pressure() {
            Err(SchedulerAcquireError::Busy)
        } else {
            lookup_scheduler()
                .try_acquire_background()
                .ok_or(SchedulerAcquireError::Busy)
        }
    } else {
        lookup_scheduler().acquire(
            session_id,
            lookup_generation,
            cancellation.as_ref(),
            priority,
            scheduler_wait,
        )
    };
    let permit = match permit_result {
        Ok(permit) => permit,
        Err(SchedulerAcquireError::Superseded) => {
            if perf_log_enabled() {
                let scheduler = lookup_scheduler().snapshot();
                runtime_log::log_engine(
                    RuntimeLogLevel::Perf,
                    "srf_ipc_lookup_superseded",
                    format!(
                        "request_id={} lookup_generation={} {} queue_p95={}us queue_p99={}us coalesced={} superseded={} active_workers={} status=superseded",
                        request_id,
                        lookup_generation,
                        runtime_log::input_fingerprint(&reading),
                        scheduler.queue_p95_us,
                        scheduler.queue_p99_us,
                        scheduler.coalesced,
                        scheduler.superseded,
                        scheduler.active_workers,
                    ),
                );
            }
            return (-8, error_payload("lookup superseded"));
        }
        Err(SchedulerAcquireError::Busy) => {
            let scheduler = lookup_scheduler().snapshot();
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_lookup_busy",
                format!(
                    "request_id={} lookup_generation={} {} queue_p50={}us queue_p95={}us queue_p99={}us admitted={} coalesced={} superseded={} busy={} background_yields={} active_workers={} status=busy",
                    request_id,
                    lookup_generation,
                    runtime_log::input_fingerprint(&reading),
                    scheduler.queue_p50_us,
                    scheduler.queue_p95_us,
                    scheduler.queue_p99_us,
                    scheduler.admitted,
                    scheduler.coalesced,
                    scheduler.superseded,
                    scheduler.busy,
                    scheduler.background_yields,
                    scheduler.active_workers,
                ),
            );
            return (-5, error_payload("shared engine busy"));
        }
        Err(SchedulerAcquireError::Poisoned) => return (-2, Vec::new()),
    };
    let queue_wait_us = permit.queue_wait_us();
    if priority == LookupPriority::Background && lookup_scheduler().has_interactive_pressure() {
        drop(permit);
        return (-5, error_payload("shared engine busy"));
    }

    let (ranked, lexicon_state, full_in_flight, engine_us) = {
        let mut state = match lock_shared_engine_for_lookup(
            &reading,
            request_id,
            lookup_generation,
            cancellation.as_ref(),
            false,
        ) {
            Ok(guard) => guard,
            Err(result) => return result,
        };
        if state.engine.is_none() {
            drop(state);
            if let Err(err) = ensure_shared_engine_loaded(None) {
                return (-3, error_payload(&err));
            }
            state = match lock_shared_engine_for_lookup(
                &reading,
                request_id,
                lookup_generation,
                cancellation.as_ref(),
                true,
            ) {
                Ok(guard) => guard,
                Err(result) => return result,
            };
        } else {
            state.schedule_full_load_if_needed();
        }
        state.last_lookup_at = Some(Instant::now());
        let lexicon_state = state.lexicon_state.as_str().to_string();
        let full_in_flight = state.full_load_in_flight;
        let engine = match state.engine_mut() {
            Ok(engine) => engine,
            Err(_) => return (-3, Vec::new()),
        };
        let engine_started = Instant::now();
        let (ranked, warning) = if let Some(session) = lookup_session {
            engine.lookup_with_session(
                &mut session.lookup,
                mode_flags,
                reading.trim(),
                lookup_generation,
            )
        } else {
            engine.set_mode_flags(mode_flags);
            engine.lookup_full_detailed_with_request_id(reading.trim(), lookup_generation)
        };
        if warning.as_deref() == Some("lookup superseded") {
            lookup_scheduler().note_superseded();
            return (-8, error_payload("lookup superseded"));
        }
        (
            ranked,
            lexicon_state,
            full_in_flight,
            engine_started.elapsed().as_micros(),
        )
    };
    let scheduler_after_engine = lookup_scheduler().snapshot();
    drop(permit);

    let result_limit = if full_result {
        LOOKUP_FULL_MAX_CANDIDATES
    } else {
        LOOKUP_FIRST_BATCH_CANDIDATES
    };
    let count = ranked.len().min(result_limit);
    let has_more = !full_result && ranked.len() > count;
    let mut payload = Vec::with_capacity(4 + count * 48);
    append_u32(&mut payload, count as u32);
    for candidate in ranked.iter().take(count) {
        append_utf16_field_u16(&mut payload, &candidate.phrase, ROW_TEXT_UNITS - 1);
        let meta = lookup_row_meta(&candidate.meta, candidate.score);
        append_utf16_field_u16(&mut payload, &meta, ROW_META_UNITS - 1);
    }
    append_u32(&mut payload, if has_more { 1 } else { 0 });
    let total_us = lookup_started.elapsed().as_micros();
    if perf_log_enabled() {
        let scheduler = scheduler_after_engine;
        let first = FIRST_LOOKUP_LOGGED.set(()).is_ok();
        if should_log_lookup_summary(first, total_us, count) {
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_ipc_lookup",
                format!(
                    "request_id={} lookup_generation={} {} candidates={} first={} response_format={} response_bytes={} lexicon_state={} full_in_flight={} queue_wait={}us queue_p50={}us queue_p95={}us queue_p99={}us admitted={} coalesced={} superseded={} busy={} background_yields={} active_workers={} engine={}us total={}us status=ok",
                    request_id,
                    lookup_generation,
                    runtime_log::input_fingerprint(&reading),
                    count,
                    if first { 1 } else { 0 },
                    "compact",
                    payload.len(),
                    lexicon_state,
                    if full_in_flight { 1 } else { 0 },
                    queue_wait_us,
                    scheduler.queue_p50_us,
                    scheduler.queue_p95_us,
                    scheduler.queue_p99_us,
                    scheduler.admitted,
                    scheduler.coalesced,
                    scheduler.superseded,
                    scheduler.busy,
                    scheduler.background_yields,
                    scheduler.active_workers,
                    engine_us,
                    total_us
                ),
            );
        }
        if total_us > LOOKUP_SLOW_THRESHOLD_US {
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_ipc_lookup_slow",
                format!(
                    "request_id={} lookup_generation={} {} candidates={} lexicon_state={} full_in_flight={} queue_wait={}us queue_p95={}us queue_p99={}us coalesced={} superseded={} busy={} background_yields={} active_workers={} engine={}us total={}us status=slow",
                    request_id,
                    lookup_generation,
                    runtime_log::input_fingerprint(&reading),
                    count,
                    lexicon_state,
                    if full_in_flight { 1 } else { 0 },
                    queue_wait_us,
                    scheduler.queue_p95_us,
                    scheduler.queue_p99_us,
                    scheduler.coalesced,
                    scheduler.superseded,
                    scheduler.busy,
                    scheduler.background_yields,
                    scheduler.active_workers,
                    engine_us,
                    total_us
                ),
            );
        }
        if count == 0 {
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_ipc_lookup_empty",
                format!(
                    "request_id={} lookup_generation={} {} lexicon_state={} full_in_flight={} queue_wait={}us queue_p95={}us queue_p99={}us coalesced={} superseded={} busy={} background_yields={} active_workers={} engine={}us total={}us status=empty",
                    request_id,
                    lookup_generation,
                    runtime_log::input_fingerprint(&reading),
                    lexicon_state,
                    if full_in_flight { 1 } else { 0 },
                    queue_wait_us,
                    scheduler.queue_p95_us,
                    scheduler.queue_p99_us,
                    scheduler.coalesced,
                    scheduler.superseded,
                    scheduler.busy,
                    scheduler.background_yields,
                    scheduler.active_workers,
                    engine_us,
                    total_us
                ),
            );
        }
    }
    (0, payload)
}

fn lookup_row_meta(meta: &CandidateMeta, score: f64) -> String {
    let mut parts = Vec::new();
    if let Some(text) = meta.display_text().filter(|text| !text.is_empty()) {
        parts.push(text.to_string());
    }
    parts.push(format!("source={}", meta.source.label()));
    parts.push(format!("match={}", meta.match_kind.label()));
    parts.push(format!("layer={}", meta.source_layer.label()));
    if meta.pinned {
        parts.push("pinned=1".to_string());
    }
    if meta.partial {
        parts.push("partial=1".to_string());
    }
    if meta.user_signal {
        parts.push("user_signal=1".to_string());
    }
    parts.push(format!("{score:.2}"));
    parts.join("\t")
}

fn handle_learn(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let reading_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let committed_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let reading = match cursor.read_utf16_string(reading_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let committed = match cursor.read_utf16_string(committed_len, MAX_LEARN_PHRASE_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let flags = if cursor.is_at_end() {
        0
    } else {
        match cursor.read_u32() {
            Some(v) => v,
            None => return (-1, Vec::new()),
        }
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    match enqueue_async_learning(AsyncLearningRequest::Commit {
        reading,
        committed,
        flags,
    }) {
        Ok(id) => (0, id.to_le_bytes().to_vec()),
        Err("shared engine busy") => (-5, error_payload("shared engine busy")),
        Err(err) => (-3, error_payload(err)),
    }
}

fn handle_learn_correction(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let raw_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let corrected_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let committed_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let raw = match cursor.read_utf16_string(raw_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let corrected = match cursor.read_utf16_string(corrected_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let committed = match cursor.read_utf16_string(committed_len, MAX_LEARN_PHRASE_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    match enqueue_async_learning(AsyncLearningRequest::Correction {
        raw,
        corrected,
        committed,
    }) {
        Ok(id) => (0, id.to_le_bytes().to_vec()),
        Err("shared engine busy") => (-5, error_payload("shared engine busy")),
        Err(err) => (-3, error_payload(err)),
    }
}

fn handle_learn_selection_feedback(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let reading_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let committed_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let selected_index = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let page = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let reading = match cursor.read_utf16_string(reading_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let committed = match cursor.read_utf16_string(committed_len, MAX_LEARN_PHRASE_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let mut skipped_candidates = Vec::new();
    if !cursor.is_at_end() {
        let skipped_count = match cursor.read_u32() {
            Some(v) => v as usize,
            None => return (-1, Vec::new()),
        };
        if skipped_count > MAX_SELECTION_FEEDBACK_SKIPPED {
            return (-1, Vec::new());
        }
        skipped_candidates.reserve(skipped_count);
        for _ in 0..skipped_count {
            let len = match cursor.read_u32() {
                Some(v) => v as usize,
                None => return (-1, Vec::new()),
            };
            let candidate = match cursor.read_utf16_string(len, MAX_LEARN_PHRASE_UNITS) {
                Some(v) => v,
                None => return (-1, Vec::new()),
            };
            skipped_candidates.push(candidate);
        }
    }
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    match enqueue_async_learning(AsyncLearningRequest::SelectionFeedback {
        reading,
        committed,
        selected_index,
        page,
        skipped_candidates,
    }) {
        Ok(id) => (0, id.to_le_bytes().to_vec()),
        Err("shared engine busy") => (-5, error_payload("shared engine busy")),
        Err(err) => (-3, error_payload(err)),
    }
}

fn handle_learn_status(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let id = match cursor.read_u64() {
        Some(id) if id != 0 => id,
        _ => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    let mut statuses = match async_learning_statuses().lock() {
        Ok(statuses) => statuses,
        Err(_) => return (-3, error_payload("learning status unavailable")),
    };
    match statuses.poll(id) {
        Some(AsyncLearningOutcome::Pending) => (ASYNC_LEARNING_PENDING_RC, Vec::new()),
        Some(AsyncLearningOutcome::Succeeded) => (0, Vec::new()),
        Some(AsyncLearningOutcome::Failed(error)) => (-3, error_payload(&error)),
        None => (-3, error_payload("learning request is unknown or expired")),
    }
}

fn handle_set_candidate_pin(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let pinned = match cursor.read_u32() {
        Some(v) => v != 0,
        None => return (-1, Vec::new()),
    };
    let reading_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let committed_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let reading = match cursor.read_utf16_string(reading_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let committed = match cursor.read_utf16_string(committed_len, MAX_LEARN_PHRASE_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    if let Err(err) = ensure_shared_engine_loaded(None) {
        return (-3, error_payload(&err));
    }
    let mut state = lock_shared_engine_recover();
    let engine = match state.engine_mut() {
        Ok(engine) => engine,
        Err(_) => return (-3, Vec::new()),
    };
    match engine.set_candidate_pin(reading.trim(), committed.trim(), pinned) {
        Ok(()) => (0, Vec::new()),
        Err(_) => (-3, Vec::new()),
    }
}

fn handle_candidate_action(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let action = match cursor.read_u32() {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let reading_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let phrase_len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let reading = match cursor.read_utf16_string(reading_len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let phrase = match cursor.read_utf16_string(phrase_len, MAX_LEARN_PHRASE_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    if let Err(err) = ensure_shared_engine_loaded(None) {
        return (-3, error_payload(&err));
    }
    let mut state = lock_shared_engine_recover();
    let engine = match state.engine_mut() {
        Ok(engine) => engine,
        Err(_) => return (-3, Vec::new()),
    };
    let result = match action {
        1 => engine.remove_user_phrase(reading.trim(), phrase.trim()),
        2 => engine.block_user_phrase(phrase.trim()),
        3 => engine.unblock_user_phrase(phrase.trim()),
        _ => return (-1, Vec::new()),
    };
    match result {
        Ok(_) => (0, Vec::new()),
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_candidate_action_failed",
                &err,
            );
            (-3, Vec::new())
        }
    }
}

fn handle_reset_learning_context(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    let mut state = lock_shared_engine_recover();
    if let Ok(engine) = state.engine_mut() {
        engine.reset_learning_context();
    }
    (0, Vec::new())
}

fn handle_syllable_bounds(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    let len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let text = match cursor.read_utf16_string(len, MAX_BRIDGE_INPUT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    if let Err(err) = ensure_shared_engine_loaded(None) {
        return (-3, error_payload(&err));
    }
    let mut state = lock_shared_engine_recover();
    let engine = match state.engine_mut() {
        Ok(engine) => engine,
        Err(_) => return (-3, Vec::new()),
    };
    let offsets = syllable_boundary_offsets_utf16(text.trim(), engine.syllable_set());
    let mut out = Vec::with_capacity(4 + offsets.len() * 4);
    out.extend_from_slice(&(offsets.len() as u32).to_le_bytes());
    for value in offsets {
        out.extend_from_slice(&value.to_le_bytes());
    }
    (0, out)
}

fn handle_record_clipboard(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let len = match cursor.read_u32() {
        Some(v) => v as usize,
        None => return (-1, Vec::new()),
    };
    let text = match cursor.read_utf16_string(len, MAX_CLIPBOARD_TEXT_UNITS) {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    match clipboard_store::record_text(&text) {
        Ok(_) => (0, Vec::new()),
        Err(_) => (-3, Vec::new()),
    }
}

fn handle_resolve_clipboard(payload: &[u8]) -> (i32, Vec<u8>) {
    let started = Instant::now();
    let used_cache = clipboard_store::cached_snapshot().is_some();
    let (status, response) =
        handle_resolve_clipboard_with(payload, clipboard_store::resolve_snapshot);
    if perf_log_enabled() {
        runtime_log::log_engine(
            RuntimeLogLevel::Perf,
            "srf_ipc_resolve_clipboard",
            format!(
                "id_len={} cache={} total_us={} status={}",
                payload.len(),
                if used_cache { 1 } else { 0 },
                started.elapsed().as_micros(),
                status
            ),
        );
    }
    (status, response)
}

fn handle_resolve_clipboard_with<F>(payload: &[u8], load_snapshot: F) -> (i32, Vec<u8>)
where
    F: FnOnce() -> Result<clipboard_store::ClipboardSnapshot, String>,
{
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let remaining = payload.len().saturating_sub(cursor.offset);
    let id = if remaining == 8 {
        match cursor.read_u64() {
            Some(v) => v.to_string(),
            None => return (-1, Vec::new()),
        }
    } else {
        let len = match cursor.read_u32() {
            Some(v) => v as usize,
            None => return (-1, Vec::new()),
        };
        match cursor.read_utf16_string(len, MAX_CLIPBOARD_TEXT_UNITS) {
            Some(v) => v,
            None => return (-1, Vec::new()),
        }
    };
    if !cursor.is_at_end() {
        return (-1, Vec::new());
    }

    let snapshot = match load_snapshot() {
        Ok(snapshot) => snapshot,
        Err(_) => return (-3, Vec::new()),
    };
    let text = clipboard_store::resolve_entry_text(&snapshot, &id);
    match text {
        Some(text) => (0, utf16_string_payload(&text, MAX_CLIPBOARD_TEXT_UNITS)),
        None => (-4, Vec::new()),
    }
}

struct PayloadCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PayloadCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_u32(&mut self) -> Option<u32> {
        let end = self.offset.checked_add(4)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u64(&mut self) -> Option<u64> {
        let end = self.offset.checked_add(8)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_utf16_string(&mut self, units: usize, max_units: usize) -> Option<String> {
        if units > max_units {
            return None;
        }
        let byte_len = units.checked_mul(2)?;
        let end = self.offset.checked_add(byte_len)?;
        let bytes = self.bytes.get(self.offset..end)?;
        self.offset = end;
        let mut utf16 = Vec::with_capacity(units);
        for chunk in bytes.chunks_exact(2) {
            utf16.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Some(String::from_utf16_lossy(&utf16))
    }

    fn is_at_end(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn read_exact(handle: HANDLE, buf: &mut [u8]) -> bool {
    read_exact_with_timeout(handle, buf, PIPE_CLIENT_IO_TIMEOUT)
}

fn read_exact_with_timeout(handle: HANDLE, mut buf: &mut [u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !buf.is_empty() {
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut read,
                null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_MORE_DATA && read > 0 {
                let consumed = read as usize;
                buf = &mut buf[consumed..];
                continue;
            }
            if err == ERROR_NO_DATA && Instant::now() < deadline {
                // 轮询粒度即每个请求的排队延迟上限：PIPE_NOWAIT 下服务端
                // 靠此循环等待客户端的下一条消息。10ms 会让持久连接上每个
                // 请求固定多吃一个轮询周期；1ms 把该开销压到 ~0.5ms 均值，
                // 空闲时每个客户端线程约 1000 次/秒唤醒，客户端数量有限，
                // CPU 代价可忽略。
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            return false;
        }
        if read == 0 {
            return false;
        }
        let consumed = read as usize;
        buf = &mut buf[consumed..];
    }
    true
}

fn write_all(handle: HANDLE, mut buf: &[u8]) -> bool {
    let deadline = Instant::now() + PIPE_CLIENT_IO_TIMEOUT;
    while !buf.is_empty() {
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                handle,
                buf.as_ptr().cast(),
                buf.len() as u32,
                &mut written,
                null_mut(),
            )
        };
        if ok == 0 || written == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_NO_DATA && Instant::now() < deadline {
                // 与读侧一致：1ms 轮询粒度，避免响应写侧放大每个请求的延迟。
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            return false;
        }
        buf = &buf[written as usize..];
    }
    true
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn learning_status_reports_success_once() {
        let mut registry = AsyncLearningStatusRegistry::default();
        registry.reserve(1, 2).unwrap();
        assert_eq!(registry.poll(1), Some(AsyncLearningOutcome::Pending));
        registry.complete(1, AsyncLearningOutcome::Succeeded);
        assert_eq!(registry.poll(1), Some(AsyncLearningOutcome::Succeeded));
        assert_eq!(registry.poll(1), None);
    }

    #[test]
    fn learning_status_reports_failure_and_releases_slot() {
        let mut registry = AsyncLearningStatusRegistry::default();
        registry.reserve(1, 1).unwrap();
        assert_eq!(
            registry.reserve(2, 1),
            Err("learning status busy"),
            "a pending request must apply backpressure"
        );
        registry.complete(
            1,
            AsyncLearningOutcome::Failed("sqlite write failed".to_string()),
        );
        assert_eq!(
            registry.poll(1),
            Some(AsyncLearningOutcome::Failed(
                "sqlite write failed".to_string()
            ))
        );
        registry.reserve(2, 1).unwrap();
    }

    #[test]
    fn learning_worker_exposes_background_write_failure() {
        let outcome = async_learning_outcome_from_result(Err((
            "srf_async_learning_failed",
            "sqlite write failed".to_string(),
        )));
        assert_eq!(
            outcome,
            AsyncLearningOutcome::Failed("sqlite write failed".to_string())
        );
    }

    #[test]
    fn learning_status_capacity_prunes_observed_terminal_state() {
        let mut registry = AsyncLearningStatusRegistry::default();
        registry.reserve(1, 1).unwrap();
        registry.complete(1, AsyncLearningOutcome::Succeeded);
        registry.reserve(2, 1).unwrap();
        assert_eq!(registry.poll(1), None);
        assert_eq!(registry.poll(2), Some(AsyncLearningOutcome::Pending));
    }

    #[test]
    fn learning_queue_is_bounded() {
        let (sender, _receiver) = mpsc::sync_channel(ASYNC_LEARNING_QUEUE_CAPACITY);
        for id in 1..=ASYNC_LEARNING_QUEUE_CAPACITY as u64 {
            sender
                .try_send(AsyncLearningTask {
                    id,
                    request: AsyncLearningRequest::Commit {
                        reading: "ni".to_string(),
                        committed: "你".to_string(),
                        flags: 0,
                    },
                })
                .unwrap();
        }
        let result = sender.try_send(AsyncLearningTask {
            id: ASYNC_LEARNING_QUEUE_CAPACITY as u64 + 1,
            request: AsyncLearningRequest::Commit {
                reading: "hao".to_string(),
                committed: "好".to_string(),
                flags: 0,
            },
        });
        assert!(matches!(result, Err(TrySendError::Full(_))));
    }

    #[test]
    fn shutdown_drain_waits_for_pending_worker() {
        let active = Arc::new(AtomicUsize::new(1));
        let event = Arc::new((Mutex::new(()), Condvar::new()));
        let worker_active = Arc::clone(&active);
        let worker_event = Arc::clone(&event);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            worker_active.store(0, Ordering::Release);
            worker_event.1.notify_all();
        });

        wait_for_async_learning_drain_state(&active, &event);
        assert_eq!(active.load(Ordering::Acquire), 0);
    }
}
