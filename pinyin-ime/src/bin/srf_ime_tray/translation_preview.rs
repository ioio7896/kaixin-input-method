//! Shared candidate / selection / OCR translation preview.
//! Runs as a single persistent warm window accepting handoffs over a current-user named pipe.
use eframe::egui;
use pinyin_ime::{
    external_translation as bridge, tool_prefs, ui_theme, win_handle::OwnedWinHandle, win_paste,
    win_single_instance, windows_security::LocalLogonPipeSecurity,
};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{ReadFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND},
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    },
    System::SystemInformation::GetTickCount64,
    System::IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
    UI::{Input::KeyboardAndMouse::GetAsyncKeyState, WindowsAndMessaging::*},
};

#[path = "../../fonts.rs"]
mod fonts;

const PREVIEW_MUTEX_NAME: &str = "Local\\KaixinInput_TranslationPreview_Mutex";
const IDLE_EXIT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

static KEY_TARGET: AtomicIsize = AtomicIsize::new(0);
static OWN_PID: AtomicU32 = AtomicU32::new(0);
static KEY_READY: AtomicBool = AtomicBool::new(false);
static KEY_CONFIRM: AtomicBool = AtomicBool::new(false);
static KEY_CANCEL: AtomicBool = AtomicBool::new(false);
static KEY_EDITED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn preview_key(code: i32, message: usize, data: isize) -> isize {
    if code >= 0
        && GetForegroundWindow() == KEY_TARGET.load(Ordering::Relaxed)
        && ![0x10, 0x11, 0x12]
            .iter()
            .any(|key| GetAsyncKeyState(*key) < 0)
    {
        // SAFETY: Windows supplies a valid KBDLLHOOKSTRUCT when code >= 0.
        let key = &*(data as *const KBDLLHOOKSTRUCT);
        if key.flags & LLKHF_INJECTED == 0
            && (key.vkCode == 0x1B || (key.vkCode == 0x0D && KEY_READY.load(Ordering::Relaxed)))
        {
            if message == WM_KEYDOWN as usize {
                if key.vkCode == 0x1B {
                    KEY_CANCEL.store(true, Ordering::Release);
                } else {
                    KEY_CONFIRM.store(true, Ordering::Release);
                }
            }
            return 1;
        }
        if message == WM_KEYDOWN as usize && (key.flags & LLKHF_INJECTED == 0) {
            KEY_EDITED.store(true, Ordering::Release);
        }
    }
    CallNextHookEx(0, code, message, data)
}

unsafe extern "system" fn preview_mouse(code: i32, message: usize, data: isize) -> isize {
    // Only invalidate on actual mouse button clicks outside our preview window;
    // ignore WM_MOUSEMOVE so moving the cursor toward preview buttons never locks them out.
    if code >= 0
        && matches!(
            message as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        )
    {
        let info = &*(data as *const MSLLHOOKSTRUCT);
        let hwnd = WindowFromPoint(info.pt);
        let mut pid = 0u32;
        if hwnd != 0 {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        let own_pid = OWN_PID.load(Ordering::Relaxed);
        if pid != 0 && pid != own_pid {
            KEY_EDITED.store(true, Ordering::Release);
        }
    }
    CallNextHookEx(0, code, message, data)
}

struct InputHooks {
    kbd: isize,
    mouse: isize,
}

impl InputHooks {
    fn install(target: isize) -> Self {
        KEY_TARGET.store(target, Ordering::Relaxed);
        OWN_PID.store(std::process::id(), Ordering::Relaxed);
        KEY_CONFIRM.store(false, Ordering::Release);
        KEY_CANCEL.store(false, Ordering::Release);
        KEY_EDITED.store(false, Ordering::Release);
        KEY_READY.store(false, Ordering::Release);
        let module = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null())
        };
        let kbd = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(preview_key), module, 0) };
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(preview_mouse), module, 0) };
        Self { kbd, mouse }
    }
}

