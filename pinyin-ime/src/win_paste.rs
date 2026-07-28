#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows_sys::Win32::Foundation::GetLastError;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
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

#[cfg(windows)]
pub fn send_ctrl_c_to_target(target_hwnd: isize) -> Result<(), String> {
    send_ctrl_key_to_target(target_hwnd, b'C' as u16, "Ctrl+C")
}

#[cfg(not(windows))]
pub fn send_ctrl_v_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("当前平台不支持系统剪贴板快粘".to_string())
}

#[cfg(not(windows))]
pub fn send_ctrl_c_to_target(_target_hwnd: isize) -> Result<(), String> {
    Err("当前平台不支持选中文本复制".to_string())
}
