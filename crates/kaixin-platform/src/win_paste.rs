#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::GetLastError;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    VK_BACK, VK_CONTROL, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

#[cfg(windows)]
fn key_input(vk: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(windows)]
fn focus_target_window(target_hwnd: isize) -> Result<(), String> {
    if target_hwnd != 0 && unsafe { GetForegroundWindow() } != target_hwnd {
        if unsafe { IsWindow(target_hwnd) } == 0 {
            return Err("目标窗口已经不存在".to_string());
        }

        let current_thread = unsafe { GetCurrentThreadId() };
        let target_thread = unsafe { GetWindowThreadProcessId(target_hwnd, std::ptr::null_mut()) };
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_thread = if foreground != 0 {
            unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) }
        } else {
            0
        };

        let attach_target = target_thread != 0
            && target_thread != current_thread
            && unsafe { AttachThreadInput(current_thread, target_thread, 1) } != 0;
        let attach_foreground = foreground_thread != 0
            && foreground_thread != current_thread
            && foreground_thread != target_thread
            && unsafe { AttachThreadInput(current_thread, foreground_thread, 1) } != 0;

        if unsafe { IsIconic(target_hwnd) } != 0 {
            unsafe {
                ShowWindow(target_hwnd, SW_RESTORE);
            }
        }
        unsafe {
            BringWindowToTop(target_hwnd);
        }
        let set_foreground_ok = unsafe { SetForegroundWindow(target_hwnd) } != 0;
        let set_foreground_error = unsafe { GetLastError() };

        if attach_foreground {
            unsafe {
                AttachThreadInput(current_thread, foreground_thread, 0);
            }
        }
        if attach_target {
            unsafe {
                AttachThreadInput(current_thread, target_thread, 0);
            }
        }

        for _ in 0..24 {
            if unsafe { GetForegroundWindow() } == target_hwnd {
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        if unsafe { GetForegroundWindow() } != target_hwnd {
            let reason = if set_foreground_ok {
                "目标窗口没有重新获得焦点".to_string()
            } else {
                format!("SetForegroundWindow 被系统拒绝，错误码 {set_foreground_error}")
            };
            return Err(format!(
                "{reason}；可能是权限、全屏独占、反作弊或宿主限制了模拟粘贴"
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn send_ctrl_key_to_target(target_hwnd: isize, key: u16, label: &str) -> Result<(), String> {
    focus_target_window(target_hwnd)?;
    let inputs = [
        key_input(VK_CONTROL, 0),
        key_input(key, 0),
        key_input(key, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(format!(
            "SendInput({label}) 只发送 {sent}/{} 个事件，错误码 {}；可能是权限、全屏独占、反作弊或宿主拒绝模拟输入",
            inputs.len(),
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub fn send_ctrl_v_to_target(target_hwnd: isize) -> Result<(), String> {
    send_ctrl_key_to_target(target_hwnd, b'V' as u16, "Ctrl+V")
}

/// Type literal Unicode without changing the user's clipboard or IME mode.
#[cfg(windows)]
pub fn send_unicode_text_to_target(target_hwnd: isize, text: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE;
    if target_hwnd == 0 || unsafe { IsWindow(target_hwnd) } == 0 {
        return Err("请先点击需要输入符号的编辑窗口".to_string());
    }
    if text.is_empty() {
        return Ok(());
    }
    focus_target_window(target_hwnd)?;
    let inputs: Vec<_> = text
        .encode_utf16()
        .flat_map(|unit| {
            [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP].map(|flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: unit,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
        })
        .collect();
    if unsafe { GetForegroundWindow() } != target_hwnd {
        return Err("目标窗口焦点已改变，请重新选择输入位置".to_string());
    }
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(format!(
            "符号输入被目标程序拒绝（{sent}/{}）；可切换到仅复制后手动粘贴",
            inputs.len()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn send_unicode_text_to_target(_target_hwnd: isize, _text: &str) -> Result<(), String> {
    Err("符号直接输入仅支持 Windows".to_string())
}

#[cfg(windows)]
pub fn send_virtual_key_to_target(
    target_hwnd: isize,
    vk: u16,
    extended: bool,
    label: &str,
) -> Result<(), String> {
    if target_hwnd == 0 || unsafe { IsWindow(target_hwnd) } == 0 {
        return Err("请先点击需要输入的编辑窗口".to_string());
    }
    focus_target_window(target_hwnd)?;
    if unsafe { GetForegroundWindow() } != target_hwnd {
        return Err("目标窗口焦点已改变，请重新选择输入位置".to_string());
    }
    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    let inputs = [key_input(vk, ext), key_input(vk, ext | KEYEVENTF_KEYUP)];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(format!(
            "按键 {label} 被目标程序拒绝（{sent}/{}），错误码 {}",
            inputs.len(),
            unsafe { GetLastError() }
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub fn send_backspace_to_target(target_hwnd: isize) -> Result<(), String> {
    send_virtual_key_to_target(target_hwnd, VK_BACK, false, "Backspace")
}

#[cfg(windows)]
pub fn send_enter_to_target(target_hwnd: isize) -> Result<(), String> {
    send_virtual_key_to_target(target_hwnd, VK_RETURN, false, "Enter")
}

#[cfg(windows)]
pub fn send_space_to_target(target_hwnd: isize) -> Result<(), String> {
    send_virtual_key_to_target(target_hwnd, VK_SPACE, false, "Space")
}

#[cfg(windows)]
pub fn send_arrow_left_to_target(target_hwnd: isize) -> Result<(), String> {
    send_virtual_key_to_target(target_hwnd, VK_LEFT, true, "Left")
}

#[cfg(windows)]
pub fn send_arrow_right_to_target(target_hwnd: isize) -> Result<(), String> {
    send_virtual_key_to_target(target_hwnd, VK_RIGHT, true, "Right")
}

#[cfg(not(windows))]
pub fn send_virtual_key_to_target(
    _target_hwnd: isize,
    _vk: u16,
    _extended: bool,
    _label: &str,
) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_backspace_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_enter_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_space_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_arrow_left_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn send_arrow_right_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("按键直接输入仅支持 Windows".to_string())
}

#[cfg(windows)]
pub fn send_ctrl_c_to_target(target_hwnd: isize) -> Result<(), String> {
    send_ctrl_key_to_target(target_hwnd, b'C' as u16, "Ctrl+C")
}

#[cfg(windows)]
enum ClipboardBackup {
    Formats {
        raw: Vec<(u32, Vec<u8>)>,
        bitmap: Option<Vec<u8>>,
    },
    Empty,
    Unsupported,
}

#[cfg(windows)]
fn capture_open_clipboard() -> ClipboardBackup {
    const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;
    let formats = clipboard_win::EnumFormats::new().collect::<Vec<_>>();
    if formats.is_empty() {
        return ClipboardBackup::Empty;
    }
    let mut raw = Vec::new();
    let mut bitmap = None;
    let mut total_bytes = 0usize;
    for format in formats {
        if format == clipboard_win::formats::CF_BITMAP {
            let Ok(value) = clipboard_win::get::<Vec<u8>, _>(clipboard_win::formats::Bitmap) else {
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

#[cfg(windows)]
fn restore_open_clipboard(backup: ClipboardBackup) -> Result<(), String> {
    match backup {
        ClipboardBackup::Formats { raw, bitmap } => {
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
        ClipboardBackup::Empty => clipboard_win::empty().map_err(|err| err.to_string()),
        ClipboardBackup::Unsupported => {
            Err("原剪贴板包含暂不支持还原的自定义格式，无法安全恢复。".to_string())
        }
    }
}

/// Insert text into `target_hwnd` with minimal clipboard disruption.
/// Short single-line text prefers direct Unicode `SendInput` (zero clipboard touch);
/// longer or multiline text uses `Ctrl+V` and restores the previous clipboard if unchanged.
#[cfg(windows)]
pub fn insert_text_preserving_clipboard(target_hwnd: isize, text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let short_single_line =
        text.chars().count() <= 120 && !text.contains('\n') && !text.contains('\r');
    if short_single_line && send_unicode_text_to_target(target_hwnd, text).is_ok() {
        return Ok(());
    }
    let (backup, written_seq) = {
        let _clipboard = clipboard_win::Clipboard::new_attempts(10)
            .map_err(|err| format!("无法打开剪贴板，已跳过本次回填：{err}"))?;
        let backup = capture_open_clipboard();
        if matches!(backup, ClipboardBackup::Unsupported) {
            return Err("原剪贴板内容无法安全备份，已保留原内容并跳过本次回填。".into());
        }
        let original_seq = clipboard_win::seq_num();
        if let Err(error) = clipboard_win::set(clipboard_win::formats::Unicode, text) {
            let original_error = format!("写入剪贴板失败：{error}");
            if original_seq.is_some() && clipboard_win::seq_num() == original_seq {
                return Err(original_error);
            }
            return match restore_open_clipboard(backup) {
                Ok(()) => Err(original_error),
                Err(restore_error) => Err(format!(
                    "{original_error}；恢复原剪贴板失败：{restore_error}"
                )),
            };
        }
        let Some(written_seq) = clipboard_win::seq_num() else {
            restore_open_clipboard(backup)?;
            return Err("无法跟踪剪贴板变化，已跳过本次回填。".into());
        };
        (backup, written_seq)
    };
    if clipboard_win::seq_num() != Some(written_seq) {
        return Err("剪贴板已变化，已跳过本次回填。".into());
    }
    let paste_res = send_ctrl_v_to_target(target_hwnd);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(260));
        let Ok(_clipboard) = clipboard_win::Clipboard::new_attempts(10) else {
            return;
        };
        if clipboard_win::seq_num() == Some(written_seq) {
            let _ = restore_open_clipboard(backup);
        }
    });
    paste_res
}

#[cfg(not(windows))]
pub fn send_ctrl_v_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("当前平台不支持系统剪贴板快粘".to_string())
}

#[cfg(not(windows))]
pub fn send_ctrl_c_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("当前平台不支持选中文本复制".to_string())
}
