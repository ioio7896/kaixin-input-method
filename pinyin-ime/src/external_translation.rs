#[cfg(windows)]
use crate::win_handle::OwnedWinHandle;
use crate::{app_paths, runtime_log};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(windows))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64 as SessionCounter, Ordering as SessionOrdering},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant};
#[cfg(not(windows))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(windows))]
use std::fs::OpenOptions;
#[cfg(not(windows))]
use std::io::{BufRead, BufReader, Write};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
#[cfg(windows)]
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
#[cfg(windows)]
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};

pub const PIPE_PATH: &str = r"\\.\pipe\WinTranslator.Request";
pub const HYMT_PIPE_PATH: &str = r"\\.\pipe\HyMT2.IME.Request";
pub const HYMT_TRANSLATOR_EXE: &str = "hy-mt2-desktop.exe";
pub const WINTRANSLATOR_EXE: &str = "WinTranslator.exe";
const CREATE_NO_WINDOW: u32 = 0x08000000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const PIPE_PHASE_TIMEOUT_MS: u32 = 2_500;
pub const PROTOCOL_VERSION: u32 = 2;
#[cfg(not(windows))]
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalTranslationRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub action: String,
    pub text: String,
    pub source: String,
    pub target: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_hwnd: Option<isize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_process_id: Option<u32>,
    pub result_action: String,
    pub interactive: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<PathBuf>,
    pub presentation: String,
    pub delivery: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_pipe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_generation: Option<u64>,
    pub replace_selection: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_request_id: Option<String>,
    #[serde(default)]
    pub query_request_id: Option<String>,
    #[serde(default)]
    pub chunk_index: usize,
    #[serde(default)]
    pub callback_ack: bool,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub generation: u64,
    #[serde(skip)]
    safety: InputSafety,
}

#[derive(Debug, Deserialize)]
struct ExternalTranslationResponse {
    ok: bool,
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    protocol_version: u32,
    #[serde(default)]
    capabilities: Option<WinTranslatorCapabilities>,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model_ready: Option<bool>,
    #[serde(default)]
    engine_ready: Option<bool>,
    #[serde(default)]
    queued: usize,
    #[serde(default)]
    result: Option<TranslationCallbackEvent>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct WinTranslatorCapabilities {
    #[serde(default)]
    protocol_versions: Vec<u32>,
    #[serde(default)]
    actions: Vec<String>,
    #[serde(default)]
    presentations: Vec<String>,
    #[serde(default)]
    deliveries: Vec<String>,
    #[serde(default)]
    callback_events: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct TranslationCallbackEvent {
    #[serde(rename = "event")]
    event_name: String,
    request_id: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    target_hwnd: Option<isize>,
    #[serde(default)]
    target_process_id: Option<u32>,
    #[serde(default)]
    focus_generation: Option<u64>,
    #[serde(default)]
    chunk_index: usize,
    #[serde(default = "one_chunk")]
    chunk_count: usize,
}
fn one_chunk() -> usize {
    1
}

#[derive(Clone, Debug, Default)]
struct InputSafety {
    clipboard: u32,
    input: u32,
    focus: isize,
    caret: isize,
    rect: [i32; 4],
}
#[cfg(windows)]
fn input_safety() -> InputSafety {
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
        WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO},
    };
    let mut info: LASTINPUTINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<LASTINPUTINFO>() as u32;
    let mut gui: GUITHREADINFO = unsafe { std::mem::zeroed() };
    gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    unsafe {
        GetLastInputInfo(&mut info);
        GetGUIThreadInfo(0, &mut gui);
    }
    InputSafety {
        clipboard: unsafe {
            windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber()
        },
        input: info.dwTime,
        focus: gui.hwndFocus,
        caret: gui.hwndCaret,
        rect: [
            gui.rcCaret.left,
            gui.rcCaret.top,
            gui.rcCaret.right,
            gui.rcCaret.bottom,
        ],
    }
}
#[cfg(not(windows))]
fn input_safety() -> InputSafety {
    InputSafety::default()
}
struct LocalSession {
    id: String,
    stop: Arc<AtomicBool>,
}
fn sessions() -> &'static Mutex<HashMap<String, LocalSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, LocalSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
pub fn cancel_pending_translations() -> usize {
    let active = sessions().lock().unwrap();
    let count = active.len();
    for session in active.values() {
        session.stop.store(true, SessionOrdering::Release);
    }
    drop(active);
    std::thread::spawn(|| {
        let mut request = ExternalTranslationRequest::new("", "settings-cancel");
        request.action = "cancel_pending".into();
        let _ = send_request_once(&request);
    });
    count
}
struct SessionGuard {
    key: String,
    id: String,
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        let mut active = sessions().lock().unwrap();
        if active.get(&self.key).is_some_and(|s| s.id == self.id) {
            active.remove(&self.key);
        }
    }
}

