#![cfg_attr(windows, windows_subsystem = "windows")]

//! 剪贴板管理器无头服务：为 Rust 界面（srf_ime_clipboard.exe）提供数据操作与
//! 粘贴链。数据层（DPAPI 加密 SQLite、剪贴板捕获、跨进程锁）全部留在本进程，
//! UI 进程只通过命名管道 JSON 协议调用。服务随 UI 进程启动，UI 崩溃后 10s
//! 无连接自动退出；管道已存在（另一实例存活）时静默退出由旧实例接管。

use pinyin_ime::app_paths;
use pinyin_ime::clipboard_store::{self, ClipboardEntry, ClipboardSnapshot};
use pinyin_ime::runtime_log::{self, RuntimeLogLevel};
use pinyin_ime::win_paste;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_LISTENING, HANDLE, INVALID_HANDLE_VALUE,
};
#[cfg(windows)]
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, SetNamedPipeHandleState, PIPE_NOWAIT,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
    PIPE_WAIT,
};

const WINDOW_CFG_NAME: &str = "clipboard_manager_window.sqlite";
const WINDOW_CFG_SCHEMA_VERSION: i32 = 2;
const PROTOCOL_VERSION: u32 = 1;
const PIPE_NAME_PREFIX: &str = r"\\.\pipe\KaixinClipboardSvc_v1";
const CAPABILITY_ENV: &str = "KAIXIN_CLIPBOARD_CAPABILITY";
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const ORPHAN_EXIT_SECS: u64 = 10;

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static EXPECTED_CAPABILITY: OnceLock<String> = OnceLock::new();

// ---------------------------------------------------------------------------
// 数据操作（自 egui 版原样迁移）
// ---------------------------------------------------------------------------

fn capture_system_clipboard_with_log(force: bool, phase: &str) {
    if let Err(err) = clipboard_store::capture_system_clipboard(force) {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_manager_capture",
            format!(
                "status=failed phase={} force={} reason={err}",
                phase,
                if force { 1 } else { 0 }
            ),
        );
    }
}

fn load_snapshot_with_log(phase: &str) -> ClipboardSnapshot {
    match clipboard_store::snapshot() {
        Ok(snapshot) => {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Verbose,
                "clipboard_snapshot",
                format!(
                    "status=ok phase={} history={} pinned={}",
                    phase,
                    snapshot.history.len(),
                    snapshot.pinned.len()
                ),
            );
            snapshot
        }
        Err(err) => {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_snapshot",
                format!("status=failed phase={} reason={err}", phase),
            );
            ClipboardSnapshot::default()
        }
    }
}

fn store_modified_string() -> String {
    clipboard_store::store_modified()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_default()
}

fn snapshot_dto(snapshot: &ClipboardSnapshot) -> serde_json::Value {
    fn entry_json(entry: &ClipboardEntry) -> serde_json::Value {
        json!({
            "id": entry.id,
            "text": entry.text,
            "captured_at": entry.captured_at,
            "first_captured_at": entry.first_captured_at,
            "copy_count": entry.copy_count,
            "source_app": entry.source_app,
        })
    }
    json!({
        "history": snapshot.history.iter().map(entry_json).collect::<Vec<_>>(),
        "pinned": snapshot.pinned.iter().map(entry_json).collect::<Vec<_>>(),
    })
}

