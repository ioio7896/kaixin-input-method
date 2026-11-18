#[cfg(windows)]
use crate::win_handle::OwnedWinHandle;

#[cfg(windows)]
pub struct SingleInstanceGuard {
    _handle: OwnedWinHandle,
}

#[cfg(windows)]
pub fn claim_or_activate_existing(
    mutex_name: &str,
    window_title: &str,
) -> Result<Option<SingleInstanceGuard>, String> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };

    let mutex_name = wide(mutex_name);
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if handle == 0 {
        return Err(format!(
            "create single-instance mutex failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let title = wide(window_title);
        let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if hwnd != 0 {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        // SAFETY: CreateMutexW returned a new handle owned by this function.
        drop(unsafe { OwnedWinHandle::from_raw(handle) }.expect("validated mutex handle"));
        return Ok(None);
    }

    // SAFETY: CreateMutexW returned a new mutex handle transferred to the guard.
    let handle = unsafe { OwnedWinHandle::from_raw(handle) }
        .map_err(|err| format!("own single-instance mutex failed: {err}"))?;
    Ok(Some(SingleInstanceGuard { _handle: handle }))
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(windows))]
pub struct SingleInstanceGuard;

#[cfg(not(windows))]
pub fn claim_or_activate_existing(
    _mutex_name: &str,
    _window_title: &str,
) -> Result<Option<SingleInstanceGuard>, String> {
    Ok(Some(SingleInstanceGuard))
}
