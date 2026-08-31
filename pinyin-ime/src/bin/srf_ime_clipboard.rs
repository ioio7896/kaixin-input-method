#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../fonts.rs"]
mod fonts;

use chrono::{Local, TimeZone};
use eframe::egui::{
    self, Align, Color32, Frame, Key, Layout, Margin, RichText, ScrollArea, Stroke,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
#[cfg(windows)]
use windows_sys::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const WINDOW_TITLE: &str = "开心输入法 剪贴板";
const SERVICE_EXE: &str = "srf_ime_clipboard_svc.exe";
const PIPE_NAME: &str = r"\\.\pipe\KaixinClipboardSvc_v1";
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_millis(900);
const PIPE_CONNECT_TIMEOUT_MS: u32 = 250;
const PIPE_REQUEST_TIMEOUT_MS: u32 = 5_000;
const PIPE_PASTE_TIMEOUT_MS: u32 = 15_000;
const SINGLE_INSTANCE_MUTEX: &str = "Local\\KaixinInput_Clipboard_SingleInstance_v2";
const WINDOW_DEFAULT_SIZE: [f64; 2] = [860.0, 640.0];
const WINDOW_MIN_SIZE: [f64; 2] = [620.0, 440.0];
const WINDOW_WORK_AREA_MARGIN_PX: i32 = 12;

#[derive(Clone, Deserialize)]
struct ClipboardEntry {
    #[serde(default)]
    text: String,
    #[serde(default)]
    captured_at: u64,
    #[serde(default)]
    copy_count: u64,
    #[serde(default)]
    source_app: Option<String>,
}

#[derive(Default, Deserialize)]
struct Snapshot {
    #[serde(default)]
    history: Vec<ClipboardEntry>,
    #[serde(default)]
    pinned: Vec<ClipboardEntry>,
}

#[derive(Serialize)]
struct Request {
    req: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    force: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_hwnd: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    was_pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    days: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keys: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rect: Option<RectDto>,
}

#[derive(Clone, Copy, Serialize)]
struct RectDto {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Request {
    fn new(req: u64, method: &str) -> Self {
        Self {
            req,
            method: method.to_string(),
            force: None,
            last_modified: None,
            text: None,
            target_hwnd: None,
            was_pinned: None,
            days: None,
            key: None,
            value: None,
            keys: None,
            rect: None,
        }
    }
}

#[derive(Deserialize)]
struct Response {
    req: u64,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    data: Value,
}

enum WorkerCommand {
    Call(Request),
    Stop,
}

struct WorkerCompletion {
    req: u64,
    result: Result<Value, String>,
}

struct AsyncServiceClient {
    commands: Sender<WorkerCommand>,
    completions: Receiver<WorkerCompletion>,
    next_req: u64,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AsyncServiceClient {
    fn new(ctx: egui::Context) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (completion_tx, completion_rx) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = Arc::clone(&stopping);
        let worker = thread::Builder::new()
            .name("kaixin-clipboard-client".to_string())
            .spawn(move || service_worker(command_rx, completion_tx, ctx, worker_stopping))
            .ok();
        Self {
            commands: command_tx,
            completions: completion_rx,
            next_req: 1,
            stopping,
            worker,
        }
    }

    fn call(&mut self, method: &str, configure: impl FnOnce(&mut Request)) -> u64 {
        self.next_req = self.next_req.wrapping_add(1).max(1);
        let mut request = Request::new(self.next_req, method);
        configure(&mut request);
        let req = request.req;
        let _ = self.commands.send(WorkerCommand::Call(request));
        req
    }

    fn try_recv(&self) -> Result<WorkerCompletion, TryRecvError> {
        self.completions.try_recv()
    }

    fn shutdown(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.commands.send(WorkerCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn service_worker(
    commands: Receiver<WorkerCommand>,
    completions: Sender<WorkerCompletion>,
    ctx: egui::Context,
    stopping: Arc<AtomicBool>,
) {
    let mut worker = ServiceWorker::new(Arc::clone(&stopping));
    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Call(request) => {
                let req = request.req;
                let result = worker.call(&request);
                let _ = completions.send(WorkerCompletion { req, result });
                ctx.request_repaint();
                if stopping.load(Ordering::Acquire) {
                    worker.shutdown_owned_service();
                    break;
                }
            }
            WorkerCommand::Stop => {
                worker.shutdown_owned_service();
                break;
            }
        }
    }
}

struct ServiceWorker {
    pipe: Option<PipeConnection>,
    child: Option<Child>,
    owns_service: bool,
    stopping: Arc<AtomicBool>,
}

impl ServiceWorker {
    fn new(stopping: Arc<AtomicBool>) -> Self {
        Self {
            pipe: None,
            child: None,
            owns_service: false,
            stopping,
        }
    }

    fn call(&mut self, request: &Request) -> Result<Value, String> {
        let timeout = if request.method == "paste" {
            PIPE_PASTE_TIMEOUT_MS
        } else {
            PIPE_REQUEST_TIMEOUT_MS
        };
        let response = match self.call_once(request, timeout, true) {
            Ok(response) => response,
            Err(first_error) => {
                self.pipe = None;
                self.ensure_service_running()?;
                if request_is_retry_safe(&request.method) {
                    self.call_once(request, timeout, true)?
                } else {
                    return Err(first_error);
                }
            }
        };
        if response.req != request.req {
            return Err("剪贴板服务响应序号不匹配".to_string());
        }
        if response.ok {
            Ok(response.data)
        } else {
            Err(response
                .error
                .unwrap_or_else(|| format!("服务请求失败：{}", request.method)))
        }
    }

    fn call_once(
        &mut self,
        request: &Request,
        timeout_ms: u32,
        cancellable: bool,
    ) -> Result<Response, String> {
        if self.pipe.is_none() {
            self.pipe = Some(PipeConnection::connect()?);
        }
        self.pipe.as_mut().expect("pipe initialized").call(
            request,
            timeout_ms,
            cancellable.then_some(self.stopping.as_ref()),
        )
    }

    fn ensure_service_running(&mut self) -> Result<(), String> {
        if self.owns_service {
            if let Some(mut child) = self.child.take() {
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            self.owns_service = false;
        }
        let service =
            find_service_exe().ok_or_else(|| format!("未找到剪贴板服务：{SERVICE_EXE}"))?;
        let mut command = Command::new(&service);
        command.current_dir(service.parent().unwrap_or_else(|| Path::new(".")));
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        self.child = Some(
            command
                .spawn()
                .map_err(|err| format!("启动剪贴板服务失败：{err}"))?,
        );
        self.owns_service = true;
        self.wait_for_pipe()?;
        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some())
        {
            self.child = None;
            self.owns_service = false;
        }
        Ok(())
    }

    fn wait_for_pipe(&mut self) -> Result<(), String> {
        for _ in 0..20 {
            if self.stopping.load(Ordering::Acquire) {
                return Err("剪贴板服务连接已取消".to_string());
            }
            match PipeConnection::connect() {
                Ok(pipe) => {
                    self.pipe = Some(pipe);
                    return Ok(());
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
        Err("剪贴板服务启动或重连超时".to_string())
    }

    fn shutdown_owned_service(&mut self) {
        if !self.owns_service {
            self.pipe = None;
            return;
        }
        let mut request = Request::new(u64::MAX, "shutdown");
        request.force = Some(false);
        let _ = self.call_once(&request, 2_000, false);
        self.pipe = None;
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    self.owns_service = false;
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        self.owns_service = false;
    }
}

fn request_is_retry_safe(method: &str) -> bool {
    matches!(
        method,
        "ping"
            | "refresh"
            | "get_prefs"
            | "set_pref"
            | "get_window_rect"
            | "set_window_rect"
            | "pin"
            | "unpin"
            | "clear_history"
            | "clear_older_than"
            | "clear_all"
            | "pin_current"
    )
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
struct PipeConnection(HANDLE);

#[cfg(windows)]
impl Drop for PipeConnection {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
            self.0 = INVALID_HANDLE_VALUE;
        }
    }
}

#[cfg(windows)]
impl PipeConnection {
    fn connect() -> Result<Self, String> {
        let pipe_name = wide(PIPE_NAME);
        if unsafe { WaitNamedPipeW(pipe_name.as_ptr(), PIPE_CONNECT_TIMEOUT_MS) } == 0 {
            return Err(format!(
                "等待剪贴板服务失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        let handle = unsafe {
            CreateFileW(
                pipe_name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!(
                "连接剪贴板服务失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(handle))
    }

    fn call(
        &mut self,
        request: &Request,
        timeout_ms: u32,
        stopping: Option<&AtomicBool>,
    ) -> Result<Response, String> {
        let payload =
            serde_json::to_vec(request).map_err(|err| format!("序列化请求失败：{err}"))?;
        if payload.is_empty() || payload.len() > MAX_FRAME_BYTES {
            return Err("剪贴板请求大小非法".to_string());
        }
        write_all_timeout(
            self.0,
            &(payload.len() as u32).to_le_bytes(),
            timeout_ms,
            stopping,
        )?;
        write_all_timeout(self.0, &payload, timeout_ms, stopping)?;
        let mut header = [0u8; 4];
        read_exact_timeout(self.0, &mut header, timeout_ms, stopping)?;
        let length = u32::from_le_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(format!("剪贴板响应大小非法：{length}"));
        }
        let mut response = vec![0u8; length];
        read_exact_timeout(self.0, &mut response, timeout_ms, stopping)?;
        serde_json::from_slice(&response).map_err(|err| format!("解析剪贴板响应失败：{err}"))
    }
}

#[cfg(windows)]
fn write_all_timeout(
    handle: HANDLE,
    mut bytes: &[u8],
    timeout_ms: u32,
    stopping: Option<&AtomicBool>,
) -> Result<(), String> {
    while !bytes.is_empty() {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let started = unsafe {
            WriteFile(
                handle,
                bytes.as_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                std::ptr::null_mut(),
                &mut overlapped,
            )
        };
        let written = finish_overlapped(
            handle,
            &mut overlapped,
            started,
            timeout_ms,
            "写入",
            stopping,
        )?;
        if written == 0 {
            return Err("剪贴板服务写入返回 0 字节".to_string());
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_timeout(
    handle: HANDLE,
    mut bytes: &mut [u8],
    timeout_ms: u32,
    stopping: Option<&AtomicBool>,
) -> Result<(), String> {
    while !bytes.is_empty() {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        let started = unsafe {
            ReadFile(
                handle,
                bytes.as_mut_ptr(),
                bytes.len().min(u32::MAX as usize) as u32,
                std::ptr::null_mut(),
                &mut overlapped,
            )
        };
        let read = finish_overlapped(
            handle,
            &mut overlapped,
            started,
            timeout_ms,
            "读取",
            stopping,
        )?;
        if read == 0 {
            return Err("剪贴板服务已断开".to_string());
        }
        let (_, rest) = bytes.split_at_mut(read as usize);
        bytes = rest;
    }
    Ok(())
}

#[cfg(not(windows))]
struct PipeConnection;

#[cfg(not(windows))]
impl PipeConnection {
    fn connect() -> Result<Self, String> {
        Err("剪贴板管理器仅支持 Windows".to_string())
    }

    fn call(
        &mut self,
        _request: &Request,
        _timeout_ms: u32,
        _stopping: Option<&AtomicBool>,
    ) -> Result<Response, String> {
        Err("剪贴板管理器仅支持 Windows".to_string())
    }
}

#[cfg(windows)]
fn finish_overlapped(
    handle: HANDLE,
    overlapped: &mut OVERLAPPED,
    started: i32,
    timeout_ms: u32,
    phase: &str,
    stopping: Option<&AtomicBool>,
) -> Result<u32, String> {
    if started == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_IO_PENDING {
            return Err(format!(
                "剪贴板服务管道{phase}失败：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut transferred = 0u32;
    loop {
        if stopping.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            unsafe {
                CancelIoEx(handle, overlapped);
                GetOverlappedResult(handle, overlapped, &mut transferred, 1);
            }
            return Err(format!("剪贴板服务管道{phase}已取消"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            unsafe {
                CancelIoEx(handle, overlapped);
                GetOverlappedResult(handle, overlapped, &mut transferred, 1);
            }
            return Err(format!("剪贴板服务无响应（管道{phase}超时）"));
        }
        let slice_ms = remaining.as_millis().clamp(1, 100) as u32;
        let completed =
            unsafe { GetOverlappedResultEx(handle, overlapped, &mut transferred, slice_ms, 0) };
        if completed != 0 {
            return Ok(transferred);
        }
        let error = unsafe { GetLastError() };
        if error != WAIT_TIMEOUT {
            return Err(format!(
                "剪贴板服务管道{phase}失败：{}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
    }
}

fn find_service_exe() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            candidates.push(parent.join(SERVICE_EXE));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs/kaixin")
                .join(SERVICE_EXE),
        );
    }
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = std::env::var_os(key) {
            candidates.push(PathBuf::from(root).join("kaixin").join(SERVICE_EXE));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Recent,
    Pinned,
}

#[derive(Clone, Copy)]
enum Action {
    Paste,
    Copy,
    TogglePin,
    Remove,
}

enum PendingKind {
    Refresh,
    Mutation {
        removed: Option<(String, bool)>,
        close_on_success: bool,
    },
    Preferences,
    WindowRect,
    Quiet,
}

struct ClipboardApp {
    client: AsyncServiceClient,
    pending: HashMap<u64, PendingKind>,
    snapshot: Snapshot,
    pinned_texts: HashSet<String>,
    tab: Tab,
    search: String,
    filter: String,
    selected: usize,
    target_hwnd: isize,
    modified: String,
    max_age_days: u64,
    last_refresh: Instant,
    status: String,
    status_error: bool,
    last_removed: Option<(String, bool)>,
    dark: bool,
    dense: bool,
    quick_paste: bool,
    refresh_pending: bool,
    window_rect_loaded: bool,
    last_saved_rect: Option<RectDto>,
    last_window_save: Instant,
}

impl ClipboardApp {
    fn new(cc: &eframe::CreationContext<'_>, target_hwnd: isize) -> Self {
        let _ = fonts::install_cjk_fonts(&cc.egui_ctx);
        let mut app = Self {
            client: AsyncServiceClient::new(cc.egui_ctx.clone()),
            pending: HashMap::new(),
            snapshot: Snapshot::default(),
            pinned_texts: HashSet::new(),
            tab: Tab::Recent,
            search: String::new(),
            filter: String::new(),
            selected: 0,
            target_hwnd,
            modified: String::new(),
            max_age_days: 30,
            last_refresh: Instant::now() - REFRESH_INTERVAL,
            status: "正在连接剪贴板服务…".to_string(),
            status_error: false,
            last_removed: None,
            dark: false,
            dense: false,
            quick_paste: true,
            refresh_pending: false,
            window_rect_loaded: false,
            last_saved_rect: None,
            last_window_save: Instant::now(),
        };
        app.request_refresh(true);
        let prefs = app.client.call("get_prefs", |request| {
            request.keys = Some(vec![
                "theme_mode".to_string(),
                "dense_rows".to_string(),
                "quick_paste".to_string(),
            ]);
        });
        app.pending.insert(prefs, PendingKind::Preferences);
        let rect = app.client.call("get_window_rect", |_| {});
        app.pending.insert(rect, PendingKind::WindowRect);
        app
    }

    fn request_refresh(&mut self, force: bool) {
        if self.refresh_pending {
            return;
        }
        let previous = self.modified.clone();
        let req = self.client.call("refresh", |request| {
            request.force = Some(force);
            request.last_modified = Some(previous);
        });
        self.pending.insert(req, PendingKind::Refresh);
        self.refresh_pending = true;
        self.last_refresh = Instant::now();
    }

    fn apply_refresh(&mut self, data: Value) -> Result<(), String> {
        self.modified = data
            .get("modified")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.max_age_days = data
            .get("max_age_days")
            .and_then(Value::as_u64)
            .unwrap_or(30);
        if data.get("changed").and_then(Value::as_bool).unwrap_or(true) {
            self.snapshot =
                serde_json::from_value(data.get("snapshot").cloned().unwrap_or_else(|| json!({})))
                    .map_err(|err| format!("解析剪贴板记录失败：{err}"))?;
            self.pinned_texts = self
                .snapshot
                .pinned
                .iter()
                .map(|entry| entry.text.clone())
                .collect();
            self.clamp_selection();
        }
        self.status = "就绪".to_string();
        self.status_error = false;
        Ok(())
    }

    fn entries(&self) -> Vec<&ClipboardEntry> {
        let source = if self.tab == Tab::Recent {
            &self.snapshot.history
        } else {
            &self.snapshot.pinned
        };
        source
            .iter()
            .filter(|entry| entry_matches(entry, &self.search, &self.filter))
            .collect()
    }

    fn selected_entry(&self) -> Option<ClipboardEntry> {
        self.entries()
            .get(self.selected)
            .map(|entry| (*entry).clone())
    }

    fn clamp_selection(&mut self) {
        let len = self.entries().len();
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    fn mutate(
        &mut self,
        method: &str,
        configure: impl FnOnce(&mut Request),
        removed: Option<(String, bool)>,
        close_on_success: bool,
    ) {
        let req = self.client.call(method, configure);
        self.pending.insert(
            req,
            PendingKind::Mutation {
                removed,
                close_on_success,
            },
        );
        self.status = "正在处理…".to_string();
        self.status_error = false;
    }

    fn save_pref(&mut self, key: &str, value: &str) {
        let key_owned = key.to_string();
        let value_owned = value.to_string();
        let req = self.client.call("set_pref", |request| {
            request.key = Some(key_owned);
            request.value = Some(value_owned);
        });
        self.pending.insert(req, PendingKind::Quiet);
    }

    fn process_completions(&mut self, ctx: &egui::Context) {
        let mut completed = Vec::new();
        loop {
            match self.client.try_recv() {
                Ok(completion) => completed.push(completion),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        for completion in completed {
            let Some(kind) = self.pending.remove(&completion.req) else {
                continue;
            };
            if matches!(kind, PendingKind::Refresh) {
                self.refresh_pending = false;
            }
            let data = match completion.result {
                Ok(data) => data,
                Err(err) => {
                    self.set_error(err);
                    continue;
                }
            };
            match kind {
                PendingKind::Refresh => {
                    if let Err(err) = self.apply_refresh(data) {
                        self.set_error(err);
                    }
                }
                PendingKind::Mutation {
                    removed,
                    close_on_success,
                } => {
                    self.status = data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("操作完成")
                        .to_string();
                    self.status_error = false;
                    if let Some(removed) = removed {
                        self.last_removed = Some(removed);
                    }
                    self.modified.clear();
                    self.request_refresh(false);
                    if close_on_success {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                PendingKind::Preferences => self.apply_preferences(&data),
                PendingKind::WindowRect => self.apply_window_rect(&data, ctx),
                PendingKind::Quiet => {}
            }
        }
    }

    fn apply_preferences(&mut self, data: &Value) {
        let Some(values) = data.get("prefs").and_then(Value::as_array) else {
            return;
        };
        for pair in values {
            let Some(pair) = pair.as_array() else {
                continue;
            };
            let (Some(key), Some(value)) = (
                pair.first().and_then(Value::as_str),
                pair.get(1).and_then(Value::as_str),
            ) else {
                continue;
            };
            match key {
                "theme_mode" => self.dark = value == "dark",
                "dense_rows" => self.dense = value == "1",
                "quick_paste" => self.quick_paste = value != "0",
                _ => {}
            }
        }
    }

    fn apply_window_rect(&mut self, data: &Value, ctx: &egui::Context) {
        if let Some(rect) = data.get("rect").filter(|value| value.is_object()) {
            let saved = RectDto {
                x: rect.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                y: rect.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                w: rect.get("w").and_then(Value::as_f64).unwrap_or(860.0),
                h: rect.get("h").and_then(Value::as_f64).unwrap_or(640.0),
            };
            let pixels_per_point = ctx
                .input(|input| input.viewport().native_pixels_per_point)
                .unwrap_or(1.0);
            let restored = constrain_window_rect_to_work_area(saved, pixels_per_point);
            if restored.w > 0.0 && restored.h > 0.0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                    WINDOW_MIN_SIZE[0].min(restored.w) as f32,
                    WINDOW_MIN_SIZE[1].min(restored.h) as f32,
                )));
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    restored.x as f32,
                    restored.y as f32,
                )));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    restored.w as f32,
                    restored.h as f32,
                )));
                if !rect_close(saved, restored) {
                    let req = self.client.call("set_window_rect", |request| {
                        request.rect = Some(restored);
                    });
                    self.pending.insert(req, PendingKind::Quiet);
                }
                self.last_saved_rect = Some(restored);
            }
        }
        self.window_rect_loaded = true;
    }

    fn maybe_save_window_rect(&mut self, ctx: &egui::Context) {
        if !self.window_rect_loaded || self.last_window_save.elapsed() < Duration::from_secs(1) {
            return;
        }
        let Some((outer, inner)) =
            ctx.input(|input| input.viewport().outer_rect.zip(input.viewport().inner_rect))
        else {
            return;
        };
        let current = RectDto {
            x: outer.min.x as f64,
            y: outer.min.y as f64,
            w: inner.width() as f64,
            h: inner.height() as f64,
        };
        if self
            .last_saved_rect
            .is_some_and(|saved| rect_close(saved, current))
        {
            return;
        }
        let req = self.client.call("set_window_rect", |request| {
            request.rect = Some(current);
        });
        self.pending.insert(req, PendingKind::Quiet);
        self.last_saved_rect = Some(current);
        self.last_window_save = Instant::now();
    }

    fn apply_action(&mut self, action: Action, entry: ClipboardEntry, _ctx: &egui::Context) {
        let text = entry.text.clone();
        match action {
            Action::Paste => {
                let target = self.target_hwnd as i64;
                self.mutate(
                    "paste",
                    |request| {
                        request.text = Some(text);
                        request.target_hwnd = Some(target);
                    },
                    None,
                    true,
                );
            }
            Action::Copy => {
                self.mutate("copy", |request| request.text = Some(text), None, false);
            }
            Action::TogglePin => {
                let pinned = self.pinned_texts.contains(&text);
                self.mutate(
                    if pinned { "unpin" } else { "pin" },
                    |request| request.text = Some(text),
                    None,
                    false,
                );
            }
            Action::Remove => {
                let pinned = self.pinned_texts.contains(&text);
                let request_text = text.clone();
                self.mutate(
                    "remove",
                    |request| request.text = Some(request_text),
                    Some((text, pinned)),
                    false,
                );
            }
        }
    }

    fn set_error(&mut self, error: String) {
        self.status = error;
        self.status_error = true;
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.key_pressed(Key::ArrowDown)) {
            self.selected = (self.selected + 1).min(self.entries().len().saturating_sub(1));
        }
        if ctx.input(|input| input.key_pressed(Key::ArrowUp)) {
            self.selected = self.selected.saturating_sub(1);
        }
        if ctx.input(|input| input.key_pressed(Key::Enter)) {
            if let Some(entry) = self.selected_entry() {
                self.apply_action(Action::Paste, entry, ctx);
            }
        }
        if ctx.input(|input| input.key_pressed(Key::Delete)) {
            if let Some(entry) = self.selected_entry() {
                self.apply_action(Action::Remove, entry, ctx);
            }
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.tab == Tab::Recent && self.quick_paste {
            let keys = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ];
            for (index, key) in keys.into_iter().enumerate() {
                if ctx.input(|input| input.key_pressed(key)) {
                    if let Some(entry) = self.entries().get(index).map(|entry| (*entry).clone()) {
                        self.apply_action(Action::Paste, entry, ctx);
                    }
                }
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("剪贴板记录");
            ui.label(format!(
                "最近 {} · 置顶 {}",
                self.snapshot.history.len(),
                self.snapshot.pinned.len()
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(if self.dark { "浅色" } else { "深色" }).clicked() {
                    self.dark = !self.dark;
                    let value = if self.dark { "dark" } else { "light" };
                    self.save_pref("theme_mode", value);
                }
                if ui.button("刷新").clicked() {
                    self.request_refresh(true);
                }
                ui.menu_button("更多", |ui| {
                    if ui.button("置顶当前剪贴板").clicked() {
                        self.mutate("pin_current", |_| {}, None, false);
                        ui.close_menu();
                    }
                    if ui.checkbox(&mut self.dense, "紧凑行高").changed() {
                        let value = if self.dense { "1" } else { "0" };
                        self.save_pref("dense_rows", value);
                    }
                    if ui
                        .checkbox(&mut self.quick_paste, "数字键快速粘贴")
                        .changed()
                    {
                        let value = if self.quick_paste { "1" } else { "0" };
                        self.save_pref("quick_paste", value);
                    }
                    if ui.button("清空未置顶历史").clicked() {
                        self.mutate("clear_history", |_| {}, None, false);
                        ui.close_menu();
                    }
                    if ui
                        .button(format!("清空 {} 天前记录", self.max_age_days))
                        .clicked()
                    {
                        let days = self.max_age_days;
                        self.mutate(
                            "clear_older_than",
                            |request| request.days = Some(days),
                            None,
                            false,
                        );
                        ui.close_menu();
                    }
                    if ui.button("清空全部").clicked() {
                        self.mutate("clear_all", |_| {}, None, false);
                        ui.close_menu();
                    }
                });
            });
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("搜索");
            let response = ui.add_sized(
                [ui.available_width() - 8.0, 30.0],
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("搜索内容、网址、代码或路径…"),
            );
            if response.changed() {
                self.selected = 0;
            }
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.set_width(112.0);
        ui.label(RichText::new("查看").strong());
        if ui
            .selectable_label(
                self.tab == Tab::Recent,
                format!("最近  {}", self.snapshot.history.len()),
            )
            .clicked()
        {
            self.tab = Tab::Recent;
            self.selected = 0;
        }
        if ui
            .selectable_label(
                self.tab == Tab::Pinned,
                format!("置顶  {}", self.snapshot.pinned.len()),
            )
            .clicked()
        {
            self.tab = Tab::Pinned;
            self.selected = 0;
        }
        ui.add_space(16.0);
        ui.label(RichText::new("筛选").strong());
        for (label, value) in [
            ("全部", ""),
            ("今天", "今天"),
            ("近 7 天", "7d"),
            ("网址", "type:url"),
            ("代码", "type:code"),
            ("路径", "type:path"),
            ("邮箱", "type:email"),
        ] {
            if ui.selectable_label(self.filter == value, label).clicked() {
                self.filter = value.to_string();
                self.selected = 0;
            }
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let entries: Vec<ClipboardEntry> = self.entries().into_iter().cloned().collect();
        if entries.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("没有匹配内容，换个关键词试试。");
            });
            return;
        }
        ScrollArea::vertical().show(ui, |ui| {
            for (index, entry) in entries.into_iter().enumerate() {
                let selected = index == self.selected;
                Frame::none()
                    .fill(if selected {
                        ui.visuals().selection.bg_fill
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::new(
                        1.0,
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(if self.dense { 6.0 } else { 10.0 }))
                    .rounding(6.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if self.tab == Tab::Recent && index < 9 {
                                ui.label(RichText::new(format!("{}", index + 1)).strong());
                            }
                            ui.label(RichText::new(kind_label(&entry.text)).small());
                            ui.label(RichText::new(entry_meta(&entry)).small().weak());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("删").clicked() {
                                    self.apply_action(Action::Remove, entry.clone(), ctx);
                                }
                                if ui.small_button("粘").clicked() {
                                    self.apply_action(Action::Paste, entry.clone(), ctx);
                                }
                                if ui.small_button("复制").clicked() {
                                    self.apply_action(Action::Copy, entry.clone(), ctx);
                                }
                                let pin = if self.pinned_texts.contains(&entry.text) {
                                    "★"
                                } else {
                                    "☆"
                                };
                                if ui.small_button(pin).clicked() {
                                    self.apply_action(Action::TogglePin, entry.clone(), ctx);
                                }
                            });
                        });
                        let preview = display_text(&entry.text);
                        let preview = if preview.chars().count() > 220 {
                            format!("{}…", preview.chars().take(220).collect::<String>())
                        } else {
                            preview
                        };
                        let response =
                            ui.add(egui::Label::new(preview).wrap().sense(egui::Sense::click()));
                        if response.clicked() {
                            self.selected = index;
                        }
                        if response.double_clicked() {
                            self.apply_action(Action::Paste, entry, ctx);
                        }
                    });
                ui.add_space(5.0);
            }
        });
    }
}

impl eframe::App for ClipboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_completions(ctx);
        if self.dark {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }
        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.request_refresh(false);
        }
        self.maybe_save_window_rect(ctx);
        self.handle_keys(ctx);
        egui::TopBottomPanel::top("top")
            .frame(Frame::side_top_panel(&ctx.style()).inner_margin(12.0))
            .show(ctx, |ui| self.top_bar(ui));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let color = if self.status_error {
                    Color32::from_rgb(190, 45, 45)
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.colored_label(color, &self.status);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label("数字键 1–9 快粘");
                    if let Some((text, pinned)) = self.last_removed.clone() {
                        if ui.button("撤销删除").clicked() {
                            self.mutate(
                                "restore",
                                |request| {
                                    request.text = Some(text);
                                    request.was_pinned = Some(pinned);
                                },
                                None,
                                false,
                            );
                            self.last_removed = None;
                        }
                    }
                });
            });
        });
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .show(ctx, |ui| self.sidebar(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.list(ui, ctx));
        ctx.request_repaint_after(REFRESH_INTERVAL);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.client.shutdown();
    }
}

fn rect_close(left: RectDto, right: RectDto) -> bool {
    (left.x - right.x).abs() < 1.0
        && (left.y - right.y).abs() < 1.0
        && (left.w - right.w).abs() < 1.0
        && (left.h - right.h).abs() < 1.0
}

#[cfg(windows)]
fn constrain_window_rect_to_work_area(saved: RectDto, pixels_per_point: f32) -> RectDto {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromRect, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL,
        MONITOR_DEFAULTTOPRIMARY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRectEx, WS_EX_APPWINDOW, WS_OVERLAPPEDWINDOW,
    };

    let scale = if pixels_per_point.is_finite() {
        f64::from(pixels_per_point).clamp(0.25, 8.0)
    } else {
        1.0
    };
    let finite_position = saved.x.is_finite() && saved.y.is_finite();
    let desired_width = if saved.w.is_finite() && saved.w > 0.0 {
        saved.w
    } else {
        WINDOW_DEFAULT_SIZE[0]
    };
    let desired_height = if saved.h.is_finite() && saved.h > 0.0 {
        saved.h
    } else {
        WINDOW_DEFAULT_SIZE[1]
    };
    let desired_x_px = if finite_position {
        (saved.x * scale).round() as i32
    } else {
        0
    };
    let desired_y_px = if finite_position {
        (saved.y * scale).round() as i32
    } else {
        0
    };
    let desired_rect = RECT {
        left: desired_x_px,
        top: desired_y_px,
        right: desired_x_px.saturating_add((desired_width * scale).round() as i32),
        bottom: desired_y_px.saturating_add((desired_height * scale).round() as i32),
    };
    let saved_monitor = if finite_position {
        unsafe { MonitorFromRect(&desired_rect, MONITOR_DEFAULTTONULL) }
    } else {
        0
    };
    let monitor = if saved_monitor != 0 {
        saved_monitor
    } else {
        unsafe { MonitorFromWindow(0, MONITOR_DEFAULTTOPRIMARY) }
    };
    if monitor == 0 {
        return RectDto {
            x: 0.0,
            y: 0.0,
            w: WINDOW_DEFAULT_SIZE[0],
            h: WINDOW_DEFAULT_SIZE[1],
        };
    }
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return RectDto {
            x: 0.0,
            y: 0.0,
            w: WINDOW_DEFAULT_SIZE[0],
            h: WINDOW_DEFAULT_SIZE[1],
        };
    }

    let mut decoration = RECT {
        left: 0,
        top: 0,
        right: (WINDOW_DEFAULT_SIZE[0] * scale).round() as i32,
        bottom: (WINDOW_DEFAULT_SIZE[1] * scale).round() as i32,
    };
    let adjusted =
        unsafe { AdjustWindowRectEx(&mut decoration, WS_OVERLAPPEDWINDOW, 0, WS_EX_APPWINDOW) }
            != 0;
    let chrome_width = if adjusted {
        (decoration.right - decoration.left)
            .saturating_sub((WINDOW_DEFAULT_SIZE[0] * scale).round() as i32)
            .max(0)
    } else {
        0
    };
    let chrome_height = if adjusted {
        (decoration.bottom - decoration.top)
            .saturating_sub((WINDOW_DEFAULT_SIZE[1] * scale).round() as i32)
            .max(0)
    } else {
        0
    };
    let available_outer_width = (info.rcWork.right - info.rcWork.left)
        .saturating_sub(WINDOW_WORK_AREA_MARGIN_PX * 2)
        .max(1);
    let available_outer_height = (info.rcWork.bottom - info.rcWork.top)
        .saturating_sub(WINDOW_WORK_AREA_MARGIN_PX * 2)
        .max(1);
    let max_inner_width = available_outer_width.saturating_sub(chrome_width).max(1);
    let max_inner_height = available_outer_height.saturating_sub(chrome_height).max(1);
    let min_inner_width = ((WINDOW_MIN_SIZE[0] * scale).round() as i32).min(max_inner_width);
    let min_inner_height = ((WINDOW_MIN_SIZE[1] * scale).round() as i32).min(max_inner_height);
    let inner_width =
        ((desired_width * scale).round() as i32).clamp(min_inner_width.max(1), max_inner_width);
    let inner_height =
        ((desired_height * scale).round() as i32).clamp(min_inner_height.max(1), max_inner_height);
    let outer_width = inner_width.saturating_add(chrome_width);
    let outer_height = inner_height.saturating_add(chrome_height);
    let min_x = info.rcWork.left.saturating_add(WINDOW_WORK_AREA_MARGIN_PX);
    let min_y = info.rcWork.top.saturating_add(WINDOW_WORK_AREA_MARGIN_PX);
    let max_x = info
        .rcWork
        .right
        .saturating_sub(WINDOW_WORK_AREA_MARGIN_PX)
        .saturating_sub(outer_width)
        .max(min_x);
    let max_y = info
        .rcWork
        .bottom
        .saturating_sub(WINDOW_WORK_AREA_MARGIN_PX)
        .saturating_sub(outer_height)
        .max(min_y);
    let (x_px, y_px) = if saved_monitor == 0 {
        (
            min_x.saturating_add((max_x - min_x) / 2),
            min_y.saturating_add((max_y - min_y) / 2),
        )
    } else {
        (
            desired_x_px.clamp(min_x, max_x),
            desired_y_px.clamp(min_y, max_y),
        )
    };
    RectDto {
        x: f64::from(x_px) / scale,
        y: f64::from(y_px) / scale,
        w: f64::from(inner_width) / scale,
        h: f64::from(inner_height) / scale,
    }
}

