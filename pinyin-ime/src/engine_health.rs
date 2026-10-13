//! Versioned, freshness-checked engine health published by the TSF bridge.

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
            3 => "异常",
            _ => "未知",
        }
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
    let value_name: Vec<u16> = "EngineHealthV1"
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

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner_pid) };
    if process == 0 {
        return EngineHealthSnapshot::UNKNOWN;
    }
    let mut image = [0u16; 2];
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
            "异常"
        );
    }
}
