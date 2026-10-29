#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[path = "srf_ime_tray/menu_style.rs"]
mod tray_menu_style;

#[cfg(windows)]
mod win {
    use crate::tray_menu_style::{
        self, Icon as TrayMenuIcon, Item as TrayMenuItem, Kind as TrayMenuKind,
    };
    use chrono::{DateTime, FixedOffset, Local};
    use pinyin_ime::runtime_log::{self, RuntimeLogLevel};
    use pinyin_ime::{
        app_paths, clipboard_store, dxgi_capture, external_translation, screenshot_region_selector,
        screenshot_store, win_paste, windows_graphics_capture,
    };
    use std::ffi::c_void;
    use std::fs;
    use std::io::Cursor;
    use std::mem::{size_of, zeroed};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT,
        POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
        DeleteDC, DeleteObject, DrawTextW, FillRect, GetDC, ReleaseDC, SelectObject, SetBkMode,
        SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
        DT_CENTER, DT_SINGLELINE, DT_VCENTER, FF_SWISS, HBITMAP, OUT_DEFAULT_PRECIS, TRANSPARENT,
    };
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
        ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD,
        NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow,
        DispatchMessageW, GetAncestor, GetClassNameW, GetCursorPos, GetForegroundWindow,
        GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        IsIconic, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, LoadIconW, MessageBoxW,
        PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SetForegroundWindow,
        SetTimer, SetWindowLongPtrW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, GA_ROOT,
        GWLP_USERDATA, ICONINFO, IDC_ARROW, IDI_APPLICATION, IDYES, MB_ICONERROR, MB_ICONWARNING,
        MB_OK, MB_YESNO, MSG, SW_SHOWNORMAL, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_CREATE,
        WM_DESTROY, WM_DRAWITEM, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_MEASUREITEM,
        WM_NCCREATE, WM_NULL, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSW, WS_OVERLAPPED,
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

    unsafe fn foreground_suppresses_tool_hotkeys() -> bool {
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
        let content = fs::read_to_string(config_path()).unwrap_or_default();
        pinyin_ime::game_keyboard_policy::suppress_tool_hotkeys(&content, &process, &class)
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
    const ICON_SIZE: i32 = 32;
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
    static SCREENSHOT_FLOW_ACTIVE: AtomicBool = AtomicBool::new(false);
    static SCREENSHOT_CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);
    static SCREENSHOT_HANDOFF_COUNTER: AtomicU64 = AtomicU64::new(0);
    const SETTINGS_OPEN_DEBOUNCE: Duration = Duration::from_millis(1200);

    pub fn run() {
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
            let items = build_tray_menu_items(&state, pure_game, true, true);
            if unsafe { tray_menu_style::preview(Path::new(path), items, dpi, theme, selected) }
                .is_err()
            {
                std::process::exit(1);
            }
            return;
        }
        if std::env::args().any(|arg| arg == "--screenshot-capture") {
            run_screenshot_capture();
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

            let _ = Box::into_raw(app);

            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    struct TrayApp {
        mutex: HANDLE,
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
        hotkey_config_mtime: Option<SystemTime>,
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
    struct TrayIconKey {
        input_mode: TrayInputMode,
        engine_state: TrayEngineState,
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

        fn accent_color(self) -> COLORREF {
            match self {
                Self::Ready => rgb(31, 154, 96),
                Self::Loading => rgb(245, 158, 11),
                Self::Failed => rgb(220, 38, 38),
                Self::Idle | Self::Unknown => rgb(107, 114, 128),
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

        fn icon_key(&self) -> TrayIconKey {
            TrayIconKey {
                input_mode: self.input_mode(),
                engine_state: self.engine_state,
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
                hotkey_config_mtime: None,
                tool_hotkeys_suppressed: false,
                foreground_hook: 0,
                clipboard_config_mtime: None,
                clipboard_background_enabled: None,
                last_capture_target: 0,
            }
        }

        unsafe fn add_tray_icon(&mut self, hwnd: HWND) -> bool {
            let state = read_tray_visual_state();
            let icon_key = state.icon_key();
            let icon = create_status_icon(icon_key);
            self.icon_owned = icon.is_some();
            self.icon = icon.unwrap_or_else(|| LoadIconW(0, IDI_APPLICATION));
            if self.icon == 0 {
                return false;
            }

            self.last_icon_key = Some(icon_key);
            self.last_visual_state = Some(state.clone());

            let mut nid = notify_icon_data(hwnd, self.icon, &state);
            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                return false;
            }
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
            true
        }

        unsafe fn restore_tray_icon_after_taskbar_created(&mut self, hwnd: HWND) {
            let state = read_tray_visual_state();
            let icon_key = state.icon_key();
            if self.icon == 0 || self.last_icon_key != Some(icon_key) {
                let old_icon = self.icon;
                let old_icon_owned = self.icon_owned;
                let icon = create_status_icon(icon_key);
                self.icon_owned = icon.is_some();
                self.icon = icon.unwrap_or_else(|| LoadIconW(0, IDI_APPLICATION));
                if old_icon_owned && old_icon != 0 && old_icon != self.icon {
                    let _ = DestroyIcon(old_icon);
                }
            }
            if self.icon == 0 {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "tray_icon_restore",
                    "reason=taskbar_created result=0 error=no_icon",
                );
                return;
            }

            self.last_icon_key = Some(icon_key);
            self.last_visual_state = Some(state.clone());

            let mut nid = notify_icon_data(hwnd, self.icon, &state);
            if Shell_NotifyIconW(NIM_ADD, &nid) != 0 {
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "tray_icon_restore",
                    "reason=taskbar_created result=1 action=add",
                );
                return;
            }

            if Shell_NotifyIconW(NIM_MODIFY, &nid) != 0 {
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "tray_icon_restore",
                    "reason=taskbar_created result=1 action=modify",
                );
                return;
            }

            runtime_log::log_tray(
                RuntimeLogLevel::Error,
                "tray_icon_restore",
                format!("reason=taskbar_created result=0 error={}", GetLastError()),
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
            let state = self
                .last_visual_state
                .clone()
                .unwrap_or_else(read_tray_visual_state);
            let nid = notify_icon_data(hwnd, self.icon, &state);
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

        unsafe fn set_visual_state(&mut self, hwnd: HWND, state: TrayVisualState, force: bool) {
            if !force && self.last_visual_state.as_ref() == Some(&state) {
                return;
            }

            let next_key = state.icon_key();
            let icon_changed = self.last_icon_key != Some(next_key) || self.icon == 0;
            let old_icon = self.icon;
            let old_icon_owned = self.icon_owned;
            let (next_icon, next_icon_owned) = if icon_changed {
                let icon = create_status_icon(next_key);
                (
                    icon.unwrap_or_else(|| LoadIconW(0, IDI_APPLICATION)),
                    icon.is_some(),
                )
            } else {
                (self.icon, self.icon_owned)
            };
            if next_icon == 0 {
                return;
            }

            let nid = notify_icon_data(hwnd, next_icon, &state);
            if Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                if icon_changed && next_icon_owned && next_icon != old_icon {
                    let _ = DestroyIcon(next_icon);
                }
                return;
            }

            self.icon = next_icon;
            self.icon_owned = next_icon_owned;
            self.last_icon_key = Some(next_key);
            self.last_visual_state = Some(state);

            if icon_changed && old_icon_owned && old_icon != 0 && old_icon != next_icon {
                let _ = DestroyIcon(old_icon);
            }
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
            let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
            let suppressed = foreground_suppresses_tool_hotkeys();
            if !force
                && mtime == self.hotkey_config_mtime
                && suppressed == self.tool_hotkeys_suppressed
            {
                return;
            }
            self.hotkey_config_mtime = mtime;
            self.tool_hotkeys_suppressed = suppressed;
            let next_screenshot = if suppressed {
                None
            } else {
                read_hotkey_config("screenshot", "hotkey", "off")
            };
            let next_clipboard = if suppressed {
                None
            } else {
                read_hotkey_config("clipboard", "hotkey", "off")
            };
            let next_settings = if suppressed {
                None
            } else {
                read_hotkey_config("tools", "settings_hotkey", "off")
            };
            let next_handwrite = if suppressed {
                None
            } else {
                read_hotkey_config("tools", "handwrite_hotkey", "off")
            };
            let next_ocr = if suppressed {
                None
            } else {
                read_hotkey_config("tools", "ocr_hotkey", "off")
            };
            let next_ocr_translate = if suppressed {
                None
            } else {
                read_hotkey_config("tools", "ocr_translate_hotkey", "off")
            };
            let translate_available = is_translate_available();
            let next_translate = if translate_available && !suppressed {
                read_hotkey_config("tools", "translate_hotkey", "off")
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
                if app.is_null() || !(*app).add_tray_icon(hwnd) {
                    DestroyWindow(hwnd);
                }
                if !app.is_null() {
                    (*app).refresh_screenshot_hotkey_if_needed(hwnd, true);
                    (*app).refresh_clipboard_background_if_needed(true);
                }
                // 轮询注册表中的 AsciiMode，更新托盘图标为「中 / 英」。
                HOTKEY_TRAY_HWND.store(hwnd, Ordering::Relaxed);
                if !app.is_null() {
                    (*app).foreground_hook =
                        SetWinEventHook(3, 3, 0, Some(on_foreground_changed), 0, 0, 0);
                }
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
                let event = tray_event_code(lparam);
                let icon_id = tray_icon_id(lparam);
                if icon_id != 0 && icon_id != TRAY_ICON_ID as u16 {
                    return 0;
                }
                match event {
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        show_menu(hwnd, Some(tray_anchor_point(wparam)))
                    }
                    // 单击托盘图标切换中/英；设置从右键菜单进入。
                    WM_LBUTTONUP if !app.is_null() => {
                        (*app).toggle_ascii_mode(hwnd);
                    }
                    WM_LBUTTONDBLCLK => open_settings(),
                    // NOTIFYICON v4：键盘激活、部分 Win10/11 壳层用 NIN_SELECT 代替鼠标 up。
                    NIN_SELECT | NIN_KEYSELECT if !app.is_null() => {
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
                    (*app).remove_tray_icon(hwnd);
                    let _ = Box::from_raw(app);
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
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
                if foreground_suppresses_tool_hotkeys() {
                    if !app.is_null() {
                        (*app).refresh_screenshot_hotkey_if_needed(hwnd, false);
                    }
                    return 0;
                }
                let target_hwnd = if !app.is_null() {
                    (*app).capture_target_or_foreground(hwnd)
                } else {
                    capture_target_from_foreground(hwnd)
                };
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

    unsafe fn notify_icon_data(
        hwnd: HWND,
        icon: isize,
        state: &TrayVisualState,
    ) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = TRAY_ICON_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon;
        let tip = tray_tooltip_text(state);
        write_wide_buffer(&mut nid.szTip, &tip);
        nid
    }

    fn build_tray_menu_items(
        state: &TrayVisualState,
        pure_game: bool,
        translate_available: bool,
        recent_screenshot: bool,
    ) -> Vec<TrayMenuItem> {
        let mode = match state.input_mode() {
            TrayInputMode::Chinese => "中文模式",
            TrayInputMode::English => "英文模式",
            TrayInputMode::Unknown => "状态未知",
        };
        let mut items = vec![TrayMenuItem::header(
            mode,
            &format!("引擎{}", state.engine_state.label()),
            matches!(state.engine_state, TrayEngineState::Ready),
        )];
        let mut retry = TrayMenuItem::action(
            ID_ENGINE_RESTART,
            if matches!(state.engine_state, TrayEngineState::Ready) {
                "重启输入引擎"
            } else {
                "重试输入引擎"
            },
            TrayMenuIcon::Status,
        );
        retry.attention = !matches!(state.engine_state, TrayEngineState::Ready);
        items.push(retry);
        items.push(TrayMenuItem::section("输入与设置"));
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
        items.push(TrayMenuItem::action(
            ID_SETTINGS,
            "输入法设置…",
            TrayMenuIcon::Settings,
        ));
        items.push(TrayMenuItem::section("常用工具"));
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
        items.push(TrayMenuItem::section("截图与识别"));
        items.push(TrayMenuItem::action(
            ID_SCREENSHOT,
            "截图",
            TrayMenuIcon::Screenshot,
        ));
        if recent_screenshot {
            items.push(TrayMenuItem::action(
                ID_LAST_SCREENSHOT_OCR,
                "识别最近截图",
                TrayMenuIcon::Ocr,
            ));
            items.push(TrayMenuItem::action(
                ID_LAST_SCREENSHOT_TRANSLATE,
                "翻译最近截图",
                TrayMenuIcon::Translate,
            ));
        }
        items.push(TrayMenuItem::action(ID_OCR, "截图 OCR", TrayMenuIcon::Ocr));
        items.push(TrayMenuItem::action(
            ID_OCR_TRANSLATE,
            "截图翻译",
            TrayMenuIcon::Translate,
        ));
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
            let items = build_tray_menu_items(&state(), false, true, true);
            let mut commands: Vec<_> = items
                .iter()
                .filter_map(|item| (item.command != 0).then_some(item.command))
                .collect();
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
            let items = build_tray_menu_items(&state(), false, false, false);
            for command in [
                ID_TRANSLATE,
                ID_LAST_SCREENSHOT_OCR,
                ID_LAST_SCREENSHOT_TRANSLATE,
            ] {
                assert!(!items.iter().any(|item| item.command == command));
            }
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
            let items = build_tray_menu_items(&state, true, false, false);
            assert!(
                items
                    .iter()
                    .find(|i| i.command == ID_PURE_GAME_MODE)
                    .unwrap()
                    .checked
            );
            assert!(
                items
                    .iter()
                    .find(|i| i.command == ID_GAME_COMPAT_MODE)
                    .unwrap()
                    .checked
            );
            assert!(
                items
                    .iter()
                    .find(|i| i.command == ID_ENGINE_RESTART)
                    .unwrap()
                    .attention
            );
            state.engine_state = TrayEngineState::Ready;
            state.mode_source = Some("chinese".into());
            let items = build_tray_menu_items(&state, false, false, false);
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
            pure_game_mode_enabled(),
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
        thread::spawn(move || {
            let text = capture_selected_text(target_hwnd)
                .or_else(|_| {
                    clipboard_win::get_clipboard::<String, _>(clipboard_win::formats::Unicode)
                        .map_err(|error| error.to_string())
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
            request.replace_selection = true;
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
        image: image::RgbaImage,
        source: SavedScreenshotSource,
        action: screenshot_region_selector::RegionAction,
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
                arg_hwnd.unwrap_or(0) as isize,
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
        run_screenshot_flow(
            capture_target,
            action_target,
            ScreenshotFlowAction::Configured,
        );
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
        if let Some(parent) = output_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|err| format!("创建 OCR 截图交接目录失败：{err}"))?;
        }
        let captured = capture_with_native_fallback(target_hwnd, mode, prefs.selector_options());
        match captured? {
            ScreenshotCaptureResult::Captured(captured) => {
                if !valid_saved_screenshot(output_path) {
                    save_screenshot_image(&captured.image, output_path, false, prefs)?;
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
        if SCREENSHOT_FLOW_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            SCREENSHOT_CANCEL_REQUESTED.store(true, Ordering::Release);
            runtime_log::log_tray(
                RuntimeLogLevel::Basic,
                "screenshot_cancel_request",
                "status=requested reason=second_invocation",
            );
            return;
        }
        let prefs = screenshot_prefs();
        let flow_target = if prefs.mode == ScreenshotModePref::CurrentWindow {
            target_hwnd
        } else {
            0
        };
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenshot_request",
            format!(
                "entry=tray target_hwnd={} flow_target_hwnd={} backend=native mode={} action={action:?}",
                target_hwnd as isize,
                flow_target as isize,
                prefs.mode.as_config()
            ),
        );
        thread::spawn(move || run_screenshot_flow(flow_target, target_hwnd, action));
    }

    fn run_screenshot_flow(
        target_hwnd: HWND,
        action_target: HWND,
        requested_action: ScreenshotFlowAction,
    ) {
        struct ActiveGuard;
        impl Drop for ActiveGuard {
            fn drop(&mut self) {
                SCREENSHOT_FLOW_ACTIVE.store(false, Ordering::Release);
            }
        }
        let _active_guard = ActiveGuard;
        SCREENSHOT_CANCEL_REQUESTED.store(false, Ordering::Release);

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
            }
        }
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenshot_flow_start",
            format!(
                "target_hwnd={} action_target_hwnd={} requested_action={requested_action:?} auto_save={} copy={} ocr={} translate={} format={} silent_copy={} backend=native mode={}",
                target_hwnd as isize,
                action_target as isize,
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
            capture_with_native_fallback(target_hwnd, prefs.mode, prefs.selector_options());

        match captured {
            Ok(ScreenshotCaptureResult::Captured(captured)) => {
                let retained = captured.clone();
                let result = finish_captured_screenshot(captured, action_target, &prefs);
                let status = match &result {
                    Ok(false) => "已取消另存，图片仍保留",
                    Err(_) => "操作失败，图片仍保留",
                    Ok(true) => match retained.action {
                        screenshot_region_selector::RegionAction::Copy => "图片已复制",
                        screenshot_region_selector::RegionAction::SaveAs => "图片已保存",
                        screenshot_region_selector::RegionAction::Ocr => "OCR 已启动",
                        screenshot_region_selector::RegionAction::Annotate => "已在画图中打开",
                        screenshot_region_selector::RegionAction::Pin => "图片已固定",
                        screenshot_region_selector::RegionAction::Configured => "截图处理完成",
                    },
                };
                if let Err(err) = &result {
                    runtime_log::log_tray(
                        RuntimeLogLevel::Error,
                        "screenshot_postprocess",
                        format!("status=failed reason={err}"),
                    );
                    show_error_message(&format!("截图完成，但后续处理失败：{err}"));
                }
                if prefs.show_result_window || !matches!(result, Ok(true)) {
                    pinned_screenshot::spawn_result(retained, action_target, status);
                }
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
    ) -> Result<ScreenshotCaptureResult, String> {
        if mode == ScreenshotModePref::CurrentWindow && target_hwnd == 0 {
            return Err("当前窗口截图缺少有效的目标窗口，未改为区域截图。".to_string());
        }
        let effective_mode = mode;

        match capture_with_wgc(target_hwnd, effective_mode, selector_options) {
            Ok(result) => return Ok(result),
            Err(wgc_err) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_capture_fallback",
                    format!("from=wgc to=dxgi reason={wgc_err}"),
                );
                match capture_with_dxgi(target_hwnd, effective_mode, selector_options) {
                    Ok(result) => return Ok(result),
                    Err(dxgi_err) => {
                        runtime_log::log_tray(
                            RuntimeLogLevel::Error,
                            "screenshot_capture_fallback",
                            format!(
                                "from=dxgi to={} reason={dxgi_err}",
                                if mode == ScreenshotModePref::CurrentWindow {
                                    "error"
                                } else {
                                    "screenclip"
                                }
                            ),
                        );
                        if mode == ScreenshotModePref::CurrentWindow {
                            if confirm_region_fallback(&wgc_err, &dxgi_err) {
                                return capture_with_native_fallback(
                                    0,
                                    ScreenshotModePref::ManualRegion,
                                    selector_options,
                                );
                            }
                            return Ok(ScreenshotCaptureResult::Cancelled);
                        }
                        return capture_with_system_screenclip().map_err(|screenclip_err| {
                            format!("WGC：{wgc_err}；DXGI：{dxgi_err}；系统截图：{screenclip_err}")
                        });
                    }
                }
            }
        }
    }

    fn capture_with_wgc(
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        selector_options: screenshot_region_selector::RegionSelectorOptions,
    ) -> Result<ScreenshotCaptureResult, String> {
        let frame = match mode {
            ScreenshotModePref::CurrentWindow => {
                windows_graphics_capture::capture_window(target_hwnd as isize)
            }
            ScreenshotModePref::ManualRegion => windows_graphics_capture::capture_virtual_desktop(),
        }
        .map_err(|err| err.to_string())?;
        captured_frame_result(
            frame,
            target_hwnd,
            mode,
            CaptureBackendUsed::Wgc,
            selector_options,
        )
    }

    fn capture_with_dxgi(
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        selector_options: screenshot_region_selector::RegionSelectorOptions,
    ) -> Result<ScreenshotCaptureResult, String> {
        let frame = match mode {
            ScreenshotModePref::CurrentWindow => dxgi_capture::capture_window(target_hwnd as isize),
            ScreenshotModePref::ManualRegion => dxgi_capture::capture_virtual_desktop(),
        }
        .map_err(|err| err.to_string())?;
        captured_frame_result(
            frame,
            target_hwnd,
            mode,
            CaptureBackendUsed::Dxgi,
            selector_options,
        )
    }

    fn captured_frame_result(
        frame: windows_graphics_capture::CapturedFrame,
        target_hwnd: HWND,
        mode: ScreenshotModePref,
        backend: CaptureBackendUsed,
        selector_options: screenshot_region_selector::RegionSelectorOptions,
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
                image: frame.image,
                source: SavedScreenshotSource::window(backend, target_hwnd),
                action: screenshot_region_selector::RegionAction::Configured,
            }));
        }

        let selected =
            screenshot_region_selector::select_region_action_with_options(&frame, selector_options)
                .map_err(|err| format!("自定义框选界面失败：{err}"))?;
        let Some((selected, action)) = selected else {
            return Ok(ScreenshotCaptureResult::Cancelled);
        };
        let local_x = i64::from(selected.x) - i64::from(frame.origin_x);
        let local_y = i64::from(selected.y) - i64::from(frame.origin_y);
        if local_x < 0
            || local_y < 0
            || local_x + i64::from(selected.width) > i64::from(frame.width())
            || local_y + i64::from(selected.height) > i64::from(frame.height())
        {
            return Err("框选区域超出已捕获的虚拟桌面范围。".to_string());
        }
        let image = image::imageops::crop_imm(
            &frame.image,
            local_x as u32,
            local_y as u32,
            selected.width,
            selected.height,
        )
        .to_image();
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
            image,
            source: SavedScreenshotSource::region(backend),
            action,
        }))
    }

    fn capture_with_system_screenclip() -> Result<ScreenshotCaptureResult, String> {
        let initial_seq = clipboard_win::seq_num().map(|n| n.get());
        Command::new("explorer.exe")
            .arg("ms-screenclip:")
            .spawn()
            .map_err(|err| format!("无法启动 Windows 截图工具：{err}"))?;
        runtime_log::log_tray(
            RuntimeLogLevel::Basic,
            "screenclip_capture",
            "status=launched",
        );
        match wait_for_clipboard_screenshot(initial_seq, Duration::from_secs(25))? {
            Some(image) => Ok(ScreenshotCaptureResult::Captured(CapturedScreenshot {
                image,
                source: SavedScreenshotSource::region(CaptureBackendUsed::ScreenClip),
                action: screenshot_region_selector::RegionAction::Configured,
            })),
            None => Ok(ScreenshotCaptureResult::Cancelled),
        }
    }

    fn wait_for_clipboard_screenshot(
        initial_seq: Option<u32>,
        timeout: Duration,
    ) -> Result<Option<image::RgbaImage>, String> {
        let deadline = Instant::now() + timeout;
        let png_format = clipboard_win::register_format("PNG").map(|value| value.get());
        let mut last_err = None;
        let mut saw_image_format = false;
        while Instant::now() < deadline {
            if SCREENSHOT_CANCEL_REQUESTED.load(Ordering::Acquire) {
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

    fn finish_captured_screenshot(
        captured: CapturedScreenshot,
        action_target: HWND,
        prefs: &ScreenshotPrefs,
    ) -> Result<bool, String> {
        if captured.action == screenshot_region_selector::RegionAction::SaveAs {
            let Some(destination) = rfd::FileDialog::new()
                .set_title("另存截图")
                .add_filter("PNG 图片", &["png"])
                .add_filter("JPG 图片", &["jpg", "jpeg"])
                .set_file_name("截图.png")
                .save_file()
            else {
                return Ok(false);
            };
            let destination = if destination.extension().is_none() {
                destination.with_extension("png")
            } else {
                destination
            };
            let mut save_prefs = screenshot_prefs();
            save_prefs.format = match destination.extension().and_then(|value| value.to_str()) {
                Some(value)
                    if value.eq_ignore_ascii_case("jpg") || value.eq_ignore_ascii_case("jpeg") =>
                {
                    "jpg"
                }
                Some(value) if value.eq_ignore_ascii_case("png") => "png",
                _ => return Err("另存截图仅支持 PNG 或 JPG 文件名。".to_string()),
            }
            .to_string();
            return save_screenshot_image(&captured.image, &destination, true, &save_prefs).map(|()| true);
        }
        if captured.action == screenshot_region_selector::RegionAction::Annotate {
            let destination = next_temporary_screenshot_path()?;
            let mut save_prefs = screenshot_prefs();
            save_prefs.format = "png".to_string();
            save_screenshot_image(&captured.image, &destination, false, &save_prefs)?;
            Command::new("mspaint.exe")
                .arg(&destination)
                .spawn()
                .map_err(|err| format!("打开画图标注失败：{err}"))?;
            return Ok(true);
        }
        if captured.action == screenshot_region_selector::RegionAction::Pin {
            pinned_screenshot::spawn(captured.image);
            return Ok(true);
        }
        let mut action_prefs = screenshot_prefs();
        match captured.action {
            screenshot_region_selector::RegionAction::Copy => {
                action_prefs.auto_save = false;
                action_prefs.copy_after_capture = true;
                action_prefs.ocr_after_capture = false;
                action_prefs.translate_after_capture = false;
            }
            screenshot_region_selector::RegionAction::Ocr => {
                action_prefs.auto_save = false;
                action_prefs.copy_after_capture = false;
                action_prefs.ocr_after_capture = true;
                action_prefs.translate_after_capture = false;
            }
            _ => {}
        }
        let prefs = if captured.action == screenshot_region_selector::RegionAction::Configured {
            prefs
        } else {
            &action_prefs
        };
        let needs_handoff_path =
            prefs.auto_save || prefs.ocr_after_capture || prefs.translate_after_capture;
        let mut path = None;
        let mut persistent = false;
        let mut errors = Vec::new();

        if needs_handoff_path {
            if prefs.auto_save {
                let destination = next_screenshot_path(prefs, action_target);
                match destination.and_then(|destination| {
                    let destination = if prefs.name_pattern.contains("{width}")
                        || prefs.name_pattern.contains("{height}")
                    {
                        screenshot_path_with_dimensions(
                            &destination,
                            prefs,
                            action_target,
                            captured.image.width(),
                            captured.image.height(),
                        )?
                    } else {
                        destination
                    };
                    save_screenshot_image(&captured.image, &destination, true, prefs)
                        .map(|()| destination)
                }) {
                    Ok(destination) => {
                        persistent = true;
                        let _ = save_silent_screenshot_copy(&destination, prefs);
                        path = Some(destination);
                    }
                    Err(err) => errors.push(format!("保存截图失败：{err}")),
                }
            }
            if path.is_none() && (prefs.ocr_after_capture || prefs.translate_after_capture) {
                match next_temporary_screenshot_path().and_then(|destination| {
                    save_screenshot_image(&captured.image, &destination, false, prefs)
                        .map(|()| destination)
                }) {
                    Ok(destination) => path = Some(destination),
                    Err(err) => errors.push(format!("创建 OCR 交接文件失败：{err}")),
                }
            }
        }

        if persistent {
            if let Some(saved_path) = path.as_deref() {
                record_last_screenshot(saved_path);
                record_screenshot_metadata_async(saved_path.to_path_buf(), captured.source);
            }
        }

        let copy_status = if !prefs.copy_after_capture {
            "disabled"
        } else if let Err(err) = copy_image_to_clipboard(&captured.image) {
            errors.push(err);
            "failed"
        } else {
            "ok"
        };

        if prefs.ocr_after_capture || prefs.translate_after_capture {
            if let Some(image_path) = path.as_deref() {
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_ocr_handoff",
                    format!(
                        "status=launch path={} target_hwnd={} translate={}",
                        runtime_log::path_for_log(image_path),
                        action_target as isize,
                        if prefs.translate_after_capture { 1 } else { 0 }
                    ),
                );
                if let Err(err) = open_ocr_with_image(
                    action_target,
                    image_path,
                    prefs.translate_after_capture,
                    !persistent,
                ) {
                    runtime_log::log_tray(
                        RuntimeLogLevel::Error,
                        "screenshot_ocr_handoff",
                        format!("status=failed reason={err}"),
                    );
                    if !persistent {
                        let _ = fs::remove_file(image_path);
                    }
                    errors.push(format!("OCR 启动失败：{err}"));
                }
            } else {
                errors.push("OCR 交接缺少可读取的截图文件。".to_string());
            }
        }

        let final_status = if errors.is_empty() { "ok" } else { "partial" };
        runtime_log::log_tray(
            if errors.is_empty() {
                RuntimeLogLevel::Basic
            } else {
                RuntimeLogLevel::Error
            },
            "screenshot_postprocess",
            format!(
                "status={} backend={} source={} path={} persistent={} copy={} ocr={} translate={} errors={}",
                final_status,
                captured.source.backend.as_config(),
                captured.source.source_key(),
                path.as_deref()
                    .map(runtime_log::path_for_log)
                    .unwrap_or_else(|| "none".to_string()),
                if persistent { 1 } else { 0 },
                copy_status,
                if prefs.ocr_after_capture { 1 } else { 0 },
                if prefs.translate_after_capture { 1 } else { 0 },
                errors.join(" | ")
            ),
        );
        if errors.is_empty() {
            Ok(true)
        } else {
            Err(errors.join("\n"))
        }
    }

    fn valid_saved_screenshot(path: &Path) -> bool {
        path.is_file()
            && path
                .metadata()
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
    }

    fn save_screenshot_image(
        image: &image::RgbaImage,
        path: &Path,
        persistent: bool,
        prefs: &ScreenshotPrefs,
    ) -> Result<(), String> {
        let format = if persistent && prefs.format == "jpg" {
            image::ImageFormat::Jpeg
        } else {
            image::ImageFormat::Png
        };
        // Desktop capture surfaces are BGRA and some drivers leave their alpha
        // channel undefined.  PNG viewers then composite an otherwise correct
        // frame against their accent colour (often purple).  A screenshot is
        // an opaque desktop image, so flatten alpha before both file encoding
        // and clipboard publication.
        image::DynamicImage::ImageRgba8(opaque_screenshot_image(image))
            .save_with_format(path, format)
            .map_err(|err| format!("写入截图 {} 失败：{err}", path.display()))
    }

    fn copy_image_to_clipboard(image: &image::RgbaImage) -> Result<(), String> {
        let image = opaque_screenshot_image(image);
        let png = encode_clipboard_png(&image)?;
        let dibv5 = encode_clipboard_dibv5(&image)?;
        write_screenshot_formats_to_clipboard(&png, &dibv5)
    }

    fn encode_clipboard_png(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|err| format!("编码剪贴板 PNG 失败：{err}"))?;
        Ok(bytes.into_inner())
    }

    /// Builds the exact CF_DIBV5 payload expected by the Windows clipboard:
    /// a 124-byte BITMAPV5HEADER followed by bottom-up BGRA pixels. Publishing
    /// this directly avoids `clipboard-win::set_bitmap_with`, which only keeps
    /// a 40-byte BITMAPINFOHEADER and corrupts the masks of RGBA V4 BMP files.
    fn encode_clipboard_dibv5(image: &image::RgbaImage) -> Result<Vec<u8>, String> {
        const HEADER_SIZE: usize = 124;
        const BI_BITFIELDS: u32 = 3;
        const LCS_SRGB: u32 = 0x7352_4742;
        const LCS_GM_IMAGES: u32 = 4;

        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 {
            return Err("剪贴板图片尺寸不能为空。".to_string());
        }
        let width_i32 = i32::try_from(width).map_err(|_| "剪贴板图片宽度过大。")?;
        let height_i32 = i32::try_from(height).map_err(|_| "剪贴板图片高度过大。")?;
        let pixel_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "剪贴板图片缓冲区尺寸溢出。".to_string())?;
        let pixel_bytes_u32 = u32::try_from(pixel_bytes)
            .map_err(|_| "剪贴板图片缓冲区超过 DIBV5 限制。".to_string())?;
        let capacity = HEADER_SIZE
            .checked_add(pixel_bytes as usize)
            .ok_or_else(|| "剪贴板 DIBV5 缓冲区尺寸溢出。".to_string())?;
        let mut dib = Vec::with_capacity(capacity);

        dib.extend_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        dib.extend_from_slice(&width_i32.to_le_bytes());
        dib.extend_from_slice(&height_i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes()); // planes
        dib.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        dib.extend_from_slice(&BI_BITFIELDS.to_le_bytes());
        dib.extend_from_slice(&pixel_bytes_u32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes()); // x pixels per metre
        dib.extend_from_slice(&0i32.to_le_bytes()); // y pixels per metre
        dib.extend_from_slice(&0u32.to_le_bytes()); // colours used
        dib.extend_from_slice(&0u32.to_le_bytes()); // important colours
        dib.extend_from_slice(&0x00ff_0000u32.to_le_bytes()); // red mask
        dib.extend_from_slice(&0x0000_ff00u32.to_le_bytes()); // green mask
        dib.extend_from_slice(&0x0000_00ffu32.to_le_bytes()); // blue mask
        dib.extend_from_slice(&0xff00_0000u32.to_le_bytes()); // alpha mask
        dib.extend_from_slice(&LCS_SRGB.to_le_bytes());
        dib.extend_from_slice(&[0u8; 36]); // CIEXYZTRIPLE endpoints
        dib.extend_from_slice(&0u32.to_le_bytes()); // gamma red
        dib.extend_from_slice(&0u32.to_le_bytes()); // gamma green
        dib.extend_from_slice(&0u32.to_le_bytes()); // gamma blue
        dib.extend_from_slice(&LCS_GM_IMAGES.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes()); // profile data offset
        dib.extend_from_slice(&0u32.to_le_bytes()); // profile size
        dib.extend_from_slice(&0u32.to_le_bytes()); // reserved
        debug_assert_eq!(dib.len(), HEADER_SIZE);

        let row_bytes = width as usize * 4;
        for row in image.as_raw().chunks_exact(row_bytes).rev() {
            for pixel in row.chunks_exact(4) {
                dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
        }
        debug_assert_eq!(dib.len(), capacity);
        Ok(dib)
    }

    fn write_screenshot_formats_to_clipboard(png: &[u8], dibv5: &[u8]) -> Result<(), String> {
        const MAX_ATTEMPTS: u32 = 12;
        let png_format = clipboard_win::register_format("PNG")
            .map(|format| format.get())
            .ok_or_else(|| "注册 Windows PNG 剪贴板格式失败。".to_string())?;
        let mut last_error = None;
        for attempt in 0..MAX_ATTEMPTS {
            let result = (|| {
                let _clipboard = clipboard_win::Clipboard::new_attempts(4)
                    .map_err(|err| format!("打开剪贴板失败：{err}"))?;
                clipboard_win::empty().map_err(|err| format!("清空剪贴板失败：{err}"))?;
                clipboard_win::raw::set_without_clear(png_format, png)
                    .map_err(|err| format!("发布剪贴板 PNG 失败：{err}"))?;
                clipboard_win::raw::set_without_clear(clipboard_win::formats::CF_DIBV5, dibv5)
                    .map_err(|err| format!("发布剪贴板 DIBV5 失败：{err}"))?;
                Ok::<(), String>(())
            })();
            match result {
                Ok(()) => return Ok(()),
                Err(err) => last_error = Some(err),
            }
            thread::sleep(Duration::from_millis(10 + u64::from(attempt) * 8));
        }
        Err(last_error.unwrap_or_else(|| "写入剪贴板失败：未知错误".to_string()))
    }

    fn opaque_screenshot_image(image: &image::RgbaImage) -> image::RgbaImage {
        let mut flattened = image.clone();
        for pixel in flattened.pixels_mut() {
            pixel[3] = 255;
        }
        flattened
    }

    /// The `clipboard-win` bitmap convenience setter deliberately preserves the
    /// rest of the clipboard. That fails with ERROR_ALREADY_EXISTS (183) when
    /// CF_BITMAP was already present. A screenshot is a complete image copy, so
    /// replace the clipboard atomically after the DIB has been created instead.
    fn next_temporary_screenshot_path() -> Result<PathBuf, String> {
        let dir = app_paths::local_data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("screenshot-handoff");
        fs::create_dir_all(&dir)
            .map_err(|err| format!("创建截图临时目录 {} 失败：{err}", dir.display()))?;
        cleanup_old_temporary_screenshots(&dir);
        let stamp = Local::now().format("%Y%m%d_%H%M%S_%3f");
        let sequence = SCREENSHOT_HANDOFF_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(dir.join(format!(
            "capture_{}_{}_{}.png",
            std::process::id(),
            stamp,
            sequence
        )))
    }

    fn cleanup_old_temporary_screenshots(dir: &Path) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(std::time::UNIX_EPOCH);
        for entry in entries.flatten() {
            let path = entry.path();
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .map(|modified| modified < cutoff)
                .unwrap_or(false);
            if stale && path.is_file() {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn save_silent_screenshot_copy(source: &Path, prefs: &ScreenshotPrefs) -> Option<PathBuf> {
        if !prefs.silent_copy_enabled {
            return None;
        }
        let Some(dir) = prefs.silent_copy_dir.as_ref() else {
            return None;
        };
        match copy_screenshot_file_to_dir(source, dir) {
            Ok(path) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_silent_copy",
                    format!(
                        "status=saved source={} path={}",
                        runtime_log::path_for_log(source),
                        runtime_log::path_for_log(&path)
                    ),
                );
                Some(path)
            }
            Err(err) => {
                runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_silent_copy",
                    format!(
                        "status=failed source={} dir={} reason={}",
                        runtime_log::path_for_log(source),
                        runtime_log::path_for_log(dir),
                        err
                    ),
                );
                None
            }
        }
    }

    fn copy_screenshot_file_to_dir(source: &Path, dir: &Path) -> Result<PathBuf, String> {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let dest = unique_screenshot_copy_path(source, dir)?;
        fs::copy(source, &dest)
            .map(|_| dest.clone())
            .map_err(|e| format!("copy {} to {}: {e}", source.display(), dest.display()))
    }

    fn unique_screenshot_copy_path(source: &Path, dir: &Path) -> Result<PathBuf, String> {
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("source path has no file name: {}", source.display()))?;
        let mut path = dir.join(file_name);
        if !path.exists() {
            return Ok(path);
        }

        let stem = source
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("screenshot");
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty());
        for idx in 2..1000 {
            let file_name = if let Some(extension) = extension {
                format!("{stem}_{idx:03}.{extension}")
            } else {
                format!("{stem}_{idx:03}")
            };
            path = dir.join(file_name);
            if !path.exists() {
                return Ok(path);
            }
        }
        Ok(path)
    }

    fn last_screenshot_record_path() -> Option<PathBuf> {
        app_paths::local_data_dir().map(|dir| dir.join("last_screenshot.txt"))
    }

    fn record_last_screenshot(path: &Path) {
        let Some(record_path) = last_screenshot_record_path() else {
            return;
        };
        if let Some(dir) = record_path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(record_path, path.display().to_string());
    }

    fn record_screenshot_metadata_async(path: PathBuf, source: SavedScreenshotSource) {
        thread::spawn(move || {
            let record = screenshot_record_for_path(path, source);
            match screenshot_store::record_screenshot(&record) {
                Ok(()) => runtime_log::log_tray(
                    RuntimeLogLevel::Basic,
                    "screenshot_library",
                    format!(
                        "status=recorded source={} path={}",
                        record.source,
                        runtime_log::path_for_log(&record.path)
                    ),
                ),
                Err(err) => runtime_log::log_tray(
                    RuntimeLogLevel::Error,
                    "screenshot_library",
                    format!(
                        "status=failed path={} reason={}",
                        runtime_log::path_for_log(&record.path),
                        err
                    ),
                ),
            }
        });
    }

    fn screenshot_record_for_path(
        path: PathBuf,
        source: SavedScreenshotSource,
    ) -> screenshot_store::ScreenshotRecord {
        screenshot_store::ScreenshotRecord {
            path,
            source: source.source_key(),
            target_hwnd: source.target_hwnd as isize,
            source_window_title: if source.kind == CapturedScreenshotKind::Window
                && source.target_hwnd != 0
            {
                Some(unsafe { window_title(source.target_hwnd) })
            } else {
                None
            },
            ..screenshot_store::ScreenshotRecord::default()
        }
    }

    fn last_screenshot_path() -> Option<PathBuf> {
        let record_path = last_screenshot_record_path()?;
        let text = fs::read_to_string(record_path).ok()?;
        let path = PathBuf::from(text.trim());
        if path.is_file() {
            Some(path)
        } else {
            None
        }
    }

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
                confirm_label: match (self.auto_save, self.copy_after_capture, self.ocr_after_capture, self.translate_after_capture) {
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
        let text = content.as_deref().unwrap_or_default();
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

    fn next_screenshot_path(prefs: &ScreenshotPrefs, target_hwnd: HWND) -> Result<PathBuf, String> {
        let mut dir = prefs
            .save_dir
            .clone()
            .unwrap_or_else(default_screenshot_dir);
        if prefs.date_subdirs {
            let now = Local::now();
            dir = dir
                .join(now.format("%Y").to_string())
                .join(now.format("%m").to_string())
                .join(now.format("%d").to_string());
        }
        fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let ext = prefs.format.as_str();
        let captured_at = Local::now().fixed_offset();
        let context = screenshot_name_context(target_hwnd);
        let base_stem = render_screenshot_name_pattern_at(
            &prefs.name_pattern,
            1,
            "screenshot",
            &captured_at,
            &context,
        );
        let mut path = dir.join(format!("{base_stem}.{ext}"));
        if prefs.conflict_strategy == "overwrite" {
            return Ok(path);
        }
        for idx in 2..1000 {
            if !path.exists() {
                return Ok(path);
            }
            let stem = if prefs.name_pattern.contains("{seq}") {
                render_screenshot_name_pattern_at(
                    &prefs.name_pattern,
                    idx,
                    "screenshot",
                    &captured_at,
                    &context,
                )
            } else {
                format!("{base_stem}_{idx:03}")
            };
            path = dir.join(format!("{stem}.{ext}"));
        }
        Ok(path)
    }

    fn screenshot_path_with_dimensions(
        source: &Path,
        prefs: &ScreenshotPrefs,
        target_hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<PathBuf, String> {
        let dir = source
            .parent()
            .ok_or_else(|| format!("截图路径没有父目录：{}", source.display()))?;
        let captured_at = Local::now().fixed_offset();
        let mut context = screenshot_name_context(target_hwnd);
        context.width = width.to_string();
        context.height = height.to_string();
        let stem = render_screenshot_name_pattern_at(
            &prefs.name_pattern,
            1,
            "screenshot",
            &captured_at,
            &context,
        );
        let ext = prefs.format.as_str();
        let mut destination = dir.join(format!("{stem}.{ext}"));
        if prefs.conflict_strategy == "overwrite" {
            return Ok(destination);
        }
        for idx in 2..1000 {
            if !destination.exists() {
                return Ok(destination);
            }
            let next_stem = if prefs.name_pattern.contains("{seq}") {
                render_screenshot_name_pattern_at(
                    &prefs.name_pattern,
                    idx,
                    "screenshot",
                    &captured_at,
                    &context,
                )
            } else {
                format!("{stem}_{idx:03}")
            };
            destination = dir.join(format!("{next_stem}.{ext}"));
        }
        Ok(destination)
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

    fn read_hotkey_config(section: &str, key: &str, fallback: &str) -> Option<HotkeySpec> {
        let content = fs::read_to_string(config_path()).ok();
        let value = content
            .as_deref()
            .and_then(|text| ini_value(text, section, key))
            .unwrap_or_else(|| fallback.to_string());
        parse_hotkey_with_fallback(&value, fallback)
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

    fn tray_icon_id(lparam: LPARAM) -> u16 {
        ((lparam as u32) >> 16) as u16
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

    unsafe fn create_status_icon(key: TrayIconKey) -> Option<isize> {
        let screen_dc = GetDC(0);
        if screen_dc == 0 {
            return None;
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        let color_bitmap = CreateCompatibleBitmap(screen_dc, ICON_SIZE, ICON_SIZE);
        let mask_bitmap = CreateBitmap(ICON_SIZE, ICON_SIZE, 1, 1, std::ptr::null());
        let _ = ReleaseDC(0, screen_dc);
        if mem_dc == 0 || color_bitmap == 0 || mask_bitmap == 0 {
            if mem_dc != 0 {
                let _ = DeleteDC(mem_dc);
            }
            if color_bitmap != 0 {
                let _ = DeleteObject(color_bitmap as _);
            }
            if mask_bitmap != 0 {
                let _ = DeleteObject(mask_bitmap as _);
            }
            return None;
        }

        let old_bitmap = SelectObject(mem_dc, color_bitmap as _);
        let bg_color = match key.input_mode {
            TrayInputMode::English => rgb(39, 50, 64),
            TrayInputMode::Chinese => rgb(17, 103, 122),
            TrayInputMode::Unknown => rgb(75, 82, 92),
        };
        let bg_brush = CreateSolidBrush(bg_color);
        let accent_brush = CreateSolidBrush(key.engine_state.accent_color());
        let font_name = wide("Segoe UI");
        let font = CreateFontW(
            -22,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_SWISS) as u32,
            font_name.as_ptr(),
        );
        let old_font = if font != 0 {
            SelectObject(mem_dc, font as _)
        } else {
            0
        };

        let background = RECT {
            left: 0,
            top: 0,
            right: ICON_SIZE,
            bottom: ICON_SIZE,
        };
        let accent = RECT {
            left: 0,
            top: ICON_SIZE - 5,
            right: ICON_SIZE,
            bottom: ICON_SIZE,
        };
        let badge = RECT {
            left: ICON_SIZE - 9,
            top: 0,
            right: ICON_SIZE,
            bottom: 9,
        };
        let _ = FillRect(mem_dc, &background, bg_brush);
        let _ = FillRect(mem_dc, &accent, accent_brush);
        let _ = FillRect(mem_dc, &badge, accent_brush);
        let _ = SetBkMode(mem_dc, TRANSPARENT as i32);
        let _ = SetTextColor(mem_dc, rgb(247, 250, 255));
        let text = match key.input_mode {
            TrayInputMode::English => "A",
            TrayInputMode::Chinese => "中",
            TrayInputMode::Unknown => "?",
        };
        let text_w = wide(text);
        let mut text_rect = RECT {
            left: 0,
            top: 1,
            right: ICON_SIZE,
            bottom: ICON_SIZE - 5,
        };
        let _ = DrawTextW(
            mem_dc,
            text_w.as_ptr(),
            (text_w.len().saturating_sub(1)) as i32,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        let icon_info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bitmap as HBITMAP,
            hbmColor: color_bitmap as HBITMAP,
        };
        let icon = CreateIconIndirect(&icon_info);

        if old_font != 0 {
            let _ = SelectObject(mem_dc, old_font);
        }
        if old_bitmap != 0 {
            let _ = SelectObject(mem_dc, old_bitmap);
        }
        if font != 0 {
            let _ = DeleteObject(font as _);
        }
        if bg_brush != 0 {
            let _ = DeleteObject(bg_brush as _);
        }
        if accent_brush != 0 {
            let _ = DeleteObject(accent_brush as _);
        }
        let _ = DeleteObject(color_bitmap as _);
        let _ = DeleteObject(mask_bitmap as _);
        let _ = DeleteDC(mem_dc);

        (icon != 0).then_some(icon)
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
        use windows_sys::Win32::Graphics::Gdi::{
            BeginPaint, EndPaint, GetSysColor, GetSysColorBrush, SetBkColor, StretchDIBits,
            BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLOR_WINDOW, COLOR_WINDOWTEXT,
            DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
        };
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetKeyState, VK_CONTROL};
        use windows_sys::Win32::UI::Accessibility::{HIGHCONTRASTW, HCF_HIGHCONTRASTON};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClientRect, MoveWindow, SendMessageW, SetWindowPos, SetWindowTextW, ShowWindow,
            CREATESTRUCTW, GWLP_USERDATA, HWND_TOPMOST, SW_SHOW, WM_CLOSE, WM_NCDESTROY,
            IsDialogMessageW, SystemParametersInfoW, MINMAXINFO,
            SPI_GETHIGHCONTRAST, WM_CTLCOLORSTATIC, WM_DPICHANGED, WM_GETMINMAXINFO, WM_KEYDOWN, WM_PAINT, WM_SETFONT, WM_SIZE, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
        };

        const CLASS: &str = "KaixinPinnedScreenshot_v1";
        const ID_COPY: usize = 2101;
        const ID_SAVE: usize = 2102;
        const ID_OCR: usize = 2103;
        const ID_ANNOTATE: usize = 2104;
        const ID_PIN: usize = 2105;
        const ID_CLOSE: usize = 2106;
        struct ResultActions {
            captured: CapturedScreenshot,
            target: HWND,
            buttons: Vec<HWND>,
            status_label: HWND,
            initial_status: &'static str,
            button_font: isize,
            status_font: isize,
            status_failed: bool,
        }
        struct State {
            image: image::RgbaImage,
            pixels: Vec<u8>,
            width: i32,
            height: i32,
            info: BITMAPINFO,
            actions: Option<ResultActions>,
        }

        pub(super) fn spawn(image: image::RgbaImage) {
            thread::spawn(move || {
                if let Err(err) = run(image, None) {
                    runtime_log::log_tray(
                        RuntimeLogLevel::Error,
                        "screenshot_pin",
                        format!("status=failed reason={err}"),
                    );
                }
            });
        }

        pub(super) fn spawn_result(captured: CapturedScreenshot, target: HWND, initial_status: &'static str) {
            thread::spawn(move || {
                let image = captured.image.clone();
                if let Err(err) = run(image, Some(ResultActions { captured, target, buttons: Vec::new(), status_label: 0, initial_status, button_font: 0, status_font: 0, status_failed: initial_status.contains("失败") })) {
                    show_error_message(&format!("无法打开截图操作窗口：{err}"));
                }
            });
        }

        fn run(image: image::RgbaImage, actions: Option<ResultActions>) -> Result<(), String> {
            let is_result = actions.is_some();
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
            let title_text = actions.as_ref().map(|_| "截图结果".to_string())
                .unwrap_or_else(|| "固定截图 · 拖动边框缩放 · Ctrl+C 复制 · 关闭取消固定".to_string());
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
            let state = Box::into_raw(Box::new(State {
                image,
                pixels,
                width,
                height,
                info,
                actions,
            }));
            let scale = (900.0 / width as f64).min(700.0 / height as f64).min(1.0);
            let client_width = (width as f64 * scale).round() as i32;
            let client_height = (height as f64 * scale).round() as i32;
            let dpi = unsafe { GetDpiForSystem() }.max(96) as i32;
            let hwnd = unsafe {
                CreateWindowExW(
                    if is_result { WS_EX_TOOLWINDOW } else { WS_EX_TOPMOST | WS_EX_TOOLWINDOW },
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    100,
                    100,
                    client_width.max(if is_result { 620 * dpi / 96 } else { 160 }) + 20,
                    client_height.max(90) + if is_result { 130 * dpi / 96 } else { 48 },
                    0,
                    0,
                    instance,
                    state.cast(),
                )
            };
            if hwnd == 0 {
                unsafe { drop(Box::from_raw(state)) };
                return Err("创建固定截图窗口失败".to_string());
            }
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                if !is_result { SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                        | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOSIZE,
                ); }
            }
            let mut msg: MSG = unsafe { zeroed() };
            while unsafe { GetMessageW(&mut msg, 0, 0, 0) } > 0 {
                unsafe {
                    if !is_result || IsDialogMessageW(hwnd, &msg) == 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            Ok(())
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
                    if let Some(state) = ptr.as_mut() {
                        match copy_image_to_clipboard(&state.image) {
                            Ok(()) => {
                                if let Some(actions) = &mut state.actions { actions.status_failed = false; SetWindowTextW(actions.status_label, wide("图片已复制").as_ptr()); }
                                else { SetWindowTextW(hwnd, wide("固定截图 · 图片已复制").as_ptr()); }
                            }
                            Err(err) => show_error_message(&format!("复制失败：{err}")),
                        }
                    }
                    0
                }
                WM_CREATE => {
                    if let Some(actions) = ptr.as_mut().and_then(|state| state.actions.as_mut()) {
                        actions.status_label = CreateWindowExW(0, wide("STATIC").as_ptr(), wide(actions.initial_status).as_ptr(),
                            WS_CHILD | WS_VISIBLE, 0, 0, 1, 1, hwnd, 0,
                            GetModuleHandleW(std::ptr::null()), std::ptr::null());
                        let specs = [(ID_COPY, "复制(&C)"), (ID_SAVE, "另存(&S)"), (ID_OCR, "OCR(&O)"),
                            (ID_ANNOTATE, "画图(&A)"), (ID_PIN, "固定(&P)"), (ID_CLOSE, "关闭(&X)")];
                        for (id, label) in specs {
                            let title = wide(if id == ID_OCR && resolve_ocr_path().is_none() { "OCR未安装" } else { label });
                            let button = CreateWindowExW(0, wide("BUTTON").as_ptr(), title.as_ptr(),
                                WS_CHILD | WS_VISIBLE | WS_TABSTOP, 0, 0, 1, 1, hwnd,
                                id as _, GetModuleHandleW(std::ptr::null()), std::ptr::null());
                            if id == ID_OCR && resolve_ocr_path().is_none() { EnableWindow(button, 0); }
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
                    SetWindowPos(hwnd, 0, suggested.left, suggested.top, suggested.right - suggested.left,
                        suggested.bottom - suggested.top, windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER |
                        windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE);
                    0
                }
                WM_SIZE => {
                    if let Some((status_label, buttons)) = ptr.as_ref().and_then(|state| state.actions.as_ref())
                        .map(|actions| (actions.status_label, actions.buttons.clone())) {
                        let mut rect: RECT = zeroed();
                        GetClientRect(hwnd, &mut rect);
                        let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                        let (gap, button_height, columns, bar_height) = layout_metrics(rect.right, dpi);
                        let width = ((rect.right - gap * (columns + 1)) / columns).max(1);
                        MoveWindow(status_label, gap, (rect.bottom - bar_height + gap).max(0),
                            (rect.right - gap * 2).max(1), 24 * dpi / 96, 1);
                        for (index, button) in buttons.iter().enumerate() {
                            let index = index as i32;
                            MoveWindow(*button, gap + (index % columns) * (width + gap),
                                (rect.bottom - bar_height + 28 * dpi / 96 + (index / columns) * (button_height + gap)).max(0), width, button_height, 1);
                        }
                    }
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_COMMAND if (wparam >> 16) == 0 => {
                    let id = wparam & 0xffff;
                    if id == ID_CLOSE { DestroyWindow(hwnd); return 0; }
                    let action = match id {
                            ID_COPY => Some(screenshot_region_selector::RegionAction::Copy),
                            ID_SAVE => Some(screenshot_region_selector::RegionAction::SaveAs),
                            ID_OCR => Some(screenshot_region_selector::RegionAction::Ocr),
                            ID_ANNOTATE => Some(screenshot_region_selector::RegionAction::Annotate),
                            ID_PIN => Some(screenshot_region_selector::RegionAction::Pin),
                            _ => None,
                    };
                    if let Some(action) = action {
                        if let Some((mut captured, target)) = ptr.as_ref().and_then(|state| state.actions.as_ref())
                            .map(|actions| (actions.captured.clone(), actions.target)) {
                            captured.action = action;
                            let result = finish_captured_screenshot(captured, target, &screenshot_prefs());
                            if IsWindow(hwnd) != 0 && GetWindowLongPtrW(hwnd, GWLP_USERDATA) == ptr as isize {
                            let (status, failed) = match &result {
                                Ok(done) => {
                                    let status = if !done { "已取消，图片仍保留" } else {
                                        match id { ID_COPY => "图片已复制", ID_SAVE => "图片已保存",
                                            ID_OCR => "OCR 已启动", ID_ANNOTATE => "已在画图中打开",
                                            ID_PIN => "图片已固定", _ => "操作完成" }
                                    };
                                    (status.to_string(), false)
                                }
                                Err(err) => (format!("操作失败：{err}"), true),
                            };
                            let status_label = ptr.as_mut().and_then(|state| state.actions.as_mut()).map(|actions| {
                                actions.status_failed = failed;
                                actions.status_label
                            });
                            if let Some(label) = status_label { SetWindowTextW(label, wide(&status).as_ptr()); }
                            }
                            if let Err(err) = result { show_error_message(&format!("截图操作失败，可重试：{err}")); }
                            return 0;
                        }
                    }
                    DefWindowProcW(hwnd, message, wparam, lparam)
                }
                WM_CTLCOLORSTATIC => {
                    if let Some(actions) = ptr.as_ref().and_then(|state| state.actions.as_ref()) {
                        if lparam as HWND == actions.status_label {
                            let mut contrast: HIGHCONTRASTW = zeroed();
                            contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
                            let high_contrast = SystemParametersInfoW(SPI_GETHIGHCONTRAST, contrast.cbSize,
                                (&mut contrast as *mut HIGHCONTRASTW).cast(), 0) != 0 &&
                                (contrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
                            let background = GetSysColor(COLOR_WINDOW);
                            let brightness = (background & 255) + ((background >> 8) & 255) + ((background >> 16) & 255);
                            let color = if high_contrast { GetSysColor(COLOR_WINDOWTEXT) }
                                else if actions.status_failed { if brightness < 384 { rgb(255, 150, 150) } else { rgb(160, 25, 25) } }
                                else if brightness < 384 { rgb(170, 225, 190) } else { rgb(26, 103, 55) };
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
                        let (_, _, _, bar_height) = layout_metrics(rect.right, GetDpiForWindow(hwnd).max(96) as i32);
                        let available_h = if state.actions.is_some() { (rect.bottom - bar_height).max(1) } else { rect.bottom.max(1) };
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
                            if actions.button_font != 0 { DeleteObject(actions.button_font); }
                            if actions.status_font != 0 { DeleteObject(actions.status_font); }
                        }
                        drop(Box::from_raw(ptr));
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
            (gap, button_height, columns, 28 * dpi / 96 + rows * (button_height + gap) + gap)
        }

        #[cfg(test)]
        mod tests {
            use super::layout_metrics;

            #[test]
            fn result_buttons_fit_wide_and_narrow_windows_at_common_dpis() {
                for dpi in [96, 120, 144, 192] {
                    for (logical_width, expected_columns) in [(350, 3), (620, 6)] {
                        let width = logical_width * dpi / 96;
                        let (gap, height, columns, bar) = layout_metrics(width, dpi);
                        let button_width = (width - gap * (columns + 1)) / columns;
                        assert_eq!(columns, expected_columns);
                        assert!(button_width >= 90 * dpi / 96, "dpi={dpi} width={logical_width}");
                        assert!(height >= 36 * dpi / 96);
                        assert!(bar >= (28 + 36 * (6 / columns)) * dpi / 96);
                    }
                }
            }
        }

        unsafe fn update_fonts(actions: &mut ResultActions, dpi: u32) {
            let button_font = screenshot_region_selector::create_screenshot_ui_font(dpi, 15);
            let status_font = screenshot_region_selector::create_screenshot_ui_font(dpi, 14);
            if button_font != 0 {
                for button in &actions.buttons { SendMessageW(*button, WM_SETFONT, button_font as usize, 1); }
                if actions.button_font != 0 { DeleteObject(actions.button_font); }
                actions.button_font = button_font;
            }
            if status_font != 0 {
                SendMessageW(actions.status_label, WM_SETFONT, status_font as usize, 1);
                if actions.status_font != 0 { DeleteObject(actions.status_font); }
                actions.status_font = status_font;
            }
        }
    }
}

#[cfg(windows)]
fn main() {
    pinyin_ime::windows_security::apply_process_hardening();
    win::run();
}
