#[cfg(windows)]
use crate::win_handle::OwnedWinHandle;
use crate::{app_paths, runtime_log};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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
    GetLastError, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
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
pub const PREVIEW_CONTROL_PIPE: &str = r"\\.\pipe\Kaixin.TranslationPreview.Control";
pub const HYMT_TRANSLATOR_EXE: &str = "hy-mt2-desktop.exe";
pub const WINTRANSLATOR_EXE: &str = "WinTranslator.exe";
const CREATE_NO_WINDOW: u32 = 0x08000000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const PIPE_PHASE_TIMEOUT_MS: u32 = 2_500;
pub const PROTOCOL_VERSION: u32 = 2;
#[cfg(not(windows))]
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Default 38-language catalog aligned with HY-MT2 `shared/languages.json`.
/// Common languages are placed first for quick picking.
pub const DEFAULT_LANGUAGES: &[(&str, &str)] = &[
    ("auto-opposite", "自动中英互译"),
    ("zh", "简体中文"),
    ("zh-Hant", "繁体中文"),
    ("en", "英语"),
    ("ja", "日语"),
    ("ko", "韩语"),
    ("fr", "法语"),
    ("de", "德语"),
    ("es", "西班牙语"),
    ("ru", "俄语"),
    ("pt", "葡萄牙语"),
    ("it", "意大利语"),
    ("ar", "阿拉伯语"),
    ("th", "泰语"),
    ("vi", "越南语"),
    ("id", "印尼语"),
    ("ms", "马来语"),
    ("tr", "土耳其语"),
    ("nl", "荷兰语"),
    ("pl", "波兰语"),
    ("cs", "捷克语"),
    ("uk", "乌克兰语"),
    ("hi", "印地语"),
    ("tl", "菲律宾语"),
    ("he", "希伯来语"),
    ("fa", "波斯语"),
    ("bn", "孟加拉语"),
    ("ta", "泰米尔语"),
    ("te", "泰卢固语"),
    ("mr", "马拉地语"),
    ("gu", "古吉拉特语"),
    ("ur", "乌尔都语"),
    ("km", "高棉语"),
    ("my", "缅甸语"),
    ("yue", "粤语"),
    ("bo", "藏语"),
    ("kk", "哈萨克语"),
    ("mn", "蒙古语"),
    ("ug", "维吾尔语"),
];

fn cached_languages() -> &'static Mutex<Vec<(String, String)>> {
    static LANGUAGES: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
    LANGUAGES.get_or_init(|| {
        Mutex::new(
            DEFAULT_LANGUAGES
                .iter()
                .map(|(code, label)| ((*code).to_string(), (*label).to_string()))
                .collect(),
        )
    })
}

pub fn supported_languages() -> Vec<(String, String)> {
    cached_languages().lock().unwrap().clone()
}

pub fn language_label(code: &str) -> String {
    let list = cached_languages().lock().unwrap();
    list.iter()
        .find(|(c, _)| c == code)
        .map(|(_, l)| l.clone())
        .unwrap_or_else(|| code.to_string())
}

fn update_cached_languages(raw: &[LanguageCatalogItem]) {
    if raw.is_empty() {
        return;
    }
    let mut merged: Vec<(String, String)> = vec![("auto-opposite".into(), "自动中英互译".into())];
    // Keep common languages first in familiar order if present in server catalog.
    let priority = [
        "zh", "zh-Hant", "en", "ja", "ko", "fr", "de", "es", "ru", "pt", "it",
    ];
    for code in priority {
        if let Some(item) = raw.iter().find(|entry| entry.code == code) {
            let label = if !item.display_name.is_empty() {
                item.display_name.clone()
            } else if !item.chinese_name.is_empty() {
                item.chinese_name.clone()
            } else {
                item.prompt_name.clone()
            };
            merged.push((item.code.clone(), label));
        }
    }
    for item in raw {
        if item.code.is_empty() || merged.iter().any(|(c, _)| c == &item.code) {
            continue;
        }
        let label = if !item.display_name.is_empty() {
            item.display_name.clone()
        } else if !item.chinese_name.is_empty() {
            item.chinese_name.clone()
        } else {
            item.prompt_name.clone()
        };
        merged.push((item.code.clone(), label));
    }
    if let Ok(mut guard) = cached_languages().lock() {
        *guard = merged;
    }
}

