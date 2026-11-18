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
        Self::with_integrity(false)
    }

    /// Only the authenticated IME engine accepts low-integrity desktop TIPs.
    /// Do not apply this policy to clipboard/translation/control pipes.
    pub fn for_ime_engine() -> std::io::Result<Self> {
        Self::with_integrity(true)
    }

    fn with_integrity(low_integrity: bool) -> std::io::Result<Self> {
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
        let sddl_text = if low_integrity {
            // No Everyone / ALL APPLICATION PACKAGES grant: AppContainer
            // boundaries must not be bypassed. The server also checks the
            // client's logon identity and existing per-request capability.
            format!("D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GRGW;;;{sid})S:(ML;;NW;;;LW)")
        } else {
            format!("D:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})S:(ML;;NW;;;ME)")
        };
        let sddl: Vec<u16> = sddl_text.encode_utf16().chain(std::iter::once(0)).collect();
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

/// Verify the security context of the last message read from the engine pipe.
/// Identification-level impersonation is sufficient; never perform I/O while
/// impersonating. Failure is closed, and reverting is mandatory on every path.
#[cfg(windows)]
pub fn verify_ime_pipe_client(pipe: windows_sys::Win32::Foundation::HANDLE) -> std::io::Result<()> {
    use crate::win_handle::OwnedWinHandle;
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, RevertToSelf, TokenIsAppContainer, TokenLogonSid,
        TokenSessionId, TOKEN_GROUPS, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
    };
    fn information(
        token: isize,
        class: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
    ) -> std::io::Result<Vec<usize>> {
        let mut needed = 0;
        unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                class,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(buffer)
    }
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            if unsafe { RevertToSelf() } == 0 {
                // Continuing under an untrusted token is not safe.
                std::process::abort();
            }
        }
    }
    let mut raw = 0;
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let server = unsafe { OwnedWinHandle::from_raw(raw) }?;
    let server_sid = information(server.as_raw(), TokenLogonSid)?;
    let server_session = information(server.as_raw(), TokenSessionId)?;
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let _revert = Revert;
    raw = 0;
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let client = unsafe { OwnedWinHandle::from_raw(raw) }?;
    let client_sid = information(client.as_raw(), TokenLogonSid)?;
    let client_session = information(client.as_raw(), TokenSessionId)?;
    let app_container = information(client.as_raw(), TokenIsAppContainer)?;
    let server_groups = unsafe { &*server_sid.as_ptr().cast::<TOKEN_GROUPS>() };
    let client_groups = unsafe { &*client_sid.as_ptr().cast::<TOKEN_GROUPS>() };
    let same_logon = server_groups.GroupCount != 0
        && client_groups.GroupCount != 0
        && unsafe { EqualSid(server_groups.Groups[0].Sid, client_groups.Groups[0].Sid) != 0 };
    // These information classes return a DWORD (not a pointer-sized value).
    let session = |value: &[usize]| unsafe { *value.as_ptr().cast::<u32>() };
    if !same_logon
        || session(&server_session) != session(&client_session)
        || session(&app_container) != 0
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "engine client identity denied",
        ));
    }
    Ok(())
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

