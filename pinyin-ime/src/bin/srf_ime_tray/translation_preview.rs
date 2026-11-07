//! Shared candidate / selection / OCR preview. Text travels over anonymous pipes only.
use eframe::egui;
use pinyin_ime::{app_paths, external_translation as bridge, win_paste};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc::Receiver}, time::Duration};
use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION}, UI::{Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO, GetAsyncKeyState}, WindowsAndMessaging::*}};

#[path = "../../fonts.rs"]
mod fonts;

const LANGUAGES: &[(&str, &str)] = &[("auto-opposite", "自动中英互译"), ("zh", "简体中文"), ("zh-Hant", "繁体中文"), ("en", "英语"), ("ja", "日语"), ("ko", "韩语"), ("fr", "法语"), ("de", "德语"), ("es", "西班牙语"), ("ru", "俄语")];

static KEY_TARGET: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static KEY_READY: AtomicBool = AtomicBool::new(false);
static KEY_CONFIRM: AtomicBool = AtomicBool::new(false);
static KEY_CANCEL: AtomicBool = AtomicBool::new(false);
static KEY_EDITED: AtomicBool = AtomicBool::new(false);
unsafe extern "system" fn preview_key(code: i32, message: usize, data: isize) -> isize {
    if code >= 0 && GetForegroundWindow() == KEY_TARGET.load(Ordering::Relaxed)
        && ![0x10, 0x11, 0x12].iter().any(|key| GetAsyncKeyState(*key) < 0) {
        let key = &*(data as *const KBDLLHOOKSTRUCT);
        if key.flags & LLKHF_INJECTED == 0 && (key.vkCode == 0x1B || (key.vkCode == 0x0D && KEY_READY.load(Ordering::Relaxed))) {
            if message == WM_KEYDOWN as usize {
                if key.vkCode == 0x1B { KEY_CANCEL.store(true, Ordering::Release); }
                else { KEY_CONFIRM.store(true, Ordering::Release); }
            }
            return 1;
        }
        if message == WM_KEYDOWN as usize { KEY_EDITED.store(true, Ordering::Release); }
    }
    CallNextHookEx(0, code, message, data)
}
struct KeyHook(isize);
impl Drop for KeyHook { fn drop(&mut self) { unsafe { UnhookWindowsHookEx(self.0); } KEY_TARGET.store(0, Ordering::Relaxed); } }

#[derive(Clone, Serialize, Deserialize)]
struct Preference { language: String, action: String }

fn preference_path() -> std::path::PathBuf {
    app_paths::local_data_dir().unwrap_or_else(std::env::temp_dir).join("translation-apps.json")
}