#[derive(Clone, Debug, Deserialize)]
struct LanguageCatalogItem {
    #[serde(default)]
    code: String,
    #[serde(default, rename = "promptName")]
    prompt_name: String,
    #[serde(default, rename = "chineseName")]
    chinese_name: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
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
    pub callback_ack: bool,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub bypass_cache: bool,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub stream_preview: bool,
    #[serde(default)]
    pub duplex: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caret_rect: Option<[i32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glossary_translation: Option<String>,
    #[serde(skip)]
    safety: InputSafety,
}

impl Default for ExternalTranslationRequest {
    fn default() -> Self {
        Self::new("", "kaixin-ime")
    }
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
    backend: String,
    #[serde(default)]
    compute_backend: String,
    #[serde(default)]
    model_ready: Option<bool>,
    #[serde(default)]
    engine_ready: Option<bool>,
    #[serde(default)]
    engine_loaded: Option<bool>,
    #[serde(default)]
    queued: usize,
    #[serde(default)]
    languages: Vec<LanguageCatalogItem>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    target_language: Option<String>,
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
    #[serde(default)]
    duplex_stream: bool,
}

#[derive(Clone, Debug)]
pub struct TranslatorStatusSnapshot {
    pub provider: String,
    pub backend: String,
    pub compute_backend: String,
    pub protocol_version: u32,
    pub model_ready: bool,
    pub engine_ready: bool,
    pub engine_loaded: bool,
    pub queued: usize,
    pub duplex_stream: bool,
    pub language_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TranslationCallbackEvent {
    #[serde(rename = "event")]
    pub event_name: String,
    request_id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub completed: usize,
    #[serde(default)]
    pub total: usize,
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
            bypass_cache: false,
            ephemeral: false,
            stream_preview: false,
            duplex: false,
            caret_rect: None,
            glossary_source: None,
            glossary_translation: None,
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
    if text.chars().any(|ch| matches!(ch, '\u{3040}'..='\u{30ff}')) {
        return "zh";
    }
    let han = text
        .chars()
        .filter(|ch| {
            matches!(
                ch,
                '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
            )
        })
        .count();
    let latin = text.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    if han > 0 && han * 3 >= latin.max(1) {
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
                PathBuf::from(&root)
                    .join("Hy-MT2")
                    .join(HYMT_TRANSLATOR_EXE),
            );
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
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
    };
    let key: Vec<u16> =
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\hy-mt2-desktop.exe\0"
            .encode_utf16()
            .collect();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut value = [0u16; 32768];
        let mut size = (value.len() * 2) as u32;
        if unsafe {
            RegGetValueW(
                hive,
                key.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                value.as_mut_ptr().cast(),
                &mut size,
            )
        } == 0
        {
            if let Some(end) = value.iter().position(|v| *v == 0) {
                let candidate = PathBuf::from(String::from_utf16_lossy(&value[..end]));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
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

pub fn query_status_snapshot() -> Result<TranslatorStatusSnapshot, String> {
    let mut request = ExternalTranslationRequest::new("", "settings-test");
    request.action = "capabilities".to_string();
    request.presentation = "background".to_string();
    let response = send_request_for_response(&request)?;
    let capabilities = response
        .capabilities
        .ok_or_else(|| "翻译服务未返回 capabilities；请升级到支持协议 v2 的版本。".to_string())?;
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
        return Err("翻译服务缺少后台返回或取消能力，请升级后再联动。".to_string());
    }
    if !response.languages.is_empty() {
        update_cached_languages(&response.languages);
    }
    Ok(TranslatorStatusSnapshot {
        provider: if response.provider.is_empty() {
            "external".into()
        } else {
            response.provider
        },
        backend: response.backend,
        compute_backend: response.compute_backend,
        protocol_version: response.protocol_version.max(PROTOCOL_VERSION),
        model_ready: response.model_ready != Some(false),
        engine_ready: response.engine_ready != Some(false),
        engine_loaded: response.engine_loaded.unwrap_or(false),
        queued: response.queued,
        duplex_stream: capabilities.duplex_stream,
        language_count: if response.languages.is_empty() {
            DEFAULT_LANGUAGES.len().saturating_sub(1)
        } else {
            response.languages.len()
        },
    })
}

pub fn test_connection() -> Result<String, String> {
    let snap = query_status_snapshot()?;
    let backend_part = if snap.backend.is_empty() {
        String::new()
    } else {
        format!("；推理后端：{}", snap.backend)
    };
    let stream_part = if snap.duplex_stream {
        "（双工流式）"
    } else {
        "（回调模式）"
    };
    Ok(format!(
        "翻译服务联动成功：协议 v{}{stream_part}，提供方：{}{backend_part}；模型：{}；引擎：{}；排队：{}；支持 {} 种语言。",
        snap.protocol_version,
        snap.provider,
        if snap.model_ready { "可用" } else { "缺失或不完整" },
        if !snap.engine_ready {
            "运行时未安装"
        } else if snap.engine_loaded {
            "已加载就绪"
        } else {
            "可用，首次翻译自动加载"
        },
        snap.queued,
        snap.language_count,
    ))
}

/// Fire-and-forget warm-up of HY-MT2 local engine (e.g. while user is taking OCR screenshot).
pub fn preload_translator_async() {
    if !is_available() {
        return;
    }
    thread::Builder::new()
        .name("kaixin-hymt-preload".to_string())
        .spawn(|| {
            let mut request = ExternalTranslationRequest::new("", "kaixin-preload");
            request.action = "preload".to_string();
            request.presentation = "background".to_string();
            let _ = send_request(&request);
        })
        .ok();
}

pub fn add_glossary_term(
    target_language: &str,
    source: &str,
    translation: &str,
) -> Result<String, String> {
    let source = source.trim();
    let translation = translation.trim();
    if source.is_empty() || translation.is_empty() {
        return Err("术语原文和译文不能为空".into());
    }
    let mut request = ExternalTranslationRequest::new("", "kaixin-glossary");
    request.action = "add_glossary".to_string();
    request.presentation = "background".to_string();
    request.target = target_language.to_string();
    request.glossary_source = Some(source.to_string());
    request.glossary_translation = Some(translation.to_string());
    let response = send_request_for_response(&request)?;
    let target = response
        .target_language
        .unwrap_or_else(|| target_language.to_string());
    let count = response.count.unwrap_or(0);
    Ok(format!("已加入 {target} 术语表（当前共 {count} 条）"))
}

pub fn present_result(id: &str) -> Result<(), String> {
    let mut request = ExternalTranslationRequest::new("", "kaixin-present");
    request.action = "present".into();
    request.query_request_id = Some(id.to_owned());
    send_request_once(&request)
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTranslationPreference {
    pub language: String,
    pub action: String,
}

pub fn app_preferences_path() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("translation-apps.json")
}

pub fn load_app_preferences() -> BTreeMap<String, AppTranslationPreference> {
    std::fs::read(app_preferences_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_app_preference(
    exe: &str,
    preference: Option<AppTranslationPreference>,
) -> Result<(), String> {
    use fs2::FileExt;
    let path = app_preferences_path();
    let parent = path
        .parent()
        .ok_or("Preference path has no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建偏好目录失败：{e}"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| format!("打开偏好锁文件失败：{e}"))?;
    lock.lock_exclusive()
        .map_err(|e| format!("锁定应用偏好失败：{e}"))?;
    let mut values = load_app_preferences();
    if let Some(value) = preference {
        values.insert(exe.to_owned(), value);
    } else {
        values.remove(exe);
    }
    let temporary = path.with_extension("tmp");
    let encoded =
        serde_json::to_vec_pretty(&values).map_err(|e| format!("序列化应用偏好失败：{e}"))?;
    std::fs::write(&temporary, encoded).map_err(|e| format!("写入应用偏好失败：{e}"))?;
    std::fs::rename(temporary, path).map_err(|e| format!("保存应用偏好失败：{e}"))
}

pub fn clear_app_preferences() -> Result<(), String> {
    use fs2::FileExt;
    let path = app_preferences_path();
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|e| format!("创建偏好目录失败：{e}"))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| format!("打开偏好锁文件失败：{e}"))?;
    lock.lock_exclusive()
        .map_err(|e| format!("锁定应用偏好失败：{e}"))?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("清空应用偏好失败：{e}"))?;
    }
    Ok(())
}

fn continuous_mode_flag_path() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("translate_continuous.flag")
}

