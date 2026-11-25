//! Ownership wrapper for Win32 kernel handles.
//!
//! Keeping the unsafe `CloseHandle` contract here prevents IPC and tool
//! modules from growing subtly different drop implementations.

use std::io;
use std::mem::ManuallyDrop;
use std::os::windows::io::{AsRawHandle, RawHandle};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

#[derive(Debug)]
pub struct OwnedWinHandle(HANDLE);

impl OwnedWinHandle {
    /// Takes ownership of a valid Win32 kernel handle.
    ///
    /// # Safety
    /// `handle` must be exclusively owned, released with `CloseHandle`, and
    /// must not be closed elsewhere after this call.
    pub unsafe fn from_raw(handle: HANDLE) -> io::Result<Self> {
        if handle == 0 || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    /// Checked spelling for call sites that want to emphasize validation.
    ///
    /// # Safety
    /// The ownership requirements are identical to [`Self::from_raw`].
    pub unsafe fn try_from_raw(handle: HANDLE) -> io::Result<Self> {
        // SAFETY: the caller promises the same ownership contract.
        unsafe { Self::from_raw(handle) }
    }

    pub fn as_raw(&self) -> HANDLE {
        self.0
    }

    pub fn is_valid(&self) -> bool {
        self.0 != 0 && self.0 != INVALID_HANDLE_VALUE
    }

    /// Relinquishes ownership without closing the handle.
    pub fn into_raw(self) -> HANDLE {
        ManuallyDrop::new(self).0
    }
}

impl AsRawHandle for OwnedWinHandle {
    fn as_raw_handle(&self) -> RawHandle {
        self.0 as RawHandle
    }
}

impl Drop for OwnedWinHandle {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: construction requires exclusive ownership of a handle
            // whose documented release operation is CloseHandle.
            unsafe {
                let _ = CloseHandle(self.0);
            }
            self.0 = 0;
        }
    }
}
