use crate::clipboard_store;
use crate::core::{
    default_phrase_lexicon_dir, validate_trusted_phrase_dir, CandidateMatchKind, CandidateMeta,
    CandidateSource, EngineLookupSnapshot, LookupCancellationToken, PinyinEngine,
    LOOKUP_FULL_MAX_CANDIDATES, TSF_MAX_CANDIDATES,
};
use crate::runtime_log::{self, RuntimeLogLevel};
use crate::segment::syllable_boundary_offsets_utf16;
use crate::shared_rules::strip_windows_verbatim_prefix;
use crate::thuocl::LexiconLayer;
use crate::user_dict::{DictionaryRequest, UserLexicon};
use crate::win_handle::OwnedWinHandle;
use crate::ENGINE_PANIC_RC;
use arc_swap::ArcSwapOption;
use bincode::Options;
use kaixin_common::engine_contract::{
    ROW_ANNOTATION, ROW_CLIPBOARD_FILTER, ROW_CLIPBOARD_ID, ROW_CLIPBOARD_PAGE,
    ROW_CLIPBOARD_PAGES, ROW_CLIPBOARD_PINNED, ROW_CLIPBOARD_QUICK, ROW_CLIPBOARD_SOURCE,
    ROW_CLIPBOARD_TIME, ROW_CLIPBOARD_TYPE, ROW_CORRECTED_READING, ROW_CORRECTION, ROW_DISPLAY,
    ROW_EXT, ROW_MAX_FIELD_UNITS, ROW_NO_LEARN, ROW_PARTIAL, ROW_PINNED, ROW_PLACEHOLDER,
    ROW_USER_COMMON, ROW_USER_FRESH, ROW_USER_MIXED, ROW_USER_OBSERVED, ROW_VERTICAL,
};
use kaixin_common::engine_wire::*;
use kaixin_platform::storage_client::{
    capability_path, open_pipe_client, read_engine_capability_token,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
#[cfg(test)]
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::{
    GetLastError, BOOL, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE};
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};

const MUTEX_NAME_ENV: &str = "SRF_ENGINE_MUTEX_NAME";
const ENGINE_APP_VERSION: &str = env!("SRF_APP_VERSION");
const ENGINE_GIT_COMMIT: &str = env!("SRF_ENGINE_GIT_COMMIT");
const ENGINE_GIT_DIRTY: &str = env!("SRF_ENGINE_GIT_DIRTY");
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
const LOOKUP_ROW_STRING_FIELDS: usize = 9;
const LOOKUP_ROW_MAX_BYTES: usize =
    4 + 4 + 2 * 4 + LOOKUP_ROW_STRING_FIELDS * (2 + ROW_MAX_FIELD_UNITS as usize * 2);
// The first candidate page should be cheap to transfer and paint.  Additional
// rows are requested only when the user reaches the end of the visible batch.
const LOOKUP_FIRST_BATCH_CANDIDATES: usize = 18;
const LOOKUP_RESPONSE_BYTES: usize = 3 + LOOKUP_FULL_MAX_CANDIDATES * LOOKUP_ROW_MAX_BYTES;
const LOOKUP_SLOW_THRESHOLD_US: u128 = 20_000;
const LOOKUP_FAST_SAMPLE_MASK: usize = 0x3f;
const LOOKUP_INTERACTIVE_BUSY_WAIT: Duration = Duration::from_millis(20);
const LOOKUP_WORKERS: usize = 4;
const PIPE_LISTENER_MAX_CONSECUTIVE_FAILURES: usize = 120;
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
static ACTIVE_PIPE_CLIENTS: AtomicUsize = AtomicUsize::new(0);
static LOOKUP_FAST_SAMPLE_COUNTER: AtomicUsize = AtomicUsize::new(0);
static SERVICE_OWNER: AtomicBool = AtomicBool::new(false);
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

pub fn build_info() -> serde_json::Value {
    serde_json::json!({
        "schema": 1, "component": "engine", "version": ENGINE_APP_VERSION,
        "git": ENGINE_GIT_COMMIT, "protocol": PROTOCOL_VERSION,
        "bits": usize::BITS,
    })
}

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
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
        crate::model_resources::model_hash(),
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
    Storage = 16,
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
            16 => Some(Self::Storage),
            _ => None,
        }
    }
}

struct PublishedLookup {
    data: EngineLookupSnapshot,
    target_dir: Option<PathBuf>,
    lexicon_state: &'static str,
}

static LOOKUP_MODEL: ArcSwapOption<PublishedLookup> = ArcSwapOption::const_empty();

#[derive(Default)]
struct SharedEngine {
    engine: Option<PinyinEngine>,
    target_dir: Option<PathBuf>,
    dictionary_error: Option<String>,
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

    fn install_model(&mut self, mut engine: PinyinEngine, trusted_dir: Option<PathBuf>) {
        if let Some(current) = &mut self.engine {
            current.install_service_model(engine);
        } else {
            if SERVICE_OWNER.load(Ordering::Acquire) {
                match UserLexicon::open_default() {
                    Ok(dictionary) => engine.attach_user_dictionary(dictionary),
                    Err(error) => {
                        runtime_log::log_engine(
                            RuntimeLogLevel::Error,
                            "srf_user_dict_open_failed",
                            error.to_string(),
                        );
                        self.dictionary_error = Some(error.to_string());
                    }
                }
            }
            if let Some(error) = crate::compiled_data::compiled_model_error() {
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_compiled_model_unavailable",
                    error,
                );
            }
            self.engine = Some(engine);
        }
        self.target_dir = trusted_dir;

        self.publish();
    }

    fn publish(&self) {
        if let Some(engine) = &self.engine {
            LOOKUP_MODEL.store(Some(Arc::new(PublishedLookup {
                data: engine.lookup_snapshot(),
                target_dir: self.target_dir.clone(),
                lexicon_state: if engine.has_phrase_lexicon() {
                    "hot"
                } else {
                    "core"
                },
            })));
        }
    }

    fn engine_mut(&mut self) -> Result<&mut PinyinEngine, String> {
        if let Some(error) = &self.dictionary_error {
            return Err(format!("user dictionary unavailable: {error}"));
        }
        self.engine
            .as_mut()
            .ok_or_else(|| "shared engine is not initialized".to_string())
    }

    fn commit_learning(&mut self) -> Result<(), String> {
        self.engine_mut()?
            .flush_pending_learning()
            .map_err(|error| error.to_string())?;
        self.publish();
        Ok(())
    }
}