pub fn continuous_translate_enabled() -> bool {
    continuous_mode_flag_path().is_file()
}

pub fn set_continuous_translate_enabled(enabled: bool) -> Result<(), String> {
    let path = continuous_mode_flag_path();
    if enabled {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, b"1").map_err(|e| e.to_string())
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

pub fn process_image_path_for_window(hwnd: isize) -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let Some(pid) = process_id_for_window(hwnd) else {
            return String::new();
        };
        unsafe {
            let Ok(handle) =
                OwnedWinHandle::from_raw(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))
            else {
                return String::new();
            };
            let mut buffer = vec![0u16; 32768];
            let mut len = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(handle.as_raw(), 0, buffer.as_mut_ptr(), &mut len);
            if ok == 0 {
                String::new()
            } else {
                String::from_utf16_lossy(&buffer[..len as usize]).to_lowercase()
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        String::new()
    }
}

const PASSWORD_MANAGER_PROCESSES: &[&str] = &[
    "1password.exe",
    "bitwarden.exe",
    "keepass.exe",
    "keepassxc.exe",
    "lastpass.exe",
    "dashlane.exe",
    "enpass.exe",
    "roboform.exe",
    "nordpass.exe",
    "protonpass.exe",
];

fn read_privacy_config() -> (bool, Vec<String>) {
    let path = app_paths::config_ini_path().unwrap_or_else(|| {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(app_paths::CONFIG_FILE_NAME)
    });
    let Ok(text) = std::fs::read_to_string(path) else {
        return (false, Vec::new());
    };
    let mut in_privacy = false;
    let mut enabled = false;
    let mut excluded = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            in_privacy = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("privacy");
            continue;
        }
        if !in_privacy {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            if key == "enabled" {
                enabled = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                );
            } else if key == "never_clipboard_processes" {
                excluded = value
                    .split([',', ';', '|'])
                    .map(|item| item.trim().to_ascii_lowercase())
                    .filter(|item| !item.is_empty())
                    .collect();
            }
        }
    }
    (enabled, excluded)
}

