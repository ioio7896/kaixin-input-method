use crate::manifest::{normalize_lexical, path_within_root, RUNTIME_PAYLOAD_ROOT_NAME};
use chrono::Local;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, FALSE, FreeLibrary, HANDLE, HLOCAL, HWND,
    LPARAM, LocalFree, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    AllocateAndInitializeSid, CheckTokenMembership, FreeSid, GetTokenInformation,
    SECURITY_NT_AUTHORITY, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_REPARSE_POINT, GetFileAttributesW,
    MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW, SetFileAttributesW,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::ProcessStatus::EnumProcesses;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ENUMERATE_SUB_KEYS,
    KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_32KEY, KEY_WOW64_64KEY, KEY_WRITE,
    REG_DWORD, REG_MULTI_SZ, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW,
    RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess, WaitForMultipleObjects,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, HWND_BROADCAST, PostMessageW, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_CLOSE, WM_SETTINGCHANGE,
};

#[link(name = "kernel32")]
extern "system" {
    fn ProcessIdToSessionId(dwProcessId: u32, pSessionId: *mut u32) -> i32;
}

pub const TEXT_SERVICE_CLSID: &str = "{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}";
pub const PROFILE_GUID: &str = "{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}";
pub const SIMPLIFIED_CHINESE_TIP: &str =
    "0804:{E5A91C40-7B2D-4F8A-9C11-8F3E6D2A1B00}{A3F0B2C1-4D5E-6789-ABCD-EF0123456789}";
pub const MICROSOFT_PINYIN_TIP: &str =
    "0804:{81D4E9C9-1D3B-41BC-9E6C-4B40BF79E35E}{FA550B04-5AD7-411F-A5AC-CA038EC515D7}";
pub const APP_DISPLAY_NAME: &str = "开心输入法";
pub const APP_PATH_NAME: &str = "kaixin";
pub const TRAY_RUN_ENTRY_NAME: &str = "开心输入法";
pub const LEGACY_TRAY_RUN_ENTRY_NAME: &str = "开心输入法托盘";
pub const ENGINE_RUN_ENTRY_NAME: &str = "开心输入法引擎";

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ILOT_UNINSTALL: u32 = 0x0000_0001;
const SECURITY_BUILTIN_DOMAIN_RID: u32 = 0x0000_0020;
const DOMAIN_ALIAS_RID_ADMINS: u32 = 0x0000_0220;

pub const HELPER_PROCESS_NAMES: &[&str] = &[
    "srf_ime_engine.exe",
    "srf_ime_desktop.exe",
    "srf_ime_tray.exe",
    "srf_ime_clipboard_svc.exe",
    "srf_ime_overlay.exe",
    "srf_ime_settings.exe",
    "srf_ime_clipboard.exe",
    "srf_ime_handwrite.exe",
    "srf_ime_symbols.exe",
    "srf_ime_keyboard.exe",
    "srf_ime_ocr.exe",
    "srf_ime_translate.exe",
    "srf_ime_translate_result.exe",
    "KaixinShareX.exe",
];

#[repr(C)]
#[derive(Clone, Copy)]
struct LayoutOrTipProfile {
    dw_profile_type: u32,
    langid: u16,
    clsid: [u8; 16],
    guid_profile: [u8; 16],
    catid: [u8; 16],
    dw_substitute_layout: u32,
    dw_flags: u32,
    sz_id: [u16; 260],
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_to_wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn from_wide_nul(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn user_state_root() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATA environment variable is not set".to_string())?;
    Ok(normalize_lexical(&PathBuf::from(local).join(APP_PATH_NAME)))
}

pub fn machine_install_root() -> PathBuf {
    let pf86 = std::env::var_os("ProgramFiles(x86)")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .unwrap_or_else(|| r"C:\Program Files (x86)".into());
    normalize_lexical(&PathBuf::from(pf86).join(APP_PATH_NAME))
}

pub fn default_user_install_root() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATA environment variable is not set".to_string())?;
    Ok(normalize_lexical(
        &PathBuf::from(local).join("Programs").join(APP_PATH_NAME),
    ))
}

pub fn is_machine_install_root(path: &Path) -> bool {
    let target = normalize_lexical(path);
    let machine = machine_install_root();
    target
        .to_string_lossy()
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(machine.to_string_lossy().trim_end_matches('\\'))
}

pub fn assert_trusted_install_root(path: &Path) -> Result<(), String> {
    let normalized = normalize_lexical(path);
    let norm_str = normalized
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_string();
    let machine = machine_install_root();
    if norm_str.eq_ignore_ascii_case(machine.to_string_lossy().trim_end_matches('\\')) {
        return Ok(());
    }
    if let Ok(user_root) = default_user_install_root() {
        if norm_str.eq_ignore_ascii_case(user_root.to_string_lossy().trim_end_matches('\\')) {
            return Ok(());
        }
    }
    Err(format!("Refusing to install to untrusted path: {norm_str}"))
}

pub fn current_session_id() -> u32 {
    unsafe {
        let mut sid = 0u32;
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut sid) != 0 {
            sid
        } else {
            0
        }
    }
}

fn process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if handle == 0 {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 || len == 0 {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&full)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }
}

pub fn find_session_pids(target_names: &[&str]) -> Vec<u32> {
    let my_pid = unsafe { GetCurrentProcessId() };
    let my_session = current_session_id();
    let mut pids = vec![0u32; 4096];
    let mut bytes_returned = 0u32;
    unsafe {
        if EnumProcesses(
            pids.as_mut_ptr(),
            (pids.len() * std::mem::size_of::<u32>()) as u32,
            &mut bytes_returned,
        ) == 0
        {
            return Vec::new();
        }
    }
    let count = bytes_returned as usize / std::mem::size_of::<u32>();
    let mut matched = Vec::new();
    for &pid in &pids[..count] {
        if pid == 0 || pid == my_pid {
            continue;
        }
        let mut proc_sid = u32::MAX;
        unsafe {
            if ProcessIdToSessionId(pid, &mut proc_sid) == 0 || proc_sid != my_session {
                continue;
            }
        }
        if let Some(name) = process_image_name(pid) {
            if target_names.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
                matched.push(pid);
            }
        }
    }
    matched
}

struct CloseWindowsCtx {
    pids: Vec<u32>,
}

unsafe extern "system" fn enum_close_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let ctx = &*(lparam as *const CloseWindowsCtx);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if ctx.pids.contains(&pid) {
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    TRUE
}

fn wait_handles_bounded(handles: &[HANDLE], timeout_ms: u32) {
    if handles.is_empty() {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64);
    let mut remaining: Vec<HANDLE> = handles.to_vec();
    while !remaining.is_empty() {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let chunk_len = remaining.len().min(64);
        let wait_ms = (deadline - now).as_millis().min(u32::MAX as u128) as u32;
        let res = unsafe {
            WaitForMultipleObjects(chunk_len as u32, remaining.as_ptr(), FALSE, wait_ms)
        };
        if res < chunk_len as u32 {
            remaining.remove(res as usize);
        } else {
            break;
        }
    }
}