#[cfg(windows)]
fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..12 {
        match clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_error = Some(e.to_string());
                std::thread::sleep(Duration::from_millis(10 + attempt * 8));
            }
        }
    }
    Err(format!(
        "剪贴板被其他程序占用或拒绝写入：{}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

#[cfg(windows)]
fn current_system_clipboard_text() -> Option<String> {
    if !clipboard_win::is_format_avail(clipboard_win::formats::CF_UNICODETEXT) {
        return None;
    }
    clipboard_win::get_clipboard(clipboard_win::formats::Unicode).ok()
}

#[cfg(windows)]
fn restore_system_clipboard_text_after_paste(previous: Option<String>, pasted: &str) {
    let Some(previous) = previous else {
        return;
    };
    if previous == pasted {
        return;
    }
    std::thread::sleep(Duration::from_millis(80));
    let _ = copy_to_system_clipboard(&previous);
}

#[cfg(not(windows))]
fn copy_to_system_clipboard(_text: &str) -> Result<(), String> {
    Err("当前平台不支持系统剪贴板快速粘贴".to_string())
}

#[cfg(not(windows))]
fn current_system_clipboard_text() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn restore_system_clipboard_text_after_paste(_previous: Option<String>, _pasted: &str) {}

/// 完整粘贴链：保存旧剪贴板 → 写入待粘贴文本 → 记录到历史 → 向目标窗口发
/// Ctrl+V → 恢复旧剪贴板。任何一步失败都会把错误带回客户端显示。
fn paste_text_worker(text: &str, target_hwnd: isize) -> Result<(), String> {
    let previous_clipboard = current_system_clipboard_text();
    copy_to_system_clipboard(text)?;
    if let Err(err) = clipboard_store::record_text(text) {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_manager_record",
            format!(
                "status=failed phase=paste units={} reason={err}",
                text.encode_utf16().count()
            ),
        );
    }
    match win_paste::send_ctrl_v_to_target(target_hwnd) {
        Ok(()) => {
            restore_system_clipboard_text_after_paste(previous_clipboard, text);
            Ok(())
        }
        Err(e) => Err(format!("{e}，文本已复制到剪贴板")),
    }
}

// ---------------------------------------------------------------------------
// 窗口位置与偏好（clipboard_manager_window.sqlite，与 egui 版共用，零迁移）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct SavedWindowRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl SavedWindowRect {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            x: row.get::<_, f64>(0)? as f32,
            y: row.get::<_, f64>(1)? as f32,
            w: row.get::<_, f64>(2)? as f32,
            h: row.get::<_, f64>(3)? as f32,
        })
    }
}

fn local_app_data_dir() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(local_app_data_dir)
        .join(WINDOW_CFG_NAME)
}

fn initialize_window_config_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS window_rect (
           id INTEGER PRIMARY KEY CHECK(id = 1),
           x REAL NOT NULL,
           y REAL NOT NULL,
           w REAL NOT NULL,
           h REAL NOT NULL
         );
         CREATE TABLE IF NOT EXISTS prefs (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );",
    )?;
    conn.execute_batch(&format!(
        "PRAGMA user_version = {WINDOW_CFG_SCHEMA_VERSION};"
    ))?;
    Ok(())
}

fn load_pref(key: &str) -> Option<String> {
    let conn = Connection::open(config_path()).ok()?;
    initialize_window_config_connection(&conn).ok()?;
    conn.query_row("SELECT value FROM prefs WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
}

fn save_pref(key: &str, value: &str) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create window cfg dir: {e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("open window cfg: {e}"))?;
    initialize_window_config_connection(&conn)
        .map_err(|e| format!("initialize window cfg: {e}"))?;
    conn.execute(
        "INSERT INTO prefs (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
    .map_err(|e| format!("write window cfg pref: {e}"))
}

fn load_saved_window_rect() -> Option<SavedWindowRect> {
    let conn = Connection::open(config_path()).ok()?;
    initialize_window_config_connection(&conn).ok()?;
    conn.query_row(
        "SELECT x, y, w, h FROM window_rect WHERE id = 1",
        [],
        SavedWindowRect::from_row,
    )
    .ok()
}

fn save_window_rect(rect: SavedWindowRect) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create window cfg dir: {e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("open window cfg: {e}"))?;
    initialize_window_config_connection(&conn)
        .map_err(|e| format!("initialize window cfg: {e}"))?;
    conn.execute(
        "INSERT INTO window_rect (id, x, y, w, h)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
           x = excluded.x,
           y = excluded.y,
           w = excluded.w,
           h = excluded.h",
        params![rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64],
    )
    .map(|_| ())
    .map_err(|e| format!("write window cfg: {e}"))
}

// ---------------------------------------------------------------------------
// JSON 协议（4 字节 LE 长度前缀 + UTF-8 JSON）
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Request {
    #[serde(default)]
    req: u64,
    #[serde(default)]
    method: String,
    #[serde(default)]
    capability: String,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    last_modified: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    target_hwnd: i64,
    #[serde(default)]
    was_pinned: bool,
    #[serde(default)]
    days: u64,
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    rect: Option<RectDto>,
}