impl Drop for InputHooks {
    fn drop(&mut self) {
        unsafe {
            if self.kbd != 0 {
                UnhookWindowsHookEx(self.kbd);
            }
            if self.mouse != 0 {
                UnhookWindowsHookEx(self.mouse);
            }
        }
        KEY_TARGET.store(0, Ordering::Relaxed);
        KEY_READY.store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Caret {
    focus: isize,
    caret: isize,
    rect: [i32; 4],
}

fn caret(hwnd: isize) -> Option<Caret> {
    if hwnd == 0 {
        return None;
    }
    unsafe {
        let thread = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if thread == 0
            || GetGUIThreadInfo(thread, &mut info) == 0
            || info.hwndFocus == 0
            || info.hwndCaret == 0
        {
            return None;
        }
        Some(Caret {
            focus: info.hwndFocus,
            caret: info.hwndCaret,
            rect: [
                info.rcCaret.left,
                info.rcCaret.top,
                info.rcCaret.right,
                info.rcCaret.bottom,
            ],
        })
    }
}

fn clipboard_sequence() -> u32 {
    unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
}

fn next_preview_generation(previous: u64, uptime_ms: u64) -> u64 {
    // Keep requests ordered even when they share a clock tick or the preview restarts.
    previous.saturating_add(1).max(uptime_ms)
}

fn target_changed(
    front: isize,
    target: isize,
    own: bool,
    source: isize,
    edited: bool,
    caret_changed: bool,
) -> bool {
    !own && !(source != 0 && front == source) && (front != target || edited || caret_changed)
}

fn clamp_position_for_window(hwnd: isize, screen_x: i32, screen_y: i32) -> Option<egui::Pos2> {
    unsafe {
        let monitor = if hwnd != 0 {
            windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(hwnd, 2)
        } else {
            let pt = windows_sys::Win32::Foundation::POINT {
                x: screen_x,
                y: screen_y,
            };
            windows_sys::Win32::Graphics::Gdi::MonitorFromPoint(pt, 2)
        };
        let mut info: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        if windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let scale = if hwnd != 0 {
            windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0
        } else {
            1.0
        };
        let w = (540.0 * scale) as i32;
        let h = (460.0 * scale) as i32;
        let x = screen_x
            .max(info.rcWork.left)
            .min((info.rcWork.right - w).max(info.rcWork.left));
        let y = screen_y
            .max(info.rcWork.top)
            .min((info.rcWork.bottom - h).max(info.rcWork.top));
        Some(egui::pos2(x as f32 / scale, y as f32 / scale))
    }
}

fn compute_anchor_position(hwnd: isize, explicit_caret: Option<[i32; 4]>) -> Option<egui::Pos2> {
    // 1. Prefer exact TSF screen caret rect when supplied.
    if let Some([left, _top, _right, bottom]) = explicit_caret {
        if left != 0 || bottom != 0 {
            return clamp_position_for_window(hwnd, left, bottom + 8);
        }
    }
    // 2. Classic Win32 caret via GetGUIThreadInfo.
    if let Some(anchor) = caret(hwnd) {
        let mut point = windows_sys::Win32::Foundation::POINT {
            x: anchor.rect[0],
            y: anchor.rect[3] + 8,
        };
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(anchor.caret, &mut point);
        }
        return clamp_position_for_window(hwnd, point.x, point.y);
    }
    // 3. Modern apps (Chrome/Edge/Electron/VS Code/WPF): anchor near mouse cursor.
    let mut cursor = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut cursor) } != 0 {
        return clamp_position_for_window(hwnd, cursor.x + 12, cursor.y + 18);
    }
    None
}

fn apply_saved_theme(ctx: &egui::Context) {
    let style = ui_theme::UiDesignStyle::from_config(&tool_prefs::ui_design_style());
    ui_theme::set_active_design_style(style);
    ui_theme::apply_tool_style(ctx);
    let pref = tool_prefs::ui_theme_preference().to_ascii_lowercase();
    match pref.as_str() {
        "dark" => ctx.set_theme(egui::Theme::Dark),
        "system" => {
            // Fall back to system theme when supported; otherwise light.
            if let Some(system_theme) = ctx.system_theme() {
                ctx.set_theme(system_theme);
            } else {
                ctx.set_theme(egui::Theme::Light);
            }
        }
        _ => ctx.set_theme(egui::Theme::Light),
    }
}