pub fn stop_session_processes(target_names: &[&str], graceful_ms: u32) -> usize {
    let pids = find_session_pids(target_names);
    if pids.is_empty() {
        return 0;
    }

    let ctx = CloseWindowsCtx { pids: pids.clone() };
    if graceful_ms > 0 {
        unsafe {
            EnumWindows(Some(enum_close_window), &ctx as *const _ as LPARAM);
        }
    }

    let mut handles: Vec<(u32, HANDLE)> = Vec::new();
    for &pid in &pids {
        let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, FALSE, pid) };
        if h != 0 {
            handles.push((pid, h));
        }
    }

    if graceful_ms > 0 && !handles.is_empty() {
        let raw_handles: Vec<HANDLE> = handles.iter().map(|(_, h)| *h).collect();
        wait_handles_bounded(&raw_handles, graceful_ms);
    }

    let mut still_running = Vec::new();
    for &(_, h) in &handles {
        let waited = unsafe { WaitForMultipleObjects(1, &h, FALSE, 0) };
        if waited != WAIT_OBJECT_0 {
            unsafe {
                TerminateProcess(h, 1);
            }
            still_running.push(h);
        }
    }

    if !still_running.is_empty() {
        wait_handles_bounded(&still_running, 1000);
    }

    for (_, h) in handles {
        unsafe {
            CloseHandle(h);
        }
    }

    pids.len()
}

pub fn stop_running_helpers() -> usize {
    stop_session_processes(HELPER_PROCESS_NAMES, 600)
}

pub fn ensure_ctfmon_running(log: &mut dyn FnMut(&str)) {
    if !find_session_pids(&["ctfmon.exe"]).is_empty() {
        return;
    }
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let ctfmon = PathBuf::from(windir).join("System32").join("ctfmon.exe");
    if !ctfmon.is_file() {
        log(&format!("WARN: ctfmon.exe missing at {}", ctfmon.display()));
        return;
    }
    match Command::new(&ctfmon)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => log("OK: started ctfmon.exe"),
        Err(e) => log(&format!("WARN: failed to start ctfmon.exe: {e}")),
    }
}

pub fn notify_language_change() {
    let intl = to_wide("intl");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            intl.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            1000,
            null_mut(),
        );
    }
}

pub fn update_text_input_services(restart_text_input_host: bool, log: &mut dyn FnMut(&str)) {
    notify_language_change();
    if restart_text_input_host {
        let _ = stop_session_processes(&["TextInputHost.exe"], 0);
    }
    ensure_ctfmon_running(log);
    log("OK: refreshed desktop input settings");
}

pub fn is_current_process_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len = 0u32;
        let elevated = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        ) != 0
            && elevation.TokenIsElevated != 0;
        CloseHandle(token);
        if elevated {
            return true;
        }

        let nt_authority = SECURITY_NT_AUTHORITY;
        let mut admin_sid = null_mut();
        if AllocateAndInitializeSid(
            &nt_authority,
            2,
            SECURITY_BUILTIN_DOMAIN_RID,
            DOMAIN_ALIAS_RID_ADMINS,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut admin_sid,
        ) == 0
        {
            return false;
        }
        let mut is_member = FALSE;
        let ok = CheckTokenMembership(0, admin_sid, &mut is_member) != 0 && is_member != 0;
        FreeSid(admin_sid);
        ok
    }
}

fn token_user_sid_string(token: HANDLE) -> Option<String> {
    unsafe {
        let mut needed = 0u32;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
        if needed == 0 {
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as *mut _,
            needed,
            &mut needed,
        ) == 0
        {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut wide_sid: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut wide_sid) == 0 || wide_sid.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *wide_sid.add(len) != 0 {
            len += 1;
        }
        let sid_str = String::from_utf16_lossy(std::slice::from_raw_parts(wide_sid, len));
        LocalFree(wide_sid as HLOCAL);
        Some(sid_str)
    }
}

pub fn current_user_sid_string() -> Option<String> {
    unsafe {
        let mut token: HANDLE = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let res = token_user_sid_string(token);
        CloseHandle(token);
        res
    }
}

pub fn find_desktop_user_sid() -> Option<String> {
    let pids = find_session_pids(&["explorer.exe"]);
    for pid in pids {
        unsafe {
            let proc_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
            if proc_handle == 0 {
                continue;
            }
            let mut token: HANDLE = 0;
            let opened = OpenProcessToken(proc_handle, TOKEN_QUERY, &mut token) != 0;
            CloseHandle(proc_handle);
            if !opened {
                continue;
            }
            let sid = token_user_sid_string(token);
            CloseHandle(token);
            if sid.is_some() {
                return sid;
            }
        }
    }
    None
}

fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

