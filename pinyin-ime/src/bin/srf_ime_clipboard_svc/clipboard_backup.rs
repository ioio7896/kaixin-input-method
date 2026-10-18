//! Own all clipboard formats before a temporary text paste.

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, GetObjectW, BITMAP};
    use windows_sys::Win32::System::Memory::GlobalSize;
    use windows_sys::Win32::System::StationsAndDesktops::*;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;

    struct IsolatedClipboard {
        station: HWINSTA,
        desktop: HDESK,
        old_station: HWINSTA,
        old_desktop: HDESK,
    }
    impl IsolatedClipboard {
        fn new() -> Self {
            assert_eq!(
                std::env::var("KAIXIN_ISOLATED_CLIPBOARD_TEST").as_deref(),
                Ok("1"),
                "these tests must never touch the user's clipboard"
            );
            unsafe {
                let old_station = GetProcessWindowStation();
                let old_desktop = GetThreadDesktop(GetCurrentThreadId());
                let name: Vec<u16> = format!("KaixinClipboardAudit_{}\0", std::process::id())
                    .encode_utf16()
                    .collect();
                let station = CreateWindowStationW(name.as_ptr(), 0, 0x37f, std::ptr::null());
                assert_ne!(
                    station,
                    0,
                    "CreateWindowStation: {}",
                    std::io::Error::last_os_error()
                );
                assert_ne!(SetProcessWindowStation(station), 0);
                let desktop_name: Vec<u16> = "AuditDesktop\0".encode_utf16().collect();
                let desktop = CreateDesktopW(
                    desktop_name.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    0x1ff,
                    std::ptr::null(),
                );
                assert_ne!(
                    desktop,
                    0,
                    "CreateDesktop: {}",
                    std::io::Error::last_os_error()
                );
                assert_ne!(
                    SetThreadDesktop(desktop),
                    0,
                    "SetThreadDesktop: {}",
                    std::io::Error::last_os_error()
                );
                Self {
                    station,
                    desktop,
                    old_station,
                    old_desktop,
                }
            }
        }
    }
    impl Drop for IsolatedClipboard {
        fn drop(&mut self) {
            unsafe {
                SetProcessWindowStation(self.old_station);
                SetThreadDesktop(self.old_desktop);
                CloseDesktop(self.desktop);
                CloseWindowStation(self.station);
            }
        }
    }

    fn read_text() -> String {
        let mut output = Vec::new();
        clipboard_win::raw::get_string(&mut output).unwrap();
        String::from_utf8(output).unwrap()
    }
    fn set_bytes(format: u32, bytes: &[u8]) {
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            assert!(!memory.is_null());
            let target = GlobalLock(memory) as *mut u8;
            assert!(!target.is_null());
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
            GlobalUnlock(memory);
            assert_ne!(SetClipboardData(format, memory as _), 0);
        }
    }
    fn bytes(format: u32) -> Vec<u8> {
        unsafe {
            let handle = GetClipboardData(format);
            assert_ne!(handle, 0);
            let ptr = GlobalLock(handle as _) as *const u8;
            assert!(!ptr.is_null());
            let bytes = std::slice::from_raw_parts(ptr, GlobalSize(handle as _)).to_vec();
            GlobalUnlock(handle as _);
            bytes
        }
    }

    #[test]
    #[ignore = "requires an isolated Windows window station and explicit test environment"]
    fn native_formats_restore_and_newer_copy_is_preserved() {
        let _isolated = IsolatedClipboard::new();
        let owner = ClipboardWindow::new().unwrap();
        let html_name: Vec<u16> = "HTML Format\0".encode_utf16().collect();
        let html = unsafe { RegisterClipboardFormatW(html_name.as_ptr()) };
        let html_bytes = b"<html><body><b>original rich text</b></body></html>\0";
        let mut dib = vec![0u8; 44];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4u32.to_le_bytes());
        dib[40..44].copy_from_slice(&[12, 34, 56, 255]);
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            clipboard_win::raw::set_string("original text").unwrap();
            set_bytes(html, html_bytes);
            set_bytes(8, &dib); // CF_DIB
            let bitmap = unsafe { CreateBitmap(1, 1, 1, 32, dib[40..].as_ptr() as _) };
            assert_ne!(bitmap, 0);
            assert_ne!(unsafe { SetClipboardData(2, bitmap) }, 0);
        }
        let backup = ClipboardBackup::replace_with_text("temporary paste").unwrap();
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            assert_eq!(read_text(), "temporary paste");
            assert_eq!(unsafe { GetClipboardData(html) }, 0);
        }
        // Same restoration path is used whether sending Ctrl+V succeeds or fails.
        backup.restore().unwrap();
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            assert_eq!(read_text(), "original text");
            assert!(bytes(html).starts_with(html_bytes));
            assert!(bytes(8).starts_with(&dib));
            let bitmap = unsafe { GetClipboardData(2) };
            let mut info: BITMAP = unsafe { std::mem::zeroed() };
            assert_ne!(
                unsafe {
                    GetObjectW(
                        bitmap,
                        std::mem::size_of::<BITMAP>() as i32,
                        &mut info as *mut _ as _,
                    )
                },
                0
            );
            assert_eq!((info.bmWidth, info.bmHeight), (1, 1));
        }

        assert!(super::super::paste_text_worker("paste to invalid target", -1).is_err());
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            assert_eq!(read_text(), "original text");
            assert!(bytes(html).starts_with(html_bytes));
            assert!(bytes(8).starts_with(&dib));
        }

        let mut backup = ClipboardBackup::replace_with_text("another temporary paste").unwrap();
        copy_text("new copy made during paste").unwrap();
        // Force the narrow post-close sequence-sampling race. The distinct
        // marker must still prevent restoring over the newer foreign copy.
        backup.sequence = clipboard_win::raw::seq_num().map(|seq| seq.get());
        backup.restore().unwrap();
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            assert_eq!(read_text(), "new copy made during paste");
            assert_ne!(unsafe { EmptyClipboard() }, 0);
        }
        let backup = ClipboardBackup::replace_with_text("from empty").unwrap();
        backup.restore().unwrap();
        {
            let _guard = clipboard_win::Clipboard::new_for(owner.0 as _).unwrap();
            assert_eq!(unsafe { EnumClipboardFormats(0) }, 0);
        }

        // Exercise actual Windows notifications on this station's clipboard.
        let config = pinyin_ime::clipboard_store::store_path()
            .parent()
            .unwrap()
            .join("kaixin.ini");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(config, "[clipboard]\nbackground_enabled=1\nmax_history_items=300\nmax_text_utf16_units=1000000\n").unwrap();
        pinyin_ime::clipboard_store::set_background_polling_enabled(true);
        std::thread::sleep(std::time::Duration::from_millis(350));
        let copied: Vec<String> = (0..16)
            .map(|index| format!("native rapid copy {index} 中文 😀"))
            .collect();
        for text in &copied {
            copy_text(text).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        std::thread::sleep(std::time::Duration::from_millis(800));
        let snapshot = pinyin_ime::clipboard_store::snapshot().unwrap();
        for text in &copied {
            assert!(
                snapshot.history.iter().any(|entry| &entry.text == text),
                "missed {text}"
            );
        }
        assert_eq!(snapshot.history[0].text, *copied.last().unwrap());
        let long = "😀中文\n".repeat(10_000);
        copy_text(&long).unwrap();
        let snapshot = pinyin_ime::clipboard_store::capture_system_clipboard_snapshot().unwrap();
        assert_eq!(snapshot.history[0].text, long);
        pinyin_ime::clipboard_store::flush_pending_ops_sync();
        pinyin_ime::clipboard_store::set_background_polling_enabled(false);
    }
}