#[cfg(all(test, windows))]
mod pipe_tests {
    use super::*;
    use crate::win_handle::OwnedWinHandle;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        GetLastError, LocalFree, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        DuplicateTokenEx, RevertToSelf, SecurityImpersonation, SetTokenInformation,
        TokenImpersonation, TokenIntegrityLevel, TOKEN_ALL_ACCESS, TOKEN_DUPLICATE,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
        SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcessToken, SetThreadToken,
    };

    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            assert_ne!(unsafe { RevertToSelf() }, 0);
        }
    }
    fn low_token() -> OwnedWinHandle {
        let mut original = 0;
        assert_ne!(
            unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_QUERY | TOKEN_DUPLICATE,
                    &mut original,
                )
            },
            0
        );
        let original = unsafe { OwnedWinHandle::from_raw(original) }.unwrap();
        let mut duplicate = 0;
        assert_ne!(
            unsafe {
                DuplicateTokenEx(
                    original.as_raw(),
                    TOKEN_ALL_ACCESS,
                    null(),
                    SecurityImpersonation,
                    TokenImpersonation,
                    &mut duplicate,
                )
            },
            0
        );
        let duplicate = unsafe { OwnedWinHandle::from_raw(duplicate) }.unwrap();
        let sid_text: Vec<u16> = "S-1-16-4096\0".encode_utf16().collect();
        let mut sid = null_mut();
        assert_ne!(
            unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &mut sid) },
            0
        );
        let label = TOKEN_MANDATORY_LABEL {
            Label: windows_sys::Win32::Security::SID_AND_ATTRIBUTES {
                Sid: sid,
                Attributes: 0x20, // SE_GROUP_INTEGRITY
            },
        };
        let result = unsafe {
            SetTokenInformation(
                duplicate.as_raw(),
                TokenIntegrityLevel,
                (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                std::mem::size_of_val(&label) as u32,
            )
        };
        unsafe { LocalFree(sid) };
        assert_ne!(result, 0);
        duplicate
    }
    #[test]
    fn low_integrity_engine_transport_keeps_other_pipes_protected() {
        let token = low_token();
        for (number, engine) in [false, true].into_iter().enumerate() {
            let security = if engine {
                LocalLogonPipeSecurity::for_ime_engine()
            } else {
                LocalLogonPipeSecurity::new()
            }
            .unwrap();
            let name: Vec<u16> = format!(
                "\\\\.\\pipe\\KaixinLowIntegrityTest-{}-{number}\0",
                unsafe { GetCurrentProcessId() }
            )
            .encode_utf16()
            .collect();
            let server = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    1024,
                    1024,
                    0,
                    security.as_ptr(),
                )
            };
            assert_ne!(server, INVALID_HANDLE_VALUE);
            let server = unsafe { OwnedWinHandle::from_raw(server) }.unwrap();
            assert_ne!(unsafe { SetThreadToken(null(), token.as_raw()) }, 0);
            let revert = Revert;
            let raw = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    0,
                )
            };
            let error = unsafe { GetLastError() };
            if !engine {
                drop(revert);
                assert_eq!(
                    raw, INVALID_HANDLE_VALUE,
                    "ordinary control pipe must reject low-integrity clients"
                );
                assert_eq!(error, 5, "MIC denies write-up");
                continue;
            }
            assert_ne!(
                raw, INVALID_HANDLE_VALUE,
                "low-integrity TIP must connect, error={error}"
            );
            let client = unsafe { OwnedWinHandle::from_raw(raw) }.unwrap();
            let mut count = 0;
            assert_ne!(
                unsafe { WriteFile(client.as_raw(), b"x".as_ptr(), 1, &mut count, null_mut()) },
                0
            );
            drop(revert);
            let connected = unsafe { ConnectNamedPipe(server.as_raw(), null_mut()) };
            assert!(connected != 0 || unsafe { GetLastError() } == 535);
            let mut byte = 0u8;
            assert_ne!(
                unsafe { ReadFile(server.as_raw(), &mut byte, 1, &mut count, null_mut()) },
                0
            );
            assert_eq!(byte, b'x');
            verify_ime_pipe_client(server.as_raw())
                .expect("same logon low-integrity identity accepted");
        }
    }
    #[test]
    fn capability_comparison_remains_required_and_exact() {
        let capability = generate_capability_token().unwrap();
        assert!(constant_time_eq(&capability, &capability));
        assert!(!constant_time_eq(&capability, ""));
        assert!(!constant_time_eq(&capability, &format!("{capability}0")));
        let mut wrong = capability.clone().into_bytes();
        wrong[0] = if wrong[0] == b'0' { b'1' } else { b'0' };
        assert!(!constant_time_eq(
            &capability,
            std::str::from_utf8(&wrong).unwrap()
        ));
    }
}