/// Returns `Some(reason)` when global privacy mode or sensitive-process rules forbid
/// capturing clipboard/selection from `target_hwnd` and sending it to an external translator.
pub fn privacy_block_reason_for_window(target_hwnd: isize) -> Option<String> {
    let (privacy_enabled, excluded) = read_privacy_config();
    if privacy_enabled {
        return Some("全局隐私模式已开启，已阻止自动抓取文本发送到外部翻译软件。".into());
    }
    let full_path = process_image_path_for_window(target_hwnd);
    if full_path.is_empty() {
        return None;
    }
    let file_name = Path::new(&full_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if PASSWORD_MANAGER_PROCESSES.contains(&file_name.as_str()) {
        return Some(format!(
            "当前窗口属于密码管理器（{file_name}），已阻止向外部翻译发送文本。"
        ));
    }
    if excluded
        .iter()
        .any(|pat| pat == &file_name || pat == &full_path)
    {
        return Some(format!(
            "当前应用（{file_name}）位于隐私排除列表，已阻止抓取并发送到外部翻译。"
        ));
    }
    None
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
        "translate"
            | "ping"
            | "capabilities"
            | "cancel"
            | "status"
            | "ack"
            | "present"
            | "preload"
            | "add_glossary"
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

/// Try sending a serialized preview request to an already-running persistent preview window.
#[cfg(windows)]
pub fn try_handoff_to_preview(bytes: &[u8]) -> bool {
    if bytes.is_empty() || bytes.len() > 1024 * 1024 {
        return false;
    }
    let mut pipe_name: Vec<u16> = PREVIEW_CONTROL_PIPE.encode_utf16().collect();
    pipe_name.push(0);
    if unsafe { WaitNamedPipeW(pipe_name.as_ptr(), 120) } == 0 {
        return false;
    }
    let raw_handle = unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            0,
        )
    };
    if raw_handle == INVALID_HANDLE_VALUE {
        return false;
    }
    let Ok(handle) = (unsafe { OwnedWinHandle::from_raw(raw_handle) }) else {
        return false;
    };
    let mut framed = Vec::with_capacity(bytes.len() + 1);
    framed.extend_from_slice(bytes);
    framed.push(b'\n');
    write_pipe_with_timeout(handle.as_raw(), &framed).is_ok()
}