#[derive(Deserialize)]
struct RectDto {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Serialize)]
struct Response {
    req: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

fn message_data(message: &str) -> serde_json::Value {
    json!({ "message": message })
}

fn refresh_data(force: bool, last_modified: Option<&str>) -> Result<serde_json::Value, String> {
    capture_system_clipboard_with_log(force, "refresh");
    let snapshot = load_snapshot_with_log("refresh");
    let modified = store_modified_string();
    let changed = last_modified.map_or(true, |previous| previous != modified);
    Ok(json!({
        "changed": changed,
        "modified": modified,
        "max_age_days": clipboard_store::configured_max_age_days(),
        "snapshot": snapshot_dto(&snapshot),
    }))
}

fn execute_request(request: &Request) -> Result<serde_json::Value, String> {
    match request.method.as_str() {
        "ping" => Ok(json!({ "version": PROTOCOL_VERSION })),
        "refresh" => refresh_data(request.force, request.last_modified.as_deref()),
        "paste" => {
            paste_text_worker(&request.text, request.target_hwnd as isize)?;
            Ok(message_data("已粘贴到原窗口。"))
        }
        "copy" => {
            copy_to_system_clipboard(&request.text)?;
            if let Err(err) = clipboard_store::record_text_and_mark_seen(&request.text) {
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Error,
                    "clipboard_manager_record",
                    format!(
                        "status=failed phase=copy units={} reason={err}",
                        request.text.encode_utf16().count()
                    ),
                );
            }
            Ok(message_data("已复制到剪贴板。"))
        }
        "pin" => {
            clipboard_store::pin_text(&request.text)?;
            Ok(message_data("已置顶。"))
        }
        "unpin" => {
            clipboard_store::unpin_text(&request.text)?;
            Ok(message_data("已取消置顶。"))
        }
        "remove" => match clipboard_store::remove_saved_text(&request.text) {
            Ok(true) => Ok(json!({ "removed": true, "message": "已删除 1 条，可撤销。" })),
            Ok(false) => Ok(json!({ "removed": false, "message": "没有找到要删除的片段。" })),
            Err(err) => Err(format!("删除失败：{err}")),
        },
        "restore" => {
            let result = if request.was_pinned {
                clipboard_store::pin_text(&request.text).map(|_| ())
            } else {
                clipboard_store::record_text(&request.text).map(|_| ())
            };
            result?;
            Ok(message_data("已撤销删除。"))
        }
        "clear_history" => {
            clipboard_store::clear_history()?;
            Ok(message_data("已清空未置顶剪贴板。"))
        }
        "clear_older_than" => {
            let count = clipboard_store::clear_older_than_days(request.days)?;
            Ok(message_data(&format!(
                "已清空 {count} 条 {} 天前的剪贴板。",
                request.days
            )))
        }
        "clear_all" => {
            clipboard_store::clear_all()?;
            Ok(message_data("已清空全部剪贴板。"))
        }
        "pin_current" => match clipboard_store::pin_current_clipboard() {
            Ok(Some(_)) => Ok(message_data("已置顶当前剪贴板。")),
            Ok(None) => Ok(message_data("当前剪贴板没有文本。")),
            Err(err) => Err(format!("置顶当前剪贴板失败：{err}")),
        },
        "get_prefs" => {
            let prefs = request
                .keys
                .iter()
                .filter_map(|key| load_pref(key).map(|value| (key.clone(), value)))
                .collect::<Vec<_>>();
            Ok(json!({ "prefs": prefs }))
        }
        "set_pref" => {
            save_pref(&request.key, &request.value)?;
            Ok(serde_json::Value::Null)
        }
        "get_window_rect" => Ok(match load_saved_window_rect() {
            Some(rect) => json!({
                "rect": { "x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h }
            }),
            None => json!({ "rect": serde_json::Value::Null }),
        }),
        "set_window_rect" => {
            let Some(ref rect) = request.rect else {
                return Err("set_window_rect 缺少 rect 字段".to_string());
            };
            save_window_rect(SavedWindowRect {
                x: rect.x as f32,
                y: rect.y as f32,
                w: rect.w as f32,
                h: rect.h as f32,
            })?;
            Ok(serde_json::Value::Null)
        }
        "shutdown" => Ok(message_data("服务退出。")),
        other => Err(format!("未知方法：{other}")),
    }
}

fn handle_request(request: Request) -> Response {
    let req = request.req;
    let authenticated = EXPECTED_CAPABILITY.get().is_some_and(|expected| {
        pinyin_ime::windows_security::constant_time_eq(expected, &request.capability)
    });
    if !authenticated {
        return Response {
            req,
            ok: false,
            error: Some("请求认证失败".to_string()),
            data: None,
        };
    }
    match execute_request(&request) {
        Ok(data) => Response {
            req,
            ok: true,
            error: None,
            data: Some(data),
        },
        Err(error) => Response {
            req,
            ok: false,
            error: Some(error),
            data: None,
        },
    }
}

// ---------------------------------------------------------------------------
// 命名管道服务（会话级 ACL + REJECT_REMOTE，模式取自 ipc_service.rs）
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn create_pipe_instance(first: bool) -> io::Result<HANDLE> {
    let security = pinyin_ime::windows_security::LocalLogonPipeSecurity::new()?;
    let capability = EXPECTED_CAPABILITY
        .get()
        .ok_or_else(|| io::Error::other("clipboard capability unavailable"))?;
    let pipe_name = format!(
        "{PIPE_NAME_PREFIX}.{}",
        capability.get(..16).unwrap_or(capability)
    );

    let flags = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let handle = unsafe {
        CreateNamedPipeW(
            wide(&pipe_name).as_ptr(),
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            security.as_ptr(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

#[cfg(windows)]
fn read_exact_pipe(handle: HANDLE, buffer: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                buffer[filled..].as_mut_ptr().cast(),
                (buffer.len() - filled) as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE {
                return Ok(false);
            }
            if err == windows_sys::Win32::Foundation::ERROR_NO_DATA {
                return Ok(false);
            }
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        if read == 0 {
            return Ok(false);
        }
        filled += read as usize;
    }
    Ok(true)
}

#[cfg(windows)]
fn write_all_pipe(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
    let mut written_total = 0usize;
    while written_total < bytes.len() {
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                handle,
                bytes[written_total..].as_ptr().cast(),
                (bytes.len() - written_total) as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "pipe write returned 0",
            ));
        }
        written_total += written as usize;
    }
    Ok(())
}

#[cfg(windows)]
fn send_response(handle: HANDLE, response: &Response) -> io::Result<()> {
    let payload = serde_json::to_vec(response)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let header = (payload.len() as u32).to_le_bytes();
    write_all_pipe(handle, &header)?;
    write_all_pipe(handle, &payload)
}

#[cfg(windows)]
fn connection_loop(handle: HANDLE) {
    let mut header = [0u8; 4];
    loop {
        match read_exact_pipe(handle, &mut header) {
            Ok(true) => {}
            Ok(false) => break,
            Err(err) => {
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Error,
                    "clipboard_manager_pipe",
                    format!("status=failed phase=read_header reason={err}"),
                );
                break;
            }
        }
        let length = u32::from_le_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            runtime_log::log_clipboard(
                RuntimeLogLevel::Error,
                "clipboard_manager_pipe",
                format!("status=failed phase=bad_frame length={length}"),
            );
            break;
        }
        let mut payload = vec![0u8; length];
        match read_exact_pipe(handle, &mut payload) {
            Ok(true) => {}
            Ok(false) => break,
            Err(err) => {
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Error,
                    "clipboard_manager_pipe",
                    format!("status=failed phase=read_payload reason={err}"),
                );
                break;
            }
        }
        let request: Request = match serde_json::from_slice(&payload) {
            Ok(request) => request,
            Err(err) => {
                let response = Response {
                    req: 0,
                    ok: false,
                    error: Some(format!("请求解析失败：{err}")),
                    data: None,
                };
                let _ = send_response(handle, &response);
                continue;
            }
        };
        let shutdown = request.method == "shutdown";
        let response = handle_request(request);
        if send_response(handle, &response).is_err() {
            break;
        }
        if shutdown {
            // 响应落盘到管道缓冲并给客户端一点读取时间，避免 Disconnect 时
            // 丢弃未读消息；随后设置退出标志，由 listen_loop 轮询感知。
            unsafe {
                let _ = FlushFileBuffers(handle);
            }
            std::thread::sleep(Duration::from_millis(150));
            SHUTDOWN_REQUESTED.store(true, Ordering::Release);
            break;
        }
    }
    unsafe {
        DisconnectNamedPipe(handle);
        CloseHandle(handle);
    }
}