impl ExternalTranslationRequest {
    pub fn new(text: impl Into<String>, origin: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            protocol_version: PROTOCOL_VERSION,
            request_id: new_request_id(),
            action: "translate".to_string(),
            text,
            source: "auto".to_string(),
            target: read_tools_value("translate_target_language")
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "auto-opposite".into()),
            origin: origin.into(),
            target_hwnd: None,
            target_process_id: None,
            result_action: "show".to_string(),
            interactive: false,
            screenshot_path: None,
            presentation: "compact".to_string(),
            delivery: "return".to_string(),
            reply_pipe: None,
            focus_generation: None,
            replace_selection: false,
            cancel_request_id: None,
            query_request_id: None,
            chunk_index: 0,
            callback_ack: true,
            session: String::new(),
            generation: {
                static GENERATION: SessionCounter = SessionCounter::new(1);
                GENERATION.fetch_add(1, SessionOrdering::Relaxed)
            },
            safety: InputSafety::default(),
        }
    }

    pub fn cancellation(request_id: impl Into<String>) -> Self {
        let mut request = Self::new("", "kaixin-ime-cancel");
        request.action = "cancel".to_string();
        request.cancel_request_id = Some(request_id.into());
        request.presentation = "background".to_string();
        request
    }
}

fn new_request_id() -> String {
    #[cfg(windows)]
    {
        crate::windows_security::generate_capability_token()
            .expect("Windows cryptographic random generator unavailable")
    }
    #[cfg(not(windows))]
    {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or_default();
        let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("{:x}-{:x}-{:x}", std::process::id(), nanos, counter)
    }
}

pub fn fresh_request_id() -> String {
    new_request_id()
}

pub fn target_language_for_text(text: &str) -> &'static str {
    if text.chars().any(|ch| {
        matches!(ch,
            '\u{3400}'..='\u{4dbf}' |
            '\u{4e00}'..='\u{9fff}' |
            '\u{f900}'..='\u{faff}')
    }) {
        "en"
    } else {
        "zh"
    }
}

#[cfg(windows)]
pub fn process_id_for_window(hwnd: isize) -> Option<u32> {
    if hwnd == 0 {
        return None;
    }
    let mut process_id = 0u32;
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            hwnd,
            &mut process_id,
        );
    }
    (process_id != 0).then_some(process_id)
}

#[cfg(not(windows))]
pub fn process_id_for_window(_hwnd: isize) -> Option<u32> {
    None
}

pub fn translator_path() -> Option<PathBuf> {
    if let Some(path) = configured_translator_path() {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = std::env::var_os("HYMT_TRANSLATOR_EXE").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = std::env::var_os("WINTRANSLATOR_EXE").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Some(path) = registered_hymt_path() {
        candidates.push(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(HYMT_TRANSLATOR_EXE));
            candidates.push(dir.join(WINTRANSLATOR_EXE));
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        candidates.push(dir.join(HYMT_TRANSLATOR_EXE));
        candidates.push(dir.join(WINTRANSLATOR_EXE));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local)
                .join("Programs")
                .join("Hy-MT2")
                .join(HYMT_TRANSLATOR_EXE),
        );
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("WinTranslator")
                .join(WINTRANSLATOR_EXE),
        );
    }
    // Development builds are discovered only through an explicit environment
    // variable. Do not scan drive-specific or repository-relative locations.
    if let Some(root) = std::env::var_os("WINTRANSLATOR_ROOT") {
        let root = PathBuf::from(root);
        candidates.extend([
            root.join(WINTRANSLATOR_EXE),
            root.join("artifacts")
                .join("publish")
                .join("WinTranslator")
                .join(WINTRANSLATOR_EXE),
            root.join("src")
                .join("WinTranslator")
                .join("bin")
                .join("Release")
                .join("net10.0-windows")
                .join("win-x64")
                .join(WINTRANSLATOR_EXE),
        ]);
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(variable) {
            candidates.push(
                PathBuf::from(root)
                    .join("WinTranslator")
                    .join(WINTRANSLATOR_EXE),
            );
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(windows)]
fn registered_hymt_path() -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key: Vec<u16> =
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\hy-mt2-desktop.exe\0"
            .encode_utf16()
            .collect();
    let mut value = [0u16; 32768];
    let mut size = (value.len() * 2) as u32;
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            std::ptr::null(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut size,
        )
    } != 0
    {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &value[..value.iter().position(|v| *v == 0)?],
    )))
}
#[cfg(not(windows))]
fn registered_hymt_path() -> Option<PathBuf> {
    None
}

fn is_hymt_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| !name.eq_ignore_ascii_case(WINTRANSLATOR_EXE))
}

fn request_pipe_path() -> &'static str {
    #[cfg(windows)]
    if pipe_path_available(HYMT_PIPE_PATH) {
        return HYMT_PIPE_PATH;
    }
    if let Some(path) = translator_path() {
        return if is_hymt_path(&path) {
            HYMT_PIPE_PATH
        } else {
            PIPE_PATH
        };
    }
    #[cfg(windows)]
    if pipe_path_available(HYMT_PIPE_PATH) {
        return HYMT_PIPE_PATH;
    }
    PIPE_PATH
}

