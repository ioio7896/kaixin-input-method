//! Shared capture recovery for tray and OCR. Each attempt owns a fresh process,
//! so WGC's cached D3D device/items never survive sleep or a display change.
use crate::windows_graphics_capture::{self as wgc, CaptureRect, CapturedFrame, CapturedSource};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const HELPER_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_PIXELS: u64 = 134_217_728;
static DISPLAY_GENERATION: AtomicU64 = AtomicU64::new(0);
static NEXT_CAPTURE: AtomicU64 = AtomicU64::new(0);
pub const INTERRUPTED: &str = "截图框选已中断：";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Wgc,
    Dxgi,
}

impl Backend {
    fn other(self) -> Self {
        match self {
            Self::Wgc => Self::Dxgi,
            Self::Dxgi => Self::Wgc,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    backend: Backend,
    window: Option<isize>,
}

#[derive(Serialize, Deserialize)]
struct FrameHeader {
    schema: u32,
    width: u32,
    height: u32,
    origin_x: i32,
    origin_y: i32,
    elapsed_us: u64,
}

pub struct CaptureOutcome {
    pub frame: CapturedFrame,
    pub backend: Backend,
    pub recovery_note: Option<String>,
}

pub fn display_changed() {
    DISPLAY_GENERATION.fetch_add(1, Ordering::AcqRel);
}

struct Handoff(PathBuf);
impl Handoff {
    fn new() -> Result<Self, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kaixin-frame-{}-{stamp}-{}",
            std::process::id(),
            NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|err| format!("创建截图交接目录失败：{err}"))?;
        Ok(Self(path))
    }
}
impl Drop for Handoff {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

/// Called at process entry, before single-instance/UI initialization. A child
/// never calls capture() again and therefore cannot recursively spawn helpers.
pub fn run_helper_from_args() -> Option<i32> {
    let args: Vec<_> = std::env::args().collect();
    let index = args
        .iter()
        .position(|arg| arg == "--capture-frame-helper" || arg == "--capture-recovery-check")?;
    let Some(path) = args.get(index + 1).map(PathBuf::from) else {
        return Some(2);
    };
    let result = if args[index] == "--capture-recovery-check" {
        run_recovery_check(&path)
    } else {
        run_helper(&path)
    };
    let status = result.as_ref().map(|_| ()).map_err(Clone::clone);
    if fs::write(
        path.join("status.json"),
        serde_json::to_vec(&status).unwrap(),
    )
    .is_err()
    {
        return Some(2);
    }
    Some(if result.is_ok() { 0 } else { 1 })
}

fn run_helper(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, SetThreadDpiAwarenessContext,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // DXGI desktop coordinates are virtualized in a DPI-unaware helper,
        // while xcap reports physical pixels. Set this before either backend
        // enumerates monitors; also cover threads created by the WGC backend.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) == 0 {
            return Err(format!(
                "无法设置截图像素坐标上下文：{}",
                std::io::Error::last_os_error()
            ));
        }
    }
    let request: Request =
        serde_json::from_slice(&fs::read(path.join("request.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let frame = match (request.backend, request.window) {
        (Backend::Wgc, Some(hwnd)) => wgc::capture_window(hwnd),
        (Backend::Wgc, None) => wgc::capture_virtual_desktop(),
        (Backend::Dxgi, Some(hwnd)) => crate::dxgi_capture::capture_window(hwnd),
        (Backend::Dxgi, None) => crate::dxgi_capture::capture_virtual_desktop(),
    }
    .map_err(|err| err.to_string())?;
    write_frame(path, &frame)
}

fn run_recovery_check(path: &Path) -> Result<(), String> {
    let request: Request =
        serde_json::from_slice(&fs::read(path.join("request.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let outcome = capture(request.window, request.backend == Backend::Dxgi, || false)?;
    write_frame(path, &outcome.frame)?;
    fs::write(
        path.join("recovery.json"),
        serde_json::to_vec(&serde_json::json!({
            "backend": outcome.backend, "note": outcome.recovery_note,
        }))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn write_frame(path: &Path, frame: &CapturedFrame) -> Result<(), String> {
    let header = FrameHeader {
        schema: 1,
        width: frame.width(),
        height: frame.height(),
        origin_x: frame.origin_x,
        origin_y: frame.origin_y,
        elapsed_us: frame.elapsed.as_micros().min(u64::MAX as u128) as u64,
    };
    fs::write(path.join("frame.rgba"), frame.image.as_raw()).map_err(|e| e.to_string())?;
    fs::write(
        path.join("frame.json"),
        serde_json::to_vec(&header).unwrap(),
    )
    .map_err(|e| e.to_string())
}

fn fresh_capture(
    backend: Backend,
    window: Option<isize>,
    cancelled: &impl Fn() -> bool,
    generation: u64,
) -> Result<CapturedFrame, String> {
    let handoff = Handoff::new()?;
    fs::write(
        handoff.0.join("request.json"),
        serde_json::to_vec(&Request { backend, window }).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let helper = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("srf_ime_tray.exe");
    let mut command = Command::new(helper);
    command
        .arg("--capture-frame-helper")
        .arg(&handoff.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = OwnedChild(
        command
            .spawn()
            .map_err(|e| format!("启动截图辅助进程失败：{e}"))?,
    );
    let started = Instant::now();
    loop {
        if cancelled() || DISPLAY_GENERATION.load(Ordering::Acquire) != generation {
            return Err(format!(
                "{INTERRUPTED}已取消或显示设备发生变化，请重新截图。"
            ));
        }
        if child.0.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if started.elapsed() >= HELPER_TIMEOUT {
            return Err("截图辅助进程超时，已回收捕获资源。".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let status: Result<(), String> = serde_json::from_slice(
        &fs::read(handoff.0.join("status.json"))
            .map_err(|e| format!("截图辅助进程没有返回结果：{e}"))?,
    )
    .map_err(|e| e.to_string())?;
    status?;
    let header: FrameHeader =
        serde_json::from_slice(&fs::read(handoff.0.join("frame.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let pixels = u64::from(header.width) * u64::from(header.height);
    let raw_path = handoff.0.join("frame.rgba");
    if header.schema != 1
        || pixels == 0
        || pixels > MAX_PIXELS
        || fs::metadata(&raw_path).map_err(|e| e.to_string())?.len() != pixels * 4
    {
        return Err("截图辅助进程返回了无效的图像尺寸。".into());
    }
    let image = RgbaImage::from_raw(
        header.width,
        header.height,
        fs::read(raw_path).map_err(|e| e.to_string())?,
    )
    .ok_or_else(|| "截图图像数据不完整。".to_string())?;
    Ok(CapturedFrame {
        image,
        origin_x: header.origin_x,
        origin_y: header.origin_y,
        source: window
            .map(CapturedSource::Window)
            .unwrap_or(CapturedSource::VirtualDesktop),
        elapsed: Duration::from_micros(header.elapsed_us),
    })
}

// Maximum two attempts. Black pixels are only a retry hint: another fresh
// capture may legitimately also be black (terminal, dark wallpaper, etc.).
fn recover(
    primary: Backend,
    window: bool,
    mut attempt: impl FnMut(Backend) -> Result<CapturedFrame, String>,
) -> Result<CaptureOutcome, String> {
    match attempt(primary) {
        Ok(frame) if wgc::is_probably_blank_capture(&frame.image) => {
            // Desktop backends can independently verify the same screen.
            // Keep isolated WGC window semantics: a black window must not be
            // replaced by a DXGI crop containing an occluding window.
            let retry_backend = if window && primary == Backend::Wgc {
                primary
            } else {
                primary.other()
            };
            let next = attempt(retry_backend);
            match next {
                Err(error) if error.starts_with(INTERRUPTED) => Err(error),
                Ok(recovered) => {
                    let both_dark = wgc::is_probably_blank_capture(&recovered.image);
                    Ok(CaptureOutcome {
                        frame: if both_dark { frame } else { recovered },
                        backend: if both_dark { primary } else { retry_backend },
                        recovery_note: Some(format!("暗色画面已用新资源复核（{primary:?} → {retry_backend:?}）；允许保存合法黑色画面。")),
                    })
                }
                Err(error) => Ok(CaptureOutcome {
                    frame,
                    backend: primary,
                    recovery_note: Some(format!("暗色画面复核未完成，保留原始画面：{error}")),
                }),
            }
        }
        Ok(frame) => Ok(CaptureOutcome {
            frame,
            backend: primary,
            recovery_note: None,
        }),
        Err(error) if error.starts_with(INTERRUPTED) => Err(error),
        Err(error) => {
            let frame = attempt(primary.other()).map_err(|fallback| {
                if fallback.starts_with(INTERRUPTED) {
                    fallback
                } else {
                    format!("{primary:?}：{error}；{:?}：{fallback}", primary.other())
                }
            })?;
            Ok(CaptureOutcome {
                frame,
                backend: primary.other(),
                recovery_note: Some(format!(
                    "{primary:?} 捕获失败，已重建资源并切换后端：{error}"
                )),
            })
        }
    }
}

pub fn capture(
    window: Option<isize>,
    prefer_dxgi: bool,
    cancelled: impl Fn() -> bool,
) -> Result<CaptureOutcome, String> {
    let generation = DISPLAY_GENERATION.load(Ordering::Acquire);
    let result = recover(
        if prefer_dxgi && window.is_none() {
            Backend::Dxgi
        } else {
            Backend::Wgc
        },
        window.is_some(),
        |backend| fresh_capture(backend, window, &cancelled, generation),
    );
    if let Ok(outcome) = &result {
        if let Some(note) = &outcome.recovery_note {
            crate::runtime_log::log_screenshot_health("screenshot_resource_recovery", note);
        }
    }
    result
}

/// Shared postprocessing with checked virtual-desktop coordinates.
pub fn crop(frame: &CapturedFrame, region: CaptureRect) -> Result<RgbaImage, String> {
    let x = i64::from(region.x) - i64::from(frame.origin_x);
    let y = i64::from(region.y) - i64::from(frame.origin_y);
    if region.width == 0
        || region.height == 0
        || x < 0
        || y < 0
        || x + i64::from(region.width) > i64::from(frame.width())
        || y + i64::from(region.height) > i64::from(frame.height())
    {
        return Err("框选区域超出已捕获的虚拟桌面范围。".into());
    }
    Ok(image::imageops::crop_imm(
        &frame.image,
        x as u32,
        y as u32,
        region.width,
        region.height,
    )
    .to_image())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(value: u8) -> CapturedFrame {
        CapturedFrame {
            image: RgbaImage::from_pixel(4, 4, image::Rgba([value, value, value, 255])),
            origin_x: -4,
            origin_y: 0,
            source: CapturedSource::VirtualDesktop,
            elapsed: Duration::ZERO,
        }
    }
    #[test]
    fn legal_black_frame_survives_bounded_fresh_retry() {
        let mut calls = 0;
        let result = recover(Backend::Wgc, false, |backend| {
            calls += 1;
            assert_eq!(
                backend,
                if calls == 1 {
                    Backend::Wgc
                } else {
                    Backend::Dxgi
                }
            );
            Ok(frame(0))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result.frame.image.get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn blank_frame_is_replaced_after_fresh_capture_and_retry_can_be_cancelled() {
        let mut calls = 0;
        let result = recover(Backend::Wgc, false, |_| {
            calls += 1;
            Ok(frame(if calls == 1 { 0 } else { 100 }))
        })
        .unwrap();
        assert_eq!(result.frame.image.get_pixel(0, 0).0, [100, 100, 100, 255]);
        let mut calls = 0;
        assert!(recover(Backend::Wgc, false, |_| {
            calls += 1;
            if calls == 1 {
                Ok(frame(0))
            } else {
                Err(format!("{INTERRUPTED}显示器变化"))
            }
        })
        .is_err());
        assert_eq!(calls, 2);
    }
    #[test]
    fn device_loss_uses_fallback_but_cancellation_does_not() {
        let mut backends = Vec::new();
        recover(Backend::Wgc, false, |backend| {
            backends.push(backend);
            if backend == Backend::Wgc {
                Err("device removed".into())
            } else {
                Ok(frame(100))
            }
        })
        .unwrap();
        assert_eq!(backends, [Backend::Wgc, Backend::Dxgi]);
        let mut calls = 0;
        assert!(recover(Backend::Wgc, false, |_| {
            calls += 1;
            Err(format!("{INTERRUPTED}取消"))
        })
        .is_err());
        assert_eq!(calls, 1);
    }
    #[test]
    fn dxgi_black_desktop_recovers_through_wgc_and_black_windows_keep_isolation() {
        let mut calls = Vec::new();
        let outcome = recover(Backend::Dxgi, false, |backend| {
            calls.push(backend);
            Ok(frame(if backend == Backend::Dxgi { 0 } else { 100 }))
        })
        .unwrap();
        assert_eq!(calls, [Backend::Dxgi, Backend::Wgc]);
        assert_eq!(outcome.backend, Backend::Wgc);
        assert!(!wgc::is_probably_blank_capture(&outcome.frame.image));
        let mut calls = Vec::new();
        recover(Backend::Wgc, true, |backend| {
            calls.push(backend);
            Ok(frame(0))
        })
        .unwrap();
        assert_eq!(calls, [Backend::Wgc, Backend::Wgc]);
    }

    #[test]
    fn crop_handles_negative_monitor_origin_and_rejects_outside_region() {
        assert_eq!(
            crop(&frame(100), CaptureRect::new(-3, 1, 2, 2))
                .unwrap()
                .dimensions(),
            (2, 2)
        );
        assert!(crop(&frame(100), CaptureRect::new(-5, 0, 2, 2)).is_err());
    }
}
