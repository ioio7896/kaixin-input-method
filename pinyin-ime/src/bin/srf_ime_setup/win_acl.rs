#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSidToSidW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
        SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
        TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CONTAINER_INHERIT_ACE,
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
        GetSecurityDescriptorControl, INHERIT_ONLY_ACE, NO_PROPAGATE_INHERIT_ACE,
        OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT, UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};

    type PsId = *mut c_void;
    const ACCESS_ALLOWED_ACE_TYPE_U8: u8 = 0;

    const APP_PACKAGE_SIDS: &[&str] = &["S-1-15-2-1", "S-1-15-2-2"];

    fn to_wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn str_to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn dacl_grants_rx(dacl: *const ACL, target_sid: PsId) -> bool {
        if dacl.is_null() {
            return false;
        }
        let mut info = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        let ok = GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        );
        if ok == 0 {
            return false;
        }
        let required_rights = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
        let required_inherit = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8;
        let forbidden_flags = (NO_PROPAGATE_INHERIT_ACE | INHERIT_ONLY_ACE) as u8;

        for idx in 0..info.AceCount {
            let mut ace_ptr = null_mut();
            if GetAce(dacl, idx, &mut ace_ptr) == 0 || ace_ptr.is_null() {
                continue;
            }
            let ace = &*(ace_ptr as *const ACCESS_ALLOWED_ACE);
            if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE_U8 {
                continue;
            }
            let flags = ace.Header.AceFlags;
            if (flags & required_inherit) != required_inherit || (flags & forbidden_flags) != 0 {
                continue;
            }
            if (ace.Mask & required_rights) != required_rights {
                continue;
            }
            let ace_sid = &ace.SidStart as *const u32 as PsId;
            if EqualSid(ace_sid, target_sid) != 0 {
                return true;
            }
        }
        false
    }

    pub fn set_install_directory_access(path: &Path, machine: bool) -> Result<(), String> {
        let mut wide_path = to_wide(path.as_os_str());
        let mut dacl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();

        unsafe {
            let err = GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut sd,
            );
            if err != ERROR_SUCCESS {
                return Err(format!(
                    "GetNamedSecurityInfoW({}) failed: win32={err}",
                    path.display()
                ));
            }

            let result = (|| -> Result<(), String> {
                let mut control: u16 = 0;
                let mut revision: u32 = 0;
                let mut unprotect = false;
                if GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0
                    && (control & SE_DACL_PROTECTED) != 0
                {
                    unprotect = true;
                }

                let mut sids_to_free: Vec<PsId> = Vec::new();
                let mut explicit_entries: Vec<EXPLICIT_ACCESS_W> = Vec::new();

                if !machine {
                    for sid_str in APP_PACKAGE_SIDS {
                        let wide_sid = str_to_wide(sid_str);
                        let mut sid: PsId = null_mut();
                        if ConvertStringSidToSidW(wide_sid.as_ptr(), &mut sid) == 0 || sid.is_null()
                        {
                            for s in &sids_to_free {
                                LocalFree(*s as HLOCAL);
                            }
                            return Err(format!("ConvertStringSidToSidW({sid_str}) failed"));
                        }
                        if dacl_grants_rx(dacl, sid) {
                            LocalFree(sid as HLOCAL);
                        } else {
                            sids_to_free.push(sid);
                            explicit_entries.push(EXPLICIT_ACCESS_W {
                                grfAccessPermissions: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                                grfAccessMode: GRANT_ACCESS,
                                grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
                                Trustee: TRUSTEE_W {
                                    pMultipleTrustee: null_mut(),
                                    MultipleTrusteeOperation: 0,
                                    TrusteeForm: TRUSTEE_IS_SID,
                                    TrusteeType: TRUSTEE_IS_UNKNOWN,
                                    ptstrName: sid as *mut u16,
                                },
                            });
                        }
                    }
                }

                if !unprotect && explicit_entries.is_empty() {
                    return Ok(());
                }

                let mut new_dacl: *mut ACL = null_mut();
                let target_dacl = if explicit_entries.is_empty() {
                    dacl
                } else {
                    let merge_err = SetEntriesInAclW(
                        explicit_entries.len() as u32,
                        explicit_entries.as_ptr(),
                        dacl,
                        &mut new_dacl,
                    );
                    if merge_err != ERROR_SUCCESS {
                        for s in &sids_to_free {
                            LocalFree(*s as HLOCAL);
                        }
                        return Err(format!(
                            "SetEntriesInAclW({}) failed: win32={merge_err}",
                            path.display()
                        ));
                    }
                    new_dacl
                };

                let mut info_flags = DACL_SECURITY_INFORMATION;
                if unprotect {
                    info_flags |= UNPROTECTED_DACL_SECURITY_INFORMATION;
                }
                let set_err = SetNamedSecurityInfoW(
                    wide_path.as_mut_ptr(),
                    SE_FILE_OBJECT,
                    info_flags,
                    null_mut(),
                    null_mut(),
                    target_dacl,
                    null_mut(),
                );

                if !new_dacl.is_null() {
                    LocalFree(new_dacl as HLOCAL);
                }
                for s in &sids_to_free {
                    LocalFree(*s as HLOCAL);
                }

                if set_err != ERROR_SUCCESS {
                    return Err(format!(
                        "SetNamedSecurityInfoW({}) failed: win32={set_err}",
                        path.display()
                    ));
                }
                Ok(())
            })();

            if !sd.is_null() {
                LocalFree(sd as HLOCAL);
            }
            result
        }
    }
}

#[cfg(windows)]
pub use imp::set_install_directory_access;

#[cfg(not(windows))]
pub fn set_install_directory_access(_path: &std::path::Path, _machine: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR,
        PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
    };

    fn is_protected(path: &std::path::Path) -> bool {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        unsafe {
            let err = GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            );
            assert_eq!(err, ERROR_SUCCESS);
            let mut control = 0u16;
            let mut rev = 0u32;
            let ok = GetSecurityDescriptorControl(sd, &mut control, &mut rev);
            LocalFree(sd as HLOCAL);
            ok != 0 && (control & SE_DACL_PROTECTED) != 0
        }
    }

    #[test]
    fn restores_inheritance_and_grants_app_package_rx_idempotently() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kaixin-acl-rust-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let child = dir.join("child.dll");
        fs::write(&child, b"x").unwrap();

        // Simulate legacy installer protecting DACL.
        let mut wide: Vec<u16> = dir
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut dacl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        unsafe {
            assert_eq!(
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    &mut dacl,
                    null_mut(),
                    &mut sd
                ),
                ERROR_SUCCESS
            );
            assert_eq!(
                SetNamedSecurityInfoW(
                    wide.as_mut_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    dacl,
                    null_mut()
                ),
                ERROR_SUCCESS
            );
            LocalFree(sd as HLOCAL);
        }
        assert!(is_protected(&dir));

        set_install_directory_access(&dir, true).unwrap();
        assert!(!is_protected(&dir));

        // User-scope adds S-1-15-2-1 / S-1-15-2-2 and is idempotent on repeat.
        set_install_directory_access(&dir, false).unwrap();
        set_install_directory_access(&dir, false).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }
}