pub fn configured_translator_path() -> Option<PathBuf> {
    read_tools_value("wintranslator_path")
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

pub fn is_available() -> bool {
    translator_path().is_some() || pipe_available()
}

pub fn availability_message() -> String {
    if let Some(path) = translator_path() {
        format!("已找到独立翻译软件：{}", path.display())
    } else if pipe_available() {
        "翻译服务正在运行，命名管道联动可用。".to_string()
    } else {
        "未找到翻译服务。请安装 HY-MT2 翻译机，或在设置中选择 hy-mt2-desktop.exe / WinTranslator.exe。".to_string()
    }
}

pub fn test_connection() -> Result<String, String> {
    let mut request = ExternalTranslationRequest::new("", "settings-test");
    request.action = "capabilities".to_string();
    request.presentation = "background".to_string();
    let response = send_request_for_response(&request)?;
    let capabilities = response.capabilities.ok_or_else(|| {
        "WinTranslator 未返回 capabilities；请升级到支持协议 v2 的版本。".to_string()
    })?;
    if !capabilities.protocol_versions.contains(&PROTOCOL_VERSION)
        || !capabilities
            .presentations
            .iter()
            .any(|value| value == "background")
        || !capabilities
            .deliveries
            .iter()
            .any(|value| value == "return")
        || !capabilities
            .callback_events
            .iter()
            .any(|value| value == "completed")
        || !capabilities.actions.iter().any(|value| value == "cancel")
    {
        return Err("WinTranslator 缺少后台返回或取消能力，请升级后再联动。".to_string());
    }
    Ok(format!(
        "翻译服务联动成功：协议 v{}，支持后台返回、进度回调与取消。",
        response.protocol_version.max(PROTOCOL_VERSION)
    ) + &format!(
        " 提供方：{}；模型：{}；引擎：{}；排队：{}。",
        response.provider,
        if response.model_ready == Some(false) {
            "缺失或不完整"
        } else {
            "可用"
        },
        if response.engine_ready == Some(false) {
            "运行时未安装"
        } else {
            "可用，首次翻译可能需要加载"
        },
        response.queued
    ))
}

#[cfg(windows)]
fn pipe_available() -> bool {
    pipe_path_available(request_pipe_path())
}

#[cfg(windows)]
fn pipe_path_available(name: &str) -> bool {
    let mut path = name.encode_utf16().collect::<Vec<_>>();
    path.push(0);
    unsafe { WaitNamedPipeW(path.as_ptr(), 0) != 0 }
}

#[cfg(not(windows))]
fn pipe_available() -> bool {
    false
}

pub fn preferred_result_action() -> String {
    let value = read_tools_value("translate_result_action").map(|value| value.to_ascii_lowercase());
    match value.as_deref() {
        Some("copy" | "auto_copy") => "copy",
        Some("paste" | "auto_paste") => "paste",
        _ => "show",
    }
    .to_string()
}

fn read_tools_value(expected_key: &str) -> Option<String> {
    let path = app_paths::config_ini_path().unwrap_or_else(|| {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(app_paths::CONFIG_FILE_NAME)
    });
    std::fs::read_to_string(path).ok().and_then(|text| {
        let mut in_tools = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                in_tools = line[1..line.len() - 1].eq_ignore_ascii_case("tools");
            } else if in_tools {
                if let Some((key, value)) = line.split_once('=') {
                    if key.trim().eq_ignore_ascii_case(expected_key) {
                        return Some(value.trim().to_string());
                    }
                }
            }
        }
        None
    })
}

pub fn open_translator() -> Result<(), String> {
    let mut request = ExternalTranslationRequest::new("", "kaixin-ime");
    request.presentation = "full".to_string();
    if send_request_once(&request).is_ok() {
        return Ok(());
    }
    let path = translator_path()
        .ok_or_else(|| "未找到翻译程序；请安装 HY-MT2 或在翻译设置中选择程序路径。".to_string())?;
    spawn_translator(&path)
}

pub fn send_request(request: &ExternalTranslationRequest) -> Result<(), String> {
    if !matches!(
        request.action.as_str(),
        "translate" | "ping" | "capabilities" | "cancel" | "status" | "ack" | "present"
    ) {
        return Err("不支持的翻译请求动作".to_string());
    }
    if request.text.len() > 1024 * 1024 {
        return Err("翻译文本超过 1 MiB 协议限制".to_string());
    }
    let started = Instant::now();
    match send_request_once(request) {
        Ok(()) => {
            log_request(request, "accepted", started.elapsed(), None);
            return Ok(());
        }
        Err(error) if error.starts_with("翻译服务拒绝：") => return Err(error),
        Err(error) => log_request(request, "initial_failed", started.elapsed(), Some(&error)),
    }

    let path = translator_path()
        .ok_or_else(|| "未找到翻译程序；请安装 HY-MT2 或在翻译设置中选择程序路径。".to_string())?;
    // Never place translation text in the external application's plaintext
    // request-file fallback. Start it, then retry its pipe until the listener
    // becomes ready.
    spawn_translator(&path)?;
    let started = Instant::now();
    while started.elapsed() < CONNECT_TIMEOUT {
        thread::sleep(Duration::from_millis(120));
        if send_request_once(request).is_ok() {
            log_request(request, "accepted_after_start", started.elapsed(), None);
            return Ok(());
        }
    }
    let error =
        "翻译服务已启动，但命名管道未就绪；为避免明文落盘，已禁用请求文件回退。".to_string();
    log_request(request, "failed", started.elapsed(), Some(&error));
    Err(error)
}

