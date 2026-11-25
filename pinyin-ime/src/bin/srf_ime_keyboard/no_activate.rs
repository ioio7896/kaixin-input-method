//! Windows-specific helper that prevents the floating on-screen keyboard from
//! stealing keyboard focus from the target editor window when keys are clicked.

#[cfg(windows)]
mod platform {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC,
        GWL_EXSTYLE, MA_NOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, WM_MOUSEACTIVATE, WM_NCDESTROY, WNDPROC, WS_EX_NOACTIVATE,
    };

    static SUBCLASSED_HWND: AtomicIsize = AtomicIsize::new(0);
    static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn no_activate_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_MOUSEACTIVATE {
            return MA_NOACTIVATE as LRESULT;
        }
        let prev = PREV_WNDPROC.load(Ordering::Acquire);
        if msg == WM_NCDESTROY {
            if prev != 0 {
                let _ = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, prev);
            }
            SUBCLASSED_HWND.store(0, Ordering::Release);
            PREV_WNDPROC.store(0, Ordering::Release);
        }
        if prev != 0 {
            let proc: WNDPROC = std::mem::transmute(prev);
            return CallWindowProcW(proc, hwnd, msg, wparam, lparam);
        }
        0
    }

    pub fn ensure_no_activate(frame: &eframe::Frame) {
        let Ok(handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(window) = handle.as_raw() else {
            return;
        };
        let hwnd = window.hwnd.get();
        if hwnd == 0 || SUBCLASSED_HWND.load(Ordering::Acquire) == hwnd {
            return;
        }
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let desired = ex_style | (WS_EX_NOACTIVATE as isize);
            if desired != ex_style {
                let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired);
                let _ = SetWindowPos(
                    hwnd,
                    0,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            let prev = SetWindowLongPtrW(
                hwnd,
                GWLP_WNDPROC,
                no_activate_wndproc as *const () as isize,
            );
            if prev != 0 {
                PREV_WNDPROC.store(prev, Ordering::Release);
                SUBCLASSED_HWND.store(hwnd, Ordering::Release);
            }
        }
    }
}

#[cfg(windows)]
pub use platform::ensure_no_activate;

#[cfg(not(windows))]
pub fn ensure_no_activate(_frame: &eframe::Frame) {}