#[cfg(not(windows))]
fn constrain_window_rect_to_work_area(saved: RectDto, _pixels_per_point: f32) -> RectDto {
    RectDto {
        x: if saved.x.is_finite() { saved.x } else { 0.0 },
        y: if saved.y.is_finite() { saved.y } else { 0.0 },
        w: if saved.w.is_finite() {
            saved.w.max(WINDOW_MIN_SIZE[0])
        } else {
            WINDOW_DEFAULT_SIZE[0]
        },
        h: if saved.h.is_finite() {
            saved.h.max(WINDOW_MIN_SIZE[1])
        } else {
            WINDOW_DEFAULT_SIZE[1]
        },
    }
}

fn entry_matches(entry: &ClipboardEntry, search: &str, filter: &str) -> bool {
    let search = search.trim().to_lowercase();
    if !search.is_empty()
        && !entry.text.to_lowercase().contains(&search)
        && !entry
            .source_app
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains(&search)
    {
        return false;
    }
    match filter {
        "" => true,
        "今天" => timestamp_within_days(entry.captured_at, 0),
        "7d" => timestamp_within_days(entry.captured_at, 7),
        "type:url" => looks_like_url(&entry.text),
        "type:email" => looks_like_email(&entry.text),
        "type:path" => looks_like_path(&entry.text),
        "type:code" => looks_like_code(&entry.text),
        _ => entry.text.to_lowercase().contains(&filter.to_lowercase()),
    }
}