/// Send a request to WinTranslator while keeping the caller responsive.
///
/// Queue work in WinTranslator without blocking an input-method tool window.
/// Translation, presentation, and configured result actions all happen in the
/// translation application's own process.
pub fn launch_full_request(request: &ExternalTranslationRequest) -> Result<(), String> {
    if !is_available() {
        return Err(availability_message());
    }
    let mut request = request.clone();
    request.safety = input_safety();
    request.session = format!(
        "{}-{}-{}",
        std::process::id(),
        request.origin,
        request.target_hwnd.unwrap_or(0)
    );
    if matches!(request.delivery.as_str(), "copy" | "paste") {
        request.presentation = "background".to_string();
        request.delivery = "return".to_string();
        request.reply_pipe = Some(format!("Kaixin.Translate.Result.{}", request.request_id));
        return launch_callback_request(request);
    }
    thread::Builder::new()
        .name("kaixin-wintranslator-request".to_string())
        .spawn(move || {
            if let Err(error) = send_request(&request) {
                runtime_log::log_tray(
                    runtime_log::RuntimeLogLevel::Error,
                    "external_translate_request_async_failed",
                    format!(
                        "request_id={} origin={} error={}",
                        request.request_id, request.origin, error
                    ),
                );
            }
        })
        .map(|_| ())
        .map_err(|error| format!("无法创建翻译联动线程：{error}"))
}

#[cfg(windows)]
fn launch_callback_request(request: ExternalTranslationRequest) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut active = sessions().lock().unwrap();
        if active.len() >= 8 && !active.contains_key(&request.session) {
            return Err("后台翻译最多同时监听 8 个会话".into());
        }
        if let Some(previous) = active.insert(
            request.session.clone(),
            LocalSession {
                id: request.request_id.clone(),
                stop: stop.clone(),
            },
        ) {
            previous.stop.store(true, SessionOrdering::Release);
        }
    }
    let guard = SessionGuard {
        key: request.session.clone(),
        id: request.request_id.clone(),
    };
    thread::Builder::new()
        .name("kaixin-wintranslator-session".to_string())
        .spawn(move || {
            let outcome = (|| {
                let reply_pipe = request
                    .reply_pipe
                    .clone()
                    .ok_or_else(|| "后台翻译缺少回调管道".to_string())?;
                let expected_request = request.clone();
                let listener_stop = stop.clone();
                let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
                thread::Builder::new()
                    .name("kaixin-wintranslator-callback".to_string())
                    .spawn(move || {
                        let _guard = guard;
                        listen_for_translation_callbacks_with_stop(
                            &reply_pipe,
                            expected_request,
                            ready_sender,
                            |_| {},
                            listener_stop,
                        )
                    })
                    .map_err(|error| format!("无法创建翻译结果监听线程：{error}"))?;
                ready_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| "创建翻译结果管道超时".to_string())??;
                send_request(&request)
            })();
            if let Err(error) = outcome {
                stop.store(true, SessionOrdering::Release);
                show_translation_error(&error);
                runtime_log::log_tray(
                    runtime_log::RuntimeLogLevel::Error,
                    "external_translate_session_failed",
                    format!("request_id={} error={error}", request.request_id),
                );
            }
        })
        .map(|_| ())
        .map_err(|error| format!("无法创建翻译会话线程：{error}"))
}

#[cfg(not(windows))]
fn launch_callback_request(_request: ExternalTranslationRequest) -> Result<(), String> {
    Err("当前平台不支持 WinTranslator 回调管道".to_string())
}

