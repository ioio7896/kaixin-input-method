//! Versioned, freshness-checked engine health published by the TSF bridge.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineFailureReason {
    VersionMismatch,
    StartupFailed,
    ConnectionTimeout,
    PermissionDenied,
    Other,
}

impl EngineFailureReason {
    pub fn from_code(code: Option<u32>) -> Self {
        match code {
            Some(0xE001_0002) => Self::VersionMismatch,
            Some(2 | 3 | 126 | 127 | 193 | 216 | 0xE001_0003) => Self::StartupFailed,
            Some(121 | 258 | 1460 | 0xE001_0004) => Self::ConnectionTimeout,
            Some(5 | 1314 | 0x8007_0005 | 0xE001_0005) => Self::PermissionDenied,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::VersionMismatch => "组件版本不一致",
            Self::StartupFailed => "引擎启动失败",
            Self::ConnectionTimeout => "引擎连接超时",
            Self::PermissionDenied => "引擎访问权限不足",
            Self::Other => "引擎异常",
        }
    }

    pub fn recovery(self) -> &'static str {
        match self {
            Self::VersionMismatch => "请用同一版本完整安装包修复安装，然后重启正在输入的应用。",
            Self::StartupFailed => "请先重启输入引擎；若仍失败，用完整安装包修复缺失的组件。",
            Self::ConnectionTimeout => "请重试或从托盘重启输入引擎；若反复超时，导出诊断日志。",
            Self::PermissionDenied => "请检查安全软件拦截记录和安装目录权限，再重试或修复安装。",
            Self::Other => "请先重启输入引擎；若仍异常，导出诊断日志。",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineHealthSnapshot {
    /// 0=idle, 1=loading, 2=ready, 3=failed, 4=unknown.
    pub state: u32,
    pub owner_pid: Option<u32>,
    pub error_code: Option<u32>,
}

impl EngineHealthSnapshot {
    pub const UNKNOWN: Self = Self {
        state: 4,
        owner_pid: None,
        error_code: None,
    };

    pub fn label(self) -> &'static str {
        match self.state {
            0 => "未启动",
            1 => "恢复中",
            2 => "正常",
            3 => self.failure_reason().label(),
            _ => "未知",
        }
    }

    pub fn failure_reason(self) -> EngineFailureReason {
        EngineFailureReason::from_code(self.error_code)
    }

    pub fn diagnostic_text(self) -> String {
        if self.state != 3 {
            return self.label().to_string();
        }
        let code = self
            .error_code
            .map(|code| format!("（故障码 0x{code:08X}）"))
            .unwrap_or_default();
        format!(
            "{}{code}；{}",
            self.label(),
            self.failure_reason().recovery()
        )
    }
}

#[cfg(any(windows, test))]
const RECORD_BYTES: usize = 24;
#[cfg(any(windows, test))]
const RECORD_MAGIC: u32 = 0x4845_584B;
#[cfg(any(windows, test))]
const RECORD_VERSION: u16 = 1;
#[cfg(windows)]
const MAX_AGE_MS: u64 = 10_000;

#[cfg(any(windows, test))]
fn decode_record(bytes: &[u8]) -> Option<(u32, u32, u64, u32)> {
    if bytes.len() != RECORD_BYTES
        || u32::from_le_bytes(bytes[0..4].try_into().ok()?) != RECORD_MAGIC
        || u16::from_le_bytes(bytes[4..6].try_into().ok()?) != RECORD_VERSION
    {
        return None;
    }
    Some((
        bytes[6] as u32,
        u32::from_le_bytes(bytes[8..12].try_into().ok()?),
        u64::from_le_bytes(bytes[12..20].try_into().ok()?),
        u32::from_le_bytes(bytes[20..24].try_into().ok()?),
    ))
}

/// Publish a terminal listener error before exit. Failed records remain readable
/// for the normal freshness window even after their owner has exited.
#[cfg(windows)]
pub(crate) fn publish_listener_failure(error: u32) {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_BINARY,
        REG_OPTION_NON_VOLATILE,
    };
    let subkey: Vec<u16> = r"Software\kaixin\State"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = crate::shared_rules::engine_health_value_name()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut bytes = [0u8; RECORD_BYTES];
    bytes[0..4].copy_from_slice(&RECORD_MAGIC.to_le_bytes());
    bytes[4..6].copy_from_slice(&RECORD_VERSION.to_le_bytes());
    bytes[6] = 3;
    bytes[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    bytes[12..20].copy_from_slice(
        &unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }.to_le_bytes(),
    );
    bytes[20..24].copy_from_slice(&error.to_le_bytes());
    unsafe {
        let mut key = 0;
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        ) == 0
        {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_BINARY,
                bytes.as_ptr(),
                bytes.len() as u32,
            );
            RegCloseKey(key);
        }
    }
}