#[cfg(windows)]
fn listen_loop(generation: &AtomicU64, active: Arc<AtomicU64>) -> io::Result<()> {
    let mut first = true;
    loop {
        if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
            return Ok(());
        }
        let handle = match create_pipe_instance(first) {
            Ok(handle) => handle,
            Err(err) => {
                // 管道已存在 = 另一实例存活，静默退出交由旧实例接管。
                let code = err.raw_os_error().unwrap_or(0);
                if code == ERROR_ACCESS_DENIED as i32 || code == ERROR_ALREADY_EXISTS as i32 {
                    runtime_log::log_clipboard(
                        RuntimeLogLevel::Basic,
                        "clipboard_manager_service_start",
                        "status=exit reason=already_running",
                    );
                    return Ok(());
                }
                return Err(err);
            }
        };
        first = false;
        // PIPE_NOWAIT：无客户端时 ConnectNamedPipe 立即返回
        // ERROR_PIPE_LISTENING，借此轮询 shutdown 标志，避免退出请求被
        // 阻塞的 ConnectNamedPipe 拖到看门狗超时。
        let mut connected = false;
        loop {
            let ok = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
            if ok != 0 {
                connected = true;
                break;
            }
            let err = unsafe { GetLastError() };
            if err == ERROR_PIPE_CONNECTED {
                connected = true;
                break;
            }
            if err != ERROR_PIPE_LISTENING {
                unsafe { CloseHandle(handle) };
                break;
            }
            if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
                unsafe { CloseHandle(handle) };
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
            if connected {
                unsafe { CloseHandle(handle) };
            }
            return Ok(());
        }
        if !connected {
            continue;
        }
        // 已接受的实例切回阻塞模式：NOWAIT 仅用于 ConnectNamedPipe 轮询
        // shutdown 标志，连接建立后恢复常规阻塞读。
        let mode = PIPE_READMODE_BYTE | PIPE_WAIT;
        unsafe {
            let _ =
                SetNamedPipeHandleState(handle, &mode, std::ptr::null_mut(), std::ptr::null_mut());
        }
        generation.fetch_add(1, Ordering::SeqCst);
        active.fetch_add(1, Ordering::SeqCst);
        let active = Arc::clone(&active);
        let _ = std::thread::Builder::new()
            .name("kaixin-clipboard-svc-client".to_string())
            .spawn(move || {
                connection_loop(handle);
                active.fetch_sub(1, Ordering::SeqCst);
            });
    }
}

