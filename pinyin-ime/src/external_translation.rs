use crate::{app_paths, runtime_log};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(windows))]
use std::fs::OpenOptions;
#[cfg(not(windows))]
use std::io::{BufRead, BufReader, Write};

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
#[cfg(windows)]
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};

pub const PIPE_PATH: &str = r"\\.\pipe\WinTranslator.Request";
const COMPACT_REQUEST_MAGIC: &[u8] = b"KXTRANS-DPAPI-1\n";
pub const WINTRANSLATOR_EXE: &str = "WinTranslator.exe";
const CREATE_NO_WINDOW: u32 = 0x08000000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const PIPE_PHASE_TIMEOUT_MS: u32 = 2_500;
pub const PROTOCOL_VERSION: u32 = 2;
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

#[derive(Clone, Debug, Deserialize)]
pub struct ExternalTranslationEvent {
    pub protocol_version: u32,
    pub event: String,
    pub request_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub message: String,
    pub target_hwnd: Option<isize>,
    pub target_process_id: Option<u32>,
    pub focus_generation: Option<u64>,
    #[serde(default)]
    pub replace_selection: bool,
}

#[derive(Debug, Deserialize)]
struct ExternalTranslationResponse {
    ok: bool,
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    error: String,
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
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{:x}-{:x}", std::process::id(), nanos, counter)
}

pub fn fresh_request_id() -> String {
    new_request_id()
}

pub fn new_reply_pipe_name() -> String {
    format!("Kaixin.Translate.Result.{}", new_request_id())
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
    send_request(&request)?;
    Ok(format!(
        "WinTranslator 联动成功：命名管道可用，协议版本 v{PROTOCOL_VERSION}。"
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

pub fn write_compact_request_file(request: &ExternalTranslationRequest) -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = local.join("kaixin").join("translation-requests");
    fs::create_dir_all(&directory).map_err(|error| format!("创建翻译请求目录失败：{error}"))?;
    let final_path = directory.join(format!("{}.json", request.request_id));
    let temporary_path = directory.join(format!("{}.tmp", request.request_id));
    let mut payload = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    let encoded =
        crate::windows_security::dpapi_protect_with_magic(COMPACT_REQUEST_MAGIC, &payload)
            .map_err(|error| format!("加密翻译请求失败：{error}"))?;
    zeroize_bytes(&mut payload);
    fs::write(&temporary_path, encoded).map_err(|error| format!("写入翻译请求失败：{error}"))?;
    fs::rename(&temporary_path, &final_path).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        format!("提交翻译请求失败：{error}")
    })?;
    Ok(final_path)
}

pub fn read_compact_request_file(path: &Path) -> Result<ExternalTranslationRequest, String> {
    let payload = fs::read(path).map_err(|error| format!("读取翻译请求失败：{error}"))?;
    let _ = fs::remove_file(path);
    if cfg!(windows)
        && !crate::windows_security::dpapi_blob_has_magic(COMPACT_REQUEST_MAGIC, &payload)
    {
        return Err("拒绝未加密的翻译请求文件".to_string());
    }
    let mut decoded =
        crate::windows_security::dpapi_unprotect_with_magic(COMPACT_REQUEST_MAGIC, &payload)
            .map_err(|error| format!("解密翻译请求失败：{error}"))?;
    let request =
        serde_json::from_slice(&decoded).map_err(|error| format!("翻译请求无效：{error}"));
    zeroize_bytes(&mut decoded);
    request
}

pub fn launch_compact_request(request: &ExternalTranslationRequest) -> Result<(), String> {
    let request_file = write_compact_request_file(request)?;
    let popup = resolve_sibling_executable("srf_ime_translate_result.exe")
        .ok_or_else(|| "未找到翻译结果浮窗 srf_ime_translate_result.exe".to_string())?;
    let mut command = Command::new(&popup);
    command.arg("--request-file").arg(&request_file);
    if let Some(parent) = popup.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command.spawn().map(|_| ()).map_err(|error| {
        let _ = fs::remove_file(request_file);
        format!("无法启动翻译结果浮窗：{error}")
    })
}

fn resolve_sibling_executable(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(name));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("kaixin")
                .join(name),
        );
    }
    candidates.into_iter().find(|path| path.is_file())
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

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
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
    let handle = OwnedWinHandle(raw_handle);
    write_pipe_with_timeout(handle.0, &payload)?;
    let mut response = vec![0u8; 4096];
    let bytes_read = read_pipe_with_timeout(handle.0, &mut response)?;
    parse_response(request, &response[..bytes_read as usize])
}

#[cfg(windows)]
struct OwnedWinHandle(HANDLE);

#[cfg(windows)]
impl Drop for OwnedWinHandle {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
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
    parse_response(request, response.as_bytes())
}

fn parse_response(request: &ExternalTranslationRequest, response: &[u8]) -> Result<(), String> {
    let response: ExternalTranslationResponse = serde_json::from_slice(response)
        .map_err(|error| format!("WinTranslator 返回无效响应：{error}"))?;
    if !response.request_id.is_empty() && response.request_id != request.request_id {
        return Err(
            "WinTranslator 响应协议不兼容（request_id 不匹配），请升级 WinTranslator。".to_string(),
        );
    }
    if response.ok {
        Ok(())
    } else if response.error.is_empty() {
        Err("WinTranslator 拒绝了请求".to_string())
    } else {
        Err(response.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_contract_uses_expected_json_fields() {
        let mut request = ExternalTranslationRequest::new("你好", "ocr");
        request.target_hwnd = Some(42);
        request.target_process_id = Some(7);
        request.screenshot_path = Some(PathBuf::from("shot.png"));
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["protocol_version"], 2);
        assert!(!value["request_id"].as_str().unwrap_or_default().is_empty());
        assert_eq!(value["action"], "translate");
        assert_eq!(value["text"], "你好");
        assert_eq!(value["source"], "auto");
        assert_eq!(value["target"], "auto-opposite");
        assert_eq!(value["origin"], "ocr");
        assert_eq!(value["target_hwnd"], 42);
        assert_eq!(value["target_process_id"], 7);
        assert_eq!(value["result_action"], "show");
        assert_eq!(value["interactive"], false);
        assert_eq!(value["screenshot_path"], "shot.png");
        assert_eq!(value["presentation"], "compact");
        assert_eq!(value["delivery"], "return");
        assert_eq!(value["replace_selection"], false);
    }

    #[test]
    fn target_language_follows_input_script() {
        assert_eq!(target_language_for_text("你好，world"), "en");
        assert_eq!(target_language_for_text("Hello world"), "zh");
    }
}