fn spawn_control_pipe_listener(tx: mpsc::Sender<bridge::ExternalTranslationRequest>) {
    std::thread::Builder::new()
        .name("kaixin-preview-control".into())
        .spawn(move || {
            let Ok(security) = LocalLogonPipeSecurity::new() else {
                return;
            };
            let pipe_name: Vec<u16> = bridge::PREVIEW_CONTROL_PIPE
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            loop {
                let raw = unsafe {
                    CreateNamedPipeW(
                        pipe_name.as_ptr(),
                        PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED,
                        PIPE_TYPE_BYTE
                            | PIPE_READMODE_BYTE
                            | PIPE_WAIT
                            | PIPE_REJECT_REMOTE_CLIENTS,
                        4,
                        0,
                        64 * 1024,
                        2_000,
                        security.as_ptr(),
                    )
                };
                if raw == INVALID_HANDLE_VALUE {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                let Ok(handle) = (unsafe { OwnedWinHandle::from_raw(raw) }) else {
                    continue;
                };
                let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
                let started = unsafe { ConnectNamedPipe(handle.as_raw(), &mut overlapped) };
                let err = unsafe { GetLastError() };
                let connected = started != 0
                    || err == ERROR_PIPE_CONNECTED
                    || (err == ERROR_IO_PENDING && {
                        let mut transferred = 0u32;
                        let ok = unsafe {
                            GetOverlappedResultEx(
                                handle.as_raw(),
                                &overlapped,
                                &mut transferred,
                                5_000,
                                0,
                            )
                        };
                        if ok == 0 {
                            unsafe {
                                CancelIoEx(handle.as_raw(), &overlapped);
                                GetOverlappedResult(
                                    handle.as_raw(),
                                    &overlapped,
                                    &mut transferred,
                                    1,
                                );
                            }
                            false
                        } else {
                            true
                        }
                    });
                if !connected {
                    continue;
                }
                let mut bytes = Vec::with_capacity(4096);
                let mut buf = [0u8; 4096];
                loop {
                    let mut read_ov: OVERLAPPED = unsafe { std::mem::zeroed() };
                    let started = unsafe {
                        ReadFile(
                            handle.as_raw(),
                            buf.as_mut_ptr(),
                            buf.len() as u32,
                            std::ptr::null_mut(),
                            &mut read_ov,
                        )
                    };
                    if started == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
                        break;
                    }
                    let mut transferred = 0u32;
                    if unsafe {
                        GetOverlappedResultEx(handle.as_raw(), &read_ov, &mut transferred, 2_000, 0)
                    } == 0
                    {
                        unsafe {
                            CancelIoEx(handle.as_raw(), &read_ov);
                            GetOverlappedResult(handle.as_raw(), &read_ov, &mut transferred, 1);
                        }
                        break;
                    }
                    if transferred == 0 {
                        break;
                    }
                    let slice = &buf[..transferred as usize];
                    if let Some(pos) = slice.iter().position(|b| *b == b'\n') {
                        bytes.extend_from_slice(&slice[..pos]);
                        break;
                    }
                    bytes.extend_from_slice(slice);
                    if bytes.len() > 1024 * 1024 {
                        bytes.clear();
                        break;
                    }
                }
                unsafe { DisconnectNamedPipe(handle.as_raw()) };
                if let Ok(req) =
                    serde_json::from_slice::<bridge::ExternalTranslationRequest>(&bytes)
                {
                    if tx.send(req).is_err() {
                        return;
                    }
                }
            }
        })
        .ok();
}