#[cfg(windows)]
fn orphan_watchdog(generation: &AtomicU64, active: &AtomicU64) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        if SHUTDOWN_REQUESTED.load(Ordering::Acquire) {
            return;
        }
        if active.load(Ordering::Acquire) == 0 {
            let observed = generation.load(Ordering::Acquire);
            let mut idle_secs = 0u64;
            while idle_secs < ORPHAN_EXIT_SECS
                && !SHUTDOWN_REQUESTED.load(Ordering::Acquire)
                && active.load(Ordering::Acquire) == 0
                && generation.load(Ordering::Acquire) == observed
            {
                std::thread::sleep(Duration::from_secs(1));
                idle_secs += 1;
            }
            // UI 崩溃且 10s 内无新连接：退出防孤儿进程。
            if idle_secs >= ORPHAN_EXIT_SECS
                && active.load(Ordering::Acquire) == 0
                && generation.load(Ordering::Acquire) == observed
            {
                runtime_log::log_clipboard(
                    RuntimeLogLevel::Basic,
                    "clipboard_manager_service_stop",
                    "status=exit reason=orphan",
                );
                clipboard_store::flush_pending_ops_sync();
                std::process::exit(0);
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    pinyin_ime::windows_security::apply_process_hardening();
    let capability = std::env::var(CAPABILITY_ENV).unwrap_or_default();
    std::env::remove_var(CAPABILITY_ENV);
    if capability.len() != 64 || !capability.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_manager_service_start",
            "status=failed reason=missing_capability",
        );
        return;
    }
    let _ = EXPECTED_CAPABILITY.set(capability);

    runtime_log::log_clipboard(
        RuntimeLogLevel::Basic,
        "clipboard_manager_service_start",
        format!(
            "status=ok pipe={PIPE_NAME_PREFIX}.* {}",
            clipboard_store::diagnostics_fields()
        ),
    );

    let generation = Arc::new(AtomicU64::new(0));
    let active = Arc::new(AtomicU64::new(0));
    let _ = std::thread::Builder::new()
        .name("kaixin-clipboard-svc-watchdog".to_string())
        .spawn({
            let generation = Arc::clone(&generation);
            let active = Arc::clone(&active);
            move || orphan_watchdog(&generation, &active)
        });
    if let Err(err) = listen_loop(&generation, Arc::clone(&active)) {
        runtime_log::log_clipboard(
            RuntimeLogLevel::Error,
            "clipboard_manager_service_stop",
            format!("status=failed reason={err}"),
        );
    }

    clipboard_store::flush_pending_ops_sync();
    runtime_log::log_clipboard(
        RuntimeLogLevel::Basic,
        "clipboard_manager_service_stop",
        "status=exit reason=shutdown",
    );
}

#[cfg(not(windows))]
fn main() {
    runtime_log::log_clipboard(
        RuntimeLogLevel::Error,
        "clipboard_manager_service_start",
        "status=failed reason=platform_not_supported",
    );
}