#[cfg(not(windows))]
pub fn try_handoff_to_preview(_bytes: &[u8]) -> bool {
    false
}

/// Send a request to the external translator while keeping the caller responsive.
pub fn launch_full_request(request: &ExternalTranslationRequest) -> Result<(), String> {
    if !is_available() {
        return Err(availability_message());
    }
    #[cfg(windows)]
    if !request.text.trim().is_empty() {
        use std::io::Write;
        use std::process::Stdio;
        let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("翻译请求超过 1 MiB，请拆分文本。".into());
        }
        if try_handoff_to_preview(&bytes) {
            return Ok(());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("srf_ime_tray.exe");
        let mut child = Command::new(executable)
            .arg("--translation-preview")
            .stdin(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("无法打开翻译预览：{e}"))?;
        child
            .stdin
            .take()
            .ok_or("翻译预览输入通道不可用")?
            .write_all(&bytes)
            .map_err(|e| e.to_string())?;
        thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
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
        .name("kaixin-translator-request".to_string())
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
        .name("kaixin-translator-session".to_string())
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
                    .name("kaixin-translator-callback".to_string())
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
    Err("当前平台不支持翻译回调管道".to_string())
}

#[cfg(windows)]
enum DuplexOutcome {
    Done,
    FallbackToCallback,
    Failed(String),
}

#[cfg(windows)]
fn open_request_pipe_with_autostart() -> Result<(OwnedWinHandle, &'static str), String> {
    let try_open = |pipe_str: &str| -> Result<OwnedWinHandle, String> {
        let mut pipe_name: Vec<u16> = pipe_str.encode_utf16().collect();
        pipe_name.push(0);
        if unsafe { WaitNamedPipeW(pipe_name.as_ptr(), PIPE_PHASE_TIMEOUT_MS) } == 0 {
            return Err(format!(
                "等待翻译服务管道超时或失败：{}",
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
                "连接翻译服务管道失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        unsafe { OwnedWinHandle::from_raw(raw_handle) }
            .map_err(|err| format!("接管翻译服务管道句柄失败：{err}"))
    };

    let pipe_str = request_pipe_path();
    if let Ok(handle) = try_open(pipe_str) {
        return Ok((handle, pipe_str));
    }
    let path = translator_path()
        .ok_or_else(|| "未找到翻译程序；请安装 HY-MT2 或在翻译设置中选择程序路径。".to_string())?;
    spawn_translator(&path)?;
    let started = Instant::now();
    while started.elapsed() < CONNECT_TIMEOUT {
        thread::sleep(Duration::from_millis(120));
        let pipe_str = request_pipe_path();
        if let Ok(handle) = try_open(pipe_str) {
            return Ok((handle, pipe_str));
        }
    }
    Err("翻译服务已启动，但命名管道未就绪；请检查程序与模型状态。".into())
}

#[cfg(windows)]
fn run_duplex_preview_stream(
    mut request: ExternalTranslationRequest,
    tx: &mpsc::Sender<TranslationCallbackEvent>,
    stop: &Arc<AtomicBool>,
) -> DuplexOutcome {
    let (handle, pipe_str) = match open_request_pipe_with_autostart() {
        Ok(pair) => pair,
        Err(error) => return DuplexOutcome::Failed(error),
    };
    if pipe_str != HYMT_PIPE_PATH {
        return DuplexOutcome::FallbackToCallback;
    }
    if stop.load(SessionOrdering::Acquire) {
        return DuplexOutcome::Done;
    }
    request.duplex = true;
    request.stream_preview = true;
    request.reply_pipe = None;
    let Ok(mut payload) = serde_json::to_vec(&request) else {
        return DuplexOutcome::Failed("编码翻译请求失败".into());
    };
    payload.push(b'\n');
    if let Err(error) = write_pipe_with_timeout(handle.as_raw(), &payload) {
        return DuplexOutcome::Failed(error);
    }

    let mut pending = Vec::with_capacity(4096);
    let mut read_buf = [0u8; 4096];

    let read_next_line = |pending: &mut Vec<u8>,
                          read_buf: &mut [u8; 4096],
                          timeout_ms: u32|
     -> Result<Option<Vec<u8>>, Result<(), String>> {
        if let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=end).collect();
            line.pop();
            return Ok(Some(line));
        }
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let started = unsafe {
            ReadFile(
                handle.as_raw(),
                read_buf.as_mut_ptr(),
                read_buf.len() as u32,
                std::ptr::null_mut(),
                &mut overlapped,
            )
        };
        if started == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_BROKEN_PIPE {
                return Ok(None);
            }
            if err != ERROR_IO_PENDING {
                return Err(Err(format!(
                    "读取翻译流失败：{}",
                    std::io::Error::from_raw_os_error(err as i32)
                )));
            }
        }
        let mut transferred = 0u32;
        let completed = unsafe {
            GetOverlappedResultEx(
                handle.as_raw(),
                &overlapped,
                &mut transferred,
                timeout_ms,
                0,
            )
        };
        if completed == 0 {
            let err = unsafe { GetLastError() };
            if err == WAIT_TIMEOUT {
                unsafe {
                    CancelIoEx(handle.as_raw(), &overlapped);
                    GetOverlappedResult(handle.as_raw(), &overlapped, &mut transferred, 1);
                }
                return Err(Ok(()));
            }
            if err == ERROR_BROKEN_PIPE {
                return Ok(None);
            }
            return Err(Err(format!(
                "读取翻译流失败：{}",
                std::io::Error::from_raw_os_error(err as i32)
            )));
        }
        if transferred == 0 {
            return Ok(None);
        }
        pending.extend_from_slice(&read_buf[..transferred as usize]);
        if pending.len() > 1024 * 1024 {
            return Err(Err("翻译流单条消息超过 1 MiB".into()));
        }
        if let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=end).collect();
            line.pop();
            return Ok(Some(line));
        }
        Err(Ok(()))
    };

    // Wait up to 3s for initial ack response line.
    let ack_deadline = Instant::now() + Duration::from_secs(3);
    let ack_line = loop {
        if stop.load(SessionOrdering::Acquire) {
            return DuplexOutcome::Done;
        }
        if Instant::now() >= ack_deadline {
            return DuplexOutcome::Failed("翻译服务响应超时".into());
        }
        match read_next_line(&mut pending, &mut read_buf, 200) {
            Ok(Some(line)) => break line,
            Ok(None) => return DuplexOutcome::Failed("翻译服务连接提前关闭".into()),
            Err(Ok(())) => continue,
            Err(Err(error)) => return DuplexOutcome::Failed(error),
        }
    };
    match parse_response(&request, &ack_line) {
        Ok(_) => {}
        Err(error) if error.contains("回调管道") => {
            // Older HY-MT2 build that does not yet accept duplex: true without reply_pipe.
            return DuplexOutcome::FallbackToCallback;
        }
        Err(error) => return DuplexOutcome::Failed(error),
    }

    let mut pieces: Vec<Option<String>> = Vec::new();
    let mut received_any_event = false;
    let deadline = Instant::now() + Duration::from_secs(31 * 60);
    loop {
        if stop.load(SessionOrdering::Acquire) || Instant::now() >= deadline {
            drop(handle);
            let _ = send_request_once(&ExternalTranslationRequest::cancellation(
                &request.request_id,
            ));
            return DuplexOutcome::Done;
        }
        let line = match read_next_line(&mut pending, &mut read_buf, 200) {
            Ok(Some(line)) => line,
            Ok(None) => {
                if !received_any_event {
                    return DuplexOutcome::FallbackToCallback;
                }
                return DuplexOutcome::Failed("翻译连接在完成前意外断开".into());
            }
            Err(Ok(())) => continue,
            Err(Err(error)) => return DuplexOutcome::Failed(error),
        };
        let Ok(mut event) = serde_json::from_slice::<TranslationCallbackEvent>(&line) else {
            continue;
        };
        if event.request_id != request.request_id {
            continue;
        }
        received_any_event = true;
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
                return DuplexOutcome::Failed("译文超出安全大小限制".into());
            }
            if pieces.iter().any(Option::is_none) {
                continue;
            }
            event.text = Some(pieces.iter().filter_map(|s| s.as_deref()).collect());
        }
        let terminal = matches!(
            event.event_name.as_str(),
            "completed" | "failed" | "cancelled"
        );
        let _ = tx.send(event);
        if terminal {
            return DuplexOutcome::Done;
        }
    }
}