struct Preview {
    _instance_guard: win_single_instance::SingleInstanceGuard,
    incoming: Receiver<bridge::ExternalTranslationRequest>,
    hooks: Option<InputHooks>,
    request: bridge::ExternalTranslationRequest,
    generation: u64,
    events: Receiver<bridge::TranslationCallbackEvent>,
    stop: Arc<AtomicBool>,
    result: String,
    status: String,
    done: bool,
    completed: bool,
    streaming_preview: bool,
    completed_chunks: usize,
    total_chunks: usize,
    target: isize,
    original: Option<Caret>,
    unsafe_target: bool,
    exe: String,
    action: String,
    remember: bool,
    language_switched: bool,
    language_filter: String,
    source_window: isize,
    clipboard: u32,
    visible: bool,
    hidden_since: Option<Instant>,
    exiting: bool,
    continuous_mode: bool,
    show_glossary_editor: bool,
    glossary_source: String,
    glossary_translation: String,
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.hooks = None;
    }
}

impl Preview {
    fn new(
        instance_guard: win_single_instance::SingleInstanceGuard,
        incoming: Receiver<bridge::ExternalTranslationRequest>,
        initial_request: bridge::ExternalTranslationRequest,
    ) -> Self {
        let (_dummy_tx, dummy_rx) = mpsc::channel();
        let mut preview = Self {
            _instance_guard: instance_guard,
            incoming,
            hooks: None,
            request: initial_request.clone(),
            generation: 0,
            events: dummy_rx,
            stop: Arc::new(AtomicBool::new(false)),
            result: String::new(),
            status: String::new(),
            done: false,
            completed: false,
            streaming_preview: false,
            completed_chunks: 0,
            total_chunks: 0,
            target: 0,
            original: None,
            unsafe_target: false,
            exe: String::new(),
            action: "show".into(),
            remember: false,
            language_switched: false,
            language_filter: String::new(),
            source_window: 0,
            clipboard: 0,
            visible: true,
            hidden_since: None,
            exiting: false,
            continuous_mode: bridge::continuous_translate_enabled(),
            show_glossary_editor: false,
            glossary_source: String::new(),
            glossary_translation: String::new(),
        };
        preview.load_request(initial_request, None);
        preview
    }

    fn load_request(
        &mut self,
        mut request: bridge::ExternalTranslationRequest,
        ctx: Option<&egui::Context>,
    ) {
        self.stop.store(true, Ordering::Release);
        self.hooks = None;

        // Wait briefly for triggering modifier keys to be released (up to 150ms max).
        for _ in 0..15 {
            if ![0x10, 0x11, 0x12, 0x0D]
                .iter()
                .any(|key| unsafe { GetAsyncKeyState(*key) } < 0)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let target = request.target_hwnd.unwrap_or(0);
        // Stable session key (without temporary process PID) so HY2 deduplicates & cancels old tasks.
        request.session = format!("kaixin-preview-{}-{target}", request.origin);

        let original = caret(target);
        let exe = bridge::process_image_path_for_window(target);
        let saved = if exe.is_empty() {
            None
        } else {
            bridge::load_app_preferences().get(&exe).cloned()
        };
        let mut action = request.result_action.clone();
        if let Some(value) = &saved {
            if request.target == "auto-opposite" || !request.origin.contains("vv-") {
                request.target = value.language.clone();
            }
            action = value.action.clone();
        }
        let target_valid = target != 0 && unsafe { IsWindow(target) } != 0;
        let is_ocr = request.origin.contains("ocr");
        let unsafe_target = !target_valid
            || (!is_ocr && unsafe { GetForegroundWindow() } != target)
            || (original.is_some() && caret(target) != original);
        let source_window = if is_ocr {
            unsafe { GetForegroundWindow() }
        } else {
            0
        };

        self.target = target;
        self.original = original;
        self.unsafe_target = unsafe_target;
        self.exe = exe;
        self.action = action;
        self.remember = saved.is_some();
        self.source_window = source_window;
        self.language_switched = false;
        self.language_filter.clear();
        self.show_glossary_editor = false;
        self.glossary_source = request.text.trim().chars().take(80).collect();
        self.glossary_translation.clear();
        self.continuous_mode = bridge::continuous_translate_enabled();
        self.request = request;
        self.visible = true;
        self.hidden_since = None;
        self.hooks = Some(InputHooks::install(target));

        if let Some(ctx) = ctx {
            apply_saved_theme(ctx);
            if let Some(point) = compute_anchor_position(target, self.request.caret_rect) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(point));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }

        self.start(false);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.stop.store(true, Ordering::Release);
        self.hooks = None;
        self.visible = false;
        self.hidden_since = Some(Instant::now());
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn start(&mut self, bypass_cache: bool) {
        self.stop.store(true, Ordering::Release);
        self.request.request_id = bridge::fresh_request_id();
        self.generation = next_preview_generation(self.generation, unsafe { GetTickCount64() });
        self.request.generation = self.generation;
        self.request.bypass_cache = bypass_cache;
        (self.events, self.stop) = bridge::preview_stream(self.request.clone());
        self.result.clear();
        self.done = false;
        self.completed = false;
        self.streaming_preview = false;
        self.completed_chunks = 0;
        self.total_chunks = 0;
        self.status = if bypass_cache {
            "正在绕过缓存重新翻译…".into()
        } else {
            "正在连接或启动翻译软件…".into()
        };
        self.clipboard = clipboard_sequence();
    }

    fn copy(&mut self, original: bool) {
        let text = if original {
            &self.request.text
        } else {
            &self.result
        };
        self.status = match clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text) {
            Ok(()) => if original {
                "已复制原文"
            } else {
                "已复制译文"
            }
            .into(),
            Err(e) => format!("复制失败：{e}"),
        };
    }