/// Learning is feedback, not part of the request/response critical path.
///
/// One bounded FIFO worker handles feedback and appends a durable journal
/// batch before publishing learned state and completing the operation.
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

type WriterJob = Box<dyn FnOnce(&mut SharedEngine) -> Result<Vec<u8>, String> + Send>;

enum EngineWriterTask {
    Feedback(AsyncLearningTask),
    Command(WriterJob, mpsc::Sender<Result<Vec<u8>, String>>),
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

fn async_learning_sender() -> Option<&'static SyncSender<EngineWriterTask>> {
    static SENDER: OnceLock<Option<SyncSender<EngineWriterTask>>> = OnceLock::new();
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
    match sender.try_send(EngineWriterTask::Feedback(AsyncLearningTask {
        id,
        request,
    })) {
        Ok(()) => Ok(id),
        Err(TrySendError::Full(_)) => {
            if let Ok(mut statuses) = async_learning_statuses().lock() {
                statuses.cancel(id);
            }
            ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
            notify_async_learning_drain();
            Err("shared engine busy")
        }
        Err(TrySendError::Disconnected(_)) => {
            if let Ok(mut statuses) = async_learning_statuses().lock() {
                statuses.cancel(id);
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

fn run_writer_command(
    job: impl FnOnce(&mut SharedEngine) -> Result<Vec<u8>, String> + Send + 'static,
) -> Result<Vec<u8>, String> {
    let (reply, result) = mpsc::channel();
    {
        let _gate = async_learning_lifecycle_gate()
            .lock()
            .map_err(|_| "learning lifecycle unavailable")?;
        if ASYNC_LEARNING_SHUTDOWN.load(Ordering::Acquire) {
            return Err("learning worker shutting down".to_owned());
        }
        let sender = async_learning_sender().ok_or("learning worker unavailable")?;
        ASYNC_LEARNING_ACTIVE.fetch_add(1, Ordering::AcqRel);
        if sender
            .try_send(EngineWriterTask::Command(Box::new(job), reply))
            .is_err()
        {
            ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
            notify_async_learning_drain();
            return Err("learning worker busy or disconnected".to_owned());
        }
    }
    result
        .recv_timeout(PIPE_CLIENT_IO_TIMEOUT)
        .map_err(|error| error.to_string())?
}

fn async_learning_worker_loop(receiver: Receiver<EngineWriterTask>) {
    while let Ok(task) = receiver.recv() {
        match task {
            EngineWriterTask::Feedback(task) => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    process_async_learning_request(task.request)
                }))
                .unwrap_or_else(|_| {
                    Err((
                        "srf_async_learning_failed",
                        "learning worker panicked".to_owned(),
                    ))
                });
                if let Err((event, error)) = &result {
                    runtime_log::log_engine(RuntimeLogLevel::Error, event, error);
                }
                finish_async_learning_task(task.id, async_learning_outcome_from_result(result));
            }
            EngineWriterTask::Command(job, reply) => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ensure_shared_engine_loaded(None)?;
                    job(&mut lock_shared_engine_recover())
                }))
                .unwrap_or_else(|_| Err("dictionary command panicked".to_owned()));
                let _ = reply.send(result);
                ASYNC_LEARNING_ACTIVE.fetch_sub(1, Ordering::AcqRel);
                notify_async_learning_drain();
            }
        }
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

    let result = match request {
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
    };
    result?;
    state
        .commit_learning()
        .map_err(|error| ("srf_async_learning_failed", error))
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
    let old_engine = std::mem::take(&mut *guard);
    shared_engine().clear_poison();
    LOOKUP_MODEL.store(None);
    runtime_log::log_engine(
        RuntimeLogLevel::Error,
        "srf_shared_engine_lock_recovered",
        format!("context={context} state=reset"),
    );
    drop(guard);
    drop(old_engine);
    lock_shared_engine_recover()
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
    running: usize,
    interactive: usize,
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
    priority: LookupPriority,
}

impl LookupPermit {
    fn queue_wait_us(&self) -> u64 {
        self.queue_wait_us
    }
}

impl Drop for LookupPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scheduler.state.lock() {
            state.running -= 1;
            state.interactive -= usize::from(self.priority == LookupPriority::Interactive);
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