pub fn invoke_desktop_user_task(
    task_prefix: &str,
    user_sid: &str,
    exe_path: &Path,
    extra_args: &[String],
) -> Result<i32, String> {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let task_name = format!("{task_prefix}-{unique}");
    let temp_dir = std::env::temp_dir();
    let xml_path = temp_dir.join(format!("{task_name}.xml"));
    let status_path = temp_dir.join(format!("{task_name}.status"));

    let mut arg_parts: Vec<String> = extra_args.to_vec();
    arg_parts.push("--status-file".to_string());
    arg_parts.push(format!("\"{}\"", status_path.display()));
    let joined_args = arg_parts.join(" ");

    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\r\n\
           <Principals>\r\n\
             <Principal id=\"Author\">\r\n\
               <UserId>{sid}</UserId>\r\n\
               <LogonType>InteractiveToken</LogonType>\r\n\
               <RunLevel>LeastPrivilege</RunLevel>\r\n\
             </Principal>\r\n\
           </Principals>\r\n\
           <Settings>\r\n\
             <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\r\n\
             <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\r\n\
             <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\r\n\
             <ExecutionTimeLimit>PT2M</ExecutionTimeLimit>\r\n\
             <Hidden>true</Hidden>\r\n\
             <Enabled>true</Enabled>\r\n\
           </Settings>\r\n\
           <Actions Context=\"Author\">\r\n\
             <Exec>\r\n\
               <Command>{cmd}</Command>\r\n\
               <Arguments>{args}</Arguments>\r\n\
             </Exec>\r\n\
           </Actions>\r\n\
         </Task>\r\n",
        sid = xml_escape(user_sid),
        cmd = xml_escape(&exe_path.display().to_string()),
        args = xml_escape(&joined_args)
    );
    let mut utf16_bytes = vec![0xFFu8, 0xFEu8];
    for unit in xml.encode_utf16() {
        utf16_bytes.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(&xml_path, utf16_bytes)
        .map_err(|e| format!("write task xml {}: {e}", xml_path.display()))?;

    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let schtasks = PathBuf::from(windir).join("System32").join("schtasks.exe");

    let create_out = Command::new(&schtasks)
        .args(["/Create", "/TN", &task_name, "/XML"])
        .arg(&xml_path)
        .arg("/F")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("launch schtasks /Create: {e}"))?;
    let _ = fs::remove_file(&xml_path);
    if !create_out.status.success() {
        return Err(format!(
            "schtasks /Create failed ({}): {}",
            create_out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&create_out.stderr).trim()
        ));
    }

    let run_result = (|| -> Result<i32, String> {
        let start_out = Command::new(&schtasks)
            .args(["/Run", "/TN", &task_name])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("launch schtasks /Run: {e}"))?;
        if !start_out.status.success() {
            return Err(format!(
                "schtasks /Run failed ({}): {}",
                start_out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&start_out.stderr).trim()
            ));
        }

        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if let Ok(text) = fs::read_to_string(&status_path) {
                if let Ok(code) = text.trim().parse::<i32>() {
                    return Ok(code);
                }
            }
            thread::sleep(Duration::from_millis(150));
        }
        Err("Desktop user setup task timed out.".to_string())
    })();

    let _ = Command::new(&schtasks)
        .args(["/Delete", "/TN", &task_name, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = fs::remove_file(&status_path);
    run_result
}

pub enum RegsvrMode {
    Register { machine: bool },
    Unregister { machine: bool },
    EnableProfile,
}

pub fn run_regsvr32(
    arch: &str,
    dll_path: &Path,
    mode: RegsvrMode,
    reg_log_path: Option<&Path>,
) -> Result<(), String> {
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let sys_dir = if arch.eq_ignore_ascii_case("x86") {
        "SysWOW64"
    } else {
        "System32"
    };
    let registrar = PathBuf::from(windir).join(sys_dir).join("regsvr32.exe");
    let mut cmd = Command::new(&registrar);
    cmd.arg("/s");
    match mode {
        RegsvrMode::Register { machine } => {
            if machine {
                cmd.args(["/n", "/i:machine"]);
            }
        }
        RegsvrMode::Unregister { machine } => {
            cmd.arg("/u");
            if machine {
                cmd.args(["/n", "/i:machine"]);
            }
        }
        RegsvrMode::EnableProfile => {
            cmd.args(["/n", "/i:enable-profile"]);
        }
    }
    cmd.arg(dll_path);
    if let Some(log_path) = reg_log_path {
        cmd.env("SRF_TSF_REG_LOG", log_path);
    }
    cmd.creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = cmd
        .status()
        .map_err(|e| format!("spawn {} for {arch}: {e}", registrar.display()))?;
    if !status.success() {
        return Err(format!(
            "{arch} regsvr32 failed (exit {}) for {}",
            status.code().unwrap_or(-1),
            dll_path.display()
        ));
    }
    Ok(())
}

fn reg_read_sz(root: HKEY, subkey: &str, value_name: Option<&str>, extra_flags: u32) -> Option<String> {
    let sub_w = to_wide(subkey);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_QUERY_VALUE | extra_flags, &mut hkey)
            != ERROR_SUCCESS
        {
            return None;
        }
        let val_w = value_name.map(to_wide);
        let val_ptr = val_w.as_ref().map(|v| v.as_ptr()).unwrap_or(null());
        let mut reg_type = 0u32;
        let mut cb = 0u32;
        let r1 = RegQueryValueExW(hkey, val_ptr, null_mut(), &mut reg_type, null_mut(), &mut cb);
        if r1 != ERROR_SUCCESS || (reg_type != REG_SZ && reg_type != 2 /* REG_EXPAND_SZ */) || cb == 0 {
            RegCloseKey(hkey);
            return None;
        }
        let mut buf = vec![0u16; (cb as usize).div_ceil(2)];
        let r2 = RegQueryValueExW(
            hkey,
            val_ptr,
            null_mut(),
            &mut reg_type,
            buf.as_mut_ptr() as *mut u8,
            &mut cb,
        );
        RegCloseKey(hkey);
        if r2 != ERROR_SUCCESS {
            return None;
        }
        Some(from_wide_nul(&buf))
    }
}

fn reg_read_multi_sz(root: HKEY, subkey: &str, value_name: &str) -> Option<Vec<String>> {
    let sub_w = to_wide(subkey);
    let val_w = to_wide(value_name);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) != ERROR_SUCCESS {
            return None;
        }
        let mut reg_type = 0u32;
        let mut cb = 0u32;
        let r1 = RegQueryValueExW(
            hkey,
            val_w.as_ptr(),
            null_mut(),
            &mut reg_type,
            null_mut(),
            &mut cb,
        );
        if r1 != ERROR_SUCCESS || reg_type != REG_MULTI_SZ || cb == 0 {
            RegCloseKey(hkey);
            return None;
        }
        let mut buf = vec![0u16; (cb as usize).div_ceil(2)];
        let r2 = RegQueryValueExW(
            hkey,
            val_w.as_ptr(),
            null_mut(),
            &mut reg_type,
            buf.as_mut_ptr() as *mut u8,
            &mut cb,
        );
        RegCloseKey(hkey);
        if r2 != ERROR_SUCCESS {
            return None;
        }
        let mut items = Vec::new();
        let mut start = 0usize;
        for (i, &c) in buf.iter().enumerate() {
            if c == 0 {
                if i > start {
                    items.push(String::from_utf16_lossy(&buf[start..i]));
                }
                start = i + 1;
            }
        }
        Some(items)
    }
}

fn reg_write_sz(root: HKEY, subkey: &str, value_name: &str, data: &str) -> Result<(), String> {
    let sub_w = to_wide(subkey);
    let val_w = to_wide(value_name);
    let data_w = to_wide(data);
    unsafe {
        let mut hkey: HKEY = 0;
        let rc = RegCreateKeyExW(
            root,
            sub_w.as_ptr(),
            0,
            null(),
            0,
            KEY_WRITE,
            null(),
            &mut hkey,
            null_mut(),
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW({subkey}) failed: win32={rc}"));
        }
        let rc2 = RegSetValueExW(
            hkey,
            val_w.as_ptr(),
            0,
            REG_SZ,
            data_w.as_ptr() as *const u8,
            (data_w.len() * 2) as u32,
        );
        RegCloseKey(hkey);
        if rc2 != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW({subkey}\\{value_name}) failed: win32={rc2}"));
        }
        Ok(())
    }
}

fn reg_write_dword(root: HKEY, subkey: &str, value_name: &str, data: u32) -> Result<(), String> {
    let sub_w = to_wide(subkey);
    let val_w = to_wide(value_name);
    unsafe {
        let mut hkey: HKEY = 0;
        let rc = RegCreateKeyExW(
            root,
            sub_w.as_ptr(),
            0,
            null(),
            0,
            KEY_WRITE,
            null(),
            &mut hkey,
            null_mut(),
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW({subkey}) failed: win32={rc}"));
        }
        let bytes = data.to_le_bytes();
        let rc2 = RegSetValueExW(
            hkey,
            val_w.as_ptr(),
            0,
            REG_DWORD,
            bytes.as_ptr(),
            bytes.len() as u32,
        );
        RegCloseKey(hkey);
        if rc2 != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW({subkey}\\{value_name}) failed: win32={rc2}"));
        }
        Ok(())
    }
}