fn timestamp_within_days(timestamp: u64, days: i64) -> bool {
    Local
        .timestamp_opt(timestamp as i64, 0)
        .single()
        .is_some_and(|when| {
            let age = Local::now()
                .date_naive()
                .signed_duration_since(when.date_naive())
                .num_days();
            age >= 0 && age <= days
        })
}

fn entry_meta(entry: &ClipboardEntry) -> String {
    let mut parts = vec![
        timestamp_label(entry.captured_at),
        format!("{} 字", entry.text.chars().count()),
    ];
    if entry.copy_count > 1 {
        parts.push(format!("复制 {} 次", entry.copy_count));
    }
    if let Some(source) = entry
        .source_app
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        parts.push(source.to_string());
    }
    parts.join(" · ")
}

fn timestamp_label(timestamp: u64) -> String {
    let Some(when) = Local.timestamp_opt(timestamp as i64, 0).single() else {
        return "-".to_string();
    };
    let now = Local::now();
    let seconds = (now - when).num_seconds().max(0);
    if seconds < 10 {
        "刚刚".to_string()
    } else if seconds < 60 {
        format!("{seconds} 秒前")
    } else if seconds < 3600 {
        format!("{} 分钟前", seconds / 60)
    } else if seconds < 86400 && when.date_naive() == now.date_naive() {
        format!("{} 小时前", seconds / 3600)
    } else {
        when.format("%m-%d %H:%M").to_string()
    }
}