#[cfg(windows)]
pub fn current() -> EngineHealthSnapshot {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY,
    };
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let subkey: Vec<u16> = r"Software\kaixin\State"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_name: Vec<u16> = crate::shared_rules::engine_health_value_name()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut bytes = [0u8; RECORD_BYTES];
    let mut byte_count = size_of::<[u8; RECORD_BYTES]>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_BINARY,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut byte_count,
        )
    };
    if rc != 0 || byte_count as usize != RECORD_BYTES {
        return EngineHealthSnapshot::UNKNOWN;
    }
    let Some((state, owner_pid, updated_tick, error_code)) = decode_record(&bytes) else {
        return EngineHealthSnapshot::UNKNOWN;
    };
    let now = unsafe { GetTickCount64() };
    if owner_pid == 0 || updated_tick > now || now.saturating_sub(updated_tick) > MAX_AGE_MS {
        return EngineHealthSnapshot::UNKNOWN;
    }

    if state == 3 {
        return EngineHealthSnapshot {
            state,
            owner_pid: Some(owner_pid),
            error_code: (error_code != 0).then_some(error_code),
        };
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner_pid) };
    if process == 0 {
        return EngineHealthSnapshot::UNKNOWN;
    }
    let mut image = [0u16; 32768];
    let mut image_len = image.len() as u32;
    let owner_alive = unsafe {
        QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut image_len) != 0
            && image_len > 0
    };
    unsafe { CloseHandle(process) };
    if !owner_alive {
        return EngineHealthSnapshot::UNKNOWN;
    }

    EngineHealthSnapshot {
        state,
        owner_pid: Some(owner_pid),
        error_code: (error_code != 0).then_some(error_code),
    }
}

#[cfg(not(windows))]
pub fn current() -> EngineHealthSnapshot {
    EngineHealthSnapshot::UNKNOWN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_versioned_health_record_and_rejects_bad_data() {
        let mut bytes = vec![0u8; RECORD_BYTES];
        bytes[0..4].copy_from_slice(&RECORD_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&RECORD_VERSION.to_le_bytes());
        bytes[6] = 3;
        bytes[8..12].copy_from_slice(&42u32.to_le_bytes());
        bytes[12..20].copy_from_slice(&1234u64.to_le_bytes());
        bytes[20..24].copy_from_slice(&2u32.to_le_bytes());

        assert_eq!(decode_record(&bytes), Some((3, 42, 1234, 2)));
        bytes[4] = 2;
        assert_eq!(decode_record(&bytes), None);
        assert_eq!(decode_record(&bytes[..RECORD_BYTES - 1]), None);
    }

    #[test]
    fn labels_are_safe_for_unknown_state_values() {
        assert_eq!(EngineHealthSnapshot::UNKNOWN.label(), "未知");
        assert_eq!(
            EngineHealthSnapshot {
                state: 3,
                owner_pid: Some(42),
                error_code: Some(2)
            }
            .label(),
            "引擎启动失败"
        );
    }

    #[test]
    fn actionable_failure_reason_preserves_win32_and_protocol_causes() {
        for (code, reason) in [
            (0xE001_0002, EngineFailureReason::VersionMismatch),
            (2, EngineFailureReason::StartupFailed),
            (193, EngineFailureReason::StartupFailed),
            (1460, EngineFailureReason::ConnectionTimeout),
            (5, EngineFailureReason::PermissionDenied),
        ] {
            let health = EngineHealthSnapshot {
                state: 3,
                owner_pid: None,
                error_code: Some(code),
            };
            assert_eq!(health.failure_reason(), reason);
            assert!(health.diagnostic_text().contains(reason.recovery()));
        }
        assert_eq!(
            EngineFailureReason::from_code(Some(9999)),
            EngineFailureReason::Other
        );
    }
}