fn reg_write_multi_sz(root: HKEY, subkey: &str, value_name: &str, items: &[String]) -> Result<(), String> {
    let sub_w = to_wide(subkey);
    let val_w = to_wide(value_name);
    let mut data_w: Vec<u16> = Vec::new();
    for item in items {
        data_w.extend(item.encode_utf16());
        data_w.push(0);
    }
    data_w.push(0);
    unsafe {
        let mut hkey: HKEY = 0;
        let rc = RegCreateKeyExW(
            root,
            sub_w.as_ptr(),
            0,
            null(),
            0,
            KEY_WRITE,
            null(),
            &mut hkey,
            null_mut(),
        );
        if rc != ERROR_SUCCESS {
            return Err(format!("RegCreateKeyExW({subkey}) failed: win32={rc}"));
        }
        let rc2 = RegSetValueExW(
            hkey,
            val_w.as_ptr(),
            0,
            REG_MULTI_SZ,
            data_w.as_ptr() as *const u8,
            (data_w.len() * 2) as u32,
        );
        RegCloseKey(hkey);
        if rc2 != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW({subkey}\\{value_name}) failed: win32={rc2}"));
        }
        Ok(())
    }
}

fn reg_delete_value(root: HKEY, subkey: &str, value_name: &str) {
    let sub_w = to_wide(subkey);
    let val_w = to_wide(value_name);
    unsafe {
        let mut hkey: HKEY = 0;
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_SET_VALUE, &mut hkey) == ERROR_SUCCESS {
            let _ = RegDeleteValueW(hkey, val_w.as_ptr());
            RegCloseKey(hkey);
        }
    }
}

fn reg_key_exists(root: HKEY, subkey: &str, extra_flags: u32) -> bool {
    let sub_w = to_wide(subkey);
    unsafe {
        let mut hkey: HKEY = 0;
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_QUERY_VALUE | extra_flags, &mut hkey)
            == ERROR_SUCCESS
        {
            RegCloseKey(hkey);
            true
        } else {
            false
        }
    }
}

fn reg_delete_tree(root: HKEY, subkey: &str, extra_flags: u32) -> bool {
    let Some((parent, leaf)) = subkey.rsplit_once('\\') else {
        return false;
    };
    let parent_w = to_wide(parent);
    let leaf_w = to_wide(leaf);
    unsafe {
        let mut hkey: HKEY = 0;
        if RegOpenKeyExW(root, parent_w.as_ptr(), 0, KEY_WRITE | extra_flags, &mut hkey)
            != ERROR_SUCCESS
        {
            return false;
        }
        let rc = RegDeleteTreeW(hkey, leaf_w.as_ptr());
        RegCloseKey(hkey);
        rc == ERROR_SUCCESS
    }
}

pub fn remove_user_com_overrides(log: &mut dyn FnMut(&str)) {
    let subkey = format!("Software\\Classes\\CLSID\\{TEXT_SERVICE_CLSID}");
    for (label, flag) in [("x64", KEY_WOW64_64KEY), ("x86", KEY_WOW64_32KEY)] {
        if reg_key_exists(HKEY_CURRENT_USER, &subkey, flag)
            && reg_delete_tree(HKEY_CURRENT_USER, &subkey, flag)
        {
            log(&format!(
                "OK: removed per-user COM override ({label}) for machine install"
            ));
        }
    }
}

pub fn check_com_registration(dll_x64: &Path, dll_x86: &Path, machine_scope: bool) -> String {
    let hive = if machine_scope {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let subkey = format!("Software\\Classes\\CLSID\\{TEXT_SERVICE_CLSID}\\InprocServer32");
    let mut failures = Vec::new();
    for (arch, flag, expected) in [
        ("x64", KEY_WOW64_64KEY, dll_x64),
        ("x86", KEY_WOW64_32KEY, dll_x86),
    ] {
        let Some(actual) = reg_read_sz(hive, &subkey, None, flag) else {
            failures.push(format!("{arch}_missing"));
            continue;
        };
        if actual.trim().is_empty() {
            failures.push(format!("{arch}_missing"));
            continue;
        }
        let actual_norm = normalize_lexical(Path::new(actual.trim()));
        let expected_norm = normalize_lexical(expected);
        if !actual_norm
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected_norm.to_string_lossy())
        {
            failures.push(format!("{arch}_mismatch"));
        }
    }
    if failures.is_empty() {
        "ok".to_string()
    } else {
        failures.join("+")
    }
}