#[cfg(all(windows, test))]
fn listen_for_translation_callbacks(
    reply_pipe: &str,
    request: ExternalTranslationRequest,
    ready_sender: mpsc::SyncSender<Result<(), String>>,
    observe: impl Fn(&TranslationCallbackEvent),
) {
    listen_for_translation_callbacks_with_stop(
        reply_pipe,
        request,
        ready_sender,
        observe,
        Arc::new(AtomicBool::new(false)),
    )
}
#[cfg(windows)]
fn listen_for_translation_callbacks_with_stop(
    reply_pipe: &str,
    request: ExternalTranslationRequest,
    ready_sender: mpsc::SyncSender<Result<(), String>>,
    observe: impl Fn(&TranslationCallbackEvent),
    stop: Arc<AtomicBool>,
) {
    let mut pieces: Vec<Option<String>> = Vec::new();
    let security = match crate::windows_security::LocalLogonPipeSecurity::new() {
        Ok(security) => security,
        Err(error) => {
            let _ = ready_sender.send(Err(format!("创建翻译回调管道安全描述符失败：{error}")));
            return;
        }
    };
    let full_pipe_name = format!(r"\\.\pipe\{reply_pipe}");
    let pipe_name = full_pipe_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut first = true;
    let deadline = Instant::now() + Duration::from_secs(31 * 60);
    loop {
        if stop.load(SessionOrdering::Acquire) || Instant::now() >= deadline {
            let _ = send_request_once(&ExternalTranslationRequest::cancellation(
                &request.request_id,
            ));
            return;
        }
        let handle = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                64 * 1024,
                3_000,
                security.as_ptr(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            if first {
                let _ = ready_sender.send(Err(format!(
                    "创建翻译结果管道失败：{}",
                    std::io::Error::last_os_error()
                )));
            }
            return;
        }
        // SAFETY: CreateNamedPipeW returned a new pipe handle for this loop.
        let handle = unsafe { OwnedWinHandle::from_raw(handle) }.expect("validated pipe handle");
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let started = unsafe { ConnectNamedPipe(handle.as_raw(), &mut overlapped) };
        let connect_error = unsafe { GetLastError() };
        if first {
            let _ = ready_sender.send(Ok(()));
            first = false;
        }
        let connected = started != 0
            || connect_error == ERROR_PIPE_CONNECTED
            || (connect_error == ERROR_IO_PENDING
                && finish_overlapped_with_timeout(
                    handle.as_raw(),
                    &mut overlapped,
                    1,
                    "回调连接",
                    2_000,
                )
                .is_ok());
        let event = if connected {
            read_callback_event(handle.as_raw())
                .and_then(|bytes| serde_json::from_slice::<TranslationCallbackEvent>(&bytes).ok())
        } else {
            let mut query = ExternalTranslationRequest::new("", "kaixin-status");
            query.action = "status".into();
            query.query_request_id = Some(request.request_id.clone());
            query.chunk_index = pieces
                .iter()
                .position(Option::is_none)
                .unwrap_or(pieces.len().saturating_sub(1));
            send_request_once_for_response(&query)
                .ok()
                .and_then(|response| response.result)
        };
        unsafe { DisconnectNamedPipe(handle.as_raw()) };
        let Some(mut event) = event else { continue };
        if event.request_id != request.request_id {
            continue;
        }
        if stop.load(SessionOrdering::Acquire) {
            continue;
        }
        if event.event_name == "completed" {
            if event.chunk_count == 0
                || event.chunk_count > 200
                || event.chunk_index >= event.chunk_count
            {
                continue;
            }
            if pieces.len() != event.chunk_count {
                pieces = vec![None; event.chunk_count];
            }
            pieces[event.chunk_index] = event.text.take();
            if pieces
                .iter()
                .filter_map(|s| s.as_ref())
                .map(String::len)
                .sum::<usize>()
                > 4 * 1024 * 1024
            {
                show_translation_error("译文超出安全大小限制");
                return;
            }
            if pieces.iter().any(Option::is_none) {
                continue;
            }
            event.text = Some(pieces.iter().filter_map(|s| s.as_deref()).collect());
        }
        observe(&event);
        let terminal = matches!(
            event.event_name.as_str(),
            "completed" | "failed" | "cancelled"
        );
        if event.event_name == "completed" {
            if let Some(text) = event.text.as_deref() {
                apply_returned_translation(&request, &event, text);
            }
        } else if event.event_name == "failed" {
            present_result(&request.request_id);
            runtime_log::log_tray(
                runtime_log::RuntimeLogLevel::Error,
                "external_translate_callback_failed",
                format!(
                    "request_id={} code={} message={}",
                    request.request_id,
                    event.error_code.as_deref().unwrap_or("translation_failed"),
                    event.message.as_deref().unwrap_or("none")
                ),
            );
        }
        if terminal {
            let mut ack = ExternalTranslationRequest::new("", "kaixin-ack");
            ack.action = "ack".into();
            ack.query_request_id = Some(request.request_id.clone());
            let _ = send_request_once(&ack);
            return;
        }
    }
}

#[cfg(windows)]
fn read_callback_event(handle: HANDLE) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(4096);
    let mut buffer = [0u8; 4096];
    loop {
        let read = read_pipe_with_timeout(handle, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read as usize];
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            result.extend_from_slice(&bytes[..newline]);
            if result.len() > 1024 * 1024 {
                return None;
            }
            break;
        }
        result.extend_from_slice(bytes);
        if result.len() > 1024 * 1024 {
            return None;
        }
    }
    Some(result)
}