            if state.running < LOOKUP_WORKERS && Self::selected_ticket(&state) == Some(ticket) {
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
                state.running += 1;
                state.interactive += usize::from(priority == LookupPriority::Interactive);
                let waited_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                self.metrics.record_wait(waited_us);
                self.metrics.admitted.fetch_add(1, Ordering::Relaxed);
                self.metrics.active_workers.fetch_add(1, Ordering::AcqRel);
                return Ok(LookupPermit {
                    scheduler: self,
                    queue_wait_us: waited_us,
                    priority,
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
        if state.running != 0 || !state.queue.is_empty() {
            self.metrics
                .background_yields
                .fetch_add(1, Ordering::Relaxed);
            return None;
        }
        state.running += 1;
        self.metrics.active_workers.fetch_add(1, Ordering::AcqRel);
        Some(LookupPermit {
            scheduler: self,
            queue_wait_us: 0,
            priority: LookupPriority::Background,
        })
    }

    fn has_interactive_pressure(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.interactive != 0
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
    last_request_id: u64,
    client_process_id: u32,
    cancellation: LookupCancellationToken,
    decoder: Option<PinyinEngine>,
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
            last_request_id: 0,
            cancellation: LookupCancellationToken::default(),
            decoder: None,
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
    let token = session.cancellation.clone();
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

fn engine_init_mutex() -> &'static Mutex<()> {
    static INIT_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    INIT_MUTEX.get_or_init(|| Mutex::new(()))
}

fn ensure_shared_engine_loaded(requested_dir: Option<&Path>) -> Result<(), String> {
    static DEFAULT_DIR: OnceLock<Result<Option<PathBuf>, String>> = OnceLock::new();
    let target = if let Some(path) = requested_dir {
        SharedEngine::trusted_dir(Some(path))?
    } else {
        DEFAULT_DIR
            .get_or_init(|| SharedEngine::trusted_dir(None))
            .clone()?
    };
    let loaded = || {
        LOOKUP_MODEL
            .load()
            .as_ref()
            .is_some_and(|model| model.target_dir == target)
    };
    if loaded() {
        return Ok(());
    }
    let _initializing = engine_init_mutex()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if !loaded() {
        let engine = PinyinEngine::with_hot_phrase_dir(target.as_deref());
        lock_shared_engine_recover().install_model(engine, target);
    }
    Ok(())
}

fn watch_lexicon_configuration() {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        if !crate::lexicon_prefs::take_phrase_lexicon_reload_flag() {
            continue;
        }
        let Some(current) = LOOKUP_MODEL.load_full() else {
            continue;
        };
        let target = current.target_dir.clone();
        let model = PinyinEngine::with_hot_phrase_dir(target.as_deref());
        let mut state = lock_shared_engine_recover();
        if state.engine.is_some() && state.target_dir == target {
            state.install_model(model, target);
        }
    }
}

pub fn engine_mutex_name_from_env() -> String {
    std::env::var(MUTEX_NAME_ENV)
        .unwrap_or_else(|_| crate::shared_rules::engine_mutex_name_for_suffix(""))
}

fn engine_pipe_name_from_env() -> String {
    std::env::var(PIPE_NAME_ENV)
        .unwrap_or_else(|_| crate::shared_rules::engine_pipe_name_for_suffix(""))
}

pub fn start_engine_service() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(error) = ensure_engine_capability_file() {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_capability_unavailable",
                error.to_string(),
            );
        }
        SERVICE_OWNER.store(true, Ordering::Release);
        clipboard_store::start_store_owner();
        let _ = std::thread::Builder::new()
            .name("srf-engine-lexicon-config".to_owned())
            .spawn(watch_lexicon_configuration);
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
        let sandbox_pipe = crate::shared_rules::sandbox_engine_pipe_name(&pipe_name);
        match std::thread::Builder::new()
            .name("srf-engine-sandbox-pipe-listener".to_string())
            .spawn(listen_loop_sandbox)
        {
            Ok(_) => runtime_log::log_engine(
                RuntimeLogLevel::Basic,
                "engine_sandbox_service_start",
                format!("status=ok listener=spawned pipe={sandbox_pipe}"),
            ),
            Err(err) => runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "engine_sandbox_service_start",
                format!("status=failed listener=spawn pipe={sandbox_pipe} reason={err}"),
            ),
        }
    });
}

pub fn warmup_shared_engine_async() {
    let _ = std::thread::Builder::new()
        .name("srf-engine-warmup".to_string())
        .spawn(|| {
            warmup_shared_engine_sync();
        });
}