    fn can_insert(&self) -> bool {
        !self.unsafe_target && self.target != 0 && unsafe { IsWindow(self.target) } != 0
    }

    fn insert_text(&mut self, text: &str) -> Result<(), String> {
        if !self.can_insert() {
            return Err("编辑位置已变化或目标窗口不可用，请复制译文后手动粘贴。".into());
        }
        if bridge::process_id_for_window(self.target) != self.request.target_process_id {
            return Err("原窗口已关闭或进程已变化，请复制译文。".into());
        }
        unsafe {
            if [0x10, 0x11, 0x12]
                .iter()
                .any(|key| GetAsyncKeyState(*key) < 0)
            {
                return Err("请松开 Shift / Ctrl / Alt 后再确认。".into());
            }
        }
        if let Some(original) = &self.original {
            if caret(self.target).as_ref() != Some(original) {
                self.unsafe_target = true;
                return Err("原窗口的编辑位置已变化，请复制译文。".into());
            }
        }
        // The hook ignores injected keys and still watches the target if insertion fails.
        win_paste::insert_text_preserving_clipboard(self.target, text)
    }

    fn guard(&mut self) {
        if !self.visible {
            return;
        }
        let front = unsafe { GetForegroundWindow() };
        let own = bridge::process_id_for_window(front) == Some(std::process::id());
        let edited = KEY_EDITED.load(Ordering::Acquire);
        let caret_moved = self.original.is_some() && caret(self.target) != self.original;
        self.unsafe_target |= target_changed(
            front,
            self.target,
            own,
            self.source_window,
            edited,
            caret_moved,
        );
    }
}