#[cfg(windows)]
fn apply_returned_translation(
    request: &ExternalTranslationRequest,
    event: &TranslationCallbackEvent,
    text: &str,
) {
    if !matches!(request.result_action.as_str(), "copy" | "paste") {
        return;
    }
    if event.target_hwnd != request.target_hwnd
        || event.target_process_id != request.target_process_id
        || event.focus_generation != request.focus_generation
    {
        return;
    }
    // Serialize committing against replacing/cancelling this local session.
    let active = sessions().lock().unwrap();
    if !request.session.is_empty()
        && active.get(&request.session).is_none_or(|session| {
            session.id != request.request_id || session.stop.load(SessionOrdering::Acquire)
        })
    {
        return;
    }
    let current = input_safety();
    let safe_window = request.target_hwnd.is_some_and(|hwnd|
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() } == hwnd
            && process_id_for_window(hwnd) == request.target_process_id);
    if !safe_to_apply(
        &request.safety,
        &current,
        request.result_action == "paste",
        safe_window,
    ) {
        drop(active);
        present_result(&request.request_id);
        return;
    }
    if clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text).is_err() {
        present_result(&request.request_id);
        return;
    }
    if request.result_action == "paste" {
        if let Some(hwnd) = request.target_hwnd {
            if crate::win_paste::send_ctrl_v_to_target(hwnd).is_err() {
                present_result(&request.request_id);
            }
        }
    }
}
fn safe_to_apply(before: &InputSafety, now: &InputSafety, paste: bool, same_window: bool) -> bool {
    before.clipboard == now.clipboard
        && (!paste
            || (same_window
                && before.input == now.input
                && before.focus != 0
                && before.focus == now.focus
                && before.caret != 0
                && before.caret == now.caret
                && before.rect == now.rect))
}
fn present_result(id: &str) {
    let mut request = ExternalTranslationRequest::new("", "kaixin-present");
    request.action = "present".into();
    request.query_request_id = Some(id.to_owned());
    let _ = send_request_once(&request);
}
#[cfg(windows)]
fn show_translation_error(error: &str) {
    let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "开心输入法 · 翻译失败"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
            0,
            message.as_ptr(),
            title.as_ptr(),
            0x10,
        );
    }
}

fn log_request(
    request: &ExternalTranslationRequest,
    status: &str,
    elapsed: Duration,
    error: Option<&str>,
) {
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(request.text.as_bytes()));
    runtime_log::log_tray(
        if error.is_some() {
            runtime_log::RuntimeLogLevel::Error
        } else {
            runtime_log::RuntimeLogLevel::Basic
        },
        "external_translate_request",
        format!(
            "request_id={} origin={} status={} chars={} text_sha256={} elapsed_ms={} error={}",
            request.request_id,
            request.origin,
            status,
            request.text.chars().count(),
            &hash[..16],
            elapsed.as_millis(),
            error.unwrap_or("none")
        ),
    );
}