pub fn check_profile_registry(machine_scope: bool) -> String {
    let subkey = format!(
        "Software\\Microsoft\\CTF\\TIP\\{TEXT_SERVICE_CLSID}\\LanguageProfile\\0x00000804\\{PROFILE_GUID}"
    );
    let hives = if machine_scope {
        [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
    } else {
        [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
    };
    for hive in hives {
        if reg_key_exists(hive, &subkey, KEY_WOW64_64KEY) {
            return "ok".to_string();
        }
    }
    "missing".to_string()
}

pub fn write_tray_run_entry(install_root: &Path, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let run_key = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let tray_exe = install_root.join("srf_ime_desktop.exe");
    if tray_exe.is_file() {
        let cmd = format!("\"{}\" --tool=tray", tray_exe.display());
        reg_write_sz(HKEY_CURRENT_USER, run_key, TRAY_RUN_ENTRY_NAME, &cmd)?;
    }
    reg_delete_value(HKEY_CURRENT_USER, run_key, ENGINE_RUN_ENTRY_NAME);
    reg_delete_value(HKEY_CURRENT_USER, run_key, LEGACY_TRAY_RUN_ENTRY_NAME);

    let approved = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    reg_delete_value(HKEY_CURRENT_USER, approved, ENGINE_RUN_ENTRY_NAME);
    reg_delete_value(HKEY_CURRENT_USER, approved, LEGACY_TRAY_RUN_ENTRY_NAME);
    log("OK: wrote single tray autorun entry; removed legacy engine autorun entry");
    Ok(())
}

pub fn remove_tray_run_entries(log: &mut dyn FnMut(&str)) {
    let run_key = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let approved = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    for key in [run_key, approved] {
        reg_delete_value(HKEY_CURRENT_USER, key, TRAY_RUN_ENTRY_NAME);
        reg_delete_value(HKEY_CURRENT_USER, key, LEGACY_TRAY_RUN_ENTRY_NAME);
        reg_delete_value(HKEY_CURRENT_USER, key, ENGINE_RUN_ENTRY_NAME);
    }
    log("OK: removed current user Run entries");
}

pub fn start_tray_helper(install_root: &Path, log: &mut dyn FnMut(&str)) {
    let tray_exe = install_root.join("srf_ime_desktop.exe");
    if !tray_exe.is_file() {
        log("WARN: srf_ime_desktop.exe missing; skipped starting tray helper");
        return;
    }
    match Command::new(&tray_exe)
        .arg("--tool=tray")
        .current_dir(install_root)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => log("OK: started tray helper"),
        Err(e) => log(&format!("WARN: failed to start tray helper: {e}")),
    }
}

fn call_input_dll_install(tip: &str, flags: u32) -> Result<(), String> {
    let dll_name = to_wide("input.dll");
    unsafe {
        let hmod = LoadLibraryExW(dll_name.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if hmod == 0 {
            return Err("LoadLibraryExW(input.dll) failed".to_string());
        }
        type InstallFn = unsafe extern "system" fn(*const u16, u32) -> i32;
        let proc = GetProcAddress(hmod, c"InstallLayoutOrTip".as_ptr().cast::<u8>());
        let res = if let Some(f) = proc {
            let install_fn: InstallFn = std::mem::transmute(f);
            let tip_w = to_wide(tip);
            if install_fn(tip_w.as_ptr(), flags) != 0 {
                Ok(())
            } else {
                Err(format!("InstallLayoutOrTip({tip}, flags={flags}) returned FALSE"))
            }
        } else {
            Err("InstallLayoutOrTip export not found".to_string())
        };
        FreeLibrary(hmod);
        res
    }
}

fn call_input_dll_set_default(tip: &str, flags: u32) -> Result<(), String> {
    let dll_name = to_wide("input.dll");
    unsafe {
        let hmod = LoadLibraryExW(dll_name.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if hmod == 0 {
            return Err("LoadLibraryExW(input.dll) failed".to_string());
        }
        type SetDefaultFn = unsafe extern "system" fn(*const u16, u32) -> i32;
        let proc = GetProcAddress(hmod, c"SetDefaultLayoutOrTip".as_ptr().cast::<u8>());
        let res = if let Some(f) = proc {
            let set_fn: SetDefaultFn = std::mem::transmute(f);
            let tip_w = to_wide(tip);
            if set_fn(tip_w.as_ptr(), flags) != 0 {
                Ok(())
            } else {
                Err(format!("SetDefaultLayoutOrTip({tip}) returned FALSE"))
            }
        } else {
            Err("SetDefaultLayoutOrTip export not found".to_string())
        };
        FreeLibrary(hmod);
        res
    }
}

fn enum_enabled_layout_or_tip_ids() -> Vec<String> {
    let dll_name = to_wide("input.dll");
    unsafe {
        let hmod = LoadLibraryExW(dll_name.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if hmod == 0 {
            return Vec::new();
        }
        type EnumFn = unsafe extern "system" fn(
            *const u16,
            *const u16,
            *const u16,
            *mut LayoutOrTipProfile,
            u32,
        ) -> u32;
        let out = if let Some(f) =
            GetProcAddress(hmod, c"EnumEnabledLayoutOrTip".as_ptr().cast::<u8>())
        {
            let enum_fn: EnumFn = std::mem::transmute(f);
            let count = enum_fn(null(), null(), null(), null_mut(), 0);
            if count > 0 {
                let mut buf = vec![std::mem::zeroed::<LayoutOrTipProfile>(); count as usize];
                let got = enum_fn(null(), null(), null(), buf.as_mut_ptr(), count);
                buf.into_iter()
                    .take(got as usize)
                    .map(|p| from_wide_nul(&p.sz_id))
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        FreeLibrary(hmod);
        out
    }
}

fn reg_enum_subkeys(root: HKEY, subkey: &str) -> Vec<String> {
    let sub_w = to_wide(subkey);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(
            root,
            sub_w.as_ptr(),
            0,
            KEY_ENUMERATE_SUB_KEYS | KEY_QUERY_VALUE,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut names = Vec::new();
        let mut index = 0u32;
        let mut buf = [0u16; 512];
        loop {
            let mut len = buf.len() as u32;
            let rc = RegEnumKeyExW(
                hkey,
                index,
                buf.as_mut_ptr(),
                &mut len,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&buf[..len as usize]));
            }
            index += 1;
        }
        RegCloseKey(hkey);
        names
    }
}

fn reg_enum_value_names(root: HKEY, subkey: &str) -> Vec<String> {
    let sub_w = to_wide(subkey);
    let mut hkey: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) != ERROR_SUCCESS {
            return Vec::new();
        }
        let mut names = Vec::new();
        let mut index = 0u32;
        let mut buf = [0u16; 512];
        loop {
            let mut len = buf.len() as u32;
            let rc = RegEnumValueW(
                hkey,
                index,
                buf.as_mut_ptr(),
                &mut len,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc == ERROR_SUCCESS {
                names.push(String::from_utf16_lossy(&buf[..len as usize]));
            }
            index += 1;
        }
        RegCloseKey(hkey);
        names
    }
}

fn find_chinese_user_profile_tag() -> String {
    let base = r"Control Panel\International\User Profile";
    let langs = reg_read_multi_sz(HKEY_CURRENT_USER, base, "Languages").unwrap_or_default();
    for tag in &langs {
        if tag.eq_ignore_ascii_case("zh-Hans-CN") || tag.eq_ignore_ascii_case("zh-CN") {
            return tag.clone();
        }
    }
    for tag in &langs {
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("zh-hans") || lower.starts_with("zh-cn") {
            return tag.clone();
        }
    }
    "zh-Hans-CN".to_string()
}

pub fn is_tip_in_user_language_list(tip: &str) -> bool {
    let enabled = enum_enabled_layout_or_tip_ids();
    if enabled.iter().any(|id| id.eq_ignore_ascii_case(tip)) {
        return true;
    }
    let base = r"Control Panel\International\User Profile";
    for sub in reg_enum_subkeys(HKEY_CURRENT_USER, base) {
        let full = format!("{base}\\{sub}");
        for val_name in reg_enum_value_names(HKEY_CURRENT_USER, &full) {
            if val_name.eq_ignore_ascii_case(tip) {
                return true;
            }
        }
    }
    false
}

fn append_language_log(message: &str) {
    if let Ok(state_root) = user_state_root() {
        let _ = fs::create_dir_all(&state_root);
        let path = state_root.join("install_language_list.log");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let stamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(f, "{stamp} {message}");
        }
    }
}

pub fn add_tip_to_current_user_language_list(
    tip: &str,
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let base = r"Control Panel\International\User Profile";
    let mut langs = reg_read_multi_sz(HKEY_CURRENT_USER, base, "Languages").unwrap_or_default();
    let target_tag = find_chinese_user_profile_tag();
    if !langs.iter().any(|l| l.eq_ignore_ascii_case(&target_tag)) {
        langs.push(target_tag.clone());
        let _ = reg_write_multi_sz(HKEY_CURRENT_USER, base, "Languages", &langs);
        let msg = format!("OK: created {target_tag} language entry for current user");
        append_language_log(&msg);
        log(&msg);
    }

    if is_tip_in_user_language_list(tip) {
        let msg = "OK: TIP already in Chinese input method list";
        append_language_log(msg);
        log(msg);
        return Ok(());
    }

    let install_res = call_input_dll_install(tip, 0);
    if let Err(ref err) = install_res {
        let msg = format!("WARN: InstallLayoutOrTip returned error ({err}); updating User Profile");
        append_language_log(&msg);
        log(&msg);
    }

    let lang_subkey = format!("{base}\\{target_tag}");
    let existing_vals = reg_enum_value_names(HKEY_CURRENT_USER, &lang_subkey);
    if !existing_vals.iter().any(|v| v.eq_ignore_ascii_case(tip)) {
        let next_order = (existing_vals.iter().filter(|v| v.contains(':')).count() as u32) + 1;
        let _ = reg_write_dword(HKEY_CURRENT_USER, &lang_subkey, tip, next_order);
    }

    if !is_tip_in_user_language_list(tip) {
        let msg = format!(
            "FAIL: Windows rejected TIP after language list registration; TSF profile is not registered: {tip}"
        );
        append_language_log(&msg);
        log(&msg);
        return Err(msg);
    }

    let msg = "OK: added and verified TIP in Chinese input method list";
    append_language_log(msg);
    log(msg);
    Ok(())
}

pub fn restore_microsoft_pinyin(log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let base = r"Control Panel\International\User Profile";
    let mut langs = reg_read_multi_sz(HKEY_CURRENT_USER, base, "Languages").unwrap_or_default();
    let target_tag = find_chinese_user_profile_tag();
    if !langs.iter().any(|l| l.eq_ignore_ascii_case(&target_tag)) {
        langs.push(target_tag.clone());
        let _ = reg_write_multi_sz(HKEY_CURRENT_USER, base, "Languages", &langs);
    }

    let _ = call_input_dll_install(MICROSOFT_PINYIN_TIP, 0);
    let _ = call_input_dll_install(SIMPLIFIED_CHINESE_TIP, ILOT_UNINSTALL);

    // Clean Kaixin TIP from every language entry under User Profile and ensure Microsoft Pinyin exists.
    for sub in reg_enum_subkeys(HKEY_CURRENT_USER, base) {
        let full = format!("{base}\\{sub}");
        reg_delete_value(HKEY_CURRENT_USER, &full, SIMPLIFIED_CHINESE_TIP);
    }
    let zh_sub = format!("{base}\\{target_tag}");
    let zh_vals = reg_enum_value_names(HKEY_CURRENT_USER, &zh_sub);
    if !zh_vals
        .iter()
        .any(|v| v.eq_ignore_ascii_case(MICROSOFT_PINYIN_TIP))
    {
        let _ = reg_write_dword(HKEY_CURRENT_USER, &zh_sub, MICROSOFT_PINYIN_TIP, 1);
    }

    let _ = call_input_dll_set_default(MICROSOFT_PINYIN_TIP, 0);
    reg_write_sz(HKEY_CURRENT_USER, base, "InputMethodOverride", MICROSOFT_PINYIN_TIP)?;
    notify_language_change();

    let override_val =
        reg_read_sz(HKEY_CURRENT_USER, base, Some("InputMethodOverride"), 0).unwrap_or_default();
    if !override_val.eq_ignore_ascii_case(MICROSOFT_PINYIN_TIP) {
        return Err("Windows did not set Microsoft Pinyin as the default input method.".to_string());
    }
    log("OK: restored Microsoft Pinyin and removed Kaixin from the current user language list");
    Ok(())
}

fn clean_tip_in_reg_key(hkey: HKEY, tip: &str, removed_values: &mut usize) {
    let mut index = 0u32;
    let mut to_delete: Vec<String> = Vec::new();
    let mut to_set_sz: Vec<(String, String)> = Vec::new();
    let mut to_set_multi: Vec<(String, Vec<String>)> = Vec::new();

    unsafe {
        let mut name_buf = [0u16; 512];
        loop {
            let mut name_len = name_buf.len() as u32;
            let mut reg_type = 0u32;
            let mut data_len = 0u32;
            let rc = RegEnumValueW(
                hkey,
                index,
                name_buf.as_mut_ptr(),
                &mut name_len,
                null_mut(),
                &mut reg_type,
                null_mut(),
                &mut data_len,
            );
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc != ERROR_SUCCESS {
                index += 1;
                continue;
            }
            let val_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            if val_name.eq_ignore_ascii_case(tip) {
                to_delete.push(val_name);
                index += 1;
                continue;
            }
            if (reg_type == REG_SZ || reg_type == REG_MULTI_SZ) && data_len > 0 && data_len <= 65536
            {
                let mut data = vec![0u8; data_len as usize];
                let mut len2 = data_len;
                let mut type2 = 0u32;
                let val_w = to_wide(&val_name);
                if RegQueryValueExW(
                    hkey,
                    val_w.as_ptr(),
                    null_mut(),
                    &mut type2,
                    data.as_mut_ptr(),
                    &mut len2,
                ) == ERROR_SUCCESS
                {
                    let units: Vec<u16> = data[..len2 as usize]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| u16::from_le_bytes(*c))
                        .collect();
                    if reg_type == REG_SZ {
                        let text = from_wide_nul(&units);
                        if text.eq_ignore_ascii_case(tip) {
                            to_delete.push(val_name);
                        } else if text.to_ascii_lowercase().contains(&tip.to_ascii_lowercase()) {
                            let parts: Vec<&str> = text
                                .split([';', ','])
                                .map(str::trim)
                                .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case(tip))
                                .collect();
                            if parts.is_empty() {
                                to_delete.push(val_name);
                            } else {
                                to_set_sz.push((val_name, parts.join(";")));
                            }
                        }
                    } else if reg_type == REG_MULTI_SZ {
                        let mut items = Vec::new();
                        let mut start = 0usize;
                        for (i, &c) in units.iter().enumerate() {
                            if c == 0 {
                                if i > start {
                                    items.push(String::from_utf16_lossy(&units[start..i]));
                                }
                                start = i + 1;
                            }
                        }
                        let orig_len = items.len();
                        items.retain(|it| !it.eq_ignore_ascii_case(tip));
                        if items.len() != orig_len {
                            if items.is_empty() {
                                to_delete.push(val_name);
                            } else {
                                to_set_multi.push((val_name, items));
                            }
                        }
                    }
                }
            }
            index += 1;
        }

        for name in to_delete {
            let w = to_wide(&name);
            if RegDeleteValueW(hkey, w.as_ptr()) == ERROR_SUCCESS {
                *removed_values += 1;
            }
        }
        for (name, val) in to_set_sz {
            let nw = to_wide(&name);
            let vw = to_wide(&val);
            if RegSetValueExW(
                hkey,
                nw.as_ptr(),
                0,
                REG_SZ,
                vw.as_ptr() as *const u8,
                (vw.len() * 2) as u32,
            ) == ERROR_SUCCESS
            {
                *removed_values += 1;
            }
        }
        for (name, items) in to_set_multi {
            let nw = to_wide(&name);
            let mut vw: Vec<u16> = Vec::new();
            for it in items {
                vw.extend(it.encode_utf16());
                vw.push(0);
            }
            vw.push(0);
            if RegSetValueExW(
                hkey,
                nw.as_ptr(),
                0,
                REG_MULTI_SZ,
                vw.as_ptr() as *const u8,
                (vw.len() * 2) as u32,
            ) == ERROR_SUCCESS
            {
                *removed_values += 1;
            }
        }
    }
}

