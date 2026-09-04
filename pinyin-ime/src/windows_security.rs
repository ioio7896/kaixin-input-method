//! Small Windows process-hardening helpers shared by installed executables.

use std::sync::OnceLock;

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= (left.get(index).copied().unwrap_or(0)
            ^ right.get(index).copied().unwrap_or(0)) as usize;
    }
    difference == 0
}

#[cfg(windows)]
pub fn generate_capability_token() -> std::io::Result<String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };
    let mut bytes = [0u8; 32];
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status));
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
pub struct LocalLogonPipeSecurity {
    attrs: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
    descriptor: *mut std::ffi::c_void,
}

#[cfg(windows)]
impl LocalLogonPipeSecurity {
    pub fn new() -> std::io::Result<Self> {
        use crate::win_handle::OwnedWinHandle;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenLogonSid, SECURITY_ATTRIBUTES, TOKEN_GROUPS, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut raw_token = 0;
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let token = unsafe { OwnedWinHandle::from_raw(raw_token) }?;
        let mut needed = 0;
        unsafe {
            GetTokenInformation(
                token.as_raw(),
                TokenLogonSid,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if needed == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; needed as usize];
        if unsafe {
            GetTokenInformation(
                token.as_raw(),
                TokenLogonSid,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let groups = unsafe { &*buffer.as_ptr().cast::<TOKEN_GROUPS>() };
        if groups.GroupCount == 0 {
            return Err(std::io::Error::other("current token has no logon SID"));
        }
        let mut sid_text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(groups.Groups[0].Sid, &mut sid_text) } == 0
            || sid_text.is_null()
        {
            return Err(std::io::Error::last_os_error());
        }
        let sid = unsafe {
            let mut len = 0;
            while *sid_text.add(len) != 0 {
                len += 1;
            }
            let value = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, len));
            LocalFree(sid_text.cast());
            value
        };
        let sddl: Vec<u16> =
            format!("D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})S:(ML;;NW;;;ME)")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
            || descriptor.is_null()
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
            descriptor,
        })
    }

    pub fn as_ptr(&self) -> *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        &self.attrs
    }
}

#[cfg(windows)]
impl Drop for LocalLogonPipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe { windows_sys::Win32::Foundation::LocalFree(self.descriptor) };
        }
    }
}

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