/// Synchronously warm up the engine after the tray/helper starts so first input
/// does not pay the initialization cost.
pub fn warmup_shared_engine_sync() {
    // Serialize engine construction with concurrent TSF Init calls so startup
    // cannot spawn multiple parallel PinyinEngine::with_hot_phrase_dir builds.
    let _ = ensure_shared_engine_loaded(None);
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
    let model = LOOKUP_MODEL.load_full().ok_or("missing lookup model")?;
    let lexicon_state = model.lexicon_state;
    let full_in_flight = false;
    let mut engine = PinyinEngine::reader(&model.data);
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

/// Ask the running helper to drain pending learning and exit cleanly.
/// Used by the tray's explicit recovery action before it starts a replacement.
pub fn request_engine_shutdown_via_pipe() -> Result<(), String> {
    EnginePipeClient::connect()?.shutdown()
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
        if response.len() < 3 || response[2] > 1 {
            return Err("invalid pipe lookup response header".to_string());
        }
        let count = u16::from_le_bytes([response[0], response[1]]) as usize;
        if count > LOOKUP_FULL_MAX_CANDIDATES {
            return Err("pipe lookup response contains too many candidates".to_string());
        }
        Ok((count, response.len()))
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        let mut payload = Vec::with_capacity(8 + self.capability.len() * 2);
        append_utf16_string(&mut payload, &self.capability);
        let mut request = Vec::with_capacity(12 + payload.len());
        request.extend_from_slice(&PROTOCOL_MAGIC.to_le_bytes());
        request.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        request.extend_from_slice(&(EngineCommand::Shutdown as u16).to_le_bytes());
        request.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        request.extend_from_slice(&payload);
        self.file
            .write_all(&request)
            .map_err(|err| format!("shutdown pipe write failed: {err}"))?;
        self.file
            .flush()
            .map_err(|err| format!("shutdown pipe flush failed: {err}"))?;

        let mut header = [0u8; 16];
        self.file
            .read_exact(&mut header)
            .map_err(|err| format!("shutdown response read failed: {err}"))?;
        let magic = u32::from_le_bytes(header[0..4].try_into().expect("fixed header"));
        let version = u16::from_le_bytes(header[4..6].try_into().expect("fixed header"));
        let command = u16::from_le_bytes(header[6..8].try_into().expect("fixed header"));
        let status = i32::from_le_bytes(header[8..12].try_into().expect("fixed header"));
        let response_len =
            u32::from_le_bytes(header[12..16].try_into().expect("fixed header")) as usize;
        if magic != PROTOCOL_MAGIC
            || version != PROTOCOL_VERSION
            || command != EngineCommand::Shutdown as u16
            || response_len > 64 * 1024
        {
            return Err("shutdown response header mismatch".to_string());
        }
        let mut response = vec![0u8; response_len];
        self.file
            .read_exact(&mut response)
            .map_err(|err| format!("shutdown response body read failed: {err}"))?;
        if status == 0 {
            Ok(())
        } else {
            Err(format!("shutdown request rejected status={status}"))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipeAccessMode {
    Full,
    SandboxReadOnly,
}

fn listen_loop() {
    if std::panic::catch_unwind(|| listen_loop_inner(PipeAccessMode::Full)).is_err() {
        runtime_log::log_engine(
            RuntimeLogLevel::Error,
            "srf_ipc_listener_panic",
            "listener loop panicked",
        );
        std::process::exit(1);
    }
}

fn listen_loop_sandbox() {
    if std::panic::catch_unwind(|| listen_loop_inner(PipeAccessMode::SandboxReadOnly)).is_err() {
        runtime_log::log_engine(
            RuntimeLogLevel::Error,
            "srf_ipc_sandbox_listener_panic",
            "sandbox listener loop panicked",
        );
    }
}

fn listen_loop_inner(access_mode: PipeAccessMode) {
    let base_pipe_name = engine_pipe_name_from_env();
    let pipe_name_text = match access_mode {
        PipeAccessMode::Full => base_pipe_name,
        PipeAccessMode::SandboxReadOnly => {
            crate::shared_rules::sandbox_engine_pipe_name(&base_pipe_name)
        }
    };
    let pipe_name = wide(&pipe_name_text);
    let security_result = match access_mode {
        PipeAccessMode::Full => crate::windows_security::LocalLogonPipeSecurity::for_ime_engine(),
        PipeAccessMode::SandboxReadOnly => {
            crate::windows_security::LocalLogonPipeSecurity::for_ime_engine_sandbox()
        }
    };
    let security = match security_result {
        Ok(security) => security,
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_security_init_failed",
                format!(
                    "mode={access_mode:?} reason=token_unavailable io_kind={:?}",
                    err.kind()
                ),
            );
            if access_mode == PipeAccessMode::Full {
                std::process::exit(1);
            }
            return;
        }
    };
    let mut consecutive_create_failures = 0usize;
    loop {
        let pipe = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
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
                        "mode={access_mode:?} pipe={} consecutive_failures={} last_error={}",
                        pipe_name_text, consecutive_create_failures, last_error
                    ),
                );
                if access_mode == PipeAccessMode::Full {
                    crate::engine_health::publish_listener_failure(last_error);
                    std::process::exit(1);
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        consecutive_create_failures = 0;
        // SAFETY: CreateNamedPipeW returned a new handle owned by this loop.
        let pipe = unsafe { OwnedWinHandle::from_raw(pipe) }.expect("validated engine pipe");

        let Ok(mut pipe) = ServerPipe::new(pipe) else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if pipe.connect() {
            let Some(client_guard) = ActivePipeClient::try_acquire() else {
                runtime_log::log_engine(
                    RuntimeLogLevel::Error,
                    "srf_ipc_client_limit",
                    format!(
                        "mode={access_mode:?} active={} limit={}",
                        ACTIVE_PIPE_CLIENTS.load(Ordering::Acquire),
                        PIPE_MAX_ACTIVE_CLIENTS
                    ),
                );
                unsafe {
                    let _ = DisconnectNamedPipe(pipe.handle.as_raw());
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            };
            if std::thread::Builder::new()
                .name(match access_mode {
                    PipeAccessMode::Full => "srf-engine-pipe-client".to_string(),
                    PipeAccessMode::SandboxReadOnly => "srf-engine-sandbox-client".to_string(),
                })
                .spawn(move || handle_client(pipe, client_guard, access_mode))
                .is_err()
            {
                std::thread::sleep(Duration::from_millis(50));
            }
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

fn ensure_engine_capability_file() -> io::Result<()> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::GENERIC_WRITE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, MoveFileExW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, MOVEFILE_WRITE_THROUGH,
    };

    let path =
        capability_path().ok_or_else(|| io::Error::other("missing engine capability path"))?;
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = zeroize::Zeroizing::new(crate::windows_security::generate_capability_token()?);
    let protected =
        crate::windows_security::dpapi_protect_with_magic(CAPABILITY_MAGIC, token.as_bytes())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let temp_wide = wide_os_path(&temp);
    let path_wide = wide_os_path(&path);
    let security = crate::windows_security::LocalLogonPipeSecurity::new()?;
    let handle = unsafe {
        CreateFileW(
            temp_wide.as_ptr(),
            GENERIC_WRITE,
            0,
            security.as_ptr(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // CREATE_NEW confines writes to our temporary file. Publication never
    // replaces a capability another TIP or engine already installed.
    let mut file = unsafe { std::fs::File::from_raw_handle(handle as _) };
    let written = file.write_all(&protected).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| {
        if unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                path_wide.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if path.is_file() {
                Ok(())
            } else {
                Err(error)
            }
        } else {
            Ok(())
        }
    });
    let _ = std::fs::remove_file(temp);
    result
}

fn wide_os_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
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
            "reason=token_mismatch status=denied",
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

fn handle_client(
    mut server: ServerPipe,
    _client_guard: ActivePipeClient,
    access_mode: PipeAccessMode,
) {
    let pipe = server.handle.as_raw();
    // A persistent pipe is owned by one TIP process. Keep composition-local
    // lookup state on that connection without changing the V5 wire format.
    let mut client_process_id = 0u32;
    unsafe {
        let _ = GetNamedPipeClientProcessId(pipe, &mut client_process_id);
    }
    let mut lookup_session = ClientLookupSession::new(client_process_id);
    while let Some((command, payload)) = read_request(&mut server) {
        let identity_ok = match access_mode {
            PipeAccessMode::Full => crate::windows_security::verify_ime_pipe_client(pipe),
            PipeAccessMode::SandboxReadOnly => {
                crate::windows_security::verify_ime_sandbox_pipe_client(pipe)
            }
        };
        if identity_ok.is_err() {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_identity_denied",
                format!("mode={access_mode:?} status=denied"),
            );
            break;
        }
        lookup_session.last_request_id = 0;
        let (status, response) =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch_request_with_session(
                    command,
                    &payload,
                    Some(&mut lookup_session),
                    access_mode,
                )
            })) {
                Ok(result) => result,
                Err(_) => (ENGINE_PANIC_RC, Vec::new()),
            };
        let shutdown_requested = access_mode == PipeAccessMode::Full
            && command == EngineCommand::Shutdown
            && status == 0;
        if status != 0
            && !(command == EngineCommand::Lookup
                && (status == -8 || (status == -5 && lookup_session.last_request_id == 0)))
            && status != -7
        {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "srf_ipc_request_failed",
                format!(
                    "client_pid={} command={} request_id={} status={}",
                    client_process_id, command as u16, lookup_session.last_request_id, status
                ),
            );
        }
        let write_started = Instant::now();
        let written = write_response(&mut server, command, status, &response);
        if command == EngineCommand::Lookup && perf_log_enabled() {
            runtime_log::log_engine(RuntimeLogLevel::Perf, "srf_ipc_lookup_write",
                format!("session_id={} request_id={} client_pid={} bytes={} write={}us status={} written={}",
                    lookup_session.id, lookup_session.last_request_id, client_process_id, response.len(),
                    write_started.elapsed().as_micros(), status, written));
        }
        if !written {
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
            {
                let mut state = lock_shared_engine_recover();
                if let Some(engine) = state.engine.as_mut() {
                    if let Err(err) = engine.flush_pending_learning() {
                        runtime_log::log_engine(
                            RuntimeLogLevel::Error,
                            "srf_shutdown_user_dict_flush_failed",
                            err.to_string(),
                        );
                    }
                }
            }
            unsafe {
                let _ = FlushFileBuffers(pipe);
                let _ = DisconnectNamedPipe(pipe);
            }
            std::process::exit(0);
        }
    }

    unsafe {
        let _ = DisconnectNamedPipe(pipe);
    }
}