fn preferences() -> BTreeMap<String, Preference> {
    std::fs::read(preference_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_preference(exe: &str, preference: Option<Preference>) -> Result<(), String> {
    use fs2::FileExt;
    let path = preference_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path.with_extension("lock")).map_err(|e| e.to_string())?;
    lock.lock_exclusive().map_err(|e| e.to_string())?;
    let mut values = preferences();
    if let Some(value) = preference { values.insert(exe.to_owned(), value); } else { values.remove(exe); }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&values).unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

fn executable(pid: Option<u32>) -> String {
    let Some(pid) = pid else { return String::new() };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 { return String::new(); }
        let mut buffer = vec![0u16; 32768]; let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 { String::new() } else { String::from_utf16_lossy(&buffer[..len as usize]).to_lowercase() }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Caret { focus: isize, caret: isize, rect: [i32; 4] }
fn caret(hwnd: isize) -> Option<Caret> {
    unsafe {
        let thread = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
        let mut info: GUITHREADINFO = std::mem::zeroed(); info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if thread == 0 || GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndFocus == 0 || info.hwndCaret == 0 { return None; }
        Some(Caret { focus: info.hwndFocus, caret: info.hwndCaret, rect: [info.rcCaret.left, info.rcCaret.top, info.rcCaret.right, info.rcCaret.bottom] })
    }
}
fn last_input() -> u32 {
    let mut value = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe { GetLastInputInfo(&mut value); } value.dwTime
}
fn clipboard_sequence() -> u32 { unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() } }

fn target_changed(front: isize, target: isize, own: bool, source: isize, input_changed: bool, caret_changed: bool) -> bool {
    !own && !(source != 0 && front == source) && (front != target || input_changed || caret_changed)
}
fn position(hwnd: isize) -> Option<egui::Pos2> {
    let anchor = caret(hwnd)?;
    unsafe {
        let mut point = windows_sys::Win32::Foundation::POINT { x: anchor.rect[0], y: anchor.rect[3] + 8 };
        windows_sys::Win32::Graphics::Gdi::ClientToScreen(anchor.caret, &mut point);
        let monitor = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(hwnd, 2);
        let mut info: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed(); info.cbSize = std::mem::size_of_val(&info) as u32;
        if windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(monitor, &mut info) == 0 { return None; }
        let scale = windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let x = point.x.max(info.rcWork.left).min((info.rcWork.right - (520.0 * scale) as i32).max(info.rcWork.left));
        let y = point.y.max(info.rcWork.top).min((info.rcWork.bottom - (420.0 * scale) as i32).max(info.rcWork.top));
        Some(egui::pos2(x as f32 / scale, y as f32 / scale))
    }
}

struct Preview {
    request: bridge::ExternalTranslationRequest, events: Receiver<bridge::TranslationCallbackEvent>, stop: Arc<AtomicBool>,
    result: String, status: String, done: bool, completed: bool, target: isize, original: Option<Caret>,
    input: u32, unsafe_target: bool, exe: String, action: String, remember: bool, changed: bool, source_window: isize, clipboard: u32,
}
impl Drop for Preview { fn drop(&mut self) { self.stop.store(true, Ordering::Release); } }
impl Preview {
    fn start(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.request.request_id = bridge::fresh_request_id();
        self.request.generation += 1;
        (self.events, self.stop) = bridge::preview_stream(self.request.clone());
        self.result.clear(); self.done = false; self.completed = false; self.status = "正在连接或启动翻译软件…".into(); self.changed = false;
        self.clipboard = clipboard_sequence();
    }
    fn copy(&mut self, original: bool) {
        let text = if original { &self.request.text } else { &self.result };
        self.status = match clipboard_win::set_clipboard(clipboard_win::formats::Unicode, text) {
            Ok(()) => if original { "已复制原文" } else { "已复制译文" }.into(), Err(e) => format!("复制失败：{e}"),
        };
    }
    fn insert(&mut self) -> Result<(), String> {
        if self.unsafe_target || self.original.is_none() { return Err("编辑位置已变化或无法确认，请复制译文后手动粘贴。".into()); }
        if bridge::process_id_for_window(self.target) != self.request.target_process_id { return Err("原窗口已关闭，请复制译文。".into()); }
        unsafe {
            if [0x10, 0x11, 0x12].iter().any(|key| GetAsyncKeyState(*key) < 0) { return Err("请松开 Shift / Ctrl / Alt 后再确认。".into()); }
            if SetForegroundWindow(self.target) == 0 { return Err("无法返回原窗口，请复制译文。".into()); }
        }
        std::thread::sleep(Duration::from_millis(80));
        if unsafe { GetForegroundWindow() } != self.target || caret(self.target) != self.original {
            self.unsafe_target = true; return Err("原窗口的编辑位置已变化，请复制译文。".into());
        }
        clipboard_win::set_clipboard(clipboard_win::formats::Unicode, &self.result).map_err(|e| e.to_string())?;
        win_paste::send_ctrl_v_to_target(self.target)
    }
    fn guard(&mut self) {
        let front = unsafe { GetForegroundWindow() };
        let own = bridge::process_id_for_window(front) == Some(std::process::id());
        self.unsafe_target |= target_changed(front, self.target, own, self.source_window, last_input() != self.input, caret(self.target) != self.original);
    }
}
impl eframe::App for Preview {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Enter/Escape only belong to this preview while its original editor is active.
        let confirm_key = KEY_CONFIRM.swap(false, Ordering::AcqRel);
        if confirm_key { self.input = last_input(); }
        if KEY_EDITED.load(Ordering::Acquire) { self.unsafe_target = true; }
        self.guard();
        while let Ok(event) = self.events.try_recv() {
            self.status = event.message.unwrap_or_default();
            if event.event_name == "completed" {
                self.result = event.text.unwrap_or_default(); self.done = true; self.completed = true;
                self.status = "翻译完成，确认后输入；Esc 取消".into();
                let automatic = !self.changed && clipboard_sequence() == self.clipboard;
                if self.action == "copy" && automatic { self.copy(false); }
                if self.action == "paste" && automatic {
                    match self.insert() { Ok(()) => { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }, Err(error) => self.status = error }
                }
                if !automatic && self.action != "show" { self.status = "语言设置或剪贴板已变化，请确认译文后手动操作。".into(); }
            } else if matches!(event.event_name.as_str(), "failed" | "cancelled") { self.done = true; }
        }
        if KEY_CANCEL.swap(false, Ordering::AcqRel) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) { self.stop.store(true, Ordering::Release); ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
        let mut confirm = false; let mut retry = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(if self.request.origin.contains("ocr") { "截图翻译 · 原译文对照" } else if self.request.replace_selection { "选中文本翻译" } else { "候选／整句翻译" });
            ui.horizontal_wrapped(|ui| {
                ui.label("译成");
                let before = self.request.target.clone();
                egui::ComboBox::from_id_salt("language").selected_text(LANGUAGES.iter().find(|(v, _)| *v == self.request.target).map(|(_, l)| *l).unwrap_or(&self.request.target)).show_ui(ui, |ui| {
                    for (value, label) in LANGUAGES { ui.selectable_value(&mut self.request.target, value.to_string(), *label); }
                });
                if before != self.request.target { self.changed = true; }
                retry = ui.button(if self.changed { "按新语言翻译" } else { "重新翻译" }).clicked();
            });
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("action").selected_text(match self.action.as_str() { "copy" => "完成后复制", "paste" => "完成后粘贴", _ => "预览后确认" }).show_ui(ui, |ui| {
                    for (v,l) in [("show","预览后确认"),("copy","完成后复制"),("paste","完成后粘贴")] { ui.selectable_value(&mut self.action, v.into(), l); }
                });
                if !self.exe.is_empty() {
                    let label = std::path::Path::new(&self.exe).file_name().unwrap_or_default().to_string_lossy();
                    ui.checkbox(&mut self.remember, format!("记住 {label}"));
                    if ui.small_button("保存偏好").clicked() {
                        self.status = match save_preference(&self.exe, self.remember.then(|| Preference { language: self.request.target.clone(), action: self.action.clone() })) {
                            Ok(()) => if self.remember { "已保存此应用的语言和操作" } else { "已恢复此应用的全局默认设置" }.into(), Err(e) => format!("保存失败：{e}"),
                        };
                    }
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height((ui.available_height() - 100.0).max(100.0)).show(ui, |ui| {
                ui.label("原文"); ui.add(egui::Label::new(&self.request.text).selectable(true));
                ui.separator(); ui.label("译文"); ui.add(egui::Label::new(&self.result).selectable(true));
            });
            ui.separator(); ui.label(&self.status);
            if self.unsafe_target { ui.colored_label(egui::Color32::from_rgb(170, 100, 30), "输入位置已变化，本次只展示结果，可复制后手动粘贴。"); }
            ui.horizontal_wrapped(|ui| {
                if ui.button("复制原文").clicked() { self.copy(true); }
                if ui.add_enabled(self.completed && !self.changed, egui::Button::new("复制译文")).clicked() { self.copy(false); }
                confirm = ui.add_enabled(self.completed && !self.changed && !self.unsafe_target && self.original.is_some(), egui::Button::new(if self.request.replace_selection { "确认替换 · Enter" } else { "插入译文 · Enter" })).clicked();
                if ui.button("取消 · Esc").clicked() { self.stop.store(true, Ordering::Release); ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
            });
        });
        if retry { self.start(); }
        KEY_READY.store(self.completed && !self.changed && !self.unsafe_target && self.original.is_some(), Ordering::Release);
        if self.completed && !self.changed && (confirm || confirm_key || (!ctx.wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))) {
            match self.insert() { Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close), Err(error) => self.status = error }
        }
        ctx.request_repaint_after(Duration::from_millis(80));
    }
}