fn spawn_translator(path: &Path) -> Result<(), String> {
    let mut command = Command::new(path);
    if is_hymt_path(path) {
        command.arg("--ime-background");
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法启动 WinTranslator：{error}；{}", path.display()))
}

#[cfg(windows)]
fn send_request_once(request: &ExternalTranslationRequest) -> Result<(), String> {
    send_request_once_for_response(request).map(|_| ())
}

#[cfg(windows)]
fn send_request_once_for_response(
    request: &ExternalTranslationRequest,
) -> Result<ExternalTranslationResponse, String> {
    let mut payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    payload.push(b'\n');
    let mut pipe_name = request_pipe_path().encode_utf16().collect::<Vec<_>>();
    pipe_name.push(0);
    if unsafe { WaitNamedPipeW(pipe_name.as_ptr(), PIPE_PHASE_TIMEOUT_MS) } == 0 {
        return Err(format!(
            "等待 WinTranslator 管道超时或失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let raw_handle = unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            0,
        )
    };
    if raw_handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "连接 WinTranslator 管道失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: CreateFileW returned a new pipe handle owned by this request.
    let handle = unsafe { OwnedWinHandle::from_raw(raw_handle) }
        .map_err(|err| format!("接管 WinTranslator 管道句柄失败：{err}"))?;
    write_pipe_with_timeout(handle.as_raw(), &payload)?;
    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if Instant::now() >= deadline {
            return Err("翻译服务响应超时".into());
        }
        let count = read_pipe_with_timeout(handle.as_raw(), &mut buffer)? as usize;
        if count == 0 {
            return Err("翻译服务响应不完整".into());
        }
        let end = buffer[..count].iter().position(|b| *b == b'\n');
        response.extend_from_slice(&buffer[..end.unwrap_or(count)]);
        if response.len() > 1024 * 1024 {
            return Err("翻译服务响应过大".into());
        }
        if end.is_some() {
            break;
        }
    }
    parse_response(request, &response)
}

#[cfg(windows)]
fn write_pipe_with_timeout(handle: HANDLE, payload: &[u8]) -> Result<(), String> {
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let started = unsafe {
        WriteFile(
            handle,
            payload.as_ptr(),
            payload.len() as u32,
            std::ptr::null_mut(),
            &mut overlapped,
        )
    };
    let written = finish_overlapped(handle, &mut overlapped, started, "写入")?;
    if written as usize != payload.len() {
        return Err("WinTranslator 管道写入不完整".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn read_pipe_with_timeout(handle: HANDLE, response: &mut [u8]) -> Result<u32, String> {
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let started = unsafe {
        ReadFile(
            handle,
            response.as_mut_ptr(),
            response.len() as u32,
            std::ptr::null_mut(),
            &mut overlapped,
        )
    };
    finish_overlapped(handle, &mut overlapped, started, "读取")
}

#[cfg(windows)]
fn finish_overlapped(
    handle: HANDLE,
    overlapped: &mut OVERLAPPED,
    started: i32,
    phase: &str,
) -> Result<u32, String> {
    finish_overlapped_with_timeout(handle, overlapped, started, phase, PIPE_PHASE_TIMEOUT_MS)
}

#[cfg(windows)]
fn finish_overlapped_with_timeout(
    handle: HANDLE,
    overlapped: &mut OVERLAPPED,
    started: i32,
    phase: &str,
    timeout_ms: u32,
) -> Result<u32, String> {
    if started == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(format!(
                "WinTranslator 管道{phase}失败：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
    }
    let mut transferred = 0u32;
    let completed =
        unsafe { GetOverlappedResultEx(handle, overlapped, &mut transferred, timeout_ms, 0) };
    if completed != 0 {
        return Ok(transferred);
    }
    let error = unsafe { GetLastError() };
    if error == WAIT_TIMEOUT {
        unsafe {
            CancelIoEx(handle, overlapped);
            // Wait for cancellation before the stack OVERLAPPED and buffers are released.
            GetOverlappedResult(handle, overlapped, &mut transferred, 1);
        }
        return Err(format!("WinTranslator 无响应（管道{phase}超时）"));
    }
    Err(format!(
        "WinTranslator 管道{phase}失败：{}",
        std::io::Error::from_raw_os_error(error as i32)
    ))
}

#[cfg(not(windows))]
fn send_request_once(request: &ExternalTranslationRequest) -> Result<(), String> {
    let mut pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_PATH)
        .map_err(|error| error.to_string())?;
    let mut payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    payload.push(b'\n');
    pipe.write_all(&payload)
        .map_err(|error| error.to_string())?;
    pipe.flush().map_err(|error| error.to_string())?;

    let mut response = String::new();
    BufReader::new(pipe)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    parse_response(request, response.as_bytes()).map(|_| ())
}

fn send_request_for_response(
    request: &ExternalTranslationRequest,
) -> Result<ExternalTranslationResponse, String> {
    #[cfg(windows)]
    {
        if let Ok(response) = send_request_once_for_response(request) {
            return Ok(response);
        }
        let path = translator_path().ok_or_else(availability_message)?;
        spawn_translator(&path)?;
        let started = Instant::now();
        while started.elapsed() < CONNECT_TIMEOUT {
            thread::sleep(Duration::from_millis(120));
            if let Ok(response) = send_request_once_for_response(request) {
                return Ok(response);
            }
        }
        Err("翻译服务已启动，但连接超时；请检查模型和程序状态。".into())
    }
    #[cfg(not(windows))]
    {
        Err("当前平台不支持 WinTranslator 命名管道".to_string())
    }
}

fn parse_response(
    request: &ExternalTranslationRequest,
    response: &[u8],
) -> Result<ExternalTranslationResponse, String> {
    let response: ExternalTranslationResponse = serde_json::from_slice(response)
        .map_err(|error| format!("WinTranslator 返回无效响应：{error}"))?;
    if !response.request_id.is_empty() && response.request_id != request.request_id {
        return Err(
            "WinTranslator 响应协议不兼容（request_id 不匹配），请升级 WinTranslator。".to_string(),
        );
    }
    if response.ok {
        Ok(response)
    } else if response.error.is_empty() {
        Err("WinTranslator 拒绝了请求".to_string())
    } else {
        Err(format!("翻译服务拒绝：{}", response.error))
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    fn before() -> InputSafety {
        InputSafety {
            clipboard: 3,
            input: 5,
            focus: 9,
            caret: 10,
            rect: [0, 1, 2, 3],
        }
    }
    #[test]
    fn changed_clipboard_never_overwritten() {
        let before = before();
        let mut now = before.clone();
        now.clipboard += 1;
        assert!(!safe_to_apply(&before, &now, false, true));
        assert!(!safe_to_apply(&before, &now, true, true));
    }
    #[test]
    fn same_window_changes_require_manual_insertion() {
        let before = before();
        for change in 0..4 {
            let mut now = before.clone();
            match change {
                0 => now.input += 1,
                1 => now.focus += 1,
                2 => now.caret += 1,
                _ => now.rect[0] += 1,
            }
            assert!(!safe_to_apply(&before, &now, true, true));
        }
        assert!(!safe_to_apply(&before, &before, true, false));
        let mut unknown = before.clone();
        unknown.caret = 0;
        assert!(!safe_to_apply(&unknown, &unknown, true, true));
        assert!(safe_to_apply(&before, &before, true, true));
    }
    #[test]
    fn parses_chunked_results_and_readiness() {
        let request = ExternalTranslationRequest::new("", "test");
        let payload = serde_json::json!({"ok": true, "request_id": request.request_id,
            "model_ready": false, "engine_ready": true, "provider": "hy-mt2", "queued": 2,
            "result": {"request_id": request.request_id, "event": "completed", "text": "译文", "chunk_index": 1, "chunk_count": 3}});
        let response = parse_response(&request, &serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(response.model_ready, Some(false));
        assert_eq!(response.queued, 2);
        let event = response.result.unwrap();
        assert_eq!(event.chunk_count, 3);
        assert_eq!(event.chunk_index, 1);
    }
}

#[cfg(all(test, windows))]
mod live_tests {
    use super::*;

    #[test]
    fn production_listener_reassembles_large_out_of_order_callbacks() {
        let mut request = ExternalTranslationRequest::new("test", "callback-unit");
        request.result_action = "show".into();
        request.reply_pipe = Some(format!("Kaixin.Translate.Result.{}", request.request_id));
        let expected = request.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (events_tx, events_rx) = mpsc::channel();
        let listener = thread::spawn(move || {
            listen_for_translation_callbacks(
                expected.reply_pipe.as_ref().unwrap(),
                expected.clone(),
                ready_tx,
                |event| {
                    if event.event_name == "completed" {
                        let _ = events_tx.send(event.text.clone().unwrap());
                    }
                },
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        let text = "中文🙂\n\"\\".repeat(90_000);
        let characters: Vec<char> = text.chars().collect();
        let chunks: Vec<String> = characters
            .chunks(24_000)
            .map(|part| part.iter().collect())
            .collect();
        assert!(text.len() > 1024 * 1024);
        let path: Vec<u16> = format!(r"\\.\pipe\{}", request.reply_pipe.as_deref().unwrap())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        for index in (0..chunks.len()).rev() {
            let deadline = Instant::now() + Duration::from_secs(3);
            let handle = loop {
                unsafe {
                    WaitNamedPipeW(path.as_ptr(), 100);
                }
                let raw = unsafe {
                    CreateFileW(
                        path.as_ptr(),
                        GENERIC_WRITE,
                        0,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OVERLAPPED,
                        0,
                    )
                };
                if raw != INVALID_HANDLE_VALUE {
                    break unsafe { OwnedWinHandle::from_raw(raw) }.unwrap();
                }
                assert!(Instant::now() < deadline, "callback pipe did not reconnect");
                thread::sleep(Duration::from_millis(10));
            };
            let mut bytes = serde_json::to_vec(&serde_json::json!({"request_id": request.request_id,
                "event": "completed", "text": chunks[index], "chunk_index": index, "chunk_count": chunks.len()})).unwrap();
            bytes.push(b'\n');
            write_pipe_with_timeout(handle.as_raw(), &bytes).unwrap();
        }
        assert_eq!(
            events_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            text
        );
        listener.join().unwrap();
    }

    #[test]
    #[ignore = "requires a running HY-MT2 desktop and HYMT_TRANSLATOR_EXE"]
    fn actual_ime_client_receives_local_model_callbacks() {
        assert_eq!(request_pipe_path(), HYMT_PIPE_PATH);
        assert!(test_connection().unwrap().contains("v2"));
        let mut request = ExternalTranslationRequest::new("你好，开心输入法。", "kaixin-ime-test");
        request.presentation = "background".into();
        // Observe the production pipe listener without changing the clipboard.
        request.result_action = "show".into();
        request.reply_pipe = Some(format!("Kaixin.Translate.Result.{}", request.request_id));
        let expected = request.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (events_tx, events_rx) = mpsc::channel();
        thread::spawn(move || {
            listen_for_translation_callbacks(
                expected.reply_pipe.as_ref().unwrap(),
                expected.clone(),
                ready_tx,
                |event| {
                    let _ = events_tx.send((event.event_name.clone(), event.text.clone()));
                },
            );
        });
        ready_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        send_request(&request).unwrap();
        let mut names = Vec::new();
        loop {
            let (name, text) = events_rx.recv_timeout(Duration::from_secs(30)).unwrap();
            names.push(name.clone());
            match name.as_str() {
                "completed" => {
                    assert!(!text.unwrap().trim().is_empty());
                    break;
                }
                "failed" | "cancelled" => panic!("unexpected translation terminal event: {name}"),
                _ => {}
            }
        }
        assert!(names.iter().any(|event| event == "started"));
        assert!(names.iter().any(|event| event == "progress"));
    }
}
