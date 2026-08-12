//! Small Windows process-hardening helpers shared by installed executables.

use std::sync::OnceLock;

static PROCESS_HARDENING: OnceLock<()> = OnceLock::new();

pub fn apply_process_hardening() {
    PROCESS_HARDENING.get_or_init(|| {
        #[cfg(windows)]
        {
            let _ = apply_windows_process_hardening();
        }
    });
}

pub fn dpapi_protect_with_magic(magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    if cfg!(windows) {
        dpapi_protect_platform(magic, data)
    } else {
        Ok(data.to_vec())
    }
}

pub fn dpapi_unprotect_with_magic(magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    if cfg!(windows) && data.starts_with(magic) {
        dpapi_unprotect_platform(magic, data)
    } else {
        Ok(data.to_vec())
    }
}

pub fn dpapi_blob_has_magic(magic: &[u8], data: &[u8]) -> bool {
    data.starts_with(magic)
}

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(windows)]
fn dpapi_protect_platform(magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let protected = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    let mut out = Vec::with_capacity(magic.len() + protected.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(&protected);
    Ok(out)
}

#[cfg(not(windows))]
fn dpapi_protect_platform(_magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(windows)]
fn dpapi_unprotect_platform(magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let payload = data.strip_prefix(magic).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing DPAPI header")
    })?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: payload.len() as u32,
        pbData: payload.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let unprotected = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        zeroize_bytes(slice);
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    Ok(unprotected)
}

#[cfg(not(windows))]
fn dpapi_unprotect_platform(_magic: &[u8], data: &[u8]) -> std::io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(windows)]
fn apply_windows_process_hardening() -> std::io::Result<()> {
    use std::io;
    use windows_sys::Win32::System::LibraryLoader::{
        SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    };
    use windows_sys::Win32::System::Threading::{
        ProcessExtensionPointDisablePolicy, ProcessImageLoadPolicy, ProcessStrictHandleCheckPolicy,
        SetProcessMitigationPolicy,
    };

    let ok = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    // These policy structures are one DWORD bitfield in the Windows ABI.
    // Apply them before GUI/runtime initialization loads optional components.
    let strict_handles: u32 = 0x1 | 0x2; // raise + permanently enable
    let disable_extensions: u32 = 0x1;
    let image_load: u32 = 0x1 | 0x2 | 0x4; // no remote, no low-IL, prefer System32
    unsafe {
        let _ = SetProcessMitigationPolicy(
            ProcessStrictHandleCheckPolicy,
            (&strict_handles as *const u32).cast(),
            std::mem::size_of_val(&strict_handles),
        );
        let _ = SetProcessMitigationPolicy(
            ProcessExtensionPointDisablePolicy,
            (&disable_extensions as *const u32).cast(),
            std::mem::size_of_val(&disable_extensions),
        );
        let _ = SetProcessMitigationPolicy(
            ProcessImageLoadPolicy,
            (&image_load as *const u32).cast(),
            std::mem::size_of_val(&image_load),
        );
    }
    Ok(())
}