// Duplicates are materialized while the clipboard is open, including delayed
// rendering. If any format cannot be duplicated, refuse to replace it.
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HANDLE};
use windows_sys::Win32::Graphics::Gdi::{
    CopyEnhMetaFileW, DeleteEnhMetaFile, DeleteMetaFile, DeleteObject,
};
use windows_sys::Win32::System::DataExchange::{
    EmptyClipboard, EnumClipboardFormats, GetClipboardData, RegisterClipboardFormatW,
    SetClipboardData, METAFILEPICT,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::OleDuplicateData;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

struct ClipboardWindow(isize);
impl ClipboardWindow {
    fn new() -> Result<Self, String> {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let handle = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                0,
                0,
                std::ptr::null(),
            )
        };
        if handle == 0 {
            return Err(format!(
                "创建剪贴板窗口失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(handle))
    }
}
impl Drop for ClipboardWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

struct SavedFormat {
    format: u32,
    handle: HANDLE,
}
impl Drop for SavedFormat {
    fn drop(&mut self) {
        if self.handle == 0 {
            return;
        }
        unsafe {
            match self.format {
                2 | 9 | 0x82 => {
                    DeleteObject(self.handle);
                }
                14 | 0x8e => {
                    DeleteEnhMetaFile(self.handle);
                }
                3 | 0x83 => {
                    let ptr = GlobalLock(self.handle as _) as *const METAFILEPICT;
                    if !ptr.is_null() {
                        DeleteMetaFile((*ptr).hMF);
                        GlobalUnlock(self.handle as _);
                    }
                    GlobalFree(self.handle as _);
                }
                _ => {
                    GlobalFree(self.handle as _);
                }
            }
        }
    }
}

pub struct ClipboardBackup {
    formats: Vec<SavedFormat>,
    sequence: Option<u32>,
    window: ClipboardWindow,
    marker: [u8; 16],
}

pub fn copy_text(text: &str) -> Result<Option<u32>, String> {
    let window = ClipboardWindow::new()?;
    let _guard = clipboard_win::Clipboard::new_attempts_for(window.0 as _, 12)
        .map_err(|err| format!("剪贴板被其他程序占用：{err}"))?;
    clipboard_win::raw::set_string(text).map_err(|err| format!("复制文本失败：{err}"))?;
    Ok(clipboard_win::raw::seq_num().map(|seq| seq.get()))
}

impl ClipboardBackup {
    pub fn replace_with_text(text: &str) -> Result<Self, String> {
        let window = ClipboardWindow::new()?;
        let _guard = clipboard_win::Clipboard::new_attempts_for(window.0 as _, 12)
            .map_err(|err| format!("无法备份原剪贴板：{err}"))?;
        let mut formats = Vec::new();
        let mut format = 0;
        loop {
            unsafe {
                SetLastError(0);
            }
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                let error = unsafe { GetLastError() };
                if error != 0 {
                    return Err(format!("枚举原剪贴板失败：{error}"));
                }
                break;
            }
            // Owner-display/private handle formats cannot be restored safely.
            if format == 0x80 || (0x200..=0x2ff).contains(&format) {
                return Err("原剪贴板包含无法安全备份的格式，请使用“复制”后自行粘贴。".into());
            }
            let source = unsafe { GetClipboardData(format) };
            if source == 0 {
                return Err(format!("读取原剪贴板格式 {format} 失败，已保留原剪贴板。"));
            }
            let handle = unsafe {
                match format {
                    14 | 0x8e => CopyEnhMetaFileW(source, std::ptr::null()),
                    0x82 => OleDuplicateData(source, 2, GMEM_MOVEABLE),
                    0x83 => OleDuplicateData(source, 3, GMEM_MOVEABLE),
                    _ => OleDuplicateData(source, format as u16, GMEM_MOVEABLE),
                }
            };
            if handle == 0 {
                return Err(format!("备份原剪贴板格式 {format} 失败，已保留原剪贴板。"));
            }
            formats.push(SavedFormat { format, handle });
        }
        let mut backup = Self {
            formats,
            sequence: None,
            window,
            marker: paste_marker(),
        };
        if let Err(err) = clipboard_win::raw::set_string(text) {
            // set_string may have emptied the clipboard before failing.
            backup.restore_open()?;
            return Err(format!("写入临时粘贴文本失败，原剪贴板已恢复：{err}"));
        }
        let marker_name: Vec<u16> = "KaixinInput.TemporaryPaste\0".encode_utf16().collect();
        let marker_format = unsafe { RegisterClipboardFormatW(marker_name.as_ptr()) };
        let marker = unsafe { GlobalAlloc(GMEM_MOVEABLE, backup.marker.len()) };
        if !marker.is_null() {
            let ptr = unsafe { GlobalLock(marker) } as *mut u8;
            if ptr.is_null() {
                unsafe {
                    GlobalFree(marker);
                }
                backup.restore_open()?;
                return Err("临时粘贴标记分配失败，原剪贴板已恢复。".into());
            }
            unsafe {
                std::ptr::copy_nonoverlapping(backup.marker.as_ptr(), ptr, backup.marker.len());
                GlobalUnlock(marker);
            }
        }
        if marker_format == 0
            || marker.is_null()
            || unsafe { SetClipboardData(marker_format, marker as _) } == 0
        {
            if !marker.is_null() {
                unsafe {
                    GlobalFree(marker);
                }
            }
            backup.restore_open()?;
            return Err("临时粘贴标记写入失败，原剪贴板已恢复。".into());
        }
        // Windows completes its clipboard sequence update at CloseClipboard.
        // A unique marker additionally protects the interval before sampling
        // that final sequence: a foreign copy can never match this marker.
        drop(_guard);
        backup.sequence = clipboard_win::raw::seq_num().map(|seq| seq.get());
        Ok(backup)
    }

    fn restore_open(&mut self) -> Result<(), String> {
        if unsafe { EmptyClipboard() } == 0 {
            return Err(format!(
                "恢复原剪贴板失败：{}",
                std::io::Error::last_os_error()
            ));
        }
        for saved in &mut self.formats {
            if saved.handle == 0 {
                continue;
            }
            if unsafe { SetClipboardData(saved.format, saved.handle) } == 0 {
                return Err(format!(
                    "恢复剪贴板格式 {} 失败：{}",
                    saved.format,
                    std::io::Error::last_os_error()
                ));
            }
            // SetClipboardData transfers ownership to Windows.
            saved.handle = 0;
        }
        Ok(())
    }

    pub fn restore(mut self) -> Result<(), String> {
        let _guard = clipboard_win::Clipboard::new_attempts_for(self.window.0 as _, 12)
            .map_err(|err| format!("恢复原剪贴板时被其他程序占用：{err}"))?;
        let current = clipboard_win::raw::seq_num().map(|seq| seq.get());
        if self.sequence.is_some() && current == self.sequence && self.owns_temporary_data() {
            self.restore_open()?;
        }
        // Preserve a copy made by the user while the paste was in progress.
        Ok(())
    }

    fn owns_temporary_data(&self) -> bool {
        let name: Vec<u16> = "KaixinInput.TemporaryPaste\0".encode_utf16().collect();
        let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        let handle = unsafe { GetClipboardData(format) };
        if handle == 0
            || unsafe { windows_sys::Win32::System::Memory::GlobalSize(handle as _) }
                < self.marker.len()
        {
            return false;
        }
        let ptr = unsafe { GlobalLock(handle as _) } as *const u8;
        if ptr.is_null() {
            return false;
        }
        let matches = unsafe { std::slice::from_raw_parts(ptr, self.marker.len()) == self.marker };
        unsafe {
            GlobalUnlock(handle as _);
        }
        matches
    }
}

fn paste_marker() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut marker = [0u8; 16];
    marker[..8].copy_from_slice(&u64::from(std::process::id()).to_le_bytes());
    marker[8..].copy_from_slice(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    marker
}