impl eframe::App for Preview {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Intercept window X button: hide and stay warm instead of terminating immediately.
        if ctx.input(|i| i.viewport().close_requested()) && !self.exiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
            return;
        }

        // Check for new handoff requests arriving over the control pipe.
        let mut latest_incoming = None;
        while let Ok(req) = self.incoming.try_recv() {
            latest_incoming = Some(req);
        }
        if let Some(req) = latest_incoming {
            self.load_request(req, Some(ctx));
        }

        if !self.visible {
            if self
                .hidden_since
                .is_some_and(|since| since.elapsed() >= IDLE_EXIT_TIMEOUT)
            {
                self.exiting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            ctx.request_repaint_after(Duration::from_millis(120));
            return;
        }

        let confirm_key = KEY_CONFIRM.swap(false, Ordering::AcqRel);
        self.guard();

        while let Ok(event) = self.events.try_recv() {
            if let Some(msg) = event.message.clone().filter(|m| !m.is_empty()) {
                self.status = msg;
            }
            if event.total > 0 {
                self.completed_chunks = event.completed;
                self.total_chunks = event.total;
            }
            match event.event_name.as_str() {
                "progress" => {
                    if let Some(partial) = event.text.filter(|t| !t.is_empty()) {
                        self.result = partial;
                        self.streaming_preview = true;
                    }
                }
                "completed" => {
                    self.result = event.text.unwrap_or_default();
                    self.done = true;
                    self.completed = true;
                    self.streaming_preview = false;
                    self.glossary_translation = self.result.trim().chars().take(80).collect();
                    self.status = if self.can_insert() {
                        "翻译完成，按 Enter 确认插入；Esc 取消".into()
                    } else {
                        "翻译完成，可复制译文后手动粘贴；Esc 关闭".into()
                    };
                    let automatic =
                        !self.language_switched && clipboard_sequence() == self.clipboard;
                    if self.action == "copy" && automatic {
                        self.copy(false);
                    }
                    if self.action == "paste" && automatic {
                        let text = self.result.clone();
                        match self.insert_text(&text) {
                            Ok(()) => {
                                self.hide(ctx);
                                return;
                            }
                            Err(error) => self.status = error,
                        }
                    }
                    if !automatic && self.action != "show" {
                        self.status = "语言已临时切换或剪贴板已变化，请确认译文后手动操作。".into();
                    }
                }
                "failed" | "cancelled" => {
                    self.done = true;
                    self.streaming_preview = false;
                }
                _ => {}
            }
        }

        if KEY_CANCEL.swap(false, Ordering::AcqRel)
            || ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.hide(ctx);
            return;
        }

        let mut confirm_insert = false;
        let mut insert_bilingual = false;
        let mut insert_original = false;
        let mut retry_bypass = false;
        let mut switched_language = false;
        let mut open_in_hymt = false;

        let languages = bridge::supported_languages();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(if self.request.origin.contains("ocr") {
                    "截图翻译 · 原译文对照"
                } else if self.request.replace_selection {
                    "选中文本翻译"
                } else {
                    "候选／整句翻译"
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .checkbox(&mut self.continuous_mode, "连续翻译")
                        .on_hover_text("开启后，候选提交自动进入翻译预览（中打英出模式）")
                        .changed()
                    {
                        let _ = bridge::set_continuous_translate_enabled(self.continuous_mode);
                    }
                });
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("译成");
                let before = self.request.target.clone();
                let current_label = bridge::language_label(&self.request.target);
                egui::ComboBox::from_id_salt("language")
                    .selected_text(current_label)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.language_filter)
                                .hint_text("搜索语言…")
                                .desired_width(128.0),
                        );
                        ui.separator();
                        let query = self.language_filter.trim().to_ascii_lowercase();
                        for (code, label) in &languages {
                            if !query.is_empty()
                                && !code.to_ascii_lowercase().contains(&query)
                                && !label.to_ascii_lowercase().contains(&query)
                            {
                                continue;
                            }
                            ui.selectable_value(
                                &mut self.request.target,
                                code.clone(),
                                label.as_str(),
                            );
                        }
                    });
                if before != self.request.target {
                    self.language_switched = true;
                    switched_language = true;
                }
                if ui
                    .button("重新翻译")
                    .on_hover_text("绕过内存缓存，重新调用本地模型生成译文")
                    .clicked()
                {
                    retry_bypass = true;
                }
                if ui
                    .button("在 HY-MT2 中打开")
                    .on_hover_text("在 HY-MT2 桌面主窗口中查看、编辑或保留此译文")
                    .clicked()
                {
                    open_in_hymt = true;
                }
            });

            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("action")
                    .selected_text(match self.action.as_str() {
                        "copy" => "完成后复制",
                        "paste" => "完成后粘贴",
                        _ => "预览后确认",
                    })
                    .show_ui(ui, |ui| {
                        for (v, l) in [
                            ("show", "预览后确认"),
                            ("copy", "完成后复制"),
                            ("paste", "完成后粘贴"),
                        ] {
                            ui.selectable_value(&mut self.action, v.into(), l);
                        }
                    });
                if !self.exe.is_empty() {
                    let label = std::path::Path::new(&self.exe)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    ui.checkbox(&mut self.remember, format!("记住 {label}"));
                    if ui.small_button("保存偏好").clicked() {
                        self.status = match bridge::save_app_preference(
                            &self.exe,
                            self.remember.then(|| bridge::AppTranslationPreference {
                                language: self.request.target.clone(),
                                action: self.action.clone(),
                            }),
                        ) {
                            Ok(()) => if self.remember {
                                "已保存此应用的语言和操作"
                            } else {
                                "已恢复此应用的全局默认设置"
                            }
                            .into(),
                            Err(e) => format!("保存失败：{e}"),
                        };
                    }
                }
            });

            if !self.done && self.total_chunks > 1 {
                let frac =
                    (self.completed_chunks as f32 / self.total_chunks as f32).clamp(0.0, 1.0);
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(format!(
                            "翻译进度：{}/{} 块",
                            self.completed_chunks, self.total_chunks
                        ))
                        .animate(true),
                );
            }

            ui.separator();
            let scroll_max = (ui.available_height()
                - if self.show_glossary_editor {
                    165.0
                } else {
                    112.0
                })
            .max(100.0);
            egui::ScrollArea::vertical()
                .max_height(scroll_max)
                .show(ui, |ui| {
                    ui.small("原文");
                    ui.add(egui::Label::new(&self.request.text).selectable(true));
                    ui.separator();
                    ui.small(if self.streaming_preview {
                        "译文（流式生成中…）"
                    } else {
                        "译文"
                    });
                    if self.result.is_empty() && !self.done {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(&self.status);
                        });
                    } else {
                        ui.add(egui::Label::new(&self.result).selectable(true));
                    }
                });

            if self.show_glossary_editor {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.small("术语");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.glossary_source)
                            .hint_text("原文术语")
                            .desired_width(130.0),
                    );
                    ui.label("→");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.glossary_translation)
                            .hint_text("固定译法")
                            .desired_width(130.0),
                    );
                    if ui.small_button("保存术语").clicked() {
                        match bridge::add_glossary_term(
                            &self.request.target,
                            &self.glossary_source,
                            &self.glossary_translation,
                        ) {
                            Ok(msg) => {
                                self.status = msg;
                                self.show_glossary_editor = false;
                            }
                            Err(err) => self.status = format!("术语保存失败：{err}"),
                        }
                    }
                });
            }

            ui.separator();
            ui.label(&self.status);
            if self.unsafe_target {
                ui.colored_label(
                    egui::Color32::from_rgb(185, 110, 30),
                    "输入位置已变化，本次只展示结果，可复制后手动粘贴。",
                );
            }
            ui.horizontal_wrapped(|ui| {
                let can_confirm = self.completed && self.can_insert();
                confirm_insert = ui
                    .add_enabled(
                        can_confirm,
                        egui::Button::new(if self.request.replace_selection {
                            "确认替换 · Enter"
                        } else {
                            "插入译文 · Enter"
                        }),
                    )
                    .clicked();
                insert_bilingual = ui
                    .add_enabled(can_confirm, egui::Button::new("双语插入"))
                    .on_hover_text("同时插入原文与译文（换行对照）")
                    .clicked();
                if self.request.origin.contains("candidate") {
                    insert_original = ui
                        .add_enabled(self.can_insert(), egui::Button::new("插入原文"))
                        .on_hover_text("恢复插入刚才输入的原中文候选")
                        .clicked();
                }
                if ui
                    .add_enabled(self.completed, egui::Button::new("复制译文"))
                    .clicked()
                {
                    self.copy(false);
                }
                if ui.button("复制原文").clicked() {
                    self.copy(true);
                }
                if ui
                    .add_enabled(self.completed, egui::Button::new("加入术语"))
                    .clicked()
                {
                    self.show_glossary_editor = !self.show_glossary_editor;
                }
                if ui.button("取消 · Esc").clicked() {
                    self.hide(ctx);
                }
            });
        });

        if switched_language {
            self.start(false);
        } else if retry_bypass {
            self.start(true);
        }

        if open_in_hymt {
            match bridge::present_result(&self.request.request_id) {
                Ok(()) => {
                    self.hide(ctx);
                    return;
                }
                Err(err) => {
                    self.status = format!("无法在 HY-MT2 中打开：{err}");
                }
            }
        }

        KEY_READY.store(self.completed && self.can_insert(), Ordering::Release);

        if insert_original {
            let text = self.request.text.clone();
            match self.insert_text(&text) {
                Ok(()) => {
                    self.hide(ctx);
                    return;
                }
                Err(error) => self.status = error,
            }
        }

        if insert_bilingual && self.completed {
            let text = format!("{}\n{}", self.request.text.trim(), self.result.trim());
            match self.insert_text(&text) {
                Ok(()) => {
                    self.hide(ctx);
                    return;
                }
                Err(error) => self.status = error,
            }
        }

        if self.completed
            && (confirm_insert
                || confirm_key
                || (!ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter))))
        {
            let text = self.result.clone();
            match self.insert_text(&text) {
                Ok(()) => {
                    self.hide(ctx);
                    return;
                }
                Err(error) => self.status = error,
            }
        }

        ctx.request_repaint_after(Duration::from_millis(45));
    }
}

