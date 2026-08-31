//! Windows Graphics Capture (WGC) one-frame screenshot backend.
//!
//! The module deliberately does not read user configuration or invoke another
//! screenshot program.  Callers can therefore select WGC as the primary
//! backend and decide whether a typed failure should fall back to DXGI,
//! the native region selector, or `ms-screenclip`.

use image::{Rgba, RgbaImage};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Maximum output allocation accepted from a caller-provided desktop region.
/// This still permits an 8K dual-monitor desktop while rejecting accidental
/// enormous rectangles before allocating hundreds of megabytes.
const MAX_CAPTURE_PIXELS: u64 = 134_217_728;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CaptureRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    fn validate(self) -> Result<Self, CaptureError> {
        if self.width == 0 || self.height == 0 {
            return Err(CaptureError::new(
                CaptureErrorKind::InvalidTarget,
                "截图区域的宽度和高度必须大于 0。",
            ));
        }
        let pixels = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or_else(|| {
                CaptureError::new(CaptureErrorKind::InvalidTarget, "截图区域尺寸溢出。")
            })?;
        if pixels > MAX_CAPTURE_PIXELS {
            return Err(CaptureError::new(
                CaptureErrorKind::InvalidTarget,
                format!(
                    "截图区域过大（{}×{}），已拒绝分配图像缓冲区。",
                    self.width, self.height
                ),
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonitorInfo {
    /// Native `HMONITOR` value truncated in the same way as xcap's public ID.
    pub id: u32,
    pub name: String,
    pub bounds: CaptureRect,
    pub is_primary: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureTarget {
    /// Native `HWND` represented as an `isize` to stay compatible with the
    /// existing tray/OCR process boundary.
    Window(isize),
    Monitor(u32),
    PrimaryMonitor,
    VirtualDesktop,
    /// Coordinates are in the Windows virtual-desktop coordinate system and
    /// may be negative. Regions crossing monitors are stitched together.
    DesktopRegion(CaptureRect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapturedSource {
    Window(isize),
    Monitor(u32),
    VirtualDesktop,
    DesktopRegion,
}

#[derive(Debug)]
pub struct CapturedFrame {
    pub image: RgbaImage,
    /// Virtual-desktop origin. Window/monitor captures use the target bounds;
    /// callers can use this to map an overlay selection back to screen space.
    pub origin_x: i32,
    pub origin_y: i32,
    pub source: CapturedSource,
    pub elapsed: Duration,
}

impl CapturedFrame {
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }

    pub fn bounds(&self) -> CaptureRect {
        CaptureRect::new(self.origin_x, self.origin_y, self.width(), self.height())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureErrorKind {
    /// WGC is unavailable (notably before Windows 10 version 1903) or disabled.
    Unsupported,
    InvalidTarget,
    /// Windows explicitly denied access. Protected content can also appear as
    /// an opaque/black frame without an HRESULT; WGC intentionally cannot
    /// bypass that operating-system protection.
    ProtectedContent,
    Timeout,
    Backend,
    Save,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureError {
    pub kind: CaptureErrorKind,
    pub message: String,
    pub hresult: Option<i32>,
}

impl CaptureError {
    fn new(kind: CaptureErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hresult: None,
        }
    }

    fn with_hresult(kind: CaptureErrorKind, message: impl Into<String>, hresult: i32) -> Self {
        Self {
            kind,
            message: message.into(),
            hresult: Some(hresult),
        }
    }

    /// Whether an upper layer should try its configured fallback backend.
    pub fn should_fallback(&self) -> bool {
        matches!(
            self.kind,
            CaptureErrorKind::Unsupported
                | CaptureErrorKind::ProtectedContent
                | CaptureErrorKind::Timeout
                | CaptureErrorKind::Backend
        )
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.hresult {
            write!(f, "{} (HRESULT=0x{:08X})", self.message, code as u32)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for CaptureError {}

#[derive(Debug)]
pub struct SavedCapture {
    pub path: PathBuf,
    pub bounds: CaptureRect,
    pub source: CapturedSource,
    pub elapsed: Duration,
}

/// Performs a lightweight runtime support probe. A successful probe does not
/// guarantee that every target is capturable (secure/protected surfaces remain
/// controlled by Windows).
#[cfg(windows)]
pub fn is_supported() -> bool {
    windows::Graphics::Capture::GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_supported() -> bool {
    false
}

#[cfg(windows)]
pub fn monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    monitor_pairs().map(|pairs| pairs.into_iter().map(|(_, info)| info).collect())
}

#[cfg(not(windows))]
pub fn monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    Err(unsupported_platform())
}

pub fn capture(target: CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    capture_impl(target)
}

/// Captures one top-level window by its native `HWND`.
pub fn capture_window(hwnd: isize) -> Result<CapturedFrame, CaptureError> {
    capture(CaptureTarget::Window(hwnd))
}

/// Captures one monitor using the ID returned by [`monitors`].
pub fn capture_monitor(id: u32) -> Result<CapturedFrame, CaptureError> {
    capture(CaptureTarget::Monitor(id))
}

/// Captures and stitches all active monitors into one virtual-desktop image.
pub fn capture_virtual_desktop() -> Result<CapturedFrame, CaptureError> {
    capture(CaptureTarget::VirtualDesktop)
}

/// Captures a global-coordinate region, stitching intersecting monitors.
pub fn capture_desktop_region(rect: CaptureRect) -> Result<CapturedFrame, CaptureError> {
    capture(CaptureTarget::DesktopRegion(rect))
}

pub fn capture_to_png(
    target: CaptureTarget,
    destination: impl AsRef<Path>,
) -> Result<SavedCapture, CaptureError> {
    let started = Instant::now();
    let frame = capture(target)?;
    let bounds = frame.bounds();
    let source = frame.source;
    let path = destination.as_ref().to_path_buf();
    save_png(&frame.image, &path)?;
    Ok(SavedCapture {
        path,
        bounds,
        source,
        elapsed: started.elapsed(),
    })
}

/// Saves through a sibling temporary file and renames it after PNG encoding so
/// OCR never observes a partially written screenshot.
pub fn save_png(image: &RgbaImage, destination: &Path) -> Result<(), CaptureError> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent).map_err(|err| {
            CaptureError::new(
                CaptureErrorKind::Save,
                format!("创建截图目录失败（{}）：{err}", parent.display()),
            )
        })?;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("capture.png");
    let temporary =
        destination.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), stamp));

    let result = (|| {
        image
            .save_with_format(&temporary, image::ImageFormat::Png)
            .map_err(|err| {
                CaptureError::new(
                    CaptureErrorKind::Save,
                    format!("编码截图失败（{}）：{err}", destination.display()),
                )
            })?;
        std::fs::rename(&temporary, destination).map_err(|err| {
            CaptureError::new(
                CaptureErrorKind::Save,
                format!("提交截图文件失败（{}）：{err}", destination.display()),
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn capture_impl(target: CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    if !is_supported() {
        return Err(CaptureError::new(
            CaptureErrorKind::Unsupported,
            "当前 Windows 版本或系统策略不支持 Windows Graphics Capture（需要 Windows 10 1903+）。",
        ));
    }

    let started = Instant::now();
    let mut frame = match target {
        CaptureTarget::Window(hwnd) => capture_window_impl(hwnd)?,
        CaptureTarget::Monitor(id) => capture_monitor_impl(id)?,
        CaptureTarget::PrimaryMonitor => capture_primary_monitor_impl()?,
        CaptureTarget::VirtualDesktop => capture_virtual_desktop_impl()?,
        CaptureTarget::DesktopRegion(rect) => capture_desktop_region_impl(rect)?,
    };
    normalize_opaque_capture(&mut frame.image);
    frame.elapsed = started.elapsed();
    Ok(frame)
}

#[cfg(not(windows))]
fn capture_impl(_target: CaptureTarget) -> Result<CapturedFrame, CaptureError> {
    Err(unsupported_platform())
}

#[cfg(windows)]
fn capture_window_impl(hwnd: isize) -> Result<CapturedFrame, CaptureError> {
    if hwnd == 0 {
        return Err(CaptureError::new(
            CaptureErrorKind::InvalidTarget,
            "窗口句柄为空。",
        ));
    }
    let id = hwnd as u32;
    let window = xcap::Window::all()
        .map_err(map_xcap_error)?
        .into_iter()
        .find(|window| window.id().ok() == Some(id))
        .ok_or_else(|| {
            CaptureError::new(
                CaptureErrorKind::InvalidTarget,
                format!("找不到可捕获窗口（HWND=0x{:X}）。", hwnd as usize),
            )
        })?;
    if window.is_minimized().unwrap_or(false) {
        return Err(CaptureError::new(
            CaptureErrorKind::InvalidTarget,
            "目标窗口已最小化，WGC 无法获得稳定的当前帧。",
        ));
    }
    let origin_x = window.x().map_err(map_xcap_error)?;
    let origin_y = window.y().map_err(map_xcap_error)?;
    let image = window.capture_image().map_err(map_xcap_error)?;
    validate_non_empty_frame(&image)?;
    Ok(CapturedFrame {
        image,
        origin_x,
        origin_y,
        source: CapturedSource::Window(hwnd),
        elapsed: Duration::ZERO,
    })
}

#[cfg(windows)]
fn capture_monitor_impl(id: u32) -> Result<CapturedFrame, CaptureError> {
    let (monitor, info) = monitor_pairs()?
        .into_iter()
        .find(|(_, info)| info.id == id)
        .ok_or_else(|| {
            CaptureError::new(
                CaptureErrorKind::InvalidTarget,
                format!("找不到显示器（ID=0x{id:X}）。"),
            )
        })?;
    let image = monitor.capture_image().map_err(map_xcap_error)?;
    validate_non_empty_frame(&image)?;
    Ok(CapturedFrame {
        image,
        origin_x: info.bounds.x,
        origin_y: info.bounds.y,
        source: CapturedSource::Monitor(id),
        elapsed: Duration::ZERO,
    })
}

#[cfg(windows)]
fn capture_primary_monitor_impl() -> Result<CapturedFrame, CaptureError> {
    let primary = monitor_pairs()?
        .into_iter()
        .find(|(_, info)| info.is_primary)
        .ok_or_else(|| CaptureError::new(CaptureErrorKind::InvalidTarget, "没有找到主显示器。"))?;
    let id = primary.1.id;
    let image = primary.0.capture_image().map_err(map_xcap_error)?;
    validate_non_empty_frame(&image)?;
    Ok(CapturedFrame {
        image,
        origin_x: primary.1.bounds.x,
        origin_y: primary.1.bounds.y,
        source: CapturedSource::Monitor(id),
        elapsed: Duration::ZERO,
    })
}

#[cfg(windows)]
fn capture_virtual_desktop_impl() -> Result<CapturedFrame, CaptureError> {
    let pairs = monitor_pairs()?;
    let bounds = virtual_bounds(pairs.iter().map(|(_, info)| info.bounds))?;
    capture_region_from_pairs(bounds, pairs, CapturedSource::VirtualDesktop)
}

#[cfg(windows)]
fn capture_desktop_region_impl(rect: CaptureRect) -> Result<CapturedFrame, CaptureError> {
    capture_region_from_pairs(
        rect.validate()?,
        monitor_pairs()?,
        CapturedSource::DesktopRegion,
    )
}

#[cfg(windows)]
fn capture_region_from_pairs(
    rect: CaptureRect,
    pairs: Vec<(xcap::Monitor, MonitorInfo)>,
    source: CapturedSource,
) -> Result<CapturedFrame, CaptureError> {
    let rect = rect.validate()?;
    let mut output = RgbaImage::from_pixel(rect.width, rect.height, Rgba([0, 0, 0, 255]));
    let mut captured_any = false;

    for (monitor, info) in pairs {
        let Some(intersection) = intersection(rect, info.bounds) else {
            continue;
        };
        let local_x = (i64::from(intersection.x) - i64::from(info.bounds.x)) as u32;
        let local_y = (i64::from(intersection.y) - i64::from(info.bounds.y)) as u32;
        let part = monitor
            .capture_region(local_x, local_y, intersection.width, intersection.height)
            .map_err(map_xcap_error)?;
        validate_non_empty_frame(&part)?;
        let output_x = i64::from(intersection.x) - i64::from(rect.x);
        let output_y = i64::from(intersection.y) - i64::from(rect.y);
        image::imageops::replace(&mut output, &part, output_x, output_y);
        captured_any = true;
    }

    if !captured_any {
        return Err(CaptureError::new(
            CaptureErrorKind::InvalidTarget,
            "截图区域没有与任何活动显示器相交。",
        ));
    }
    Ok(CapturedFrame {
        image: output,
        origin_x: rect.x,
        origin_y: rect.y,
        source,
        elapsed: Duration::ZERO,
    })
}

#[cfg(windows)]
fn monitor_pairs() -> Result<Vec<(xcap::Monitor, MonitorInfo)>, CaptureError> {
    let monitors = xcap::Monitor::all().map_err(map_xcap_error)?;
    let mut result = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let id = monitor.id().map_err(map_xcap_error)?;
        let x = monitor.x().map_err(map_xcap_error)?;
        let y = monitor.y().map_err(map_xcap_error)?;
        let width = monitor.width().map_err(map_xcap_error)?;
        let height = monitor.height().map_err(map_xcap_error)?;
        let name = monitor
            .friendly_name()
            .or_else(|_| monitor.name())
            .unwrap_or_else(|_| format!("显示器 {id:X}"));
        let is_primary = monitor.is_primary().unwrap_or(false);
        result.push((
            monitor,
            MonitorInfo {
                id,
                name,
                bounds: CaptureRect::new(x, y, width, height),
                is_primary,
            },
        ));
    }
    if result.is_empty() {
        return Err(CaptureError::new(
            CaptureErrorKind::InvalidTarget,
            "没有找到活动显示器。",
        ));
    }
    Ok(result)
}

fn virtual_bounds(
    rects: impl IntoIterator<Item = CaptureRect>,
) -> Result<CaptureRect, CaptureError> {
    let mut iter = rects.into_iter();
    let first = iter.next().ok_or_else(|| {
        CaptureError::new(CaptureErrorKind::InvalidTarget, "没有找到活动显示器。")
    })?;
    let mut left = i64::from(first.x);
    let mut top = i64::from(first.y);
    let mut right = first.right();
    let mut bottom = first.bottom();
    for rect in iter {
        left = left.min(i64::from(rect.x));
        top = top.min(i64::from(rect.y));
        right = right.max(rect.right());
        bottom = bottom.max(rect.bottom());
    }
    let width = u32::try_from(right - left)
        .map_err(|_| CaptureError::new(CaptureErrorKind::InvalidTarget, "虚拟桌面宽度溢出。"))?;
    let height = u32::try_from(bottom - top)
        .map_err(|_| CaptureError::new(CaptureErrorKind::InvalidTarget, "虚拟桌面高度溢出。"))?;
    CaptureRect::new(left as i32, top as i32, width, height).validate()
}

fn intersection(left: CaptureRect, right: CaptureRect) -> Option<CaptureRect> {
    let x1 = i64::from(left.x).max(i64::from(right.x));
    let y1 = i64::from(left.y).max(i64::from(right.y));
    let x2 = left.right().min(right.right());
    let y2 = left.bottom().min(right.bottom());
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(CaptureRect::new(
        x1 as i32,
        y1 as i32,
        (x2 - x1) as u32,
        (y2 - y1) as u32,
    ))
}

fn validate_non_empty_frame(image: &RgbaImage) -> Result<(), CaptureError> {
    if image.width() == 0 || image.height() == 0 {
        return Err(CaptureError::new(
            CaptureErrorKind::Backend,
            "Windows Graphics Capture 返回了空图像。",
        ));
    }
    Ok(())
}

/// Desktop capture surfaces sometimes leave alpha undefined even though their
/// RGB channels already contain the final, non-premultiplied desktop colour.
/// Treating that alpha as a premultiplication factor amplifies RGB and produces
/// severe cyan/magenta clipping. Preserve RGB exactly and normalize alpha only.
fn normalize_opaque_capture(image: &mut RgbaImage) {
    for pixel in image.pixels_mut() {
        pixel[3] = 255;
    }
}

#[cfg(windows)]
fn map_xcap_error(error: xcap::XCapError) -> CaptureError {
    use xcap::XCapError;

    match error {
        XCapError::NotSupported => CaptureError::new(
            CaptureErrorKind::Unsupported,
            "Windows Graphics Capture 不受支持。",
        ),
        XCapError::InvalidCaptureRegion(message) => {
            CaptureError::new(CaptureErrorKind::InvalidTarget, message)
        }
        XCapError::StdSyncMpscRecvTimeoutError(error) => CaptureError::new(
            CaptureErrorKind::Timeout,
            format!("等待 Windows Graphics Capture 帧超时：{error}"),
        ),
        XCapError::WindowsCoreError(error) => {
            let code = error.code().0;
            let kind = match code as u32 {
                0x8007_0005 => CaptureErrorKind::ProtectedContent, // E_ACCESSDENIED
                0x8004_0154 | 0x8000_4002 => CaptureErrorKind::Unsupported, // class/no interface
                0x8007_0057 => CaptureErrorKind::InvalidTarget,    // E_INVALIDARG
                _ => CaptureErrorKind::Backend,
            };
            CaptureError::with_hresult(kind, error.to_string(), code)
        }
        other => CaptureError::new(CaptureErrorKind::Backend, other.to_string()),
    }
}

#[cfg(not(windows))]
fn unsupported_platform() -> CaptureError {
    CaptureError::new(
        CaptureErrorKind::Unsupported,
        "Windows Graphics Capture 仅支持 Windows 10 1903 及更高版本。",
    )
}
