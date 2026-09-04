#[cfg(windows)]
use crate::win_handle::OwnedWinHandle;
use crate::{app_paths, runtime_log};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(windows))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::mpsc;
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
            target: "auto-opposite".to_string(),
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
    if let Some(path) = std::env::var_os("WINTRANSLATOR_EXE").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(WINTRANSLATOR_EXE));
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        candidates.push(dir.join(WINTRANSLATOR_EXE));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
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
        "WinTranslator 正在运行，命名管道联动可用。".to_string()
    } else {
        "未找到 WinTranslator。请在设置页选择 WinTranslator.exe，或安装到默认位置。".to_string()
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
        "WinTranslator 联动成功：协议 v{}，支持后台返回、进度回调与取消。",
        response.protocol_version.max(PROTOCOL_VERSION)
    ))
}

#[cfg(windows)]
fn pipe_available() -> bool {
    let mut path = PIPE_PATH.encode_utf16().collect::<Vec<_>>();
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
    let path = translator_path().ok_or_else(|| {
        "未找到 WinTranslator.exe；请在设置页选择程序路径或安装独立翻译软件。".to_string()
    })?;
    spawn_translator(&path)
}

pub fn send_request(request: &ExternalTranslationRequest) -> Result<(), String> {
    if !matches!(
        request.action.as_str(),
        "translate" | "ping" | "capabilities" | "cancel"
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
        Err(error) => log_request(request, "initial_failed", started.elapsed(), Some(&error)),
    }

    let path = translator_path().ok_or_else(|| {
        "未找到 WinTranslator.exe；请在设置页选择程序路径或安装独立翻译软件。".to_string()
    })?;
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
        "WinTranslator 已启动，但命名管道未就绪；为避免明文落盘，已禁用请求文件回退。".to_string();
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
    thread::Builder::new()
        .name("kaixin-wintranslator-session".to_string())
        .spawn(move || {
            let outcome = (|| {
                let reply_pipe = request
                    .reply_pipe
                    .clone()
                    .ok_or_else(|| "后台翻译缺少回调管道".to_string())?;
                let expected_request = request.clone();
                let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
                thread::Builder::new()
                    .name("kaixin-wintranslator-callback".to_string())
                    .spawn(move || {
                        listen_for_translation_callbacks(
                            &reply_pipe,
                            expected_request,
                            ready_sender,
                        )
                    })
                    .map_err(|error| format!("无法创建翻译结果监听线程：{error}"))?;
                ready_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| "创建翻译结果管道超时".to_string())??;
                send_request(&request)
            })();
            if let Err(error) = outcome {
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

#[cfg(windows)]
fn listen_for_translation_callbacks(
    reply_pipe: &str,
    request: ExternalTranslationRequest,
    ready_sender: mpsc::SyncSender<Result<(), String>>,
) {
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
    loop {
        let handle = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_INBOUND,
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
        if first {
            let _ = ready_sender.send(Ok(()));
            first = false;
        }
        let connected = unsafe { ConnectNamedPipe(handle.as_raw(), std::ptr::null_mut()) } != 0
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            return;
        }
        let event = read_callback_event(handle.as_raw())
            .and_then(|bytes| serde_json::from_slice::<TranslationCallbackEvent>(&bytes).ok());
        unsafe { DisconnectNamedPipe(handle.as_raw()) };
        let Some(event) = event else { continue };
        if event.request_id != request.request_id {
            continue;
        }
        let terminal = matches!(
            event.event_name.as_str(),
            "completed" | "failed" | "cancelled"
        );
        if event.event_name == "completed" {
            if let Some(text) = event.text.as_deref() {
                apply_returned_translation(&request, &event, text);
            }
        } else if event.event_name == "failed" {
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
            return;
        }
    }
}

#[cfg(windows)]
fn read_callback_event(handle: HANDLE) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(4096);
    let mut buffer = [0u8; 4096];
    loop {
        let mut read = 0u32;
        if unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return None;
        }
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read as usize];
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            result.extend_from_slice(&bytes[..newline]);
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
    if event.target_hwnd != request.target_hwnd
        || event.target_process_id != request.target_process_id
        || event.focus_generation != request.focus_generation
    {
        return;
    }
    if clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text).is_err() {
        return;
    }
    if request.result_action == "paste" {
        if let Some(hwnd) = request.target_hwnd {
            if process_id_for_window(hwnd) == request.target_process_id {
                let _ = crate::win_paste::send_ctrl_v_to_target(hwnd);
            }
        }
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
    let mut pipe_name = PIPE_PATH.encode_utf16().collect::<Vec<_>>();
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
    let mut response = vec![0u8; 4096];
    let bytes_read = read_pipe_with_timeout(handle.as_raw(), &mut response)?;
    parse_response(request, &response[..bytes_read as usize])
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
    let completed = unsafe {
        GetOverlappedResultEx(
            handle,
            overlapped,
            &mut transferred,
            PIPE_PHASE_TIMEOUT_MS,
            0,
        )
    };
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
        send_request_once_for_response(request)
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
        Err(response.error)
    }
}
