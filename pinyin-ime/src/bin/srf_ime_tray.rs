#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[path = "srf_ime_tray/menu_style.rs"]
mod tray_menu_style;

#[cfg(windows)]
#[path = "srf_ime_tray/status_icon.rs"]
mod tray_status_icon;

#[cfg(windows)]
#[path = "srf_ime_tray/translation_preview.rs"]
mod translation_preview;

#[cfg(windows)]
mod win {
    use crate::tray_menu_style::{
        self, Icon as TrayMenuIcon, Item as TrayMenuItem, Kind as TrayMenuKind,
    };
    use crate::tray_status_icon::{self, IconKey as TrayIconKey, Mode as TrayIconMode};
    use chrono::{DateTime, FixedOffset, Local};
    use pinyin_ime::runtime_log::{self, RuntimeLogLevel};
    use pinyin_ime::screenshot_trace::ScreenshotTrace;
    use pinyin_ime::{
        app_paths, clipboard_store, external_translation, screenshot_capture,
        screenshot_region_selector, screenshot_store, win_paste, windows_graphics_capture,
    };
    use sha2::{Digest, Sha256};
    use std::ffi::c_void;
    use std::fs;
    use std::mem::{size_of, zeroed};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT,
        POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_DWORD, RRF_RT_REG_DWORD, RRF_RT_REG_QWORD, RRF_RT_REG_SZ,
    };
    use windows_sys::Win32::System::SystemInformation::{GetTickCount, GetTickCount64};
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODT_MENU};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteW, Shell_NotifyIconW, NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, GetAncestor,
        GetClassNameW, GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, KillTimer, LoadCursorW, LoadIconW, MessageBoxW, PostMessageW,
        PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetTimer,
        SetWindowLongPtrW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, GA_ROOT, GWLP_USERDATA,
        IDC_ARROW, IDI_APPLICATION, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MSG,
        PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND, SW_SHOWNORMAL, WM_APP,
        WM_COMMAND, WM_CONTEXTMENU, WM_CREATE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
        WM_DRAWITEM, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MEASUREITEM, WM_NCCREATE,
        WM_NULL, WM_POWERBROADCAST, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WM_USER, WNDCLASSW,
        WS_OVERLAPPED,
    };

    use std::sync::atomic::AtomicIsize;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    static HOTKEY_TRAY_HWND: AtomicIsize = AtomicIsize::new(0);
    const WM_FOREGROUND_CHANGED: u32 = WM_APP + 19;

    unsafe extern "system" fn on_foreground_changed(
        _: isize,
        _: u32,
        _: HWND,
        _: i32,
        _: i32,
        _: u32,
        _: u32,
    ) {
        let hwnd = HOTKEY_TRAY_HWND.load(Ordering::Relaxed);
        if hwnd != 0 {
            PostMessageW(hwnd, WM_FOREGROUND_CHANGED, 0, 0);
        }
    }

    unsafe fn foreground_suppresses_tool_hotkeys(content: &str) -> bool {
        if pure_game_mode_enabled() {
            return true;
        }
        let hwnd = GetForegroundWindow();
        let name = process_name_for_hwnd(hwnd);
        let process = if name.is_empty() {
            String::new()
        } else {
            format!("{name}.exe")
        };
        let class = window_class_name(hwnd);
        pinyin_ime::game_keyboard_policy::suppress_tool_hotkeys(content, &process, &class)
    }

    const CLASS_NAME: &str = "KaixinImeTrayWindow";
    const WINDOW_TITLE: &str = "开心输入法 托盘";
    const MUTEX_NAME: &str = "Local\\KaixinInput_Tray_Mutex";
    const TRAY_TIP: &str = app_paths::APP_DISPLAY_NAME;
    const WM_TRAYICON: u32 = WM_APP + 17;
    const WM_STATE_CHANGED: u32 = WM_APP + 18;
    const TRAY_ICON_ID: u32 = 1;
    const ID_SETTINGS: usize = 1001;
    const ID_CLIPBOARD_MANAGER: usize = 1002;
    const ID_SCREENSHOT: usize = 1003;
    const ID_EXIT: usize = 1004;
    const ID_HANDWRITE: usize = 1005;
    const ID_OCR: usize = 1006;
    const ID_GAME_COMPAT_MODE: usize = 1007;
    const ID_TRANSLATE: usize = 1008;
    const ID_CALCULATOR: usize = 1009;
    const ID_OCR_TRANSLATE: usize = 1010;
    const ID_LAST_SCREENSHOT_OCR: usize = 1011;
    const ID_LAST_SCREENSHOT_TRANSLATE: usize = 1012;
    const ID_SYMBOLS: usize = 1013;
    const ID_ENGINE_RESTART: usize = 1014;
    const ID_PURE_GAME_MODE: usize = 1015;
    const SETTINGS_EXE: &str = "srf_ime_settings.exe";
    const CLIPBOARD_MANAGER_EXE: &str = "srf_ime_clipboard.exe";
    const ENGINE_EXE: &str = "srf_ime_engine.exe";
    const HANDWRITE_EXE: &str = "srf_ime_handwrite.exe";
    const SYMBOLS_EXE: &str = "srf_ime_symbols.exe";
    const OCR_EXE: &str = "srf_ime_ocr.exe";
    const TIMER_ID_STATE_POLL: usize = 1;
    const STATE_POLL_MS: u32 = 600;
    const SCREENSHOT_HOTKEY_ID: i32 = 0x4B58_5301;
    const CLIPBOARD_HOTKEY_ID: i32 = 0x4B58_4301;
    const SETTINGS_HOTKEY_ID: i32 = 0x4B58_5401;
    const HANDWRITE_HOTKEY_ID: i32 = 0x4B58_4801;
    const OCR_HOTKEY_ID: i32 = 0x4B58_4F01;
    const TRANSLATE_HOTKEY_ID: i32 = 0x4B58_5201;
    const OCR_TRANSLATE_HOTKEY_ID: i32 = 0x4B58_4F54;
    const SETTINGS_WINDOW_TITLE_BYTES: &[u8] = "开心输入法 设置".as_bytes();
    const CLIPBOARD_WINDOW_TITLE_BYTES: &[u8] = "开心输入法 剪贴板".as_bytes();
    const HANDWRITE_WINDOW_TITLE_BYTES: &[u8] = "开心输入法 手写查字".as_bytes();
    const OCR_WINDOW_TITLE_BYTES: &[u8] = "开心输入法 OCR".as_bytes();

    // 与 TSF 模块（tsf-tip/src/srf_tip.cpp）保持一致：用于持久化中/英状态（AsciiMode=1 为英文）。
    const STATE_REG_PATH: &str = r"Software\kaixin\State";
    const STATE_REG_VALUE_ASCII: &str = "AsciiMode";
    // Shared with TSF's kStatePureGameModeValue. This override never rewrites
    // normal input preferences, app rules, or their configured hotkeys.
    const STATE_REG_VALUE_PURE_GAME_MODE: &str = "PureGameMode";
    const STATE_REG_VALUE_INPUT_ASCII: &str = "InputAsciiMode";
    const STATE_REG_VALUE_INPUT_MODE_SOURCE: &str = "InputModeSource";
    const STATE_REG_VALUE_INPUT_OWNER_PROCESS_ID: &str = "InputOwnerProcessId";
    const STATE_REG_VALUE_INPUT_OWNER_THREAD_ID: &str = "InputOwnerThreadId";
    const STATE_REG_VALUE_INPUT_OWNER_HWND: &str = "InputOwnerHwnd";
    const STATE_REG_VALUE_INPUT_UPDATED_TICK: &str = "InputUpdatedTick";
    const STATE_REG_VALUE_INPUT_SEQUENCE: &str = "InputSequence";
    const STATE_REG_VALUE_FULL_SHAPE: &str = "FullShape";
    const STATE_REG_VALUE_CHINESE_PUNCTUATION: &str = "ChinesePunctuation";
    const STATE_REG_VALUE_LAST_ENGINE_RECOVERY_REASON: &str = "LastEngineRecoveryReason";
    const STATE_REG_VALUE_LAST_ENGINE_RECOVERY_TIME: &str = "LastEngineRecoveryTime";
    const STATE_REG_VALUE_INSTALL_MAINTENANCE: &str = "InstallMaintenance";
    const STATE_REG_VALUE_INSTALL_MAINTENANCE_TICK: &str = "InstallMaintenanceTick";
    const INSTALL_MAINTENANCE_MAX_AGE_MS: u32 = 30 * 60 * 1000;
    /// 避免双击或菜单重复拉起设置窗口。
    static LAST_SETTINGS_OPEN: Mutex<Option<Instant>> = Mutex::new(None);
    static ACTIVE_SCREENSHOT: Mutex<Option<Arc<ScreenshotSession>>> = Mutex::new(None);
    static SCREENSHOT_RESUME_RECOVERY_PENDING: AtomicBool = AtomicBool::new(false);
    static SCREENSHOT_HANDOFF_COUNTER: AtomicU64 = AtomicU64::new(0);
    const SETTINGS_OPEN_DEBOUNCE: Duration = Duration::from_millis(1200);

    fn schedule_deferred_runtime_cleanup() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(root) = exe.parent().map(Path::to_path_buf) else {
            return;
        };
        let setup_exe = root.join("srf_ime_setup.exe");
        if !setup_exe.is_file() {
            return;
        }
        let _ = thread::Builder::new()
            .name("kaixin-runtime-cleanup".to_string())
            .spawn(move || {
                thread::sleep(Duration::from_secs(4));
                use std::os::windows::process::CommandExt;
                let _ = Command::new(&setup_exe)
                    .arg("cleanup-runtime")
                    .arg("--root")
                    .arg(&root)
                    .creation_flags(0x0800_0000)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            });
    }

    pub fn run() {
        if let Some(code) = screenshot_capture::run_helper_from_args() {
            std::process::exit(code);
        }
        let args: Vec<String> = std::env::args().collect();
        if let Some(index) = args.iter().position(|arg| arg == "--menu-preview") {
            let Some(path) = args.get(index + 1) else {
                std::process::exit(2);
            };
            let dpi = args
                .get(index + 2)
                .and_then(|v| v.parse().ok())
                .unwrap_or(96);
            let theme = args.get(index + 3).map(String::as_str).unwrap_or("light");
            let mut state = read_tray_visual_state();
            state.ascii_mode = Some(false);
            state.engine_state = match args.get(index + 4).map(String::as_str) {
                Some("ready") => TrayEngineState::Ready,
                Some("failed") => TrayEngineState::Failed,
                _ => TrayEngineState::Unknown,
            };
            let selected = args
                .get(index + 5)
                .and_then(|v| v.parse().ok())
                .unwrap_or(ID_CLIPBOARD_MANAGER);
            let pure_game = args.iter().any(|v| v == "--game-on");
            apply_pure_game_visual_override(&mut state, pure_game);
            let items = build_tray_menu_items(&state, true, true);
            if unsafe { tray_menu_style::preview(Path::new(path), items, dpi, theme, selected) }
                .is_err()
            {
                std::process::exit(1);
            }
            return;
        }
        if std::env::args().any(|arg| arg == "--screenshot-capture") {
            run_screenshot_capture();
            let _ = runtime_log::flush_pending_logs(Duration::from_millis(500));
            return;
        }
        unsafe {
            let mutex = create_single_instance_mutex();
            if mutex == 0 {
                return;
            }
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "tray_start",
                format!(
                    "status=ok single_instance=1 {} {}",
                    runtime_log::config_diagnostics_fields(),
                    clipboard_store::diagnostics_fields()
                ),
            );
            ensure_engine_helper_running();
            schedule_deferred_runtime_cleanup();
            let class_name = wide(CLASS_NAME);
            let title = wide(WINDOW_TITLE);
            let hinstance = GetModuleHandleW(std::ptr::null());
            let mut app = Box::new(TrayApp::new(mutex));
            let app_ptr: *mut TrayApp = &mut *app;

            let cursor = LoadCursorW(0, IDC_ARROW);
            let wc = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance,
                lpszClassName: class_name.as_ptr(),
                hCursor: cursor,
                ..zeroed()
            };
            if RegisterClassW(&wc) == 0 {
                return;
            }

            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                0,
                0,
                hinstance,
                app_ptr.cast::<c_void>(),
            );
            if hwnd == 0 {
                return;
            }

            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Keep the window's borrowed state and the singleton mutex alive
            // through window destruction, including an unexpected pump exit.
            if IsWindow(hwnd) != 0 {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    struct TrayApp {
        mutex: HANDLE,
        icon_guid: Option<GUID>,
        icon: isize,
        icon_owned: bool,
        last_icon_key: Option<TrayIconKey>,
        last_visual_state: Option<TrayVisualState>,
        screenshot_hotkey: Option<HotkeySpec>,
        clipboard_hotkey: Option<HotkeySpec>,
        settings_hotkey: Option<HotkeySpec>,
        handwrite_hotkey: Option<HotkeySpec>,
        ocr_hotkey: Option<HotkeySpec>,
        ocr_translate_hotkey: Option<HotkeySpec>,
        translate_hotkey: Option<HotkeySpec>,
        hotkey_config_stamp: Option<(PathBuf, SystemTime, u64)>,
        hotkey_config_text: String,
        tool_hotkeys_suppressed: bool,
        foreground_hook: isize,
        clipboard_config_mtime: Option<SystemTime>,
        clipboard_background_enabled: Option<bool>,
        last_capture_target: HWND,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TrayEngineState {
        Idle,
        Loading,
        Ready,
        Failed,
        Unknown,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum TrayInputMode {
        Chinese,
        English,
        Unknown,
    }

    #[derive(Clone, PartialEq, Eq)]
    struct TrayVisualState {
        ascii_mode: Option<bool>,
        mode_source: Option<String>,
        full_shape: Option<bool>,
        chinese_punctuation: Option<bool>,
        owner_process_id: Option<u32>,
        owner_thread_id: Option<u32>,
        owner_hwnd: Option<u64>,
        updated_tick: Option<u64>,
        sequence: Option<u64>,
        engine_state: TrayEngineState,
        engine_error_code: Option<u32>,
        last_recovery_reason: Option<String>,
        last_recovery_time: Option<String>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct HotkeySpec {
        modifiers: u32,
        vk: u32,
    }

    impl TrayEngineState {
        fn from_registry(value: Option<u32>) -> Self {
            match value {
                Some(0) => Self::Idle,
                Some(1) => Self::Loading,
                Some(2) => Self::Ready,
                Some(3) => Self::Failed,
                _ => Self::Unknown,
            }
        }

        fn label(self) -> &'static str {
            match self {
                Self::Idle => "未启动",
                Self::Loading => "恢复中",
                Self::Ready => "正常",
                Self::Failed => "异常",
                Self::Unknown => "未知",
            }
        }
    }

    impl TrayVisualState {
        fn input_mode(&self) -> TrayInputMode {
            match self.ascii_mode {
                Some(true) => TrayInputMode::English,
                Some(false) => TrayInputMode::Chinese,
                None => TrayInputMode::Unknown,
            }
        }

        fn icon_key(&self, size: i32) -> TrayIconKey {
            let mode = if self.mode_source.as_deref() == Some("pure_game") {
                TrayIconMode::Gamepad
            } else {
                match self.input_mode() {
                    TrayInputMode::Chinese => TrayIconMode::Chinese,
                    TrayInputMode::English => TrayIconMode::English,
                    TrayInputMode::Unknown => TrayIconMode::Unknown,
                }
            };
            TrayIconKey {
                mode,
                failed: self.engine_state == TrayEngineState::Failed,
                size,
            }
        }
    }

    fn screenshot_hotkey_status_path() -> Option<PathBuf> {
        app_paths::local_data_dir().map(|dir| dir.join("screenshot_hotkey_status.txt"))
    }

    fn hotkey_spec_label(spec: HotkeySpec) -> String {
        let mut parts = Vec::new();
        if spec.modifiers & MOD_CONTROL != 0 {
            parts.push("Ctrl".to_string());
        }
        if spec.modifiers & MOD_SHIFT != 0 {
            parts.push("Shift".to_string());
        }
        if spec.modifiers & MOD_ALT != 0 {
            parts.push("Alt".to_string());
        }
        if spec.modifiers & MOD_WIN != 0 {
            parts.push("Win".to_string());
        }
        let key = if (b'A' as u32..=b'Z' as u32).contains(&spec.vk)
            || (b'0' as u32..=b'9' as u32).contains(&spec.vk)
        {
            char::from_u32(spec.vk).unwrap_or('?').to_string()
        } else if (0x70..=0x87).contains(&spec.vk) {
            format!("F{}", spec.vk - 0x70 + 1)
        } else {
            match spec.vk {
                0x20 => "Space".to_string(),
                0x09 => "Tab".to_string(),
                0x0D => "Enter".to_string(),
                0x1B => "Esc".to_string(),
                0xBA => "Semicolon".to_string(),
                0xBB => "Equal".to_string(),
                0xBC => "Comma".to_string(),
                0xBD => "Minus".to_string(),
                0xBE => "Period".to_string(),
                0xBF => "Slash".to_string(),
                0xDE => "Quote".to_string(),
                _ => format!("VK{:X}", spec.vk),
            }
        };
        parts.push(key);
        parts.join("+")
    }

    fn write_screenshot_hotkey_status(line: String) {
        let Some(path) = screenshot_hotkey_status_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, line);
    }

    fn hotkey_status_line(status_key: &str, spec: Option<HotkeySpec>) -> String {
        match spec {
            Some(spec) => format!("{status_key}=1 hotkey={}", hotkey_spec_label(spec)),
            None => format!("{status_key}=0 hotkey=off reason=disabled"),
        }
    }

    unsafe fn register_tray_hotkey(
        hwnd: HWND,
        id: i32,
        spec: Option<HotkeySpec>,
        slot: &mut Option<HotkeySpec>,
        status_key: &str,
        status_parts: &mut Vec<String>,
    ) {
        if let Some(spec) = spec {
            if RegisterHotKey(hwnd, id, spec.modifiers | MOD_NOREPEAT, spec.vk) != 0 {
                *slot = Some(spec);
                status_parts.push(format!("{status_key}=1 hotkey={}", hotkey_spec_label(spec)));
            } else {
                let err = GetLastError();
                status_parts.push(format!(
                    "{status_key}=0 hotkey={} error={err}",
                    hotkey_spec_label(spec)
                ));
            }
        } else {
            status_parts.push(format!("{status_key}=0 hotkey=off reason=disabled"));
        }
    }

    fn taskbar_created_message() -> u32 {
        static TASKBAR_CREATED_MESSAGE: OnceLock<u32> = OnceLock::new();
        *TASKBAR_CREATED_MESSAGE.get_or_init(|| unsafe {
            let name = wide("TaskbarCreated");
            RegisterWindowMessageW(name.as_ptr())
        })
    }

    impl TrayApp {
        fn new(mutex: HANDLE) -> Self {
            Self {
                mutex,
                icon_guid: tray_icon_guid(),
                icon: 0,
                icon_owned: false,
                last_icon_key: None,
                last_visual_state: None,
                screenshot_hotkey: None,
                clipboard_hotkey: None,
                settings_hotkey: None,
                handwrite_hotkey: None,
                ocr_hotkey: None,
                ocr_translate_hotkey: None,
                translate_hotkey: None,
                hotkey_config_stamp: None,
                hotkey_config_text: String::new(),
                tool_hotkeys_suppressed: false,
                foreground_hook: 0,
                clipboard_config_mtime: None,
                clipboard_background_enabled: None,
                last_capture_target: 0,
            }
        }

        unsafe fn tray_icon_identity(&self, hwnd: HWND) -> NOTIFYICONDATAW {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ICON_ID;
            if let Some(guid) = self.icon_guid {
                nid.uFlags = NIF_GUID;
                nid.guidItem = guid;
            }
            nid
        }

        unsafe fn restore_tray_icon_after_taskbar_created(&mut self, hwnd: HWND) {
            let restored = self.set_visual_state(hwnd, read_tray_visual_state(), true);
            runtime_log::log_tray(
                if restored {
                    RuntimeLogLevel::Basic
                } else {
                    RuntimeLogLevel::Error
                },
                "tray_icon_restore",
                format!("reason=taskbar_created result={}", u8::from(restored)),
            );
        }

        unsafe fn destroy_owned_icon(&mut self) {
            if self.icon_owned && self.icon != 0 {
                let _ = DestroyIcon(self.icon);
            }
            self.icon = 0;
            self.icon_owned = false;
            self.last_icon_key = None;
        }

        unsafe fn remove_tray_icon(&mut self, hwnd: HWND) {
            let nid = self.tray_icon_identity(hwnd);
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            self.destroy_owned_icon();
        }

        unsafe fn refresh_icon_if_needed(&mut self, hwnd: HWND) {
            let state = read_tray_visual_state();
            self.set_visual_state(hwnd, state, false);
        }

        unsafe fn remember_capture_target_from_foreground(&mut self, tray_hwnd: HWND) {
            let target = capture_target_from_foreground(tray_hwnd);
            if target != 0 {
                self.last_capture_target = target;
            }
        }

        unsafe fn capture_target_or_foreground(&mut self, tray_hwnd: HWND) -> HWND {
            let target = capture_target_from_foreground(tray_hwnd);
            if target != 0 {
                self.last_capture_target = target;
                return target;
            }
            if is_capture_target_candidate(self.last_capture_target, tray_hwnd) {
                self.last_capture_target
            } else {
                0
            }
        }

        unsafe fn refresh_visual_state(&mut self, hwnd: HWND, ascii_override: Option<bool>) {
            let mut state = read_tray_visual_state();
            if let Some(ascii) = ascii_override.filter(|_| !pure_game_mode_enabled()) {
                state.ascii_mode = Some(ascii);
            }
            self.set_visual_state(hwnd, state, false);
        }

        unsafe fn set_visual_state(
            &mut self,
            hwnd: HWND,
            state: TrayVisualState,
            force: bool,
        ) -> bool {
            if !force && self.last_visual_state.as_ref() == Some(&state) {
                return true;
            }

            let next_key = state.icon_key(tray_status_icon::taskbar_icon_size());
            let icon_changed = self.last_icon_key != Some(next_key) || self.icon == 0;
            let old_icon = self.icon;
            let old_icon_owned = self.icon_owned;
            let (next_icon, next_icon_owned) = if icon_changed {
                let icon = tray_status_icon::create(next_key);
                (
                    icon.unwrap_or_else(|| LoadIconW(0, IDI_APPLICATION)),
                    icon.is_some(),
                )
            } else {
                (self.icon, self.icon_owned)
            };
            if next_icon == 0 {
                self.last_visual_state = None;
                return false;
            }

            let identity = self.tray_icon_identity(hwnd);
            let mut nid = identity;
            nid.uFlags |= NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            nid.uCallbackMessage = WM_TRAYICON;
            nid.hIcon = next_icon;
            write_wide_buffer(&mut nid.szTip, &tray_tooltip_text(&state));
            if old_icon == 0 || Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                // The singleton is held. Remove a previous process's entry
                // before registering this window as the callback owner.
                if old_icon == 0 && self.icon_guid.is_some() {
                    let _ = Shell_NotifyIconW(NIM_DELETE, &identity);
                }
                if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                    if icon_changed && next_icon_owned && next_icon != old_icon {
                        let _ = DestroyIcon(next_icon);
                    }
                    // Explorer may still be starting. Retry on the next poll.
                    self.last_visual_state = None;
                    return false;
                }
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
            }

            self.icon = next_icon;
            self.icon_owned = next_icon_owned;
            self.last_icon_key = Some(next_key);
            self.last_visual_state = Some(state);

            if icon_changed && old_icon_owned && old_icon != 0 && old_icon != next_icon {
                let _ = DestroyIcon(old_icon);
            }
            true
        }

        unsafe fn toggle_ascii_mode(&mut self, hwnd: HWND) {
            if pure_game_mode_enabled() {
                return;
            }
            let next_ascii = !read_ascii_mode().unwrap_or(false);
            if write_ascii_mode(next_ascii) {
                self.refresh_visual_state(hwnd, Some(next_ascii));
            }
        }

        unsafe fn refresh_screenshot_hotkey_if_needed(&mut self, hwnd: HWND, force: bool) {
            let path = config_path();
            let (stamp, read_config) = match fs::metadata(&path) {
                Ok(meta) => (
                    meta.modified()
                        .ok()
                        .map(|mtime| (path.clone(), mtime, meta.len())),
                    true,
                ),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (None, true),
                Err(_) => (None, false),
            };
            let mut changed = false;
            if read_config && (force || stamp.is_none() || stamp != self.hotkey_config_stamp) {
                let content = match fs::read_to_string(&path) {
                    Ok(content) => Some(content),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
                    Err(_) => None,
                };
                if let Some(content) = content {
                    changed = content != self.hotkey_config_text;
                    self.hotkey_config_text = content;
                    self.hotkey_config_stamp = stamp;
                }
            }
            let suppressed = foreground_suppresses_tool_hotkeys(&self.hotkey_config_text);
            if !force && !changed && suppressed == self.tool_hotkeys_suppressed {
                return;
            }
            self.tool_hotkeys_suppressed = suppressed;
            let hotkey =
                |section, key| read_hotkey_config(&self.hotkey_config_text, section, key, "off");
            let next_screenshot = if suppressed {
                None
            } else {
                hotkey("screenshot", "hotkey")
            };
            let next_clipboard = if suppressed {
                None
            } else {
                hotkey("clipboard", "hotkey")
            };
            let next_settings = if suppressed {
                None
            } else {
                hotkey("tools", "settings_hotkey")
            };
            let next_handwrite = if suppressed {
                None
            } else {
                hotkey("tools", "handwrite_hotkey")
            };
            let next_ocr = if suppressed {
                None
            } else {
                hotkey("tools", "ocr_hotkey")
            };
            let next_ocr_translate = if suppressed {
                None
            } else {
                hotkey("tools", "ocr_translate_hotkey")
            };
            let translate_available = is_translate_available();
            let next_translate = if translate_available && !suppressed {
                hotkey("tools", "translate_hotkey")
            } else {
                None
            };
            if next_screenshot == self.screenshot_hotkey
                && next_clipboard == self.clipboard_hotkey
                && next_settings == self.settings_hotkey
                && next_handwrite == self.handwrite_hotkey
                && next_ocr == self.ocr_hotkey
                && next_ocr_translate == self.ocr_translate_hotkey
                && next_translate == self.translate_hotkey
            {
                if force {
                    write_screenshot_hotkey_status(
                        [
                            hotkey_status_line("registered", self.screenshot_hotkey),
                            hotkey_status_line("clipboard_registered", self.clipboard_hotkey),
                            hotkey_status_line("settings_registered", self.settings_hotkey),
                            hotkey_status_line("handwrite_registered", self.handwrite_hotkey),
                            hotkey_status_line("ocr_registered", self.ocr_hotkey),
                            hotkey_status_line(
                                "ocr_translate_registered",
                                self.ocr_translate_hotkey,
                            ),
                            hotkey_status_line("translate_registered", self.translate_hotkey),
                        ]
                        .join(" "),
                    );
                }
                return;
            }
            let _ = UnregisterHotKey(hwnd, SCREENSHOT_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, CLIPBOARD_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, SETTINGS_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, HANDWRITE_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, OCR_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, OCR_TRANSLATE_HOTKEY_ID);
            let _ = UnregisterHotKey(hwnd, TRANSLATE_HOTKEY_ID);
            self.screenshot_hotkey = None;
            self.clipboard_hotkey = None;
            self.settings_hotkey = None;
            self.handwrite_hotkey = None;
            self.ocr_hotkey = None;
            self.ocr_translate_hotkey = None;
            self.translate_hotkey = None;
            let mut status_parts = Vec::new();
            register_tray_hotkey(
                hwnd,
                SCREENSHOT_HOTKEY_ID,
                next_screenshot,
                &mut self.screenshot_hotkey,
                "registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                CLIPBOARD_HOTKEY_ID,
                next_clipboard,
                &mut self.clipboard_hotkey,
                "clipboard_registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                SETTINGS_HOTKEY_ID,
                next_settings,
                &mut self.settings_hotkey,
                "settings_registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                HANDWRITE_HOTKEY_ID,
                next_handwrite,
                &mut self.handwrite_hotkey,
                "handwrite_registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                OCR_HOTKEY_ID,
                next_ocr,
                &mut self.ocr_hotkey,
                "ocr_registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                OCR_TRANSLATE_HOTKEY_ID,
                next_ocr_translate,
                &mut self.ocr_translate_hotkey,
                "ocr_translate_registered",
                &mut status_parts,
            );
            register_tray_hotkey(
                hwnd,
                TRANSLATE_HOTKEY_ID,
                next_translate,
                &mut self.translate_hotkey,
                "translate_registered",
                &mut status_parts,
            );
            write_screenshot_hotkey_status(status_parts.join(" "));
        }

        fn refresh_clipboard_background_if_needed(&mut self, force: bool) {
            let path = config_path();
            let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
            if !force && mtime == self.clipboard_config_mtime {
                return;
            }
            self.clipboard_config_mtime = mtime;

            let enabled = read_bool_config("clipboard", "background_enabled", false);
            if force || self.clipboard_background_enabled != Some(enabled) {
                clipboard_store::set_background_polling_enabled(enabled);
                self.clipboard_background_enabled = Some(enabled);
            }
        }
    }

    impl Drop for TrayApp {
        fn drop(&mut self) {
            unsafe {
                self.destroy_owned_icon();
                if self.mutex != 0 {
                    let _ = CloseHandle(self.mutex);
                    self.mutex = 0;
                }
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_NCCREATE {
            let create =
                lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !create.is_null() {
                let app = (*create).lpCreateParams as *mut TrayApp;
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            }
            return 1;
        }

        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayApp;
        let taskbar_created = taskbar_created_message();
        if taskbar_created != 0 && msg == taskbar_created {
            if !app.is_null() {
                (*app).restore_tray_icon_after_taskbar_created(hwnd);
            }
            return 0;
        }

        match msg {
            WM_CREATE => {
                if app.is_null() {
                    return -1;
                }
                (*app).refresh_icon_if_needed(hwnd);
                (*app).refresh_screenshot_hotkey_if_needed(hwnd, true);
                (*app).refresh_clipboard_background_if_needed(true);
                // 图标跟随前台输入状态和引擎状态。
                HOTKEY_TRAY_HWND.store(hwnd, Ordering::Relaxed);
                (*app).foreground_hook =
                    SetWinEventHook(3, 3, 0, Some(on_foreground_changed), 0, 0, 0);
                let _ = SetTimer(hwnd, TIMER_ID_STATE_POLL, STATE_POLL_MS, None);
                0
            }
            WM_MEASUREITEM => {
                let measure = lparam as *mut MEASUREITEMSTRUCT;
                if measure.is_null() || (*measure).CtlType != ODT_MENU {
                    return 0;
                }
                let item = (*measure).itemData as *const TrayMenuItem;
                if item.is_null() {
                    return 0;
                }
                let dc = GetDC(hwnd);
                (*measure).itemHeight = (*item).height();
                (*measure).itemWidth = (*item).width(dc);
                if dc != 0 {
                    ReleaseDC(hwnd, dc);
                }
                1
            }
            WM_DRAWITEM => {
                let draw = lparam as *const DRAWITEMSTRUCT;
                if draw.is_null() || (*draw).CtlType != ODT_MENU {
                    return 0;
                }
                let item = (*draw).itemData as *const TrayMenuItem;
                if item.is_null() {
                    return 0;
                }
                tray_menu_style::draw(&*draw, &*item);
                1
            }
            WM_COMMAND => {
                handle_menu_command(hwnd, wparam & 0xFFFF, 0);
                0
            }
            WM_CONTEXTMENU => {
                show_menu(hwnd, None);
                0
            }
            WM_TRAYICON => {
                if app.is_null() {
                    return 0;
                }
                // This window receives callbacks for a single GUID-based icon.
                match tray_event_code(lparam) {
                    WM_RBUTTONUP => show_menu(hwnd, None),
                    WM_CONTEXTMENU => show_menu(hwnd, Some(tray_anchor_point(wparam))),
                    // 单击托盘图标切换中/英；设置从右键菜单进入。
                    WM_LBUTTONUP => {
                        (*app).toggle_ascii_mode(hwnd);
                    }
                    WM_LBUTTONDBLCLK => open_settings(),
                    // NOTIFYICON v4：键盘激活、部分 Win10/11 壳层用 NIN_SELECT 代替鼠标 up。
                    NIN_SELECT | NIN_KEYSELECT => {
                        (*app).toggle_ascii_mode(hwnd);
                    }
                    _ => {}
                }
                0
            }
            msg if msg == WM_STATE_CHANGED => {
                if !app.is_null() {
                    let ascii = match wparam {
                        0 => Some(false),
                        1 => Some(true),
                        _ => read_ascii_mode(),
                    };
                    (*app).refresh_visual_state(hwnd, ascii);
                }
                0
            }
            WM_DISPLAYCHANGE => {
                screenshot_capture::display_changed();
                SCREENSHOT_RESUME_RECOVERY_PENDING.store(true, Ordering::Release);
                if !app.is_null() {
                    (*app).set_visual_state(hwnd, read_tray_visual_state(), true);
                }
                0
            }
            WM_DPICHANGED | WM_SETTINGCHANGE => {
                if !app.is_null() {
                    (*app).set_visual_state(hwnd, read_tray_visual_state(), true);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_POWERBROADCAST => {
                if matches!(
                    wparam as u32,
                    PBT_APMRESUMECRITICAL | PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND
                ) {
                    screenshot_capture::display_changed();
                    SCREENSHOT_RESUME_RECOVERY_PENDING.store(true, Ordering::Release);
                    runtime_log::log_tray(
                        RuntimeLogLevel::Basic,
                        "screenshot_resume_recovery",
                        format!("status=armed event={}", wparam as u32),
                    );
                    1
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_DESTROY => {
                HOTKEY_TRAY_HWND.store(0, Ordering::Relaxed);
                if !app.is_null() && (*app).foreground_hook != 0 {
                    UnhookWinEvent((*app).foreground_hook);
                }
                let _ = KillTimer(hwnd, TIMER_ID_STATE_POLL);
                let _ = UnregisterHotKey(hwnd, SCREENSHOT_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, CLIPBOARD_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, SETTINGS_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, HANDWRITE_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, OCR_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, OCR_TRANSLATE_HOTKEY_ID);
                let _ = UnregisterHotKey(hwnd, TRANSLATE_HOTKEY_ID);
                if !app.is_null() {
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    (*app).remove_tray_icon(hwnd);
                }
                PostQuitMessage(0);
                0
            }
            WM_FOREGROUND_CHANGED => {
                if !app.is_null() {
                    (*app).refresh_screenshot_hotkey_if_needed(hwnd, false);
                }
                0
            }
            WM_HOTKEY => {
                if matches!(
                    wparam as i32,
                    SCREENSHOT_HOTKEY_ID | OCR_HOTKEY_ID | OCR_TRANSLATE_HOTKEY_ID
                ) && cancel_active_screenshot()
                {
                    return 0;
                }
                if app.is_null() {
                    return 0;
                }
                (*app).refresh_screenshot_hotkey_if_needed(hwnd, false);
                if (*app).tool_hotkeys_suppressed {
                    return 0;
                }
                let target_hwnd = (*app).capture_target_or_foreground(hwnd);
                if wparam as i32 == SCREENSHOT_HOTKEY_ID {
                    open_screenshot_autosave(target_hwnd);
                } else if wparam as i32 == CLIPBOARD_HOTKEY_ID {
                    open_clipboard_manager();
                } else if wparam as i32 == SETTINGS_HOTKEY_ID {
                    open_settings();
                } else if wparam as i32 == HANDWRITE_HOTKEY_ID {
                    open_handwrite();
                } else if wparam as i32 == OCR_HOTKEY_ID {
                    open_screenshot_action(target_hwnd, ScreenshotFlowAction::Ocr);
                } else if wparam as i32 == OCR_TRANSLATE_HOTKEY_ID {
                    open_screenshot_action(target_hwnd, ScreenshotFlowAction::Translate);
                } else if wparam as i32 == TRANSLATE_HOTKEY_ID {
                    open_translate(target_hwnd);
                }
                0
            }
            WM_TIMER => {
                if !app.is_null() && wparam == TIMER_ID_STATE_POLL {
                    (*app).remember_capture_target_from_foreground(hwnd);
                    (*app).refresh_icon_if_needed(hwnd);
                    (*app).refresh_screenshot_hotkey_if_needed(hwnd, false);
                    (*app).refresh_clipboard_background_if_needed(false);
                }
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn create_single_instance_mutex() -> HANDLE {
        let name = wide(MUTEX_NAME);
        let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if mutex == 0 {
            return 0;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return 0;
        }
        mutex
    }

    fn tray_icon_guid() -> Option<GUID> {
        // Explorer binds a notification GUID to its executable path. Keep
        // the ID stable across restarts/upgrades at the same install path.
        let exe = std::env::current_exe().ok()?;
        let path = pinyin_ime::shared_rules::strip_windows_verbatim_prefix(
            exe.canonicalize().unwrap_or(exe),
        );
        let mut hash = Sha256::new();
        hash.update(b"KaixinImeTray/status-icon\0");
        hash.update(path.to_string_lossy().to_lowercase().as_bytes());
        let mut id = [0; 16];
        id.copy_from_slice(&hash.finalize()[..16]);
        id[6] = (id[6] & 0x0f) | 0x80; // UUID version 8 (custom payload)
        id[8] = (id[8] & 0x3f) | 0x80; // RFC variant
        Some(GUID::from_u128(u128::from_be_bytes(id)))
    }

    fn build_tray_menu_items(
        state: &TrayVisualState,
        translate_available: bool,
        recent_screenshot: bool,
    ) -> Vec<TrayMenuItem> {
        let pure_game = state.mode_source.as_deref() == Some("pure_game");
        let failed = state.engine_state == TrayEngineState::Failed;
        let status = if pure_game {
            "纯游戏模式 · 键盘直通".to_owned()
        } else {
            let mode = match state.input_mode() {
                TrayInputMode::Chinese => "中文输入",
                TrayInputMode::English => "英文输入",
                TrayInputMode::Unknown => "当前输入状态未知",
            };
            if state.engine_state == TrayEngineState::Ready {
                mode.to_owned()
            } else {
                format!("{mode} · 引擎{}", state.engine_state.label())
            }
        };
        let mut items = vec![TrayMenuItem::header(&status, failed && !pure_game)];
        if failed {
            let mut retry =
                TrayMenuItem::action(ID_ENGINE_RESTART, "重试输入引擎", TrayMenuIcon::Status);
            retry.attention = true;
            items.push(retry);
        }
        items.push(TrayMenuItem::action(
            ID_SETTINGS,
            "输入法设置…",
            TrayMenuIcon::Settings,
        ));
        let mut game = TrayMenuItem::action(ID_PURE_GAME_MODE, "纯游戏模式", TrayMenuIcon::Gamepad);
        game.subtitle = wide("键盘完全直通");
        game.checked = pure_game;
        items.push(game);
        let mut compat =
            TrayMenuItem::action(ID_GAME_COMPAT_MODE, "游戏输入设置…", TrayMenuIcon::Keyboard);
        compat.checked = matches!(
            state.mode_source.as_deref(),
            Some("game" | "game_chat" | "fullscreen")
        );
        items.push(compat);
        items.push(TrayMenuItem::section("截图与识别"));
        items.push(TrayMenuItem::action(
            ID_SCREENSHOT,
            "截图",
            TrayMenuIcon::Screenshot,
        ));
        items.push(TrayMenuItem::action(
            ID_OCR,
            "截图识字（OCR）",
            TrayMenuIcon::Ocr,
        ));
        items.push(TrayMenuItem::action(
            ID_OCR_TRANSLATE,
            "截图翻译",
            TrayMenuIcon::Translate,
        ));
        if recent_screenshot {
            items.push(TrayMenuItem::submenu(
                "最近截图",
                TrayMenuIcon::Recent,
                vec![
                    TrayMenuItem::action(ID_LAST_SCREENSHOT_OCR, "识别最近截图", TrayMenuIcon::Ocr),
                    TrayMenuItem::action(
                        ID_LAST_SCREENSHOT_TRANSLATE,
                        "翻译最近截图",
                        TrayMenuIcon::Translate,
                    ),
                ],
            ));
        }
        items.push(TrayMenuItem::section("其他工具"));
        items.push(TrayMenuItem::action(
            ID_CLIPBOARD_MANAGER,
            "剪贴板管理器",
            TrayMenuIcon::Clipboard,
        ));
        items.push(TrayMenuItem::action(
            ID_HANDWRITE,
            "手写查字",
            TrayMenuIcon::Handwrite,
        ));
        items.push(TrayMenuItem::action(
            ID_SYMBOLS,
            "符号大全",
            TrayMenuIcon::Symbols,
        ));
        if translate_available {
            items.push(TrayMenuItem::action(
                ID_TRANSLATE,
                "中英翻译",
                TrayMenuIcon::Translate,
            ));
        }
        items.push(TrayMenuItem::action(
            ID_CALCULATOR,
            "计算器",
            TrayMenuIcon::Calculator,
        ));
        if !failed {
            items.push(TrayMenuItem::section("输入引擎维护"));
            items.push(TrayMenuItem::action(
                ID_ENGINE_RESTART,
                if state.engine_state == TrayEngineState::Ready {
                    "重启输入引擎"
                } else {
                    "重试输入引擎"
                },
                TrayMenuIcon::Status,
            ));
        }
        let mut exit = TrayMenuItem::action(ID_EXIT, "退出托盘图标", TrayMenuIcon::Exit);
        exit.kind = TrayMenuKind::Footer;
        items.push(exit);
        items
    }

    #[cfg(test)]
    mod menu_tests {
        use super::*;

        fn state() -> TrayVisualState {
            TrayVisualState {
                ascii_mode: Some(false),
                mode_source: Some("chinese".into()),
                full_shape: Some(false),
                chinese_punctuation: Some(true),
                owner_process_id: None,
                owner_thread_id: None,
                owner_hwnd: None,
                updated_tick: None,
                sequence: None,
                engine_state: TrayEngineState::Ready,
                engine_error_code: None,
                last_recovery_reason: None,
                last_recovery_time: None,
            }
        }

        #[test]
        fn every_existing_command_survives_the_visual_reorganization() {
            let items = build_tray_menu_items(&state(), true, true);
            assert_eq!(items[1].command, ID_SETTINGS);
            assert_eq!(items[items.len() - 2].command, ID_ENGINE_RESTART);
            let mut commands = Vec::new();
            for item in &items {
                if item.command != 0 {
                    commands.push(item.command);
                }
                if let TrayMenuKind::Submenu(children) = &item.kind {
                    assert_eq!(
                        children
                            .iter()
                            .map(|child| child.command)
                            .collect::<Vec<_>>(),
                        [ID_LAST_SCREENSHOT_OCR, ID_LAST_SCREENSHOT_TRANSLATE]
                    );
                    commands.extend(children.iter().map(|child| child.command));
                }
            }
            commands.sort_unstable();
            assert_eq!(
                commands,
                vec![
                    ID_SETTINGS,
                    ID_CLIPBOARD_MANAGER,
                    ID_SCREENSHOT,
                    ID_EXIT,
                    ID_HANDWRITE,
                    ID_OCR,
                    ID_GAME_COMPAT_MODE,
                    ID_TRANSLATE,
                    ID_CALCULATOR,
                    ID_OCR_TRANSLATE,
                    ID_LAST_SCREENSHOT_OCR,
                    ID_LAST_SCREENSHOT_TRANSLATE,
                    ID_SYMBOLS,
                    ID_ENGINE_RESTART,
                    ID_PURE_GAME_MODE
                ]
            );
        }

        #[test]
        fn optional_tools_follow_the_same_availability_rules() {
            let items = build_tray_menu_items(&state(), false, false);
            assert!(!items.iter().any(|item| item.command == ID_TRANSLATE));
            assert!(!items
                .iter()
                .any(|item| matches!(item.kind, TrayMenuKind::Submenu(_))));
            for command in [
                ID_SETTINGS,
                ID_SCREENSHOT,
                ID_OCR,
                ID_OCR_TRANSLATE,
                ID_EXIT,
            ] {
                assert!(items.iter().any(|item| item.command == command));
            }
        }

        #[test]
        fn game_switch_and_recovery_attention_reflect_live_state() {
            let mut state = state();
            state.engine_state = TrayEngineState::Failed;
            state.mode_source = Some("game_chat".into());
            let items = build_tray_menu_items(&state, false, false);
            assert_eq!(items[1].command, ID_ENGINE_RESTART);
            assert!(items[1].attention);
            assert!(
                items
                    .iter()
                    .find(|i| i.command == ID_GAME_COMPAT_MODE)
                    .unwrap()
                    .checked
            );
            apply_pure_game_visual_override(&mut state, true);
            let items = build_tray_menu_items(&state, false, false);
            assert_eq!(
                state.icon_key(16),
                TrayIconKey {
                    mode: TrayIconMode::Gamepad,
                    failed: true,
                    size: 16,
                }
            );
            assert!(
                matches!(&items[0].kind, TrayMenuKind::Header(status) if *status == wide("纯游戏模式 · 键盘直通"))
            );
            assert!(
                items
                    .iter()
                    .find(|i| i.command == ID_PURE_GAME_MODE)
                    .unwrap()
                    .checked
            );
            apply_pure_game_visual_override(&mut state, false);
            state.ascii_mode = Some(false);
            state.engine_state = TrayEngineState::Ready;
            state.mode_source = Some("chinese".into());
            let items = build_tray_menu_items(&state, false, false);
            assert!(
                !items
                    .iter()
                    .find(|i| i.command == ID_PURE_GAME_MODE)
                    .unwrap()
                    .checked
            );
            assert!(
                !items
                    .iter()
                    .find(|i| i.command == ID_GAME_COMPAT_MODE)
                    .unwrap()
                    .checked
            );
            assert!(
                !items
                    .iter()
                    .find(|i| i.command == ID_ENGINE_RESTART)
                    .unwrap()
                    .attention
            );
        }
    }

    unsafe fn show_menu(hwnd: HWND, anchor: Option<POINT>) {
        let point = match anchor {
            Some(point) => point,
            None => {
                let mut point: POINT = zeroed();
                if GetCursorPos(&mut point) == 0 {
                    return;
                }
                point
            }
        };
        let Some(mut menu) = tray_menu_style::Menu::new(point) else {
            return;
        };
        for item in build_tray_menu_items(
            &read_tray_visual_state(),
            is_translate_available(),
            last_screenshot_path().is_some(),
        ) {
            menu.push(item);
        }
        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayApp;
        let target_hwnd = if !app.is_null() {
            (*app).capture_target_or_foreground(hwnd)
        } else {
            capture_target_from_foreground(hwnd)
        };
        SetForegroundWindow(hwnd);
        let command = menu.track(hwnd, point);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        drop(menu);
        if command != 0 {
            handle_menu_command(hwnd, command as usize, target_hwnd);
        }
    }

    unsafe fn handle_menu_command(hwnd: HWND, command: usize, target_hwnd: HWND) {
        match command {
            ID_PURE_GAME_MODE => toggle_pure_game_mode(hwnd),
            ID_ENGINE_RESTART => restart_engine_helper(),
            ID_GAME_COMPAT_MODE => open_settings_section(true),
            ID_SETTINGS => open_settings(),
            ID_CLIPBOARD_MANAGER => open_clipboard_manager(),
            ID_HANDWRITE => open_handwrite(),
            ID_SYMBOLS => open_symbols(target_hwnd),
            ID_TRANSLATE => open_translate(target_hwnd),
            ID_SCREENSHOT => open_screenshot_autosave(target_hwnd),
            ID_LAST_SCREENSHOT_OCR => open_last_screenshot_ocr(target_hwnd, false),
            ID_LAST_SCREENSHOT_TRANSLATE => open_last_screenshot_ocr(target_hwnd, true),
            ID_CALCULATOR => open_calculator(),
            ID_OCR => open_screenshot_action(target_hwnd, ScreenshotFlowAction::Ocr),
            ID_OCR_TRANSLATE => {
                open_screenshot_action(target_hwnd, ScreenshotFlowAction::Translate)
            }
            ID_EXIT => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }

    fn open_settings() {
        open_settings_section(false);
    }

    unsafe fn toggle_pure_game_mode(hwnd: HWND) {
        let enabled = !pure_game_mode_enabled();
        if let Err(error) = write_pure_game_mode(enabled) {
            show_error_message(&format!("无法切换纯游戏模式：{error}"));
            return;
        }
        // Release RegisterHotKey reservations immediately. TSF independently
        // observes the same flag before consuming any keyboard event.
        let app = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayApp;
        if !app.is_null() {
            (*app).refresh_screenshot_hotkey_if_needed(hwnd, true);
            (*app).refresh_icon_if_needed(hwnd);
        }
    }

    fn write_pure_game_mode(enabled: bool) -> std::io::Result<()> {
        let subkey = wide(STATE_REG_PATH);
        let name = wide(STATE_REG_VALUE_PURE_GAME_MODE);
        let mut key = 0;
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
        let value = u32::from(enabled);
        let status = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_DWORD,
                (&value as *const u32).cast(),
                size_of::<u32>() as u32,
            )
        };
        unsafe {
            RegCloseKey(key);
        }
        if status != 0 {
            Err(std::io::Error::from_raw_os_error(status as i32))
        } else {
            Ok(())
        }
    }

    fn open_settings_section(game_settings: bool) {
        let Some(path) = resolve_settings_path() else {
            show_error_message(
                "未找到设置程序 srf_ime_settings.exe。\n\
                 请确认已安装到「开始菜单」对应目录，或以下路径之一存在该文件：\n\
                 • 本程序同目录\n\
                 • %LOCALAPPDATA%\\Programs\\kaixin\\\n\
                 • %ProgramFiles(x86)%\\kaixin\\",
            );
            return;
        };

        {
            let mut last = match LAST_SETTINGS_OPEN.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let now = Instant::now();
            if let Some(t) = *last {
                if now.saturating_duration_since(t) < SETTINGS_OPEN_DEBOUNCE {
                    return;
                }
            }
            *last = Some(now);
        }

        let work_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);

        let mut cmd = Command::new(&path);
        if game_settings {
            cmd.arg("--game-settings");
        }
        if let Some(ref dir) = work_dir {
            cmd.current_dir(dir);
        }

        if let Err(e) = cmd.spawn() {
            if let Ok(mut last) = LAST_SETTINGS_OPEN.lock() {
                *last = None;
            }
            show_error_message(&format!("无法启动设置程序：{e}\n{}", path.display()));
        }
    }

    fn write_ascii_mode(ascii: bool) -> bool {
        let subkey = wide(STATE_REG_PATH);
        let mut key = 0;
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                std::ptr::null_mut(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if rc != 0 || key == 0 {
            return false;
        }

        let value: u32 = if ascii { 1 } else { 0 };
        let name = wide(STATE_REG_VALUE_ASCII);
        let ok = unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_DWORD,
                (&value as *const u32).cast(),
                size_of::<u32>() as u32,
            ) == 0
        };
        if ok {
            let live_name = wide(STATE_REG_VALUE_INPUT_ASCII);
            let _ = unsafe {
                RegSetValueExW(
                    key,
                    live_name.as_ptr(),
                    0,
                    REG_DWORD,
                    (&value as *const u32).cast(),
                    size_of::<u32>() as u32,
                )
            };
        }
        unsafe {
            RegCloseKey(key);
        }
        ok
    }

    fn open_clipboard_manager() {
        let Some(path) = resolve_clipboard_manager_path() else {
            show_error_message(
                "未找到剪贴板管理器 srf_ime_clipboard.exe。\n\
                 请确认已安装到「开始菜单」对应目录，或以下路径之一存在该文件：\n\
                 • 本程序同目录\n\
                 • %LOCALAPPDATA%\\Programs\\kaixin\\\n\
                 • %ProgramFiles(x86)%\\kaixin\\",
            );
            return;
        };
        let work_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let mut cmd = Command::new(&path);
        if let Some(ref dir) = work_dir {
            cmd.current_dir(dir);
        }
        if let Err(e) = cmd.spawn() {
            show_error_message(&format!("无法启动剪贴板管理器：{e}\n{}", path.display()));
        }
    }

    fn open_handwrite() {
        let Some(path) = resolve_handwrite_path() else {
            show_error_message(
                "未找到手写查字程序 srf_ime_handwrite.exe。\n\
                 请确认已经安装到开始菜单对应目录，或以下路径之一存在该文件：\n\
                 - 本程序同目录\n\
                 - %LOCALAPPDATA%\\Programs\\kaixin\\\n\
                 - %ProgramFiles(x86)%\\kaixin\\",
            );
            return;
        };
        let work_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let mut cmd = Command::new(&path);
        if let Some(ref dir) = work_dir {
            cmd.current_dir(dir);
        }
        if let Err(e) = cmd.spawn() {
            show_error_message(&format!("无法启动手写查字：{e}\n{}", path.display()));
        }
    }

    fn open_symbols(target_hwnd: HWND) {
        let Some(path) = resolve_runtime_exe_path(SYMBOLS_EXE, RuntimeExeKind::Symbols) else {
            show_error_message("未找到符号大全程序 srf_ime_symbols.exe，请重新安装完整安装包。");
            return;
        };
        let mut cmd = Command::new(&path);
        if let Some(dir) = path.parent() {
            cmd.current_dir(dir);
        }
        cmd.args(["--target-hwnd", &target_hwnd.to_string()]);
        if let Err(error) = cmd.spawn() {
            show_error_message(&format!("无法启动符号大全：{error}\n{}", path.display()));
        }
    }

    fn open_last_screenshot_ocr(target_hwnd: HWND, translate_after_ocr: bool) {
        let Some(image_path) = last_screenshot_path() else {
            show_error_message("没有可识别的最近截图。");
            return;
        };
        if let Err(err) = open_ocr_with_image(target_hwnd, &image_path, translate_after_ocr, false)
        {
            show_error_message(&err);
        }
    }

    fn open_ocr_with_image(
        target_hwnd: HWND,
        image_path: &Path,
        translate_after_ocr: bool,
        delete_source_after_import: bool,
    ) -> Result<(), String> {
        let Some(path) = resolve_ocr_path() else {
            return Err("未找到 OCR 程序 srf_ime_ocr.exe。".to_string());
        };
        let work_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf);
        let mut cmd = Command::new(&path);
        if let Some(ref dir) = work_dir {
            cmd.current_dir(dir);
        }
        cmd.arg("--image").arg(image_path);
        cmd.args(["--target-hwnd", &target_hwnd.to_string()]);
        if translate_after_ocr {
            external_translation::preload_translator_async();
            cmd.arg("--translate");
        }
        if delete_source_after_import {
            cmd.arg("--delete-source-after-import");
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("无法启动 OCR：{e}\n{}", path.display()))
    }

    fn open_translate(target_hwnd: HWND) {
        if let Some(reason) = external_translation::privacy_block_reason_for_window(target_hwnd) {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "translate_blocked_privacy",
                format!("target_hwnd={target_hwnd} reason={reason}"),
            );
            show_error_message(&reason);
            return;
        }
        thread::spawn(move || {
            let selection = capture_selected_text(target_hwnd)
                .ok()
                .filter(|s| !s.trim().is_empty());
            let replace_selection = selection.is_some();
            let text = selection
                .or_else(|| {
                    clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode).ok()
                })
                .unwrap_or_default();
            let mut request =
                external_translation::ExternalTranslationRequest::new(text, "kaixin-ime");
            request.target_hwnd = (target_hwnd != 0).then_some(target_hwnd);
            request.target_process_id = external_translation::process_id_for_window(target_hwnd);
            request.result_action = external_translation::preferred_result_action();
            request.presentation = "full".to_string();
            request.interactive = true;
            request.delivery = request.result_action.clone();
            request.replace_selection = replace_selection;
            if let Err(error) = external_translation::launch_full_request(&request) {
                show_error_message(&error);
            }
        });
    }

    fn capture_selected_text(target_hwnd: HWND) -> Result<String, String> {
        if target_hwnd == 0 {
            return Err("没有原窗口目标".to_string());
        }
        let backup = capture_clipboard_backup();
        if matches!(&backup, ClipboardBackup::Unsupported) {
            return Err(
                "当前剪贴板包含无法安全备份的系统对象；为避免覆盖原内容，本次未抓取选中文本。"
                    .to_string(),
            );
        }
        let initial_sequence = clipboard_win::seq_num().map(|value| value.get());
        win_paste::send_ctrl_c_to_target(target_hwnd)?;
        let started = Instant::now();
        let mut captured = None;
        let mut operation_sequence = None;
        while started.elapsed() < Duration::from_millis(750) {
            thread::sleep(Duration::from_millis(25));
            let current_sequence = clipboard_win::seq_num().map(|value| value.get());
            if matches!((initial_sequence, current_sequence), (Some(before), Some(after)) if before == after)
            {
                continue;
            }
            operation_sequence = current_sequence;
            if let Ok(text) =
                clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode)
            {
                if !text.trim().is_empty() {
                    captured = Some(text.trim().to_string());
                    break;
                }
            }
        }
        let current_sequence = clipboard_win::seq_num().map(|value| value.get());
        if operation_sequence.is_some() && current_sequence == operation_sequence {
            if let Err(error) = restore_clipboard_backup(backup) {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "translate_selection_clipboard_restore",
                    format!("status=skipped_or_failed reason={error}"),
                );
            }
        } else {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "translate_selection_clipboard_restore",
                "status=skipped reason=clipboard_changed_by_user",
            );
        }
        captured.ok_or_else(|| "目标应用没有提供可复制的选中文本".to_string())
    }

    fn open_calculator() {
        if let Err(uri_err) = shell_open("calculator:") {
            if let Err(exe_err) = Command::new("calc.exe").spawn() {
                show_error_message(&format!(
                    "无法启动系统计算器。\ncalculator: {uri_err}\ncalc.exe: {exe_err}"
                ));
            }
        }
    }

    fn shell_open(target: &str) -> Result<(), String> {
        let verb = wide("open");
        let target = wide(target);
        let result = unsafe {
            ShellExecuteW(
                0,
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!("ShellExecuteW 返回 {result}"));
        }
        Ok(())
    }

    enum ScreenshotCaptureResult {
        Captured(CapturedScreenshot),
        Cancelled,
    }

    #[derive(Clone)]
    struct CapturedScreenshot {
        image: Arc<image::RgbaImage>,
        source: SavedScreenshotSource,
        action: screenshot_region_selector::RegionAction,
        trace: ScreenshotTrace,
    }

    #[derive(Default)]
    struct ScreenshotSession {
        cancellation: screenshot_region_selector::RegionSelectorCancellation,
        trace: ScreenshotTrace,
    }

    impl ScreenshotSession {
        fn cancel(&self) {
            if self.cancellation.request() {
                runtime_log::log_screenshot_health(
                    "screenshot_cancel_request",
                    format!("capture_id={} reason=second_invocation", self.trace.id()),
                );
            }
        }

        fn is_cancelled(&self) -> bool {
            self.cancellation.is_requested()
        }
    }

    struct ActiveScreenshot(Arc<ScreenshotSession>);

    impl ActiveScreenshot {
        fn begin() -> Option<Self> {
            let mut active = ACTIVE_SCREENSHOT.lock().unwrap();
            if let Some(session) = active.as_ref() {
                session.cancel();
                return None;
            }
            let session = Arc::new(ScreenshotSession::default());
            *active = Some(session.clone());
            Some(Self(session))
        }
    }

    impl Drop for ActiveScreenshot {
        fn drop(&mut self) {
            *ACTIVE_SCREENSHOT.lock().unwrap() = None;
        }
    }

    fn cancel_active_screenshot() -> bool {
        let active = ACTIVE_SCREENSHOT.lock().unwrap();
        let Some(session) = active.as_ref() else {
            return false;
        };
        session.cancel();
        true
    }

    #[cfg(test)]
    mod screenshot_tests {
        use super::*;

        struct OutputDir(PathBuf);

        impl OutputDir {
            fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "kaixin-screenshot-test-{}-{}",
                    std::process::id(),
                    SCREENSHOT_HANDOFF_COUNTER.fetch_add(1, Ordering::Relaxed),
                ));
                fs::create_dir(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for OutputDir {
            fn drop(&mut self) {
                if let Ok(entries) = fs::read_dir(&self.0) {
                    for entry in entries.flatten() {
                        let _ = fs::remove_file(entry.path());
                    }
                }
                let _ = fs::remove_dir(&self.0);
            }
        }

        #[test]
        fn a_second_invocation_cancels_the_current_capture_without_affecting_the_next() {
            let capture = ActiveScreenshot::begin().unwrap();
            let session = capture.0.clone();
            assert!(cancel_active_screenshot());
            assert!(session.is_cancelled());
            drop(capture);
            assert!(!cancel_active_screenshot());
            let next = ActiveScreenshot::begin().unwrap();
            assert!(!next.0.is_cancelled());
        }

        #[test]
        fn overlapping_automatic_saves_do_not_replace_each_others_files() {
            let dir = OutputDir::new();
            let prefs = screenshot_prefs_from_text(
                "[screenshot]\nformat=png\nname_pattern=shot_{width}x{height}_{seq}\n",
            );
            let prefs = ScreenshotPrefs {
                save_dir: Some(dir.0.clone()),
                ..prefs
            };
            let first = image::RgbaImage::from_pixel(8, 8, image::Rgba([24, 48, 96, 255]));
            let second = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]));
            let gate = std::sync::Barrier::new(2);
            let (first_path, second_path) = thread::scope(|scope| {
                let save = |image| {
                    gate.wait();
                    save_automatic_screenshot(image, &prefs, 0, ScreenshotTrace::default()).unwrap()
                };
                let first_pixels = &first;
                let worker = scope.spawn(move || save(first_pixels));
                let second_path = save(&second);
                (worker.join().unwrap(), second_path)
            });
            assert_ne!(first_path, second_path);
            let mut names = [
                first_path.file_name().unwrap(),
                second_path.file_name().unwrap(),
            ];
            names.sort();
            assert_eq!(names, ["shot_8x8_001.png", "shot_8x8_002.png"]);
            assert_eq!(image::open(first_path).unwrap().to_rgba8(), first);
            assert_eq!(image::open(second_path).unwrap().to_rgba8(), second);
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 2);
        }

        #[test]
        fn failed_save_keeps_existing_file_and_jpeg_can_replace_it() {
            let dir = OutputDir::new();
            let path = dir.0.join("shot.jpg");
            fs::write(&path, b"previous").unwrap();
            let image = image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 50, 120, 255]));
            let trace = ScreenshotTrace::default();
            assert!(save_screenshot_image(&image, &path, image::ImageFormat::Gif, trace).is_err());
            assert_eq!(fs::read(&path).unwrap(), b"previous");
            save_screenshot_image(&image, &path, image::ImageFormat::Jpeg, trace).unwrap();
            assert_eq!(image::open(path).unwrap().to_rgb8().dimensions(), (8, 8));
            assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CaptureBackendUsed {
        Wgc,
        Dxgi,
        ScreenClip,
    }

    impl CaptureBackendUsed {
        fn as_config(self) -> &'static str {
            match self {
                Self::Wgc => "wgc",
                Self::Dxgi => "dxgi",
                Self::ScreenClip => "screenclip",
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum CapturedScreenshotKind {
        Region,
        Window,
    }

    #[derive(Clone, Copy, Debug)]
    struct SavedScreenshotSource {
        backend: CaptureBackendUsed,
        kind: CapturedScreenshotKind,
        target_hwnd: HWND,
    }

    impl SavedScreenshotSource {
        fn region(backend: CaptureBackendUsed) -> Self {
            Self {
                backend,
                kind: CapturedScreenshotKind::Region,
                target_hwnd: 0,
            }
        }

        fn window(backend: CaptureBackendUsed, target_hwnd: HWND) -> Self {
            Self {
                backend,
                kind: CapturedScreenshotKind::Window,
                target_hwnd,
            }
        }

        fn source_key(self) -> String {
            let kind = match self.kind {
                CapturedScreenshotKind::Region => "region",
                CapturedScreenshotKind::Window => "window",
            };
            format!("{}_{}", self.backend.as_config(), kind)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ScreenshotModePref {
        ManualRegion,
        CurrentWindow,
    }

    impl ScreenshotModePref {
        fn from_config(value: &str) -> Self {
            match value.trim().to_ascii_lowercase().as_str() {
                "current_window" | "window" => Self::CurrentWindow,
                _ => Self::ManualRegion,
            }
        }

        fn as_config(self) -> &'static str {
            match self {
                Self::ManualRegion => "manual_region",
                Self::CurrentWindow => "current_window",
            }
        }
    }

    struct ScreenshotCaptureOnlyArgs {
        output_path: PathBuf,
        status_path: Option<PathBuf>,
        mode: Option<ScreenshotModePref>,
    }

    /// Starts the configured capture backend from the one-shot helper process.
    fn run_screenshot_capture() {
        let arg_hwnd = screenshot_target_hwnd_from_args();
        let foreground_hwnd = unsafe { GetForegroundWindow() };
        let prefs = screenshot_prefs();
        let capture_only = screenshot_capture_only_args();
        let mode = capture_only
            .as_ref()
            .and_then(|args| args.mode)
            .unwrap_or(prefs.mode);
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenshot_helper_start",
            format!(
                "target_hwnd={} foreground_hwnd={} target_source={} backend={} mode={} capture_only={}",
                { arg_hwnd.unwrap_or(0) },
                foreground_hwnd as isize,
                if arg_hwnd.is_some() {
                    "arg"
                } else {
                    "foreground"
                },
                "native",
                mode.as_config(),
                if capture_only.is_some() { 1 } else { 0 }
            ),
        );
        let action_target = arg_hwnd.unwrap_or(foreground_hwnd);
        let capture_target = if mode == ScreenshotModePref::CurrentWindow {
            action_target
        } else {
            0
        };
        if let Some(args) = capture_only {
            let result =
                run_screenshot_capture_only(capture_target, mode, &prefs, &args.output_path);
            write_screenshot_capture_only_status(args.status_path.as_deref(), &result);
            return;
        }
        if let Some(active) = ActiveScreenshot::begin() {
            run_screenshot_flow(
                active,
                action_target,
                ScreenshotFlowAction::Configured,
                true,
            );
        }
    }

    fn screenshot_capture_only_args() -> Option<ScreenshotCaptureOnlyArgs> {
        let mut output_path = None;
        let mut status_path = None;
        let mut mode = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--capture-only-output" => output_path = args.next().map(PathBuf::from),
                "--capture-status-file" => status_path = args.next().map(PathBuf::from),
                "--capture-mode" => {
                    mode = args.next().as_deref().map(ScreenshotModePref::from_config)
                }
                _ => {}
            }
        }
        output_path.map(|output_path| ScreenshotCaptureOnlyArgs {
            output_path,
            status_path,
            mode,
        })
    }

    fn run_screenshot_capture_only(
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        prefs: &ScreenshotPrefs,
        output_path: &Path,
    ) -> Result<bool, String> {
        let session = ScreenshotSession::default();
        if let Some(parent) = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|err| format!("创建 OCR 截图交接目录失败：{err}"))?;
        }
        let captured =
            capture_with_native_fallback(target_hwnd, mode, prefs.selector_options(), &session);
        match captured? {
            ScreenshotCaptureResult::Captured(captured) => {
                if !valid_saved_screenshot(output_path) {
                    save_screenshot_image(
                        &captured.image,
                        output_path,
                        image::ImageFormat::Png,
                        session.trace,
                    )?;
                }
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_capture_only",
                    format!(
                        "status=ok backend={} source={} path={}",
                        captured.source.backend.as_config(),
                        captured.source.source_key(),
                        runtime_log::path_for_log(output_path)
                    ),
                );
                Ok(true)
            }
            ScreenshotCaptureResult::Cancelled => {
                let _ = fs::remove_file(output_path);
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_capture_only",
                    "status=cancelled",
                );
                Ok(false)
            }
        }
    }

    fn write_screenshot_capture_only_status(path: Option<&Path>, result: &Result<bool, String>) {
        let Some(path) = path else {
            return;
        };
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            let _ = fs::create_dir_all(parent);
        }
        let status = match result {
            Ok(true) => "ok".to_string(),
            Ok(false) => "cancelled".to_string(),
            Err(err) => format!("error\n{err}"),
        };
        if let Err(err) = fs::write(path, status) {
            runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "screenshot_capture_only_status",
                format!(
                    "status=failed path={} reason={err}",
                    runtime_log::path_for_log(path)
                ),
            );
        }
    }

    fn screenshot_target_hwnd_from_args() -> Option<HWND> {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--target-hwnd" {
                return args
                    .next()
                    .and_then(|value| value.parse::<isize>().ok())
                    .map(|value| value as HWND)
                    .filter(|hwnd| *hwnd != 0);
            }
        }
        None
    }

    unsafe fn capture_target_from_foreground(tray_hwnd: HWND) -> HWND {
        let hwnd = normalized_capture_target(GetForegroundWindow());
        if is_capture_target_candidate(hwnd, tray_hwnd) {
            hwnd
        } else {
            0
        }
    }

    unsafe fn normalized_capture_target(hwnd: HWND) -> HWND {
        if hwnd == 0 {
            return 0;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root != 0 {
            root
        } else {
            hwnd
        }
    }

    unsafe fn is_capture_target_candidate(hwnd: HWND, tray_hwnd: HWND) -> bool {
        let hwnd = normalized_capture_target(hwnd);
        if hwnd == 0 || hwnd == tray_hwnd {
            return false;
        }
        if IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return false;
        }

        let mut rect: RECT = zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 || rect.right <= rect.left || rect.bottom <= rect.top
        {
            return false;
        }

        let class_name = window_class_name(hwnd);
        if matches!(
            class_name.as_str(),
            CLASS_NAME
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "NotifyIconOverflowWindow"
                | "Progman"
                | "WorkerW"
        ) {
            return false;
        }

        let title = window_title(hwnd);
        if title.starts_with("开心输入法 ") {
            return false;
        }
        true
    }

    unsafe fn window_class_name(hwnd: HWND) -> String {
        let mut buffer = [0u16; 256];
        let len = GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    unsafe fn window_title(hwnd: HWND) -> String {
        let mut buffer = [0u16; 512];
        let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    fn open_screenshot_autosave(target_hwnd: HWND) {
        open_screenshot_action(target_hwnd, ScreenshotFlowAction::Configured);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ScreenshotFlowAction {
        Configured,
        Ocr,
        Translate,
    }

    fn open_screenshot_action(target_hwnd: HWND, action: ScreenshotFlowAction) {
        let Some(active) = ActiveScreenshot::begin() else {
            return;
        };
        if let Err(err) = thread::Builder::new()
            .name("screenshot-capture".into())
            .spawn(move || run_screenshot_flow(active, target_hwnd, action, false))
        {
            runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "screenshot_flow_start",
                format!("status=failed reason={err}"),
            );
        }
    }

    fn run_screenshot_flow(
        active: ActiveScreenshot,
        action_target: HWND,
        requested_action: ScreenshotFlowAction,
        one_shot: bool,
    ) {
        let session = active.0.clone();
        let capture = session.trace.stage("capture");
        let prepare = session.trace.stage("capture_prepare");
        let mut prefs = screenshot_prefs();
        match requested_action {
            ScreenshotFlowAction::Configured => {}
            ScreenshotFlowAction::Ocr => {
                prefs.ocr_after_capture = true;
                prefs.translate_after_capture = false;
            }
            ScreenshotFlowAction::Translate => {
                prefs.ocr_after_capture = true;
                prefs.translate_after_capture = true;
                external_translation::preload_translator_async();
            }
        }
        let target_hwnd = if prefs.mode == ScreenshotModePref::CurrentWindow {
            action_target
        } else {
            0
        };
        let selector_options = prefs.selector_options();
        prepare.finish("ok");
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenshot_flow_start",
            format!(
                "target_hwnd={} action_target_hwnd={} requested_action={requested_action:?} auto_save={} copy={} ocr={} translate={} format={} silent_copy={} backend=native mode={}",
                { target_hwnd },
                { action_target },
                if prefs.auto_save { 1 } else { 0 },
                if prefs.copy_after_capture { 1 } else { 0 },
                if prefs.ocr_after_capture { 1 } else { 0 },
                if prefs.translate_after_capture { 1 } else { 0 },
                prefs.format,
                if prefs.silent_copy_enabled { 1 } else { 0 },
                prefs.mode.as_config()
            ),
        );

        let captured =
            capture_with_native_fallback(target_hwnd, prefs.mode, selector_options, &session);
        drop(active);
        let captured = if session.is_cancelled() {
            Ok(ScreenshotCaptureResult::Cancelled)
        } else {
            captured
        };
        capture.finish(match &captured {
            Ok(ScreenshotCaptureResult::Captured(_)) => "ok",
            Ok(ScreenshotCaptureResult::Cancelled) => "cancelled",
            Err(err) if err.starts_with(screenshot_capture::INTERRUPTED) => "interrupted",
            Err(_) => "failed",
        });

        match captured {
            Ok(ScreenshotCaptureResult::Captured(captured)) => {
                handle_captured_screenshot(
                    captured,
                    action_target,
                    prefs,
                    selector_options.ocr_available,
                    one_shot,
                );
            }
            Ok(ScreenshotCaptureResult::Cancelled) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_flow_end",
                    "status=cancelled",
                );
            }
            Err(err) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_flow_end",
                    format!("status=failed reason={err}"),
                );
                show_error_message(&format!("截图失败：{err}"));
            }
        }
    }

    enum ClipboardBackup {
        Formats {
            raw: Vec<(u32, Vec<u8>)>,
            bitmap: Option<Vec<u8>>,
        },
        Empty,
        Unsupported,
    }

    fn capture_clipboard_backup() -> ClipboardBackup {
        const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
        let Ok(_clipboard) = clipboard_win::Clipboard::new_attempts(10) else {
            return ClipboardBackup::Unsupported;
        };
        let formats = clipboard_win::EnumFormats::new().collect::<Vec<_>>();
        if formats.is_empty() {
            return ClipboardBackup::Empty;
        }
        let mut raw = Vec::new();
        let mut bitmap = None;
        let mut total_bytes = 0usize;
        for format in formats {
            if format == clipboard_win::formats::CF_BITMAP {
                let Ok(value) = clipboard_win::get::<Vec<u8>, _>(clipboard_win::formats::Bitmap)
                else {
                    return ClipboardBackup::Unsupported;
                };
                total_bytes = total_bytes.saturating_add(value.len());
                bitmap = Some(value);
            } else {
                let mut value = Vec::new();
                if clipboard_win::raw::get_vec(format, &mut value).is_err() {
                    return ClipboardBackup::Unsupported;
                }
                total_bytes = total_bytes.saturating_add(value.len());
                raw.push((format, value));
            }
            if total_bytes > MAX_BACKUP_BYTES {
                return ClipboardBackup::Unsupported;
            }
        }
        ClipboardBackup::Formats { raw, bitmap }
    }

    fn restore_clipboard_backup(backup: ClipboardBackup) -> Result<(), String> {
        match backup {
            ClipboardBackup::Formats { raw, bitmap } => {
                let _clipboard =
                    clipboard_win::Clipboard::new_attempts(10).map_err(|err| err.to_string())?;
                clipboard_win::empty().map_err(|err| err.to_string())?;
                for (format, value) in raw {
                    clipboard_win::raw::set_without_clear(format, &value)
                        .map_err(|err| format!("还原剪贴板格式 {format} 失败：{err}"))?;
                }
                if let Some(bitmap) = bitmap {
                    clipboard_win::raw::set_bitmap_with(&bitmap, clipboard_win::options::NoClear)
                        .map_err(|err| format!("还原剪贴板位图失败：{err}"))?;
                }
                Ok(())
            }
            ClipboardBackup::Empty => {
                let _clipboard =
                    clipboard_win::Clipboard::new_attempts(10).map_err(|err| err.to_string())?;
                clipboard_win::empty().map_err(|err| err.to_string())
            }
            ClipboardBackup::Unsupported => {
                Err("原剪贴板包含暂不支持还原的自定义格式，无法安全恢复。".to_string())
            }
        }
    }

    fn capture_with_native_fallback(
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        selector_options: screenshot_region_selector::RegionSelectorOptions,
        session: &ScreenshotSession,
    ) -> Result<ScreenshotCaptureResult, String> {
        if session.is_cancelled() {
            return Ok(ScreenshotCaptureResult::Cancelled);
        }
        if mode == ScreenshotModePref::CurrentWindow && target_hwnd == 0 {
            return Err("当前窗口截图缺少有效的目标窗口，未改为区域截图。".to_string());
        }
        let window = (mode == ScreenshotModePref::CurrentWindow).then_some(target_hwnd);
        let recovery = SCREENSHOT_RESUME_RECOVERY_PENDING.swap(false, Ordering::AcqRel);
        let captured = screenshot_capture::capture_traced(
            window,
            recovery,
            || session.is_cancelled(),
            session.trace,
        );
        if session.is_cancelled() {
            return Ok(ScreenshotCaptureResult::Cancelled);
        }
        match captured {
            Ok(outcome) => {
                let backend = match outcome.backend {
                    screenshot_capture::Backend::Wgc => CaptureBackendUsed::Wgc,
                    screenshot_capture::Backend::Dxgi => CaptureBackendUsed::Dxgi,
                };
                // Selection runs once, after capture recovery. Cancellation or a
                // display change must never open a second selector/fallback UI.
                captured_frame_result(
                    outcome.frame,
                    target_hwnd,
                    mode,
                    backend,
                    selector_options,
                    session,
                )
            }
            Err(error) if error.starts_with(screenshot_capture::INTERRUPTED) => Err(error),
            Err(error) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_capture_fallback",
                    &error,
                );
                if mode == ScreenshotModePref::CurrentWindow {
                    let stage = session.trace.stage("region_fallback_prompt");
                    let confirmed = confirm_region_fallback(&error, "已尝试两次独立捕获");
                    stage.finish(if confirmed && !session.is_cancelled() {
                        "ok"
                    } else {
                        "cancelled"
                    });
                    if confirmed && !session.is_cancelled() {
                        return capture_with_native_fallback(
                            0,
                            ScreenshotModePref::ManualRegion,
                            selector_options,
                            session,
                        );
                    }
                    return Ok(ScreenshotCaptureResult::Cancelled);
                }
                capture_with_system_screenclip(session)
                    .map_err(|fallback| format!("{error}；系统截图：{fallback}"))
            }
        }
    }

    fn captured_frame_result(
        frame: windows_graphics_capture::CapturedFrame,
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        backend: CaptureBackendUsed,
        selector_options: screenshot_region_selector::RegionSelectorOptions,
        session: &ScreenshotSession,
    ) -> Result<ScreenshotCaptureResult, String> {
        if mode == ScreenshotModePref::CurrentWindow {
            runtime_log::log_screenshot_health(
                "screenshot_capture_health",
                format!(
                    "status=ok backend={} mode=current_window width={} height={} capture_ms={}",
                    backend.as_config(),
                    frame.width(),
                    frame.height(),
                    frame.elapsed.as_millis()
                ),
            );
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_capture",
                format!(
                    "status=captured backend={} mode=current_window width={} height={} elapsed_ms={}",
                    backend.as_config(),
                    frame.width(),
                    frame.height(),
                    frame.elapsed.as_millis()
                ),
            );
            return Ok(ScreenshotCaptureResult::Captured(CapturedScreenshot {
                image: Arc::new(frame.image),
                source: SavedScreenshotSource::window(backend, target_hwnd),
                action: screenshot_region_selector::RegionAction::Configured,
                trace: session.trace,
            }));
        }

        let selected = screenshot_region_selector::select_region_action_controlled(
            &frame,
            selector_options,
            &session.cancellation,
            session.trace,
        )
        .map_err(|err| {
            if err.message().starts_with(screenshot_capture::INTERRUPTED) {
                err.to_string()
            } else {
                format!("自定义框选界面失败：{err}")
            }
        })?;
        let Some((selected, action)) = selected else {
            return Ok(ScreenshotCaptureResult::Cancelled);
        };
        let image = session
            .trace
            .measure("crop", || screenshot_capture::crop(&frame, selected))?;
        runtime_log::log_screenshot_health(
            "screenshot_capture_health",
            format!(
                "status=ok backend={} mode=manual_region desktop_width={} desktop_height={} width={} height={} capture_ms={}",
                backend.as_config(), frame.width(), frame.height(), selected.width, selected.height,
                frame.elapsed.as_millis()
            ),
        );
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenshot_capture",
            format!(
                "status=captured backend={} mode=manual_region x={} y={} width={} height={} elapsed_ms={}",
                backend.as_config(),
                selected.x,
                selected.y,
                selected.width,
                selected.height,
                frame.elapsed.as_millis()
            ),
        );
        Ok(ScreenshotCaptureResult::Captured(CapturedScreenshot {
            image: Arc::new(image),
            source: SavedScreenshotSource::region(backend),
            action,
            trace: session.trace,
        }))
    }

    fn capture_with_system_screenclip(
        session: &ScreenshotSession,
    ) -> Result<ScreenshotCaptureResult, String> {
        if session.is_cancelled() {
            return Ok(ScreenshotCaptureResult::Cancelled);
        }
        let initial_seq = clipboard_win::seq_num().map(|n| n.get());
        session.trace.measure("screenclip_launch", || {
            Command::new("explorer.exe")
                .arg("ms-screenclip:")
                .spawn()
                .map_err(|err| format!("无法启动 Windows 截图工具：{err}"))
        })?;
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenclip_capture",
            "status=launched",
        );
        let stage = session.trace.stage("screenclip_wait");
        let image = wait_for_clipboard_screenshot(initial_seq, Duration::from_secs(25), session);
        stage.finish(match &image {
            Ok(Some(_)) => "ok",
            Ok(None) if session.is_cancelled() => "cancelled",
            Ok(None) => "no_image",
            Err(_) => "failed",
        });
        match image? {
            Some(image) => Ok(ScreenshotCaptureResult::Captured(CapturedScreenshot {
                image: Arc::new(image),
                source: SavedScreenshotSource::region(CaptureBackendUsed::ScreenClip),
                action: screenshot_region_selector::RegionAction::Configured,
                trace: session.trace,
            })),
            None => Ok(ScreenshotCaptureResult::Cancelled),
        }
    }

    fn wait_for_clipboard_screenshot(
        initial_seq: Option<u32>,
        timeout: Duration,
        session: &ScreenshotSession,
    ) -> Result<Option<image::RgbaImage>, String> {
        let deadline = Instant::now() + timeout;
        let png_format = clipboard_win::register_format("PNG").map(|value| value.get());
        let mut last_err = None;
        let mut saw_image_format = false;
        while Instant::now() < deadline {
            if session.is_cancelled() {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(120));
            let seq_changed = match (initial_seq, clipboard_win::seq_num().map(|n| n.get())) {
                (Some(before), Some(now)) => now != before,
                _ => true,
            };
            if seq_changed {
                if let Some(format) =
                    png_format.filter(|format| clipboard_win::is_format_avail(*format))
                {
                    saw_image_format = true;
                    match clipboard_win::get_clipboard::<Vec<u8>, _>(
                        clipboard_win::formats::RawData(format),
                    ) {
                        Ok(bytes) if !bytes.is_empty() => {
                            match image::load_from_memory_with_format(
                                &bytes,
                                image::ImageFormat::Png,
                            ) {
                                Ok(image) => {
                                    return Ok(Some(opaque_screenshot_image(&image.to_rgba8())))
                                }
                                Err(err) => last_err = Some(format!("decode clipboard PNG: {err}")),
                            }
                        }
                        Ok(_) => last_err = Some("clipboard PNG is empty".to_string()),
                        Err(err) => last_err = Some(err.to_string()),
                    }
                }

                if clipboard_win::is_format_avail(clipboard_win::formats::CF_BITMAP) {
                    saw_image_format = true;
                    match clipboard_win::get_clipboard(clipboard_win::formats::Bitmap) {
                        Ok(bytes) if !bytes.is_empty() => {
                            let image = image::load_from_memory_with_format(
                                &bytes,
                                image::ImageFormat::Bmp,
                            )
                            .map_err(|e| format!("decode clipboard bitmap: {e}"))?;
                            return Ok(Some(opaque_screenshot_image(&image.to_rgba8())));
                        }
                        Ok(_) => last_err = Some("clipboard bitmap is empty".to_string()),
                        Err(err) => last_err = Some(err.to_string()),
                    }
                }
            }
        }
        if saw_image_format {
            Err(last_err.unwrap_or_else(|| "timeout waiting for clipboard image".to_string()))
        } else {
            Ok(None)
        }
    }

    fn confirm_region_fallback(wgc_error: &str, dxgi_error: &str) -> bool {
        let title = wide("当前窗口截图失败");
        let message = wide(&format!(
            "无法捕获当前窗口。是否改用智能框选？\n\nWGC：{wgc_error}\nDXGI：{dxgi_error}"
        ));
        unsafe {
            MessageBoxW(
                0,
                message.as_ptr(),
                title.as_ptr(),
                MB_YESNO | MB_ICONWARNING,
            ) == IDYES
        }
    }

    include!("srf_ime_tray/screenshot_postprocess.rs");

    #[derive(Clone)]
    struct ScreenshotPrefs {
        auto_save: bool,
        copy_after_capture: bool,
        ocr_after_capture: bool,
        translate_after_capture: bool,
        save_dir: Option<PathBuf>,
        silent_copy_enabled: bool,
        silent_copy_dir: Option<PathBuf>,
        name_pattern: String,
        date_subdirs: bool,
        conflict_strategy: String,
        format: String,
        mode: ScreenshotModePref,
        confirm_on_release: bool,
        show_instructions: bool,
        show_quick_actions: bool,
        show_result_window: bool,
    }

    impl ScreenshotPrefs {
        fn selector_options(&self) -> screenshot_region_selector::RegionSelectorOptions {
            screenshot_region_selector::RegionSelectorOptions {
                confirm_on_release: self.confirm_on_release,
                show_instructions: self.show_instructions,
                show_quick_actions: self.show_quick_actions,
                ocr_available: resolve_ocr_path().is_some(),
                confirm_label: match (
                    self.auto_save,
                    self.copy_after_capture,
                    self.ocr_after_capture,
                    self.translate_after_capture,
                ) {
                    (true, false, false, false) => "保存完成",
                    (false, true, false, false) => "复制完成",
                    (false, false, true, false) => "识别完成",
                    (false, false, false, false) => "完成",
                    _ => "完成多项",
                },
            }
        }
    }

    fn screenshot_prefs() -> ScreenshotPrefs {
        let content = app_paths::read_config_text(config_path()).ok();
        screenshot_prefs_from_text(content.as_deref().unwrap_or_default())
    }

    fn screenshot_prefs_from_text(text: &str) -> ScreenshotPrefs {
        let config_version = config_version_from_ini(text);
        let auto_save = if config_version < 4 {
            true
        } else {
            ini_bool(text, "screenshot", "auto_save", true)
        };
        let copy_after_capture = if config_version < 6 {
            true
        } else {
            ini_bool(text, "screenshot", "copy_after_capture", true)
        };
        let mut ocr_after_capture = ini_bool(text, "screenshot", "ocr_after_capture", false);
        let mut translate_after_capture =
            ini_bool(text, "screenshot", "translate_after_capture", false);
        if config_version < 7 {
            ocr_after_capture = false;
            translate_after_capture = false;
        }
        if translate_after_capture {
            ocr_after_capture = true;
        }
        let save_dir = ini_value(text, "screenshot", "save_dir")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let silent_copy_enabled = ini_bool(text, "screenshot", "silent_copy_enabled", false);
        let silent_copy_dir = ini_value(text, "screenshot", "silent_copy_dir")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let mut name_pattern = ini_value(text, "screenshot", "name_pattern")
            .unwrap_or_else(|| "{timestamp}".to_string());
        if config_version < 11 && name_pattern.trim() == "{datetime}" {
            name_pattern = "{timestamp}".to_string();
        }
        let date_subdirs = ini_bool(text, "screenshot", "date_subdirs", false);
        let conflict_strategy = ini_value(text, "screenshot", "conflict_strategy")
            .unwrap_or_else(|| "increment".to_string())
            .trim()
            .to_ascii_lowercase();
        let format = match ini_value(text, "screenshot", "format")
            .unwrap_or_else(|| "png".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "jpg" | "jpeg" => "jpg".to_string(),
            _ => "png".to_string(),
        };
        let mode = ini_value(text, "screenshot", "mode")
            .as_deref()
            .map(ScreenshotModePref::from_config)
            .unwrap_or(ScreenshotModePref::ManualRegion);
        let confirm_on_release = ini_bool(text, "screenshot", "confirm_on_release", false);
        let show_instructions = ini_bool(text, "screenshot", "show_instructions", true);
        let show_quick_actions = ini_bool(text, "screenshot", "show_quick_actions", true);
        let show_result_window = ini_bool(text, "screenshot", "show_result_window", true);
        ScreenshotPrefs {
            auto_save,
            copy_after_capture,
            ocr_after_capture,
            translate_after_capture,
            save_dir,
            silent_copy_enabled,
            silent_copy_dir,
            name_pattern,
            date_subdirs,
            conflict_strategy: if conflict_strategy == "overwrite" {
                "overwrite".to_string()
            } else {
                "increment".to_string()
            },
            format,
            mode,
            confirm_on_release,
            show_instructions,
            show_quick_actions,
            show_result_window,
        }
    }

    fn save_automatic_screenshot(
        image: &image::RgbaImage,
        prefs: &ScreenshotPrefs,
        target_hwnd: HWND,
        trace: ScreenshotTrace,
    ) -> Result<PathBuf, String> {
        let captured_at = Local::now().fixed_offset();
        let overwrite = prefs.conflict_strategy == "overwrite";
        let (dir, output_dir, context) = trace.measure("save_prepare", || {
            let mut dir = prefs
                .save_dir
                .clone()
                .unwrap_or_else(default_screenshot_dir);
            if prefs.date_subdirs {
                dir = dir
                    .join(captured_at.format("%Y").to_string())
                    .join(captured_at.format("%m").to_string())
                    .join(captured_at.format("%d").to_string());
            }
            fs::create_dir_all(&dir)
                .map_err(|err| format!("创建截图目录 {} 失败：{err}", dir.display()))?;
            let output_dir = if overwrite {
                dir.clone()
            } else {
                fs::canonicalize(&dir)
                    .map_err(|err| format!("读取截图目录 {} 失败：{err}", dir.display()))?
            };
            let mut context = screenshot_name_context(target_hwnd);
            context.width = image.width().to_string();
            context.height = image.height().to_string();
            Ok::<_, String>((dir, output_dir, context))
        })?;
        let ext = prefs.format.as_str();
        let format = if ext == "jpg" {
            image::ImageFormat::Jpeg
        } else {
            image::ImageFormat::Png
        };
        let mut output = encode_screenshot_file(image, &output_dir, format, trace)?;
        trace.measure("save_publish", || {
            let stem_for = |index| {
                render_screenshot_name_pattern_at(
                    &prefs.name_pattern,
                    index,
                    "screenshot",
                    &captured_at,
                    &context,
                )
            };
            let base = stem_for(1);
            for index in 1..=999 {
                let stem = if index == 1 {
                    base.clone()
                } else if prefs.name_pattern.contains("{seq}") {
                    stem_for(index)
                } else {
                    format!("{base}_{index:03}")
                };
                let name = format!("{stem}.{ext}");
                match output.publish(&output_dir.join(&name), overwrite) {
                    Ok(()) => return Ok(dir.join(name)),
                    Err(err) if !overwrite && err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => return Err(format!("发布截图文件失败：{err}")),
                }
            }
            Err("截图文件名冲突过多，请更换命名规则。".to_string())
        })
    }

    fn render_screenshot_name_pattern_at(
        pattern: &str,
        seq: usize,
        fallback: &str,
        now: &DateTime<FixedOffset>,
        context: &ScreenshotNameContext,
    ) -> String {
        let pattern = if pattern.trim().is_empty() {
            "{timestamp}"
        } else {
            pattern.trim()
        };
        let rendered = pattern
            .replace("{timestamp}", &now.format("%Y%m%d_%H%M%S_%3f").to_string())
            .replace("{datetime}", &now.format("%Y%m%d_%H%M%S").to_string())
            .replace("{date}", &now.format("%Y%m%d").to_string())
            .replace("{time}", &now.format("%H%M%S").to_string())
            .replace("{year}", &now.format("%Y").to_string())
            .replace("{month}", &now.format("%m").to_string())
            .replace("{day}", &now.format("%d").to_string())
            .replace("{hour}", &now.format("%H").to_string())
            .replace("{minute}", &now.format("%M").to_string())
            .replace("{second}", &now.format("%S").to_string())
            .replace("{ms}", &now.format("%3f").to_string())
            .replace("{app}", &context.app)
            .replace("{window}", &context.window)
            .replace("{width}", &context.width)
            .replace("{height}", &context.height)
            .replace("{seq}", &format!("{seq:03}"));
        let sanitized = sanitize_filename_stem(&rendered);
        if sanitized.is_empty() {
            fallback.to_string()
        } else {
            sanitized
        }
    }

    struct ScreenshotNameContext {
        app: String,
        window: String,
        width: String,
        height: String,
    }

    fn screenshot_name_context(hwnd: HWND) -> ScreenshotNameContext {
        if hwnd == 0 {
            return ScreenshotNameContext {
                app: "desktop".to_string(),
                window: "desktop".to_string(),
                width: "0".to_string(),
                height: "0".to_string(),
            };
        }
        let window = unsafe { window_title(hwnd) };
        let app = unsafe { process_name_for_hwnd(hwnd) };
        ScreenshotNameContext {
            app: if app.is_empty() {
                "window".to_string()
            } else {
                app
            },
            window: if window.trim().is_empty() {
                "window".to_string()
            } else {
                window
            },
            width: "0".to_string(),
            height: "0".to_string(),
        }
    }

    unsafe fn process_name_for_hwnd(hwnd: HWND) -> String {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return String::new();
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return String::new();
        }
        let mut buffer = [0u16; 512];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0;
        CloseHandle(process);
        if !ok || length == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        Path::new(&path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("window")
            .to_string()
    }

    fn sanitize_filename_stem(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                out.push('_');
            } else {
                out.push(ch);
            }
        }
        let trimmed = out.trim_matches([' ', '.']).to_string();
        if trimmed.eq_ignore_ascii_case("png")
            || trimmed.eq_ignore_ascii_case("jpg")
            || trimmed.eq_ignore_ascii_case("jpeg")
        {
            return String::new();
        }
        for ext in [".png", ".jpg", ".jpeg"] {
            if trimmed.to_ascii_lowercase().ends_with(ext) {
                let end = trimmed.len().saturating_sub(ext.len());
                return trimmed[..end].trim_matches([' ', '.']).to_string();
            }
        }
        trimmed
    }

    fn default_screenshot_dir() -> PathBuf {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|p| p.join("Pictures").join("Kaixin Screenshots"))
            .or_else(|| app_paths::local_data_dir().map(|p| p.join("screenshots")))
            .unwrap_or_else(|| std::env::temp_dir().join("Kaixin Screenshots"))
    }

    fn config_path() -> PathBuf {
        app_paths::config_ini_path().unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(app_paths::CONFIG_FILE_NAME)
        })
    }

    fn read_hotkey_config(
        content: &str,
        section: &str,
        key: &str,
        fallback: &str,
    ) -> Option<HotkeySpec> {
        let value = ini_value(content, section, key);
        parse_hotkey_with_fallback(value.as_deref().unwrap_or(fallback), fallback)
    }

    fn read_bool_config(section: &str, key: &str, fallback: bool) -> bool {
        let content = fs::read_to_string(config_path()).ok();
        let value = content
            .as_deref()
            .and_then(|text| ini_value(text, section, key));
        value
            .as_deref()
            .map(|text| parse_bool_config(text, fallback))
            .unwrap_or(fallback)
    }

    fn config_version_from_ini(text: &str) -> usize {
        ini_value(text, "general", "config_version")
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0)
    }

    fn parse_bool_config(value: &str, fallback: bool) -> bool {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" | "开启" | "开" => true,
            "0" | "false" | "no" | "off" | "关闭" | "关" => false,
            _ => fallback,
        }
    }

    fn ini_bool(text: &str, section: &str, key: &str, fallback: bool) -> bool {
        ini_value(text, section, key)
            .as_deref()
            .map(|value| parse_bool_config(value, fallback))
            .unwrap_or(fallback)
    }

    fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
        let mut in_section = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
                continue;
            }
            if !in_section {
                continue;
            }
            let Some((k, v)) = trimmed.split_once('=') else {
                continue;
            };
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
        None
    }

    fn parse_hotkey(value: &str) -> Option<HotkeySpec> {
        let lowered = value.trim().to_ascii_lowercase();
        if lowered.is_empty() || matches!(lowered.as_str(), "none" | "disabled" | "off" | "关闭")
        {
            return None;
        }

        let normalized = lowered.replace(['_', '-'], "+");
        let mut modifiers = 0u32;
        let mut vk = None;
        for part in normalized
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            match part {
                "ctrl" | "control" => modifiers |= MOD_CONTROL,
                "alt" => modifiers |= MOD_ALT,
                "shift" => modifiers |= MOD_SHIFT,
                "win" | "windows" | "super" | "meta" => modifiers |= MOD_WIN,
                key => vk = hotkey_vk(key),
            }
        }
        let vk = vk?;
        if modifiers == 0 {
            return None;
        }
        Some(HotkeySpec { modifiers, vk })
    }

    fn parse_hotkey_with_fallback(value: &str, fallback: &str) -> Option<HotkeySpec> {
        const DEFAULT_MODIFIERS: u32 = MOD_CONTROL | MOD_SHIFT | MOD_ALT;
        let lowered = value.trim().to_ascii_lowercase();
        if lowered.is_empty()
            || matches!(
                lowered.as_str(),
                "none" | "disabled" | "off" | "\u{5173}\u{95ed}"
            )
        {
            return None;
        }

        let fallback_spec = parse_hotkey(fallback).unwrap_or(HotkeySpec {
            modifiers: DEFAULT_MODIFIERS,
            vk: b'A' as u32,
        });
        let normalized = lowered.replace(['_', '-'], "+");
        let mut modifiers = 0u32;
        let mut vk = None;
        for part in normalized
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            match part {
                "ctrl" | "control" => modifiers |= MOD_CONTROL,
                "alt" => modifiers |= MOD_ALT,
                "shift" => modifiers |= MOD_SHIFT,
                "win" | "windows" | "super" | "meta" => modifiers |= MOD_WIN,
                key => {
                    let Some(key_vk) = hotkey_vk(key) else {
                        return Some(fallback_spec);
                    };
                    if vk.replace(key_vk).is_some() {
                        return Some(fallback_spec);
                    }
                }
            }
        }

        Some(HotkeySpec {
            modifiers: if modifiers == 0 {
                fallback_spec.modifiers
            } else {
                modifiers
            },
            vk: vk.unwrap_or(fallback_spec.vk),
        })
    }

    fn hotkey_vk(key: &str) -> Option<u32> {
        let upper = key.to_ascii_uppercase();
        if upper.len() == 1 {
            let b = upper.as_bytes()[0];
            if b.is_ascii_alphanumeric() {
                return Some(b as u32);
            }
        }
        if let Some(rest) = upper.strip_prefix('F') {
            if let Ok(n) = rest.parse::<u32>() {
                if (1..=24).contains(&n) {
                    return Some(0x70 + n - 1);
                }
            }
        }
        match upper.as_str() {
            "SPACE" => Some(0x20),
            "TAB" => Some(0x09),
            "ENTER" | "RETURN" => Some(0x0D),
            "ESC" | "ESCAPE" => Some(0x1B),
            "SEMICOLON" => Some(0xBA),
            "EQUAL" | "EQUALS" => Some(0xBB),
            "COMMA" => Some(0xBC),
            "MINUS" => Some(0xBD),
            "PERIOD" | "DOT" => Some(0xBE),
            "SLASH" => Some(0xBF),
            "QUOTE" | "APOSTROPHE" => Some(0xDE),
            _ => None,
        }
    }

    /// 与 TSF 安装布局一致：托盘目录、每用户 ProgramFiles、系统 ProgramFiles。
    #[derive(Clone, Copy)]
    enum RuntimeExeKind {
        Settings,
        ClipboardManager,
        Handwrite,
        Symbols,
        Ocr,
    }

    fn resolve_settings_path() -> Option<PathBuf> {
        resolve_runtime_exe_path(SETTINGS_EXE, RuntimeExeKind::Settings)
    }

    fn resolve_clipboard_manager_path() -> Option<PathBuf> {
        resolve_runtime_exe_path(CLIPBOARD_MANAGER_EXE, RuntimeExeKind::ClipboardManager)
    }

    fn resolve_handwrite_path() -> Option<PathBuf> {
        resolve_runtime_exe_path(HANDWRITE_EXE, RuntimeExeKind::Handwrite)
    }

    fn resolve_ocr_path() -> Option<PathBuf> {
        resolve_runtime_exe_path(OCR_EXE, RuntimeExeKind::Ocr)
    }

    fn is_translate_available() -> bool {
        external_translation::is_available()
    }

    fn resolve_runtime_exe_path(exe_name: &str, kind: RuntimeExeKind) -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(exe_name));
            }
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(la)
                    .join("Programs")
                    .join(app_paths::APP_PATH_NAME)
                    .join(exe_name),
            );
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            candidates.push(
                PathBuf::from(pf)
                    .join(app_paths::APP_PATH_NAME)
                    .join(exe_name),
            );
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            candidates.push(
                PathBuf::from(pf86)
                    .join(app_paths::APP_PATH_NAME)
                    .join(exe_name),
            );
        }
        candidates
            .into_iter()
            .find(|p| p.is_file() && runtime_exe_matches_kind(p, kind))
    }

    fn runtime_exe_matches_kind(path: &Path, kind: RuntimeExeKind) -> bool {
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        let has_settings_title = contains_bytes(&bytes, SETTINGS_WINDOW_TITLE_BYTES);
        let has_clipboard_title = contains_bytes(&bytes, CLIPBOARD_WINDOW_TITLE_BYTES)
            || contains_utf16le(&bytes, "开心输入法 剪贴板");
        let has_handwrite_title = contains_bytes(&bytes, HANDWRITE_WINDOW_TITLE_BYTES);
        let has_ocr_title = contains_bytes(&bytes, OCR_WINDOW_TITLE_BYTES);
        match kind {
            RuntimeExeKind::Settings => {
                has_settings_title && !has_clipboard_title && !has_ocr_title
            }
            RuntimeExeKind::ClipboardManager => has_clipboard_title,
            RuntimeExeKind::Handwrite => has_handwrite_title,
            RuntimeExeKind::Symbols => contains_bytes(&bytes, "开心输入法 符号大全".as_bytes()),
            RuntimeExeKind::Ocr => has_ocr_title,
        }
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|window| window == needle)
    }

    /// 检测 UTF-16LE 编码的窗口标题（WPF/XAML 资源通常以 UTF-16 存储）。
    fn contains_utf16le(haystack: &[u8], needle: &str) -> bool {
        if needle.is_empty() {
            return false;
        }
        let encoded: Vec<u8> = needle
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        contains_bytes(haystack, &encoded)
    }

    /// shellapi.h：`NIN_SELECT` / `NIN_KEYSELECT`（= `WM_USER+0/+1`），通知区 v4 常用。
    fn normalized_install_root() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?.to_path_buf();
        Some(pinyin_ime::shared_rules::normalize_install_root(&dir))
    }

    fn ensure_engine_helper_running() {
        if install_maintenance_active() {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "engine_helper_start",
                "spawned=0 reason=install_maintenance",
            );
            return;
        }

        let Some(install_root) = normalized_install_root() else {
            return;
        };
        let engine = install_root.join(ENGINE_EXE);
        if !engine.is_file() {
            return;
        }
        let suffix =
            pinyin_ime::shared_rules::engine_instance_suffix_for_install_root(&install_root);
        let pipe_name = pinyin_ime::shared_rules::engine_pipe_name_for_suffix(&suffix);
        let mutex_name = pinyin_ime::shared_rules::engine_mutex_name_for_suffix(&suffix);
        let lexicon_dir = install_root.join("lexicon");
        let log_detail = format!(
            "spawned=1 install_root={} suffix={suffix} lexicon_dir={}",
            install_root.display(),
            lexicon_dir.display()
        );
        let mut command = Command::new(engine);
        command
            .arg("--pipe-name")
            .arg(pipe_name)
            .arg("--mutex-name")
            .arg(mutex_name);
        if lexicon_dir.is_dir() {
            command.arg("--lexicon-dir").arg(&lexicon_dir);
        }
        let _ = command
            .current_dir(install_root)
            .spawn()
            .map(|_| {
                runtime_log::log_tray(RuntimeLogLevel::Basic, "engine_helper_start", log_detail)
            })
            .map_err(|err| {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "engine_helper_start",
                    format!("spawned=0 error={err}"),
                )
            });
    }

    fn restart_engine_helper() {
        if install_maintenance_active() {
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "engine_helper_restart",
                "status=skipped reason=install_maintenance",
            );
            return;
        }
        let Some(install_root) = normalized_install_root() else {
            return;
        };
        let engine = install_root.join(ENGINE_EXE);
        if !engine.is_file() {
            show_error_message("未找到输入引擎程序，请修复或重新安装开心输入法。");
            return;
        }
        let suffix =
            pinyin_ime::shared_rules::engine_instance_suffix_for_install_root(&install_root);
        let pipe_name = pinyin_ime::shared_rules::engine_pipe_name_for_suffix(&suffix);
        let mutex_name = pinyin_ime::shared_rules::engine_mutex_name_for_suffix(&suffix);
        let lexicon_dir = install_root.join("lexicon");
        let mut command = Command::new(engine);
        command
            .arg("--restart")
            .arg("--pipe-name")
            .arg(pipe_name)
            .arg("--mutex-name")
            .arg(mutex_name);
        if lexicon_dir.is_dir() {
            command.arg("--lexicon-dir").arg(&lexicon_dir);
        }
        match command.current_dir(install_root).spawn() {
            Ok(_) => runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "engine_helper_restart",
                "status=queued",
            ),
            Err(err) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "engine_helper_restart",
                    format!("status=failed error_kind={:?}", err.kind()),
                );
                show_error_message("无法启动引擎恢复程序，请检查安装目录和安全软件拦截记录。");
            }
        }
    }

    const NIN_SELECT: u32 = WM_USER;
    const NIN_KEYSELECT: u32 = WM_USER + 1;

    fn tray_event_code(lparam: LPARAM) -> u32 {
        (lparam as u32) & 0xFFFF
    }

    fn tray_anchor_point(wparam: WPARAM) -> POINT {
        let packed = wparam as u32;
        POINT {
            x: (packed & 0xFFFF) as i16 as i32,
            y: ((packed >> 16) & 0xFFFF) as i16 as i32,
        }
    }

    fn show_error_message(text: &str) {
        let title = wide(app_paths::APP_DISPLAY_NAME);
        let msg = wide(text);
        unsafe {
            MessageBoxW(0, msg.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }

    include!("srf_ime_tray/input_state.rs");

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn write_wide_buffer(buffer: &mut [u16], value: &str) {
        if buffer.is_empty() {
            return;
        }
        let encoded = wide(value);
        let max_len = buffer
            .len()
            .saturating_sub(1)
            .min(encoded.len().saturating_sub(1));
        buffer[..max_len].copy_from_slice(&encoded[..max_len]);
        buffer[max_len] = 0;
    }

    fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
        (r as COLORREF) | ((g as COLORREF) << 8) | ((b as COLORREF) << 16)
    }

    mod pinned_screenshot {
        use super::*;
        use screenshot_region_selector::RegionAction;
        use std::sync::mpsc::{self, Receiver, TryRecvError};
        use windows_sys::Win32::Graphics::Gdi::{
            BeginPaint, DeleteObject, EndPaint, GetSysColor, GetSysColorBrush, SetBkColor,
            SetTextColor, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLOR_WINDOW,
            COLOR_WINDOWTEXT, DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
        };
        use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            EnableWindow, GetKeyState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClientRect, IsDialogMessageW, MoveWindow, SendMessageW, SetWindowPos,
            SetWindowTextW, ShowWindow, SystemParametersInfoW, CREATESTRUCTW, GWLP_USERDATA,
            HWND_TOPMOST, MINMAXINFO, SPI_GETHIGHCONTRAST, SW_SHOW, SW_SHOWNOACTIVATE, WM_CLOSE,
            WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_PAINT,
            WM_SETFONT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
        };

        const CLASS: &str = "KaixinPinnedScreenshot_v1";
        const ID_COPY: usize = 2101;
        const ID_SAVE: usize = 2102;
        const ID_OCR: usize = 2103;
        const ID_ANNOTATE: usize = 2104;
        const ID_PIN: usize = 2105;
        const ID_CLOSE: usize = 2106;
        const RESULT_TIMER_ID: usize = 1;
        const RESULT_POLL_MS: u32 = 80;

        pub(super) enum InitialAction {
            Run(ScreenshotPrefs),
            Completed(Result<bool, String>),
        }

        struct PendingAction {
            action: RegionAction,
            receiver: Receiver<Result<bool, String>>,
        }

        struct ActionJob {
            worker: thread::JoinHandle<()>,
            receiver: Receiver<Result<bool, String>>,
        }

        fn spawn_action_job(
            work: impl FnOnce() -> (Result<bool, String>, Option<SilentScreenshotCopy>) + Send + 'static,
        ) -> io::Result<ActionJob> {
            let (sender, receiver) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("screenshot-output".into())
                .spawn(move || {
                    let (result, silent_copy) = work();
                    let _ = sender.send(result);
                    if let Some(copy) = silent_copy {
                        copy.run();
                    }
                })?;
            Ok(ActionJob { worker, receiver })
        }

        struct ResultActions {
            captured: CapturedScreenshot,
            target: HWND,
            ocr_available: bool,
            buttons: Vec<HWND>,
            status_label: HWND,
            initial_status: String,
            button_font: isize,
            status_font: isize,
            status_failed: bool,
            pending: Vec<PendingAction>,
            wait_for_workers: bool,
            workers: Vec<thread::JoinHandle<()>>,
        }
        struct State {
            image: Arc<image::RgbaImage>,
            trace: ScreenshotTrace,
            pixels: Vec<u8>,
            width: i32,
            height: i32,
            info: BITMAPINFO,
            actions: Option<ResultActions>,
        }

        pub(super) fn spawn(image: Arc<image::RgbaImage>, trace: ScreenshotTrace) {
            thread::spawn(move || {
                if let Err(err) = run(image, trace, None, None) {
                    runtime_log::log_tray(
                        RuntimeLogLevel::Error,
                        "screenshot_pin",
                        format!("status=failed reason={err}"),
                    );
                }
            });
        }

        pub(super) fn show_result(
            captured: CapturedScreenshot,
            target: HWND,
            ocr_available: bool,
            initial: InitialAction,
            wait_for_workers: bool,
        ) -> Result<(), String> {
            let (initial_status, status_failed, initial_job) = match initial {
                InitialAction::Run(prefs) => {
                    (pending_status(captured.action).into(), false, Some(prefs))
                }
                InitialAction::Completed(result) => {
                    let (status, failed) = completed_status(captured.action, result);
                    (status, failed, None)
                }
            };
            run(
                captured.image.clone(),
                captured.trace,
                Some(ResultActions {
                    captured,
                    target,
                    ocr_available,
                    buttons: Vec::new(),
                    status_label: 0,
                    initial_status,
                    button_font: 0,
                    status_font: 0,
                    status_failed,
                    pending: Vec::new(),
                    wait_for_workers,
                    workers: Vec::new(),
                }),
                initial_job,
            )
        }

        fn run(
            image: Arc<image::RgbaImage>,
            trace: ScreenshotTrace,
            actions: Option<ResultActions>,
            initial_job: Option<ScreenshotPrefs>,
        ) -> Result<(), String> {
            let is_result = actions.is_some();
            let stage = trace.stage(if is_result {
                "preview_first_paint"
            } else {
                "pin_first_paint"
            });
            let width = i32::try_from(image.width()).map_err(|_| "固定截图宽度过大")?;
            let height = i32::try_from(image.height()).map_err(|_| "固定截图高度过大")?;
            let pixels = image
                .pixels()
                .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], 255])
                .collect::<Vec<u8>>();
            let mut info: BITMAPINFO = unsafe { zeroed() };
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let class = wide(CLASS);
            let title_text = actions
                .as_ref()
                .map(|_| "截图结果".to_string())
                .unwrap_or_else(|| {
                    "固定截图 · 拖动边框缩放 · Ctrl+C 复制 · 关闭取消固定".to_string()
                });
            let title = wide(&title_text);
            let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
            let wc = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: unsafe { LoadCursorW(0, IDC_ARROW) },
                hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
                style: CS_HREDRAW | CS_VREDRAW,
                ..unsafe { zeroed() }
            };
            unsafe { RegisterClassW(&wc) };
            let mut state = Box::new(State {
                image,
                trace,
                pixels,
                width,
                height,
                info,
                actions,
            });
            let state_ptr = &raw mut *state;
            let scale = (900.0 / width as f64).min(700.0 / height as f64).min(1.0);
            let client_width = (width as f64 * scale).round() as i32;
            let client_height = (height as f64 * scale).round() as i32;
            let dpi = unsafe { GetDpiForSystem() }.max(96) as i32;
            let hwnd = unsafe {
                CreateWindowExW(
                    if is_result {
                        WS_EX_TOOLWINDOW
                    } else {
                        WS_EX_TOPMOST | WS_EX_TOOLWINDOW
                    },
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    client_width.max(if is_result { 620 * dpi / 96 } else { 160 }) + 20,
                    client_height.max(90) + if is_result { 130 * dpi / 96 } else { 48 },
                    0,
                    0,
                    instance,
                    state_ptr.cast(),
                )
            };
            if hwnd == 0 {
                stage.finish("failed");
                return Err("创建固定截图窗口失败".to_string());
            }
            unsafe {
                let show = if is_result && ACTIVE_SCREENSHOT.lock().unwrap().is_some() {
                    SW_SHOWNOACTIVATE
                } else {
                    SW_SHOW
                };
                ShowWindow(hwnd, show);
                windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                if !is_result {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                            | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                    );
                }
            }
            stage.finish("ok");
            if let (Some(prefs), Some(actions)) = (initial_job, state.actions.as_ref()) {
                unsafe { queue_action(hwnd, state_ptr, actions.captured.action, Some(prefs)) };
            }
            let mut msg: MSG = unsafe { zeroed() };
            while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {
                unsafe {
                    if !is_result || msg.message == WM_TIMER || IsDialogMessageW(hwnd, &msg) == 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            if unsafe { IsWindow(hwnd) } != 0 {
                unsafe { DestroyWindow(hwnd) };
            }
            if let Some(actions) = state.actions.as_mut() {
                actions.pending.clear();
                for worker in actions.workers.drain(..) {
                    let _ = worker.join();
                }
            }
            Ok(())
        }

        fn pending_status(action: RegionAction) -> &'static str {
            match action {
                RegionAction::SaveAs => "正在另存，可继续截图",
                RegionAction::Copy => "正在复制图片",
                RegionAction::Ocr => "正在启动 OCR",
                RegionAction::Annotate => "正在准备画图",
                RegionAction::Pin => "正在固定图片",
                RegionAction::Configured => "正在处理截图，可继续截图",
            }
        }

        fn completed_status(action: RegionAction, result: Result<bool, String>) -> (String, bool) {
            let status = match result {
                Ok(true) => match action {
                    RegionAction::SaveAs => "图片已保存",
                    RegionAction::Copy => "图片已复制",
                    RegionAction::Ocr => "OCR 已启动",
                    RegionAction::Annotate => "已在画图中打开",
                    RegionAction::Pin => "图片已固定",
                    RegionAction::Configured => "截图处理完成",
                },
                Ok(false) => "已取消另存，图片仍保留",
                Err(error) => return (format!("操作失败，可重试：{error}"), true),
            };
            (status.to_string(), false)
        }

        fn action_for_button(id: usize) -> Option<RegionAction> {
            match id {
                ID_COPY => Some(RegionAction::Copy),
                ID_SAVE => Some(RegionAction::SaveAs),
                ID_OCR => Some(RegionAction::Ocr),
                ID_ANNOTATE => Some(RegionAction::Annotate),
                ID_PIN => Some(RegionAction::Pin),
                _ => None,
            }
        }

        unsafe fn refresh_action_buttons(ptr: *mut State) {
            let Some(actions) = ptr.as_ref().and_then(|state| state.actions.as_ref()) else {
                return;
            };
            let states = [ID_COPY, ID_SAVE, ID_OCR, ID_ANNOTATE, ID_PIN]
                .into_iter()
                .zip(&actions.buttons)
                .filter_map(|(id, &button)| {
                    let action = action_for_button(id)?;
                    let enabled = (action != RegionAction::Ocr || actions.ocr_available)
                        && !actions.pending.iter().any(|job| job.action == action);
                    Some((button, enabled))
                })
                .collect::<Vec<_>>();
            for (button, enabled) in states {
                EnableWindow(button, i32::from(enabled));
            }
        }

        unsafe fn update_action_status(ptr: *mut State, mut status: String, failed: bool) {
            let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) else {
                return;
            };
            if let Some(job) = actions.pending.last() {
                if status != pending_status(job.action) {
                    let wait_status = pending_status(job.action);
                    status.push_str(" · ");
                    status.push_str(wait_status);
                }
            }
            actions.status_failed = failed;
            let label = actions.status_label;
            SetWindowTextW(label, wide(&status).as_ptr());
        }

        unsafe fn queue_action(
            hwnd: HWND,
            ptr: *mut State,
            action: RegionAction,
            prefs: Option<ScreenshotPrefs>,
        ) {
            let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) else {
                return;
            };
            if (action == RegionAction::Ocr && !actions.ocr_available)
                || actions.pending.iter().any(|job| job.action == action)
            {
                return;
            }
            let stage = actions.captured.trace.stage("output_dispatch");
            if actions.pending.is_empty()
                && SetTimer(hwnd, RESULT_TIMER_ID, RESULT_POLL_MS, None) == 0
            {
                stage.finish("failed");
                update_action_status(ptr, "无法启动截图后台操作，请重试。".into(), true);
                return;
            }
            let mut captured = actions.captured.clone();
            captured.action = action;
            let target = actions.target;
            let job = spawn_action_job(move || {
                let stage = captured.trace.stage("output_prepare");
                let prefs = prefs.unwrap_or_else(screenshot_prefs);
                stage.finish("ok");
                process_captured_screenshot(&captured, target, &prefs)
            });
            match job {
                Ok(ActionJob { worker, receiver }) => {
                    stage.finish("ok");
                    actions.pending.push(PendingAction { action, receiver });
                    if actions.wait_for_workers {
                        actions.workers.retain(|worker| !worker.is_finished());
                        actions.workers.push(worker);
                    }
                    refresh_action_buttons(ptr);
                    update_action_status(ptr, pending_status(action).into(), false);
                }
                Err(err) => {
                    stage.finish("failed");
                    if actions.pending.is_empty() {
                        KillTimer(hwnd, RESULT_TIMER_ID);
                    }
                    update_action_status(ptr, format!("无法启动截图后台操作：{err}"), true);
                }
            }
        }

        unsafe fn poll_actions(hwnd: HWND, ptr: *mut State) {
            let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) else {
                return;
            };
            let mut completed = None;
            actions.pending.retain(|job| {
                let result = match job.receiver.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => return true,
                    Err(TryRecvError::Disconnected) => Err("截图后台操作已中断。".to_string()),
                };
                let status = completed_status(job.action, result);
                if status.1 || completed.as_ref().is_none_or(|(_, failed)| !failed) {
                    completed = Some(status);
                }
                false
            });
            if actions.pending.is_empty() {
                KillTimer(hwnd, RESULT_TIMER_ID);
            }
            if let Some((status, failed)) = completed {
                refresh_action_buttons(ptr);
                update_action_status(ptr, status, failed);
            }
        }

        unsafe extern "system" fn window_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if message == WM_NCCREATE {
                let create = &*(lparam as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            }
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            match message {
                WM_KEYDOWN if wparam == 0x43 && GetKeyState(i32::from(VK_CONTROL)) < 0 => {
                    if let Some(state) = ptr.as_ref() {
                        if state.actions.is_some() {
                            queue_action(hwnd, ptr, RegionAction::Copy, None);
                            return 0;
                        }
                        match copy_image_to_clipboard(&state.image, state.trace) {
                            Ok(()) => {
                                SetWindowTextW(hwnd, wide("固定截图 · 图片已复制").as_ptr());
                            }
                            Err(err) => show_error_message(&format!("复制失败：{err}")),
                        }
                    }
                    0
                }
                WM_CREATE => {
                    if let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) {
                        actions.status_label = CreateWindowExW(
                            0,
                            wide("STATIC").as_ptr(),
                            wide(&actions.initial_status).as_ptr(),
                            WS_CHILD | WS_VISIBLE,
                            0,
                            0,
                            1,
                            1,
                            hwnd,
                            0,
                            GetModuleHandleW(std::ptr::null()),
                            std::ptr::null(),
                        );
                        let specs = [
                            (ID_COPY, "复制(&C)"),
                            (ID_SAVE, "另存(&S)"),
                            (ID_OCR, "OCR(&O)"),
                            (ID_ANNOTATE, "画图(&A)"),
                            (ID_PIN, "固定(&P)"),
                            (ID_CLOSE, "关闭(&X)"),
                        ];
                        for (id, label) in specs {
                            let title = wide(if id == ID_OCR && !actions.ocr_available {
                                "OCR未安装"
                            } else {
                                label
                            });
                            let button = CreateWindowExW(
                                0,
                                wide("BUTTON").as_ptr(),
                                title.as_ptr(),
                                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                0,
                                0,
                                1,
                                1,
                                hwnd,
                                id as _,
                                GetModuleHandleW(std::ptr::null()),
                                std::ptr::null(),
                            );
                            if id == ID_OCR && !actions.ocr_available {
                                EnableWindow(button, 0);
                            }
                            actions.buttons.push(button);
                        }
                        update_fonts(actions, GetDpiForWindow(hwnd).max(96));
                    }
                    0
                }
                WM_GETMINMAXINFO if ptr.as_ref().is_some_and(|state| state.actions.is_some()) => {
                    let info = &mut *(lparam as *mut MINMAXINFO);
                    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                    info.ptMinTrackSize.x = 350 * dpi / 96;
                    info.ptMinTrackSize.y = 260 * dpi / 96;
                    0
                }
                WM_DPICHANGED if ptr.as_ref().is_some_and(|state| state.actions.is_some()) => {
                    let dpi = (wparam & 0xffff) as u32;
                    if let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) {
                        update_fonts(actions, dpi);
                    }
                    let suggested = &*(lparam as *const RECT);
                    SetWindowPos(
                        hwnd,
                        0,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                            | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
                    );
                    0
                }
                WM_SIZE => {
                    if let Some((status_label, buttons)) = ptr
                        .as_ref()
                        .and_then(|state| state.actions.as_ref())
                        .map(|actions| (actions.status_label, actions.buttons.clone()))
                    {
                        let mut rect: RECT = zeroed();
                        GetClientRect(hwnd, &mut rect);
                        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                        let (gap, button_height, columns, bar_height) =
                            layout_metrics(rect.right, dpi);
                        let width = ((rect.right - gap * (columns + 1)) / columns).max(1);
                        MoveWindow(
                            status_label,
                            gap,
                            (rect.bottom - bar_height + gap).max(0),
                            (rect.right - gap * 2).max(1),
                            24 * dpi / 96,
                            1,
                        );
                        for (index, button) in buttons.iter().enumerate() {
                            let index = index as i32;
                            MoveWindow(
                                *button,
                                gap + (index % columns) * (width + gap),
                                (rect.bottom - bar_height
                                    + 28 * dpi / 96
                                    + (index / columns) * (button_height + gap))
                                    .max(0),
                                width,
                                button_height,
                                1,
                            );
                        }
                    }
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_COMMAND if (wparam >> 16) == 0 => {
                    let id = wparam & 0xffff;
                    if id == ID_CLOSE {
                        DestroyWindow(hwnd);
                        return 0;
                    }
                    if let Some(action) = action_for_button(id) {
                        queue_action(hwnd, ptr, action, None);
                        return 0;
                    }
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_TIMER if wparam == RESULT_TIMER_ID => {
                    poll_actions(hwnd, ptr);
                    0
                }
                WM_CTLCOLORSTATIC => {
                    if let Some(actions) = ptr.as_ref().and_then(|state| state.actions.as_ref()) {
                        if lparam as HWND == actions.status_label {
                            let mut contrast: HIGHCONTRASTW = zeroed();
                            contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
                            let high_contrast = SystemParametersInfoW(
                                SPI_GETHIGHCONTRAST,
                                contrast.cbSize,
                                (&mut contrast as *mut HIGHCONTRASTW).cast(),
                                0,
                            ) != 0
                                && (contrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
                            let background = GetSysColor(COLOR_WINDOW);
                            let brightness = (background & 255)
                                + ((background >> 8) & 255)
                                + ((background >> 16) & 255);
                            let color = if high_contrast {
                                GetSysColor(COLOR_WINDOWTEXT)
                            } else if actions.status_failed {
                                if brightness < 384 {
                                    rgb(255, 150, 150)
                                } else {
                                    rgb(160, 25, 25)
                                }
                            } else if brightness < 384 {
                                rgb(170, 225, 190)
                            } else {
                                rgb(26, 103, 55)
                            };
                            SetTextColor(wparam as isize, color);
                            SetBkColor(wparam as isize, background);
                            return GetSysColorBrush(COLOR_WINDOW) as isize;
                        }
                    }
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_PAINT => {
                    let mut paint: PAINTSTRUCT = zeroed();
                    let hdc = BeginPaint(hwnd, &mut paint);
                    if let Some(state) = ptr.as_ref() {
                        let mut rect: RECT = zeroed();
                        GetClientRect(hwnd, &mut rect);
                        let (_, _, _, bar_height) =
                            layout_metrics(rect.right, GetDpiForWindow(hwnd).max(96) as i32);
                        let available_h = if state.actions.is_some() {
                            (rect.bottom - bar_height).max(1)
                        } else {
                            rect.bottom.max(1)
                        };
                        let scale = (rect.right.max(1) as f64 / state.width as f64)
                            .min(available_h as f64 / state.height as f64);
                        let draw_width = (state.width as f64 * scale).round() as i32;
                        let draw_height = (state.height as f64 * scale).round() as i32;
                        StretchDIBits(
                            hdc,
                            (rect.right - draw_width) / 2,
                            (available_h - draw_height) / 2,
                            draw_width,
                            draw_height,
                            0,
                            0,
                            state.width,
                            state.height,
                            state.pixels.as_ptr().cast(),
                            &state.info,
                            DIB_RGB_COLORS,
                            SRCCOPY,
                        );
                    }
                    EndPaint(hwnd, &paint);
                    0
                }
                WM_CLOSE => {
                    DestroyWindow(hwnd);
                    0
                }
                WM_NCDESTROY => {
                    if !ptr.is_null() {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                        if let Some(actions) = (*ptr).actions.as_ref() {
                            if actions.button_font != 0 {
                                DeleteObject(actions.button_font);
                            }
                            if actions.status_font != 0 {
                                DeleteObject(actions.status_font);
                            }
                        }
                    }
                    windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                _ => DefWindowProcW(hwnd, message, wparam, lparam),
            }
        }

        fn layout_metrics(width: i32, dpi: i32) -> (i32, i32, i32, i32) {
            let dpi = dpi.max(96);
            let gap = (6 * dpi / 96).max(1);
            let button_height = (36 * dpi / 96).max(1);
            let columns = if width >= 600 * dpi / 96 { 6 } else { 3 };
            let rows = 6 / columns;
            (
                gap,
                button_height,
                columns,
                28 * dpi / 96 + rows * (button_height + gap) + gap,
            )
        }

        unsafe fn update_fonts(actions: &mut ResultActions, dpi: u32) {
            let button_font = screenshot_region_selector::create_screenshot_ui_font(dpi, 15);
            let status_font = screenshot_region_selector::create_screenshot_ui_font(dpi, 14);
            if button_font != 0 {
                for button in &actions.buttons {
                    SendMessageW(*button, WM_SETFONT, button_font as usize, 1);
                }
                if actions.button_font != 0 {
                    DeleteObject(actions.button_font);
                }
                actions.button_font = button_font;
            }
            if status_font != 0 {
                SendMessageW(actions.status_label, WM_SETFONT, status_font as usize, 1);
                if actions.status_font != 0 {
                    DeleteObject(actions.status_font);
                }
                actions.status_font = status_font;
            }
        }

        #[cfg(test)]
        mod tests {
            use super::{layout_metrics, spawn_action_job};
            use std::sync::mpsc::{self, TryRecvError};
            use std::time::Duration;

            #[test]
            fn a_slow_output_job_does_not_hold_up_another_result_action() {
                let (started, has_started) = mpsc::channel();
                let (release, blocked) = mpsc::channel();
                let deadline = Duration::from_secs(5);
                let slow = spawn_action_job(move || {
                    started.send(()).unwrap();
                    blocked.recv_timeout(deadline).unwrap();
                    (Ok(true), None)
                })
                .unwrap();
                has_started.recv_timeout(deadline).unwrap();
                assert!(matches!(slow.receiver.try_recv(), Err(TryRecvError::Empty)));

                let other = spawn_action_job(|| (Ok(true), None)).unwrap();
                assert_eq!(other.receiver.recv_timeout(deadline).unwrap(), Ok(true));
                release.send(()).unwrap();
                assert_eq!(slow.receiver.recv_timeout(deadline).unwrap(), Ok(true));
                slow.worker.join().unwrap();
                other.worker.join().unwrap();
            }

            #[test]
            fn result_buttons_fit_wide_and_narrow_windows_at_common_dpis() {
                for dpi in [96, 120, 144, 192] {
                    for (logical_width, expected_columns) in [(350, 3), (620, 6)] {
                        let width = logical_width * dpi / 96;
                        let (gap, height, columns, bar) = layout_metrics(width, dpi);
                        let button_width = (width - gap * (columns + 1)) / columns;
                        assert_eq!(columns, expected_columns);
                        assert!(
                            button_width >= 90 * dpi / 96,
                            "dpi={dpi} width={logical_width}"
                        );
                        assert!(height >= 36 * dpi / 96);
                        assert!(bar >= (28 + 36 * (6 / columns)) * dpi / 96);
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    pinyin_ime::windows_security::apply_process_hardening();
    if std::env::args().any(|arg| arg == "--translation-preview") {
        if let Err(error) = translation_preview::run() {
            let message: Vec<u16> = error.encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "翻译预览".encode_utf16().chain(Some(0)).collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    0,
                    message.as_ptr(),
                    title.as_ptr(),
                    0x10,
                );
            }
        }
        return;
    }
    win::run();
}