/// Preview owns user interaction; transport prefers single-connection duplex streaming
/// on HY-MT2 and falls back to v2 callback pipes for older builds / WinTranslator.
#[cfg(windows)]
pub fn preview_stream(
    mut request: ExternalTranslationRequest,
) -> (mpsc::Receiver<TranslationCallbackEvent>, Arc<AtomicBool>) {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    request.result_action = "show".into();
    request.delivery = "return".into();
    request.presentation = "background".into();
    request.stream_preview = true;
    thread::spawn(move || {
        match run_duplex_preview_stream(request.clone(), &tx, &worker_stop) {
            DuplexOutcome::Done => return,
            DuplexOutcome::Failed(error) => {
                worker_stop.store(true, SessionOrdering::Release);
                if let Ok(event) = serde_json::from_value(serde_json::json!({
                    "event": "failed", "request_id": request.request_id, "message": error,
                })) {
                    let _ = tx.send(event);
                }
                return;
            }
            DuplexOutcome::FallbackToCallback => {}
        }

        // Legacy v2 callback fallback
        request.duplex = false;
        request.request_id = fresh_request_id();
        request.reply_pipe = Some(format!("Kaixin.Translate.Result.{}", request.request_id));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let listener_request = request.clone();
        let events = tx.clone();
        let listener_stop = worker_stop.clone();
        thread::spawn(move || {
            listen_for_translation_callbacks_with_stop(
                listener_request.reply_pipe.as_ref().unwrap(),
                listener_request.clone(),
                ready_tx,
                |event| {
                    let _ = events.send(event.clone());
                },
                listener_stop,
            );
        });
        let result = ready_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "创建翻译结果通道超时".to_string())
            .and_then(|ready| ready)
            .and_then(|_| {
                if worker_stop.load(SessionOrdering::Acquire) {
                    return Err("翻译已取消".into());
                }
                let result = send_request(&request);
                if worker_stop.load(SessionOrdering::Acquire) {
                    let _ = send_request_once(&ExternalTranslationRequest::cancellation(
                        &request.request_id,
                    ));
                }
                result
            });
        if let Err(error) = result {
            worker_stop.store(true, SessionOrdering::Release);
            if let Ok(event) = serde_json::from_value(serde_json::json!({
                "event": "failed", "request_id": request.request_id, "message": error,
            })) {
                let _ = tx.send(event);
            }
        }
    });
    (rx, stop)
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
                4,
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
            let read = read_callback_event(handle.as_raw())
                .and_then(|bytes| serde_json::from_slice::<TranslationCallbackEvent>(&bytes).ok());
            unsafe { DisconnectNamedPipe(handle.as_raw()) };
            drop(handle);
            read
        } else {
            // Release pipe handle before polling `status` so we never hold a busy unconnected instance.
            drop(handle);
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
            if request.result_action != "show" {
                let _ = present_result(&request.request_id);
            }
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
    let active = sessions().lock().unwrap();
    if !request.session.is_empty()
        && active.get(&request.session).is_none_or(|session| {
            session.id != request.request_id || session.stop.load(SessionOrdering::Acquire)
        })
    {
        return;
    }
    let current = input_safety();
    let safe_window = request.target_hwnd.is_some_and(|hwnd| {
        let fg = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        fg == hwnd && process_id_for_window(hwnd) == request.target_process_id
    });
    if !safe_to_apply(
        &request.safety,
        &current,
        request.result_action == "paste",
        safe_window,
    ) {
        drop(active);
        let _ = present_result(&request.request_id);
        return;
    }
    if request.result_action == "paste" {
        if let Some(hwnd) = request.target_hwnd {
            if crate::win_paste::insert_text_preserving_clipboard(hwnd, text).is_err() {
                let _ = present_result(&request.request_id);
            }
        }
    } else if clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text).is_err() {
        let _ = present_result(&request.request_id);
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
        .map_err(|error| format!("无法启动翻译服务：{error}；{}", path.display()))
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
            "等待翻译服务管道超时或失败：{}",
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
            "连接翻译服务管道失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    let handle = unsafe { OwnedWinHandle::from_raw(raw_handle) }
        .map_err(|err| format!("接管翻译服务管道句柄失败：{err}"))?;
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
        return Err("翻译服务管道写入不完整".to_string());
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
                "翻译服务管道{phase}失败：{}",
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
            GetOverlappedResult(handle, overlapped, &mut transferred, 1);
        }
        return Err(format!("翻译服务无响应（管道{phase}超时）"));
    }
    Err(format!(
        "翻译服务管道{phase}失败：{}",
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
        Err("当前平台不支持翻译命名管道".to_string())
    }
}

