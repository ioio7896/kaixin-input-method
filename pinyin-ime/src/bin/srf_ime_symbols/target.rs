//! Remember the last external foreground window while the keyboard is open.
//! A WinEvent hook catches even brief switches before the user clicks a key.
#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
        IsWindowVisible,
    };

    static TARGET: AtomicIsize = AtomicIsize::new(0);
    static TARGET_PID: AtomicU32 = AtomicU32::new(0);

    fn candidate(hwnd: HWND) -> Option<u32> {
        if hwnd == 0 || unsafe { IsWindow(hwnd) == 0 || IsWindowVisible(hwnd) == 0 } {
            return None;
        }
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
        }
        if pid == 0 || pid == std::process::id() {
            return None;
        }
        let mut class = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        if matches!(
            class.as_str(),
            "KaixinImeTrayWindow"
                | "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "NotifyIconOverflowWindow"
                | "Progman"
                | "WorkerW"
                | "#32768"
        ) {
            return None;
        }
        let mut title = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if String::from_utf16_lossy(&title[..len.max(0) as usize]).starts_with("开心输入法 ") {
            return None;
        }
        Some(pid)
    }

    fn remember(hwnd: HWND) {
        if let Some(pid) = candidate(hwnd) {
            TARGET_PID.store(pid, Ordering::Relaxed);
            TARGET.store(hwnd, Ordering::Relaxed);
        }
    }

    pub fn remember_foreground() {
        remember(unsafe { GetForegroundWindow() });
    }

    unsafe extern "system" fn foreground_changed(
        _hook: isize,
        _event: u32,
        hwnd: HWND,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        remember(hwnd);
    }

    pub struct Tracker {
        hook: isize,
    }
    impl Tracker {
        #[cfg(test)]
        pub fn for_test() -> Self {
            Self { hook: 0 }
        }
        pub fn new() -> Self {
            remember_foreground();
            let mut args = std::env::args().skip(1);
            while let Some(arg) = args.next() {
                if arg == "--target-hwnd" {
                    if let Some(hwnd) = args.next().and_then(|value| value.parse().ok()) {
                        remember(hwnd);
                    }
                }
            }
            // EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT | SKIPOWNPROCESS.
            Self {
                hook: unsafe { SetWinEventHook(3, 3, 0, Some(foreground_changed), 0, 0, 2) },
            }
        }
    }
    impl Drop for Tracker {
        fn drop(&mut self) {
            if self.hook != 0 {
                unsafe {
                    UnhookWinEvent(self.hook);
                }
            }
        }
    }

    pub fn validated_target() -> Result<isize, String> {
        let hwnd = TARGET.load(Ordering::Relaxed);
        let pid = TARGET_PID.load(Ordering::Relaxed);
        if candidate(hwnd) == Some(pid) && pid != 0 {
            Ok(hwnd)
        } else {
            Err("请先点击需要输入符号的编辑框，再回来点选符号；也可勾选仅复制。".into())
        }
    }
}
#[cfg(windows)]
pub use platform::*;

#[cfg(not(windows))]
pub struct Tracker;
#[cfg(not(windows))]
impl Tracker {
    pub fn new() -> Self {
        Self
    }
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self
    }
}
#[cfg(not(windows))]
pub fn remember_foreground() {}
#[cfg(not(windows))]
pub fn validated_target() -> Result<isize, String> {
    Err("符号软键盘直接输入仅支持 Windows".into())
}