fn walk_and_clean_tip(
    root: HKEY,
    subkey: &str,
    tip: &str,
    removed_values: &mut usize,
    removed_keys: &mut usize,
) {
    let children = reg_enum_subkeys(root, subkey);
    for child in children {
        let child_path = format!("{subkey}\\{child}");
        if child.eq_ignore_ascii_case(tip) {
            if reg_delete_tree(root, &child_path, 0) {
                *removed_keys += 1;
            }
        } else {
            walk_and_clean_tip(root, &child_path, tip, removed_values, removed_keys);
        }
    }

    let sub_w = to_wide(subkey);
    unsafe {
        let mut hkey: HKEY = 0;
        if RegOpenKeyExW(root, sub_w.as_ptr(), 0, KEY_READ | KEY_WRITE, &mut hkey) == ERROR_SUCCESS
        {
            clean_tip_in_reg_key(hkey, tip, removed_values);
            RegCloseKey(hkey);
        }
    }
}

pub fn remove_tip_from_loaded_user_registry(tip: &str, log: &mut dyn FnMut(&str)) {
    let mut visited_users = 0usize;
    let mut removed_values = 0usize;
    let mut removed_keys = 0usize;

    let mut index = 0u32;
    let mut sid_names = Vec::new();
    unsafe {
        let mut buf = [0u16; 256];
        loop {
            let mut len = buf.len() as u32;
            let rc = RegEnumKeyExW(
                HKEY_USERS,
                index,
                buf.as_mut_ptr(),
                &mut len,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            );
            if rc == ERROR_NO_MORE_ITEMS {
                break;
            }
            if rc == ERROR_SUCCESS {
                sid_names.push(String::from_utf16_lossy(&buf[..len as usize]));
            }
            index += 1;
        }
    }

    for sid in sid_names {
        if sid.is_empty()
            || sid.ends_with("_Classes")
            || !sid.starts_with("S-")
            || sid.matches('-').count() < 3
        {
            continue;
        }
        visited_users += 1;
        for tail in [
            r"Control Panel\International\User Profile",
            r"Software\Microsoft\CTF",
        ] {
            let root_path = format!("{sid}\\{tail}");
            if reg_key_exists(HKEY_USERS, &root_path, 0) {
                walk_and_clean_tip(
                    HKEY_USERS,
                    &root_path,
                    tip,
                    &mut removed_values,
                    &mut removed_keys,
                );
            }
        }
    }

    log(&format!(
        "OK: loaded user language registry cleanup complete; users={visited_users} removed_values={removed_values} removed_keys={removed_keys}"
    ));
}