fn parse_response(
    request: &ExternalTranslationRequest,
    response: &[u8],
) -> Result<ExternalTranslationResponse, String> {
    let response: ExternalTranslationResponse = serde_json::from_slice(response)
        .map_err(|error| format!("翻译服务返回无效响应：{error}"))?;
    if !response.request_id.is_empty() && response.request_id != request.request_id {
        return Err("翻译服务响应协议不兼容（request_id 不匹配），请升级翻译软件。".to_string());
    }
    if response.ok {
        Ok(response)
    } else if response.error.is_empty() {
        Err("翻译服务拒绝了请求".to_string())
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
            "model_ready": false, "engine_ready": true, "engine_loaded": true, "provider": "hy-mt2", "backend": "llama.cpp / Q4_K_M · NVIDIA CUDA", "queued": 2,
            "result": {"request_id": request.request_id, "event": "completed", "text": "译文", "completed": 2, "total": 2, "chunk_index": 1, "chunk_count": 3}});
        let response = parse_response(&request, &serde_json::to_vec(&payload).unwrap()).unwrap();
        assert_eq!(response.model_ready, Some(false));
        assert_eq!(response.engine_loaded, Some(true));
        assert_eq!(response.queued, 2);
        let event = response.result.unwrap();
        assert_eq!(event.chunk_count, 3);
        assert_eq!(event.chunk_index, 1);
        assert_eq!(event.completed, 2);
        assert_eq!(event.total, 2);
    }
    #[test]
    fn default_languages_include_full_hymt_catalog() {
        let langs = supported_languages();
        assert!(langs.len() >= 38);
        assert_eq!(langs[0].0, "auto-opposite");
        assert!(langs.iter().any(|(c, _)| c == "yue"));
        assert!(langs.iter().any(|(c, _)| c == "th"));
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
        request.result_action = "show".into();
        let (rx, _stop) = preview_stream(request);
        let mut names = Vec::new();
        loop {
            let event = rx.recv_timeout(Duration::from_secs(30)).unwrap();
            names.push(event.event_name.clone());
            match event.event_name.as_str() {
                "completed" => {
                    assert!(!event.text.unwrap_or_default().trim().is_empty());
                    break;
                }
                "failed" | "cancelled" => {
                    panic!(
                        "unexpected translation terminal event: {}",
                        event.event_name
                    )
                }
                _ => {}
            }
        }
        assert!(names.iter().any(|event| event == "started"));
    }
}