pub fn run() -> Result<(), String> {
    let mut bytes = Vec::new(); std::io::stdin().take(1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 { return Err("翻译请求过长".into()); }
    let mut request: bridge::ExternalTranslationRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    request.request_id = bridge::fresh_request_id();
    let target = request.target_hwnd.unwrap_or(0);
    request.session = format!("kaixin-preview-{}-{target}", std::process::id());
    let original = caret(target);
    // Let the triggering shortcut be released before opening the preview.
    for _ in 0..30 {
        if ![0x10, 0x11, 0x12, 0x0D].iter().any(|key| unsafe { GetAsyncKeyState(*key) } < 0) { break; }
        std::thread::sleep(Duration::from_millis(10));
    }
    let input = last_input();
    let exe = executable(request.target_process_id);
    let saved = preferences().get(&exe).cloned();
    let mut action = request.result_action.clone();
    if let Some(value) = &saved { request.target = value.language.clone(); action = value.action.clone(); }
    let unsafe_target = caret(target) != original || (!request.origin.contains("ocr") && unsafe { GetForegroundWindow() } != target);
    let source_window = if request.origin.contains("ocr") { unsafe { GetForegroundWindow() } } else { 0 };
    KEY_TARGET.store(target, Ordering::Relaxed);
    let _hook = KeyHook(unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(preview_key), windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()), 0) });
    let (events, stop) = bridge::preview_stream(request.clone());
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([520.0, 420.0]).with_min_inner_size([420.0, 300.0]).with_always_on_top().with_active(false);
    if let Some(point) = position(target) { viewport = viewport.with_position(point); }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("开心输入法 · 翻译预览", options, Box::new(move |cc| {
        fonts::install_cjk_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        Ok(Box::new(Preview { request, events, stop, result: String::new(), status: "正在连接或启动翻译软件…".into(), done: false, completed: false, target, original, input, unsafe_target, exe, action, remember: saved.is_some(), changed: false, source_window, clipboard: clipboard_sequence() }))
    })).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(request.text, "你好🙂");
    }
}