fn read_request(pipe: &mut ServerPipe) -> Option<(EngineCommand, Vec<u8>)> {
    let mut header = [0u8; 12];
    if !pipe.read_exact(&mut header, PIPE_HEADER_IO_TIMEOUT) {
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
    let limit = if command == EngineCommand::Storage {
        MAX_STORAGE_REQUEST_BYTES
    } else {
        MAX_REQUEST_BYTES
    };
    if payload_len > limit {
        return None;
    }
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 && !pipe.read_exact(&mut payload, PIPE_CLIENT_IO_TIMEOUT) {
        return None;
    }
    Some((command, payload))
}

fn write_response(
    pipe: &mut ServerPipe,
    command: EngineCommand,
    status: i32,
    payload: &[u8],
) -> bool {
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(&PROTOCOL_MAGIC.to_le_bytes());
    header.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    header.extend_from_slice(&(command as u16).to_le_bytes());
    header.extend_from_slice(&status.to_le_bytes());
    header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    pipe.write_all(&header) && (payload.is_empty() || pipe.write_all(payload))
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
    access_mode: PipeAccessMode,
) -> (i32, Vec<u8>) {
    if access_mode == PipeAccessMode::SandboxReadOnly {
        return match command {
            EngineCommand::Init => handle_init(payload, access_mode),
            EngineCommand::Lookup => {
                handle_lookup(payload, lookup_session.as_deref_mut(), access_mode)
            }
            EngineCommand::SyllableBounds => handle_syllable_bounds(payload),
            EngineCommand::Health => handle_health(access_mode),
            EngineCommand::CancelLookup => handle_cancel_lookup(payload, lookup_session.as_deref()),
            _ => (-6, error_payload("sandbox pipe denies privileged command")),
        };
    }
    match command {
        EngineCommand::Init => handle_init(payload, access_mode),
        EngineCommand::Lookup => handle_lookup(payload, lookup_session.as_deref_mut(), access_mode),
        EngineCommand::Learn => handle_learn(payload),
        EngineCommand::SyllableBounds => handle_syllable_bounds(payload),
        EngineCommand::RecordClipboard => handle_record_clipboard(payload),
        EngineCommand::SetCandidatePin => handle_set_candidate_pin(payload),
        EngineCommand::Health => handle_health(access_mode),
        EngineCommand::ResolveClipboard => handle_resolve_clipboard(payload),
        EngineCommand::Shutdown => handle_shutdown(payload),
        EngineCommand::LearnCorrection => handle_learn_correction(payload),
        EngineCommand::LearnSelectionFeedback => handle_learn_selection_feedback(payload),
        EngineCommand::CandidateAction => handle_candidate_action(payload),
        EngineCommand::ResetLearningContext => handle_reset_learning_context(payload),
        EngineCommand::CancelLookup => handle_cancel_lookup(payload, lookup_session.as_deref()),
        EngineCommand::LearnStatus => handle_learn_status(payload),
        EngineCommand::Storage => handle_storage(payload),
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) enum StorageRequest {
    Dictionary(DictionaryRequest),
    Clipboard(clipboard_store::ClipboardRequest),
}

fn handle_storage(payload: &[u8]) -> (i32, Vec<u8>) {
    let mut cursor = PayloadCursor::new(payload);
    if let Err(status) = verify_capability_token(&mut cursor) {
        return (status, Vec::new());
    }
    let request = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_STORAGE_REQUEST_BYTES as u64)
        .deserialize(&payload[cursor.offset..]);
    let result = match request {
        Ok(StorageRequest::Dictionary(request)) => manage_dictionary(request),
        Ok(StorageRequest::Clipboard(request)) => clipboard_store::handle_storage_request(request),
        Err(_) => return (-1, Vec::new()),
    };
    match result {
        Ok(payload) if payload.len() <= MAX_STORAGE_RESPONSE_BYTES => (0, payload),
        Ok(_) => (-3, b"storage response exceeds the size limit".to_vec()),
        Err(error) => (-3, error.into_bytes()),
    }
}

fn manage_dictionary(request: DictionaryRequest) -> Result<Vec<u8>, String> {
    run_writer_command(move |state| {
        let changes = request.changes_data();
        let engine = state.engine_mut()?;
        let response = engine
            .storage_dictionary_mut()
            .manage(request)
            .map_err(|error| error.to_string())?;
        if changes {
            engine.note_user_lexicon_updated();
            state.commit_learning()?;
        }
        Ok(response)
    })
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

fn append_lookup_text(out: &mut Vec<u8>, value: &str) {
    let start = out.len();
    out.extend_from_slice(&[0; 2]);
    let mut units = 0_u16;
    for ch in value.chars() {
        let mut buffer = [0_u16; 2];
        let encoded = ch.encode_utf16(&mut buffer);
        if units as usize + encoded.len() > ROW_MAX_FIELD_UNITS as usize {
            break;
        }
        for unit in encoded {
            out.extend_from_slice(&unit.to_le_bytes());
            units += 1;
        }
    }
    out[start..start + 2].copy_from_slice(&units.to_le_bytes());
}

fn handle_health(access_mode: PipeAccessMode) -> (i32, Vec<u8>) {
    let model = LOOKUP_MODEL.load();
    let loaded = model.is_some();
    let target_dir = model.as_ref().and_then(|model| model.target_dir.clone());
    let lexicon_state = model
        .as_ref()
        .map_or("core", |model| model.lexicon_state)
        .to_string();
    let full_in_flight = false;
    let cache_signature =
        engine_cache_signature(&lexicon_state, full_in_flight, target_dir.as_deref());

    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    let advertised_pipe = match access_mode {
        PipeAccessMode::Full => engine_pipe_name_from_env(),
        PipeAccessMode::SandboxReadOnly => {
            crate::shared_rules::sandbox_engine_pipe_name(&engine_pipe_name_from_env())
        }
    };

    let mut out = Vec::new();
    append_u32(&mut out, PROTOCOL_VERSION as u32);
    append_utf16_string(&mut out, &engine_build_id());
    append_utf16_string(&mut out, &exe);
    append_utf16_string(&mut out, "");
    append_utf16_string(&mut out, "");
    append_utf16_string(&mut out, &advertised_pipe);
    append_utf16_string(&mut out, &engine_mutex_name_from_env());
    append_u32(&mut out, if loaded { 1 } else { 0 });
    append_utf16_string(&mut out, &lexicon_state);
    append_utf16_string(&mut out, "");
    append_u32(&mut out, if full_in_flight { 1 } else { 0 });
    append_utf16_string(&mut out, crate::model_resources::model_hash());
    append_utf16_string(&mut out, &cache_signature);
    (0, out)
}

fn handle_init(payload: &[u8], access_mode: PipeAccessMode) -> (i32, Vec<u8>) {
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

    // Never allow a sandboxed AppContainer client to redirect the shared
    // engine's lexicon directory; always bind it to the engine's trusted default.
    let requested_dir = if access_mode == PipeAccessMode::SandboxReadOnly || path.is_empty() {
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
    mut lookup_session: Option<&mut ClientLookupSession>,
    access_mode: PipeAccessMode,
) -> (i32, Vec<u8>) {
    static FIRST_LOOKUP_LOGGED: OnceLock<()> = OnceLock::new();
    let lookup_started = Instant::now();
    let mut cursor = PayloadCursor::new(payload);
    if access_mode == PipeAccessMode::SandboxReadOnly {
        // Keep the V6 wire framing intact while authenticating via same-user /
        // same-session pipe impersonation and unconditionally disabling
        // clipboard access below.
        let len = match cursor.read_u32() {
            Some(v) => v as usize,
            None => return (-1, Vec::new()),
        };
        if cursor.read_utf16_string(len, 128).is_none() {
            return (-1, Vec::new());
        }
    } else {
        // Lookup can enter clipboard utility modes and return clipboard previews.
        // Authenticate every lookup rather than trying to classify readings here:
        // classification after parsing has repeatedly left alternate query paths
        // capable of exposing the same protected store.
        if let Err(status) = verify_capability_token(&mut cursor) {
            return (status, Vec::new());
        }
    }
    let raw_mode_flags = match cursor.read_u32() {
        Some(v) => v,
        None => return (-1, Vec::new()),
    };
    let mode_flags = if access_mode == PipeAccessMode::SandboxReadOnly {
        (raw_mode_flags | kaixin_common::engine_contract::MODE_CLIPBOARD_DISABLED)
            & !kaixin_common::engine_contract::MODE_CLIPBOARD_BACKGROUND
    } else {
        raw_mode_flags
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
    if let Some(session) = lookup_session.as_deref_mut() {
        session.last_request_id = request_id;
    }
    let (session_id, lookup_generation, cancellation) =
        if let Some(session) = lookup_session.as_deref() {
            let generation = session.cancellation.next_generation();
            (session.id, generation, Some(session.cancellation.clone()))
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
    let init_started = Instant::now();
    let mut model = LOOKUP_MODEL.load_full();
    if model.is_none() {
        if priority == LookupPriority::Background {
            return (-5, error_payload("shared engine busy"));
        }
        if let Err(error) = ensure_shared_engine_loaded(None) {
            return (-3, error_payload(&error));
        }
        model = LOOKUP_MODEL.load_full();
    }
    if model.is_none() {
        return (-3, error_payload("missing lookup model"));
    }
    let init_us = init_started.elapsed().as_micros();
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
            LOOKUP_INTERACTIVE_BUSY_WAIT,
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
            // Request id zero is reserved for background prewarm/probe work.
            // Yielding to interactive input is expected scheduling behavior,
            // so keep it out of the error stream while preserving opt-in
            // diagnostics for capacity tuning.
            let log_level = if priority == LookupPriority::Background {
                RuntimeLogLevel::Perf
            } else {
                RuntimeLogLevel::Error
            };
            runtime_log::log_engine(
                log_level,
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

    let Some(model) = LOOKUP_MODEL.load_full() else {
        return (-3, error_payload("missing lookup model"));
    };
    let lexicon_state = model.lexicon_state;
    let full_in_flight = false;
    let engine_started = Instant::now();
    let (ranked, warning) = if let Some(session) = lookup_session {
        let decoder = session
            .decoder
            .get_or_insert_with(|| PinyinEngine::reader(&model.data));
        decoder.refresh_lookup_snapshot(&model.data);
        decoder.lookup_for_client(
            session.cancellation.clone(),
            mode_flags,
            reading.trim(),
            lookup_generation,
        )
    } else {
        let mut decoder = PinyinEngine::reader(&model.data);
        decoder.set_mode_flags(mode_flags);
        decoder.lookup_full_detailed_with_request_id(reading.trim(), lookup_generation)
    };
    if warning.as_deref() == Some("lookup superseded") {
        lookup_scheduler().note_superseded();
        return (-8, error_payload("lookup superseded"));
    }
    let engine_us = engine_started.elapsed().as_micros();
    let lock_wait_us = 0;
    let scheduler_after_engine = lookup_scheduler().snapshot();
    drop(permit);

    if cancellation
        .as_ref()
        .is_some_and(|token| token.is_superseded(lookup_generation))
    {
        lookup_scheduler().note_superseded();
        return (-8, error_payload("lookup superseded"));
    }
    let serialize_started = Instant::now();

    let result_limit = if full_result {
        LOOKUP_FULL_MAX_CANDIDATES
    } else {
        LOOKUP_FIRST_BATCH_CANDIDATES
    };
    let count = ranked.len().min(result_limit);
    let has_more = !full_result && ranked.len() > count;
    let mut payload = Vec::with_capacity(3 + count * 32);
    payload.extend_from_slice(&(count as u16).to_le_bytes());
    payload.push(u8::from(has_more));
    for candidate in ranked.iter().take(count) {
        append_lookup_row(
            &mut payload,
            &candidate.phrase,
            &candidate.meta,
            candidate.score,
        );
    }
    let serialize_us = serialize_started.elapsed().as_micros();
    let total_us = lookup_started.elapsed().as_micros();
    if perf_log_enabled() {
        let scheduler = scheduler_after_engine;
        let first = FIRST_LOOKUP_LOGGED.set(()).is_ok();
        if should_log_lookup_summary(first, total_us, count) {
            runtime_log::log_engine(
                RuntimeLogLevel::Perf,
                "srf_ipc_lookup",
                format!(
                    "request_id={} lookup_generation={} {} candidates={} first={} response_format={} response_bytes={} lexicon_state={} full_in_flight={} queue_wait={}us queue_p50={}us queue_p95={}us queue_p99={}us admitted={} coalesced={} superseded={} busy={} background_yields={} active_workers={} engine={}us lock_wait={}us init={}us serialize={}us total={}us session_id={} status=ok",
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
                    lock_wait_us,
                    init_us,
                    serialize_us,
                    total_us,
                    session_id
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

// Protocol v6. Each row has a flag word, an i32 score in hundredths and its
// UTF-16LE phrase. Optional strings follow in flag order, then optional u32
// clipboard page numbers. Strings carry a u16 count of UTF-16 code units.
fn append_lookup_row(out: &mut Vec<u8>, phrase: &str, meta: &CandidateMeta, score: f64) {
    const USER_MASK: u32 = ROW_USER_COMMON | ROW_USER_FRESH | ROW_USER_MIXED | ROW_USER_OBSERVED;
    const STRING_FLAGS: [u32; 8] = [
        ROW_DISPLAY,
        ROW_ANNOTATION,
        ROW_CORRECTED_READING,
        ROW_CLIPBOARD_ID,
        ROW_CLIPBOARD_SOURCE,
        ROW_CLIPBOARD_TIME,
        ROW_CLIPBOARD_TYPE,
        ROW_CLIPBOARD_FILTER,
    ];
    let mut flags = 0;
    if meta.pinned {
        flags |= ROW_PINNED;
    }
    if meta.partial {
        flags |= ROW_PARTIAL;
    }
    if meta.source == CandidateSource::Correction
        || meta.match_kind == CandidateMatchKind::Correction
    {
        flags |= ROW_CORRECTION;
    }
    if matches!(meta.source_layer, LexiconLayer::Ext | LexiconLayer::Large) {
        flags |= ROW_EXT;
    }

    let mut strings = [""; STRING_FLAGS.len()];
    let mut annotations = Vec::new();
    let mut page = None;
    let mut pages = None;
    for token in meta
        .display_text()
        .unwrap_or_default()
        .split('\t')
        .map(str::trim)
    {
        if let Some((key, value)) = token.split_once('=') {
            let value = value.trim();
            match key {
                "display" => strings[0] = value,
                "corrected_reading" => strings[2] = value,
                "clipboard_key" => strings[3] = value,
                "clipboard_source" => strings[4] = value,
                "clipboard_time" => strings[5] = value,
                "clipboard_type" => strings[6] = value,
                "clipboard_filter" => strings[7] = value,
                "clipboard_page" => page = Some(lookup_clipboard_page(value)),
                "clipboard_pages" => pages = Some(lookup_clipboard_page(value)),
                _ => {}
            }
        }
        match token {
            "no_learn" | "no_learn=1" | "learn=0" => flags |= ROW_NO_LEARN,
            "clipboard_quick" | "clipboard_quick=1" => flags |= ROW_CLIPBOARD_QUICK,
            "clipboard_pinned" | "clipboard_pinned=1" => flags |= ROW_CLIPBOARD_PINNED,
            "layout=vertical" | "force_vertical=1" => flags |= ROW_VERTICAL,
            "partial" | "partial=1" => flags |= ROW_PARTIAL,
            "pinned" | "pinned=1" | "pin=1" => flags |= ROW_PINNED,
            "prefix_placeholder" | "prefix_placeholder=1" | "placeholder=prefix" => {
                flags |= ROW_PLACEHOLDER
            }
            "source=correction" | "match=correction" | "typo" | "typo=1" => flags |= ROW_CORRECTION,
            "layer=ext" | "layer=large" => flags |= ROW_EXT,
            "source=user_fresh" => flags = (flags & !USER_MASK) | ROW_USER_FRESH,
            "mixed_pinyin_user" | "source=user_mixed" => {
                flags = (flags & !USER_MASK) | ROW_USER_MIXED
            }
            "observed_user" | "source=user_observed" => {
                flags = (flags & !USER_MASK) | ROW_USER_OBSERVED
            }
            "user" | "user=1" | "用户词" | "source=user" | "source=user_common" => {
                if flags & USER_MASK == 0 {
                    flags |= ROW_USER_COMMON;
                }
            }
            _ if token.starts_with("correction=") => flags |= ROW_CORRECTION,
            _ if token.starts_with("纠错") || token.starts_with("音近") => {
                flags |= ROW_CORRECTION;
                annotations.push(token);
            }
            _ if token.starts_with("Emoji:") || token.starts_with("符号:") => {
                annotations.push(token)
            }
            _ => {}
        }
    }
    let annotation = annotations.join(" | ");
    strings[1] = &annotation;
    if meta.source == CandidateSource::User && flags & USER_MASK == 0 {
        flags |= ROW_USER_COMMON;
    }
    for (value, flag) in strings.iter().zip(STRING_FLAGS) {
        if !value.is_empty() {
            flags |= flag;
        }
    }
    if page.is_some() {
        flags |= ROW_CLIPBOARD_PAGE;
    }
    if pages.is_some() {
        flags |= ROW_CLIPBOARD_PAGES;
    }
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&((score * 100.0).round() as i32).to_le_bytes());
    append_lookup_text(out, phrase);
    for value in strings.iter().filter(|value| !value.is_empty()) {
        append_lookup_text(out, value);
    }
    for number in [page, pages].into_iter().flatten() {
        out.extend_from_slice(&number.to_le_bytes());
    }
}

fn lookup_clipboard_page(value: &str) -> u32 {
    value.parse::<u32>().unwrap_or(1).clamp(1, 100_000)
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

    match run_writer_command(move |state| {
        state
            .engine_mut()?
            .set_candidate_pin(reading.trim(), committed.trim(), pinned)?;
        state.commit_learning()?;
        Ok(Vec::new())
    }) {
        Ok(response) => (0, response),
        Err(error) => (-3, error_payload(&error)),
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

    if !matches!(action, 1..=3) {
        return (-1, Vec::new());
    }
    match run_writer_command(move |state| {
        let engine = state.engine_mut()?;
        match action {
            1 => engine.remove_user_phrase(reading.trim(), phrase.trim())?,
            2 => engine.block_user_phrase(phrase.trim())?,
            3 => engine.unblock_user_phrase(phrase.trim())?,
            _ => unreachable!(),
        };
        state.commit_learning()?;
        Ok(Vec::new())
    }) {
        Ok(response) => (0, response),
        Err(error) => (-3, error_payload(&error)),
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

    match run_writer_command(|state| {
        state.engine_mut()?.reset_learning_context();
        state.publish();
        Ok(Vec::new())
    }) {
        Ok(response) => (0, response),
        Err(error) => (-3, error_payload(&error)),
    }
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

    let offsets =
        syllable_boundary_offsets_utf16(text.trim(), crate::core::shared_engine_model_syllables());
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
        for chunk in bytes.as_chunks::<2>().0 {
            utf16.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        Some(String::from_utf16_lossy(&utf16))
    }

    fn is_at_end(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

struct ServerPipe {
    handle: OwnedWinHandle,
    event: OwnedWinHandle,
}

impl ServerPipe {
    fn new(handle: OwnedWinHandle) -> io::Result<Self> {
        let event =
            unsafe { OwnedWinHandle::from_raw(CreateEventW(null_mut(), 1, 0, null_mut())) }?;
        Ok(Self { handle, event })
    }

    fn overlapped(&self) -> OVERLAPPED {
        OVERLAPPED {
            hEvent: self.event.as_raw(),
            ..unsafe { std::mem::zeroed() }
        }
    }

    fn connect(&mut self) -> bool {
        let mut operation = self.overlapped();
        let mut transferred = 0;
        let started = unsafe { ConnectNamedPipe(self.handle.as_raw(), &mut operation) };
        matches!(
            self.complete(started, &mut operation, &mut transferred, INFINITE),
            Ok(()) | Err(ERROR_PIPE_CONNECTED)
        )
    }

    fn complete(
        &self,
        started: BOOL,
        operation: &mut OVERLAPPED,
        transferred: &mut u32,
        timeout_ms: u32,
    ) -> Result<(), u32> {
        if started != 0 {
            *transferred = operation.InternalHigh as u32;
            return Ok(());
        }
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(error);
        }
        let handle = self.handle.as_raw();
        if unsafe { GetOverlappedResultEx(handle, operation, transferred, timeout_ms, 0) } != 0 {
            return Ok(());
        }
        let error = unsafe { GetLastError() };
        if matches!(error, WAIT_TIMEOUT | ERROR_IO_INCOMPLETE) {
            unsafe {
                CancelIoEx(handle, operation);
                GetOverlappedResult(handle, operation, transferred, 1);
            }
        }
        Err(error)
    }

    fn read_exact(&mut self, mut buf: &mut [u8], timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while !buf.is_empty() {
            let mut operation = self.overlapped();
            let mut read = 0u32;
            let started = unsafe {
                ReadFile(
                    self.handle.as_raw(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    null_mut(),
                    &mut operation,
                )
            };
            let timeout_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u32;
            let transferred = match self.complete(started, &mut operation, &mut read, timeout_ms) {
                Ok(()) => read as usize,
                // Message mode reports this after filling the requested buffer.
                // Bytes after that buffer remain available to the next read.
                Err(ERROR_MORE_DATA) => buf.len(),
                Err(_) => return false,
            };
            if transferred == 0 || transferred > buf.len() {
                return false;
            }
            buf = &mut buf[transferred..];
        }
        true
    }

    fn write_all(&mut self, mut buf: &[u8]) -> bool {
        let deadline = Instant::now() + PIPE_CLIENT_IO_TIMEOUT;
        while !buf.is_empty() {
            let mut operation = self.overlapped();
            let mut written = 0u32;
            let started = unsafe {
                WriteFile(
                    self.handle.as_raw(),
                    buf.as_ptr(),
                    buf.len() as u32,
                    null_mut(),
                    &mut operation,
                )
            };
            let timeout_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis() as u32;
            if self
                .complete(started, &mut operation, &mut written, timeout_ms)
                .is_err()
                || written == 0
                || written as usize > buf.len()
            {
                return false;
            }
            buf = &buf[written as usize..];
        }
        true
    }
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
    fn overlapped_pipe_cancels_idle_read_and_preserves_message_remainder() {
        let stamp = SystemTime::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let name = format!("\\\\.\\pipe\\kaixin-io-test-{}-{stamp}", std::process::id());
        let handle = unsafe {
            OwnedWinHandle::from_raw(CreateNamedPipeW(
                wide(&name).as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                null_mut(),
            ))
        }
        .unwrap();
        let mut server = ServerPipe::new(handle).unwrap();
        let mut client = OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .unwrap();
        assert!(server.connect());

        let mut chunk = [0; 3];
        assert!(!server.read_exact(&mut chunk, Duration::from_millis(5)));
        client.write_all(b"abcdef").unwrap();
        assert!(server.read_exact(&mut chunk, Duration::from_secs(1)));
        assert_eq!(&chunk, b"abc");
        assert!(server.read_exact(&mut chunk, Duration::from_secs(1)));
        assert_eq!(&chunk, b"def");
        assert!(server.write_all(b"ok"));
        let mut reply = [0; 2];
        client.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ok");
    }

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

    #[test]
    fn sandbox_pipe_denies_privileged_and_write_commands() {
        for cmd in [
            EngineCommand::Learn,
            EngineCommand::RecordClipboard,
            EngineCommand::SetCandidatePin,
            EngineCommand::ResolveClipboard,
            EngineCommand::Shutdown,
            EngineCommand::LearnCorrection,
            EngineCommand::LearnSelectionFeedback,
            EngineCommand::CandidateAction,
            EngineCommand::ResetLearningContext,
            EngineCommand::LearnStatus,
        ] {
            let (status, _payload) =
                dispatch_request_with_session(cmd, &[], None, PipeAccessMode::SandboxReadOnly);
            assert_eq!(status, -6, "sandbox pipe must deny command {}", cmd as u16);
        }
        let (health_status, _) = dispatch_request_with_session(
            EngineCommand::Health,
            &[],
            None,
            PipeAccessMode::SandboxReadOnly,
        );
        assert_eq!(health_status, 0, "sandbox pipe must allow Health");
    }
}