pub fn is_reparse_point(path: &Path) -> bool {
    let w = os_to_wide(path.as_os_str());
    let attrs = unsafe { GetFileAttributesW(w.as_ptr()) };
    attrs != u32::MAX && (attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

pub fn remove_tree_no_follow(path: &Path) -> Result<(), String> {
    if !path.exists() && !is_reparse_point(path) {
        return Ok(());
    }
    if is_reparse_point(path) {
        if path.is_dir() {
            fs::remove_dir(path)
                .map_err(|e| format!("remove junction {}: {e}", path.display()))?;
        } else {
            fs::remove_file(path)
                .map_err(|e| format!("remove symlink {}: {e}", path.display()))?;
        }
        return Ok(());
    }
    if path.is_dir() {
        let entries = fs::read_dir(path)
            .map_err(|e| format!("read dir {}: {e}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("read dir entry {}: {e}", path.display()))?;
            remove_tree_no_follow(&entry.path())?;
        }
        fs::remove_dir(path).map_err(|e| format!("remove dir {}: {e}", path.display()))?;
    } else {
        set_writable(path);
        fs::remove_file(path).map_err(|e| format!("remove file {}: {e}", path.display()))?;
    }
    Ok(())
}

fn set_writable(path: &Path) {
    let w = os_to_wide(path.as_os_str());
    unsafe {
        let attrs = GetFileAttributesW(w.as_ptr());
        if attrs != u32::MAX && (attrs & FILE_ATTRIBUTE_READONLY) != 0 {
            SetFileAttributesW(w.as_ptr(), attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }
}

pub fn clear_transient_user_state(log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let state_root = user_state_root()?;
    if !state_root.is_dir() {
        return Ok(());
    }
    let transient = ["cache", "logs", "install_user.log", "install_language_list.log"];
    if let Ok(entries) = fs::read_dir(&state_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if transient.iter().any(|t| t.eq_ignore_ascii_case(&name_str)) {
                let _ = remove_tree_no_follow(&entry.path());
            }
        }
    }
    log("OK: cleaned known transient user state while preserving all other settings and user data");
    Ok(())
}

pub fn remove_kaixin_user_data(log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let run_once = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    for val in reg_enum_value_names(HKEY_CURRENT_USER, run_once) {
        if val.to_ascii_lowercase().starts_with("kaixininputcleanup-") {
            reg_delete_value(HKEY_CURRENT_USER, run_once, &val);
        }
    }

    let state_key = format!("Software\\{APP_PATH_NAME}");
    let _ = reg_delete_tree(HKEY_CURRENT_USER, &state_key, 0);

    let state_root = user_state_root()?;
    remove_tree_no_follow(&state_root)?;

    let temp_root = std::env::temp_dir();
    if let Ok(entries) = fs::read_dir(&temp_root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let path = entry.path();
            if (path.is_dir() && is_internal_temp_dir(&name_str))
                || (path.is_file() && is_internal_ocr_temp_file(&name_str))
            {
                let _ = remove_tree_no_follow(&path);
            }
        }
    }
    log("OK: removed current user data");
    Ok(())
}

fn is_internal_temp_dir(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("kaixin-frame-")
        .or_else(|| name.strip_prefix("kaixin-uia-"))
    else {
        return false;
    };
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_internal_ocr_temp_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".png") else {
        return false;
    };
    for prefix in [
        "kaixin_ocr_capture_",
        "kaixin_ocr_crop_",
        "kaixin_ocr_processed_",
        "kaixin_ocr_border_",
        "kaixin_ocr_file_",
    ] {
        if let Some(digits) = stem.strip_prefix(prefix) {
            return digits.len() == 17 && digits.bytes().all(|b| b.is_ascii_digit());
        }
    }
    false
}

pub fn remove_machine_state_root() -> Result<(), String> {
    let program_data = std::env::var_os("ProgramData")
        .or_else(|| std::env::var_os("ALLUSERSPROFILE"))
        .unwrap_or_else(|| r"C:\ProgramData".into());
    let path = PathBuf::from(program_data).join(APP_PATH_NAME);
    if path.exists() || is_reparse_point(&path) {
        remove_tree_no_follow(&path)?;
    }
    Ok(())
}

pub fn schedule_pending_delete_tree(target: &Path, log: &mut dyn FnMut(&str)) -> bool {
    if !target.exists() {
        return true;
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    collect_files_and_dirs(target, &mut files, &mut dirs);
    dirs.sort_by_key(|p| std::cmp::Reverse(p.as_os_str().len()));

    let mut all_ok = true;
    for f in files {
        let w = os_to_wide(f.as_os_str());
        if unsafe { MoveFileExW(w.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0 {
            all_ok = false;
        }
    }
    for d in dirs {
        let w = os_to_wide(d.as_os_str());
        if unsafe { MoveFileExW(w.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0 {
            all_ok = false;
        }
    }
    let root_w = os_to_wide(target.as_os_str());
    if unsafe { MoveFileExW(root_w.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) } == 0 {
        all_ok = false;
    }
    if all_ok {
        log(&format!(
            "OK: marked pending reboot deletion: {}",
            target.display()
        ));
    }
    all_ok
}

fn collect_files_and_dirs(root: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_reparse_point(&path) {
            files.push(path);
        } else if path.is_dir() {
            collect_files_and_dirs(&path, files, dirs);
            dirs.push(path);
        } else {
            files.push(path);
        }
    }
}

pub fn register_hkcu_runonce_cleanup(
    target: &Path,
    marker_path: Option<&Path>,
    log: &mut dyn FnMut(&str),
) -> bool {
    if !target.exists() {
        return true;
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let value_name = format!("KaixinInputCleanup-{unique}");
    let target_s = target.display().to_string();
    let command = if let Some(marker) = marker_path {
        format!(
            "cmd.exe /C \"if not exist \"{}\" rd /s /q \"{}\"",
            marker.display(),
            target_s
        )
    } else {
        format!("cmd.exe /C \"rd /s /q \"{}\"", target_s)
    };
    let run_once = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    match reg_write_sz(HKEY_CURRENT_USER, run_once, &value_name, &command) {
        Ok(()) => {
            log(&format!(
                "OK: registered HKCU RunOnce cleanup for {target_s}"
            ));
            true
        }
        Err(e) => {
            log(&format!(
                "WARN: could not register HKCU RunOnce cleanup for {target_s}: {e}"
            ));
            false
        }
    }
}

pub fn remove_uninstall_registry_entry(machine_scope: bool, log: &mut dyn FnMut(&str)) {
    let hive = if machine_scope {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let key = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{APP_DISPLAY_NAME}"
    );
    if reg_delete_tree(hive, &key, 0) {
        log(&format!("OK: removed uninstall registry entry: {key}"));
    } else {
        log(&format!(
            "OK: uninstall registry entry already absent: {key}"
        ));
    }
}

pub fn cleanup_old_runtime_payloads(
    install_root: &Path,
    current_payload_root: &Path,
    log: &mut dyn FnMut(&str),
) {
    let runtime_root = normalize_lexical(&install_root.join(RUNTIME_PAYLOAD_ROOT_NAME));
    if !runtime_root.is_dir() {
        return;
    }
    let current = normalize_lexical(current_payload_root);
    let current_key = current
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_ascii_lowercase();

    let Ok(entries) = fs::read_dir(&runtime_root) else {
        return;
    };
    let mut removed = 0usize;
    let mut preserved = 0usize;
    for entry in entries.flatten() {
        let path = normalize_lexical(&entry.path());
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if !(lower.starts_with("pkg-") || lower.starts_with(".delete-pending-")) {
            continue;
        }
        let key = path
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_ascii_lowercase();
        if key == current_key {
            continue;
        }
        if !path_within_root(&runtime_root, &path) {
            continue;
        }
        match remove_tree_no_follow(&path) {
            Ok(()) => {
                removed += 1;
                log(&format!("OK: removed old runtime payload {name}"));
            }
            Err(_) => {
                // Still loaded by a running host process; leave intact so the host does not crash.
                preserved += 1;
                log(&format!(
                    "OK: preserving runtime payload still in use by a host process: {name}"
                ));
            }
        }
    }
    log(&format!(
        "OK: runtime payload cleanup complete; removed={removed} preserved={preserved}"
    ));
}

pub struct EngineProbeResult {
    pub status: String,
    pub exit_code: String,
    pub direct: String,
    pub pipe: String,
    pub reason: String,
}

pub fn probe_engine_health(install_root: &Path) -> EngineProbeResult {
    let engine_exe = install_root.join("srf_ime_engine.exe");
    if !engine_exe.is_file() {
        return EngineProbeResult {
            status: "missing".to_string(),
            exit_code: "(none)".to_string(),
            direct: "unknown".to_string(),
            pipe: "unknown".to_string(),
            reason: "engine_missing".to_string(),
        };
    }

    let mut child = match Command::new(&engine_exe)
        .args(["--install-health-check", "--probe", "nihao"])
        .current_dir(install_root)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return EngineProbeResult {
                status: "error".to_string(),
                exit_code: "(none)".to_string(),
                direct: "unknown".to_string(),
                pipe: "unknown".to_string(),
                reason: e.to_string(),
            };
        }
    };

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return EngineProbeResult {
                        status: "timeout".to_string(),
                        exit_code: "(none)".to_string(),
                        direct: "unknown".to_string(),
                        pipe: "unknown".to_string(),
                        reason: "timeout".to_string(),
                    };
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return EngineProbeResult {
                    status: "error".to_string(),
                    exit_code: "(none)".to_string(),
                    direct: "unknown".to_string(),
                    pipe: "unknown".to_string(),
                    reason: e.to_string(),
                };
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            return EngineProbeResult {
                status: "error".to_string(),
                exit_code: "(none)".to_string(),
                direct: "unknown".to_string(),
                pipe: "unknown".to_string(),
                reason: e.to_string(),
            };
        }
    };
    let exit_code_num = output.status.code();
    let exit_code_ok = exit_code_num == Some(0);
    let exit_code = exit_code_num
        .map(|c| c.to_string())
        .unwrap_or_else(|| "(none)".to_string());
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut direct = if exit_code_ok { "ok" } else { "unknown" }.to_string();
    let mut pipe = if exit_code_ok { "ok" } else { "unknown" }.to_string();
    let mut saw_probe = false;
    let mut reason = "none".to_string();

    for token in text.split_whitespace() {
        if let Some(v) = token.strip_prefix("direct=") {
            direct = v.to_string();
            saw_probe = true;
        } else if let Some(v) = token.strip_prefix("pipe=") {
            pipe = v.to_string();
            saw_probe = true;
        } else if let Some(v) = token.strip_prefix("direct_reason=") {
            reason = format!("direct_{v}");
        } else if let Some(v) = token.strip_prefix("pipe_reason=") {
            if v != "none" {
                reason = format!("pipe_{v}");
            }
        }
    }

    let ok = if saw_probe {
        direct == "ok" && pipe == "ok"
    } else {
        exit_code_ok
    };
    EngineProbeResult {
        status: if ok { "ok" } else { "failed" }.to_string(),
        exit_code,
        direct,
        pipe,
        reason,
    }
}

pub fn format_health_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "(none)".to_string();
    }
    let mut out = String::new();
    let mut prev_sep = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() || matches!(ch, ',' | ';' | '=') {
            if !prev_sep {
                out.push('_');
                prev_sep = true;
            }
        } else {
            out.push(ch);
            prev_sep = false;
        }
        if out.len() >= 240 {
            break;
        }
    }
    out
}

pub fn write_install_maintenance_flag(enabled: bool) {
    let key = format!("Software\\{APP_PATH_NAME}\\State");
    if enabled {
        let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
        let _ = reg_write_dword(HKEY_CURRENT_USER, &key, "InstallMaintenanceTick", tick);
        let _ = reg_write_dword(HKEY_CURRENT_USER, &key, "InstallMaintenance", 1);
    } else {
        reg_delete_value(HKEY_CURRENT_USER, &key, "InstallMaintenance");
        reg_delete_value(HKEY_CURRENT_USER, &key, "InstallMaintenanceTick");
    }
}

pub fn schedule_self_deferred_dir_delete(target: &Path) {
    let cmd = format!(
        "ping 127.0.0.1 -n 3 >NUL & rd /s /q \"{}\"",
        target.display()
    );
    let _ = Command::new("cmd.exe")
        .args(["/C", &cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