pub fn run() -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("翻译请求过长".into());
    }
    let request: bridge::ExternalTranslationRequest =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;

    // If a persistent preview instance is already running, hand off instantaneously and exit.
    let Some(instance_guard) = win_single_instance::claim(PREVIEW_MUTEX_NAME)? else {
        for _ in 0..10 {
            if bridge::try_handoff_to_preview(&bytes) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        return Err("已有翻译预览实例正在运行，但无法投递请求。".into());
    };

    let (incoming_tx, incoming_rx) = mpsc::channel();
    spawn_control_pipe_listener(incoming_tx);

    let target = request.target_hwnd.unwrap_or(0);
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([540.0, 440.0])
        .with_min_inner_size([440.0, 320.0])
        .with_always_on_top()
        .with_active(false);
    if let Some(point) = compute_anchor_position(target, request.caret_rect) {
        viewport = viewport.with_position(point);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "开心输入法 · 翻译预览",
        options,
        Box::new(move |cc| {
            fonts::install_cjk_fonts(&cc.egui_ctx);
            apply_saved_theme(&cc.egui_ctx);
            Ok(Box::new(Preview::new(instance_guard, incoming_rx, request)))
        }),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_generations_increase_across_retries_and_new_candidates() {
        let first = next_preview_generation(0, 1_000);
        let retry = next_preview_generation(first, 1_000);
        let next_candidate = next_preview_generation(retry, 1_000);
        let restarted = next_preview_generation(0, 2_000);

        assert!(first < retry && retry < next_candidate && next_candidate < restarted);
    }

    #[test]
    fn preview_interaction_preserves_target_but_editing_invalidates_it() {
        assert!(!target_changed(2, 1, true, 0, true, true));
        assert!(!target_changed(1, 1, false, 0, false, false));
        assert!(target_changed(1, 1, false, 0, true, false));
        assert!(target_changed(1, 1, false, 0, false, true));
        assert!(target_changed(3, 1, false, 0, false, false));
        assert!(!target_changed(3, 1, false, 3, true, false));
    }
    #[test]
    fn candidate_payload_uses_compatible_defaults() {
        let request: bridge::ExternalTranslationRequest = serde_json::from_str(r#"{"text":"你好🙂","origin":"kaixin-ime-candidate","target_hwnd":123,"result_action":"show"}"#).unwrap();
        assert_eq!(request.protocol_version, 2);
        assert!(request.callback_ack);
        assert!(!request.replace_selection);
        assert!(!request.bypass_cache);
        assert!(!request.ephemeral);
        assert_eq!(request.text, "你好🙂");
    }
}