fn kind_label(text: &str) -> &'static str {
    if looks_like_url(text) {
        "网址"
    } else if looks_like_email(text) {
        "邮箱"
    } else if looks_like_path(text) {
        "路径"
    } else if looks_like_code(text) {
        "代码"
    } else {
        "文本"
    }
}

fn looks_like_url(text: &str) -> bool {
    let value = text.trim_start().to_lowercase();
    value.starts_with("http://")
        || value.starts_with("https://")
        || value.starts_with("www.")
        || value.contains("://")
}

fn looks_like_email(text: &str) -> bool {
    text.split_once('@')
        .is_some_and(|(left, right)| !left.is_empty() && right.contains('.'))
}

fn looks_like_path(text: &str) -> bool {
    let value = text.trim();
    (value.len() > 1 && value.as_bytes().get(1) == Some(&b':'))
        || value.starts_with("\\\\")
        || value.starts_with('/')
        || value.contains('\\')
}

fn looks_like_code(text: &str) -> bool {
    let value = text.trim();
    value.contains("```")
        || value.contains("=>")
        || value.contains("::")
        || value.contains("fn ")
        || value.contains("function ")
        || (value.contains('{') && value.contains('}'))
        || (value.contains(';') && value.contains('='))
}

fn display_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch == '\t' || (ch < ' ' && ch != '\n' && ch != '\r') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn main() -> eframe::Result<()> {
    let _single_instance_guard = match pinyin_ime::win_single_instance::claim_or_activate_existing(
        SINGLE_INSTANCE_MUTEX,
        WINDOW_TITLE,
    ) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => return Ok(()),
        Err(_) => None,
    };
    #[cfg(windows)]
    let target_hwnd = unsafe { GetForegroundWindow() } as isize;
    #[cfg(not(windows))]
    let target_hwnd = 0;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(WINDOW_TITLE)
            .with_inner_size(WINDOW_DEFAULT_SIZE.map(|value| value as f32))
            .with_min_inner_size(WINDOW_MIN_SIZE.map(|value| value as f32)),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(move |cc| Ok(Box::new(ClipboardApp::new(cc, target_hwnd)))),
    )
}
