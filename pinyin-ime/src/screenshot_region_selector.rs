//! Native region-selection overlay for an already captured desktop frame.
//!
//! The selector is deliberately independent from the capture backend. WGC,
//! DXGI duplication, or tests can provide an RGBA image plus its origin in the
//! Windows virtual-desktop coordinate space. The returned [`CaptureRect`]
//! uses the same global coordinates, including negative monitor positions.

use crate::windows_graphics_capture::{CaptureRect, CapturedFrame};
use image::RgbaImage;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegionSelectorError {
    message: String,
}

#[derive(Clone, Copy, Debug)]
pub struct RegionSelectorOptions {
    pub confirm_on_release: bool,
    pub show_instructions: bool,
}

impl Default for RegionSelectorOptions {
    fn default() -> Self {
        Self {
            confirm_on_release: false,
            show_instructions: true,
        }
    }
}

impl RegionSelectorError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RegionSelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RegionSelectorError {}

/// Opens a modal, topmost selector over the bounds represented by `frame`.
///
/// `Ok(Some(rect))` is a confirmed global-coordinate selection, `Ok(None)`
/// means that the user pressed Escape/right-clicked/closed the overlay, and an
/// `Err` indicates that the native overlay could not be created or painted.
pub fn select_region(frame: &CapturedFrame) -> Result<Option<CaptureRect>, RegionSelectorError> {
    select_region_with_options(frame, RegionSelectorOptions::default())
}

pub fn select_region_with_options(
    frame: &CapturedFrame,
    options: RegionSelectorOptions,
) -> Result<Option<CaptureRect>, RegionSelectorError> {
    select_region_from_image_with_options(&frame.image, frame.origin_x, frame.origin_y, options)
}

/// Opens the selector for an RGBA virtual-desktop image.
///
/// `origin_x` and `origin_y` are physical-pixel coordinates in the Windows
/// virtual desktop. They can be negative when a monitor is located to the left
/// of or above the primary display.
pub fn select_region_from_image(
    image: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
) -> Result<Option<CaptureRect>, RegionSelectorError> {
    select_region_from_image_with_options(
        image,
        origin_x,
        origin_y,
        RegionSelectorOptions::default(),
    )
}

pub fn select_region_from_image_with_options(
    image: &RgbaImage,
    origin_x: i32,
    origin_y: i32,
    options: RegionSelectorOptions,
) -> Result<Option<CaptureRect>, RegionSelectorError> {
    platform::select_region_from_image(image, origin_x, origin_y, options)
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn select_region_from_image(
        _image: &RgbaImage,
        _origin_x: i32,
        _origin_y: i32,
        _options: RegionSelectorOptions,
    ) -> Result<Option<CaptureRect>, RegionSelectorError> {
        Err(RegionSelectorError::new("截图区域选择器仅支持 Windows。"))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::null;
    use std::sync::mpsc::{self, Receiver, TryRecvError};
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows_sys::Win32::Graphics::Gdi::{
        AlphaBlend, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
        CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect,
        GetStockObject, IntersectClipRect, InvalidateRect, Rectangle, RestoreDC, SaveDC,
        ScreenToClient, SelectObject, SetBkMode, SetTextColor, StretchDIBits, UpdateWindow,
        AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, CLIP_DEFAULT_PRECIS,
        DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, FF_DONTCARE, FW_SEMIBOLD, NULL_BRUSH,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, PROOF_QUALITY, PS_SOLID, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F1,
        VK_F2, VK_F3, VK_F4, VK_F5, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumChildWindows,
        EnumWindows, GetClientRect, GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowRect,
        IsIconic, IsWindow, IsWindowVisible, LoadCursorW, RegisterClassW, SetCursor,
        SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
        CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, IDC_CROSS,
        MSG, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SW_SHOW, WM_CLOSE, WM_DISPLAYCHANGE, WM_DPICHANGED,
        WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN,
        WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    const CLASS_NAME: &str = "KaixinScreenshotRegionSelector_v1";
    const WINDOW_TITLE: &str = "开心输入法截图区域选择";

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct LocalRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[derive(Clone, Copy, Debug)]
    struct SnapCandidate {
        rect: LocalRect,
        is_control: bool,
        z_rank: usize,
    }

    #[derive(Clone, Copy, Debug)]
    enum DragKind {
        Create,
        Move { offset_x: i32, offset_y: i32 },
        Resize { horizontal: i8, vertical: i8 },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SelectorAction {
        Confirm,
        Cancel,
    }

    struct ActionToolbarLayout {
        info: RECT,
        confirm: RECT,
        cancel: RECT,
    }

    impl LocalRect {
        fn from_points(anchor: POINT, current: POINT) -> Self {
            Self {
                left: anchor.x.min(current.x),
                top: anchor.y.min(current.y),
                right: anchor.x.max(current.x),
                bottom: anchor.y.max(current.y),
            }
        }

        fn width(self) -> i32 {
            self.right.saturating_sub(self.left)
        }

        fn height(self) -> i32 {
            self.bottom.saturating_sub(self.top)
        }

        fn is_valid(self) -> bool {
            self.width() > 0 && self.height() > 0
        }
    }

    fn point_in_rect(point: POINT, rect: LocalRect) -> bool {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    struct CandidateCollector {
        origin_x: i32,
        origin_y: i32,
        desktop: LocalRect,
        candidates: Vec<SnapCandidate>,
        top_windows: Vec<(HWND, usize)>,
    }

    fn enumerate_snap_candidates(
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
    ) -> (Vec<SnapCandidate>, Receiver<Vec<SnapCandidate>>) {
        let mut collector = CandidateCollector {
            origin_x,
            origin_y,
            desktop: LocalRect {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            candidates: Vec::new(),
            top_windows: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(
                Some(collect_top_level_candidate),
                (&mut collector as *mut CandidateCollector) as LPARAM,
            );
        }
        sort_snap_candidates(&mut collector.candidates);
        let uia_candidates = start_uia_candidate_scan(&collector);
        (collector.candidates, uia_candidates)
    }

    fn sort_snap_candidates(candidates: &mut Vec<SnapCandidate>) {
        candidates.sort_by_key(|candidate| {
            let area = i64::from(candidate.rect.width()) * i64::from(candidate.rect.height());
            (
                candidate.z_rank,
                !candidate.is_control,
                area,
                candidate.rect.left,
                candidate.rect.top,
                candidate.rect.right,
                candidate.rect.bottom,
            )
        });
        candidates
            .dedup_by(|left, right| left.rect == right.rect && left.is_control == right.is_control);
    }

    unsafe extern "system" fn collect_top_level_candidate(hwnd: HWND, lparam: LPARAM) -> i32 {
        let collector = &mut *(lparam as *mut CandidateCollector);
        if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1;
        }
        if let Some(rect) = window_local_rect(hwnd, collector) {
            let z_rank = collector.top_windows.len();
            collector.top_windows.push((hwnd, z_rank));
            collector.candidates.push(SnapCandidate {
                rect,
                is_control: false,
                z_rank,
            });
            let _ = EnumChildWindows(hwnd, Some(collect_child_candidate), lparam);
        }
        1
    }

    unsafe extern "system" fn collect_child_candidate(hwnd: HWND, lparam: LPARAM) -> i32 {
        let collector = &mut *(lparam as *mut CandidateCollector);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        if let Some(rect) = window_local_rect(hwnd, collector) {
            if rect.width() >= 8 && rect.height() >= 8 {
                let z_rank = collector.top_windows.len().saturating_sub(1);
                collector.candidates.push(SnapCandidate {
                    rect,
                    is_control: true,
                    z_rank,
                });
            }
        }
        1
    }

    fn start_uia_candidate_scan(collector: &CandidateCollector) -> Receiver<Vec<SnapCandidate>> {
        let windows = collector.top_windows.clone();
        let origin_x = collector.origin_x;
        let origin_y = collector.origin_y;
        let desktop = collector.desktop;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _dpi_guard = DpiContextGuard::enter();
            let _ = sender.send(enumerate_uia_candidates(
                windows, origin_x, origin_y, desktop,
            ));
        });
        receiver
    }

    fn enumerate_uia_candidates(
        top_windows: Vec<(HWND, usize)>,
        origin_x: i32,
        origin_y: i32,
        desktop: LocalRect,
    ) -> Vec<SnapCandidate> {
        use windows::Win32::Foundation::HWND as WinHwnd;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        };
        use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation};

        const MAX_UIA_ELEMENTS: usize = 2_000;
        let mut result = Vec::new();
        unsafe {
            if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                return result;
            }
            struct ComGuard;
            impl Drop for ComGuard {
                fn drop(&mut self) {
                    unsafe { CoUninitialize() };
                }
            }
            let _guard = ComGuard;
            let Ok(automation): Result<IUIAutomation, _> =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            else {
                return result;
            };
            let Ok(walker) = automation.RawViewWalker() else {
                return result;
            };
            for (hwnd, z_rank) in top_windows {
                if result.len() >= MAX_UIA_ELEMENTS {
                    break;
                }
                let Ok(root) = automation.ElementFromHandle(WinHwnd(hwnd as *mut _)) else {
                    continue;
                };
                let mut stack = Vec::new();
                if let Ok(child) = walker.GetFirstChildElement(&root) {
                    stack.push(child);
                }
                while let Some(element) = stack.pop() {
                    if result.len() >= MAX_UIA_ELEMENTS {
                        break;
                    }
                    if element
                        .CurrentIsOffscreen()
                        .map(|value| !value.as_bool())
                        .unwrap_or(false)
                    {
                        if let Ok(rect) = element.CurrentBoundingRectangle() {
                            let local = LocalRect {
                                left: rect.left.saturating_sub(origin_x).max(desktop.left),
                                top: rect.top.saturating_sub(origin_y).max(desktop.top),
                                right: rect.right.saturating_sub(origin_x).min(desktop.right),
                                bottom: rect.bottom.saturating_sub(origin_y).min(desktop.bottom),
                            };
                            if local.width() >= 8 && local.height() >= 8 {
                                result.push(SnapCandidate {
                                    rect: local,
                                    is_control: true,
                                    z_rank,
                                });
                            }
                        }
                    }
                    if let Ok(sibling) = walker.GetNextSiblingElement(&element) {
                        stack.push(sibling);
                    }
                    if let Ok(child) = walker.GetFirstChildElement(&element) {
                        stack.push(child);
                    }
                }
            }
        }
        result
    }

    unsafe fn window_local_rect(hwnd: HWND, collector: &CandidateCollector) -> Option<LocalRect> {
        let mut global: RECT = zeroed();
        let dwm_result = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut global as *mut RECT).cast::<c_void>(),
            size_of::<RECT>() as u32,
        );
        if dwm_result < 0 && GetWindowRect(hwnd, &mut global) == 0 {
            return None;
        }
        let rect = LocalRect {
            left: global.left.saturating_sub(collector.origin_x),
            top: global.top.saturating_sub(collector.origin_y),
            right: global.right.saturating_sub(collector.origin_x),
            bottom: global.bottom.saturating_sub(collector.origin_y),
        };
        let clipped = LocalRect {
            left: rect.left.max(collector.desktop.left),
            top: rect.top.max(collector.desktop.top),
            right: rect.right.min(collector.desktop.right),
            bottom: rect.bottom.min(collector.desktop.bottom),
        };
        clipped.is_valid().then_some(clipped)
    }

    struct SelectorState {
        origin_x: i32,
        origin_y: i32,
        image_width: i32,
        image_height: i32,
        bitmap_info: BITMAPINFO,
        original_bgra: Vec<u8>,
        anchor: POINT,
        current: POINT,
        cursor: POINT,
        selection: Option<LocalRect>,
        snap_candidates: Vec<SnapCandidate>,
        uia_candidates: Option<Receiver<Vec<SnapCandidate>>>,
        snap_matches: Vec<usize>,
        snap_match_index: usize,
        pressed_snap: Option<LocalRect>,
        drag_kind: DragKind,
        drag_start_selection: Option<LocalRect>,
        dragging: bool,
        manual_selection_locked: bool,
        confirm_on_release: bool,
        show_instructions: bool,
        fixed_size: Option<(i32, i32)>,
        locked_ratio: Option<(u32, u32)>,
        back_buffer_dc: isize,
        back_buffer: isize,
        back_buffer_old_bitmap: isize,
        background_dc: isize,
        background_bitmap: isize,
        background_old_bitmap: isize,
        dim_dc: isize,
        dim_bitmap: isize,
        dim_old_bitmap: isize,
        completed: bool,
        cancelled: bool,
        result: Option<CaptureRect>,
    }

    impl SelectorState {
        fn new(
            image: &RgbaImage,
            origin_x: i32,
            origin_y: i32,
            options: RegionSelectorOptions,
        ) -> Result<Self, RegionSelectorError> {
            let image_width = i32::try_from(image.width()).map_err(|_| {
                RegionSelectorError::new("截图宽度超过 Win32 覆盖窗口可支持的范围。")
            })?;
            let image_height = i32::try_from(image.height()).map_err(|_| {
                RegionSelectorError::new("截图高度超过 Win32 覆盖窗口可支持的范围。")
            })?;
            if image_width <= 0 || image_height <= 0 {
                return Err(RegionSelectorError::new("无法为零尺寸截图创建框选窗口。"));
            }
            let right = i64::from(origin_x) + i64::from(image_width);
            let bottom = i64::from(origin_y) + i64::from(image_height);
            if right > i64::from(i32::MAX)
                || right < i64::from(i32::MIN)
                || bottom > i64::from(i32::MAX)
                || bottom < i64::from(i32::MIN)
            {
                return Err(RegionSelectorError::new(
                    "截图范围超出 Win32 虚拟桌面坐标范围。",
                ));
            }

            let required_bytes = usize::try_from(image.width())
                .ok()
                .and_then(|width| {
                    usize::try_from(image.height())
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| RegionSelectorError::new("截图预览缓冲区尺寸溢出。"))?;
            if image.as_raw().len() != required_bytes {
                return Err(RegionSelectorError::new("截图 RGBA 缓冲区尺寸不完整。"));
            }

            let mut original_bgra = Vec::with_capacity(required_bytes);
            for pixel in image.as_raw().chunks_exact(4) {
                let red = pixel[0];
                let green = pixel[1];
                let blue = pixel[2];
                original_bgra.extend_from_slice(&[blue, green, red, 255]);
            }

            let mut bitmap_info: BITMAPINFO = unsafe { zeroed() };
            bitmap_info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: image_width,
                // A negative height makes this a top-down DIB, matching image's
                // row order and avoiding a full vertical flip.
                biHeight: -image_height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: required_bytes.min(u32::MAX as usize) as u32,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };

            let (snap_candidates, uia_candidates) =
                enumerate_snap_candidates(origin_x, origin_y, image_width, image_height);
            Ok(Self {
                origin_x,
                origin_y,
                image_width,
                image_height,
                bitmap_info,
                original_bgra,
                anchor: POINT { x: 0, y: 0 },
                current: POINT { x: 0, y: 0 },
                cursor: POINT {
                    x: image_width / 2,
                    y: image_height / 2,
                },
                selection: None,
                snap_candidates,
                uia_candidates: Some(uia_candidates),
                snap_matches: Vec::new(),
                snap_match_index: 0,
                pressed_snap: None,
                drag_kind: DragKind::Create,
                drag_start_selection: None,
                dragging: false,
                manual_selection_locked: false,
                confirm_on_release: options.confirm_on_release,
                show_instructions: options.show_instructions,
                fixed_size: None,
                locked_ratio: None,
                back_buffer_dc: 0,
                back_buffer: 0,
                back_buffer_old_bitmap: 0,
                background_dc: 0,
                background_bitmap: 0,
                background_old_bitmap: 0,
                dim_dc: 0,
                dim_bitmap: 0,
                dim_old_bitmap: 0,
                completed: false,
                cancelled: false,
                result: None,
            })
        }

        fn clamp_point(&self, mut point: POINT) -> POINT {
            point.x = point.x.clamp(0, self.image_width);
            point.y = point.y.clamp(0, self.image_height);
            point
        }

        fn update_selection(&mut self, point: POINT, lock_ratio: bool) {
            self.current = self.clamp_point(point);
            if let Some((width, height)) = self.fixed_size {
                self.selection = Some(self.fixed_rect(self.current, width, height));
                return;
            }
            let raw = if alt_down() {
                let dx = (self.current.x - self.anchor.x).abs();
                let dy = (self.current.y - self.anchor.y).abs();
                LocalRect {
                    left: self.anchor.x.saturating_sub(dx),
                    top: self.anchor.y.saturating_sub(dy),
                    right: self.anchor.x.saturating_add(dx),
                    bottom: self.anchor.y.saturating_add(dy),
                }
            } else {
                LocalRect::from_points(self.anchor, self.current)
            };
            let selection = if lock_ratio {
                self.ratio_rect(raw)
            } else {
                self.locked_ratio = None;
                raw
            };
            self.selection = selection
                .is_valid()
                .then_some(self.snap_rect_to_desktop(selection));
        }

        fn begin_existing_selection_drag(&mut self, point: POINT) -> bool {
            let Some(selection) = self.selection else {
                return false;
            };
            let tolerance = 7;
            let near_left = (point.x - selection.left).abs() <= tolerance;
            let near_right = (point.x - selection.right).abs() <= tolerance;
            let near_top = (point.y - selection.top).abs() <= tolerance;
            let near_bottom = (point.y - selection.bottom).abs() <= tolerance;
            let inside_x =
                point.x >= selection.left - tolerance && point.x <= selection.right + tolerance;
            let inside_y =
                point.y >= selection.top - tolerance && point.y <= selection.bottom + tolerance;
            let horizontal = if near_left && inside_y {
                -1
            } else if near_right && inside_y {
                1
            } else {
                0
            };
            let vertical = if near_top && inside_x {
                -1
            } else if near_bottom && inside_x {
                1
            } else {
                0
            };
            self.drag_kind = if horizontal != 0 || vertical != 0 {
                DragKind::Resize {
                    horizontal,
                    vertical,
                }
            } else if point_in_rect(point, selection) {
                DragKind::Move {
                    offset_x: point.x - selection.left,
                    offset_y: point.y - selection.top,
                }
            } else {
                return false;
            };
            self.drag_start_selection = Some(selection);
            true
        }

        fn update_existing_selection_drag(&mut self, point: POINT) {
            let point = self.clamp_point(point);
            let Some(original) = self.drag_start_selection else {
                return;
            };
            let next = match self.drag_kind {
                DragKind::Move { offset_x, offset_y } => {
                    let left = (point.x - offset_x).clamp(0, self.image_width - original.width());
                    let top = (point.y - offset_y).clamp(0, self.image_height - original.height());
                    LocalRect {
                        left,
                        top,
                        right: left + original.width(),
                        bottom: top + original.height(),
                    }
                }
                DragKind::Resize {
                    horizontal,
                    vertical,
                } => {
                    let mut rect = original;
                    if horizontal < 0 {
                        rect.left = point.x.min(rect.right - 1);
                        if alt_down() {
                            rect.right = (original.right + (original.left - rect.left))
                                .clamp(rect.left + 1, self.image_width);
                        }
                    }
                    if horizontal > 0 {
                        rect.right = point.x.max(rect.left + 1);
                        if alt_down() {
                            rect.left = (original.left - (rect.right - original.right))
                                .clamp(0, rect.right - 1);
                        }
                    }
                    if vertical < 0 {
                        rect.top = point.y.min(rect.bottom - 1);
                        if alt_down() {
                            rect.bottom = (original.bottom + (original.top - rect.top))
                                .clamp(rect.top + 1, self.image_height);
                        }
                    }
                    if vertical > 0 {
                        rect.bottom = point.y.max(rect.top + 1);
                        if alt_down() {
                            rect.top = (original.top - (rect.bottom - original.bottom))
                                .clamp(0, rect.bottom - 1);
                        }
                    }
                    if shift_down() {
                        let ratio_w = original.width().max(1) as i64;
                        let ratio_h = original.height().max(1) as i64;
                        if horizontal != 0 {
                            let target_height = ((i64::from(rect.width()) * ratio_h) / ratio_w)
                                .max(1)
                                .min(i64::from(self.image_height))
                                as i32;
                            let center = (rect.top + rect.bottom) / 2;
                            rect.top = (center - target_height / 2)
                                .clamp(0, self.image_height - target_height);
                            rect.bottom = rect.top + target_height;
                        } else if vertical != 0 {
                            let target_width = ((i64::from(rect.height()) * ratio_w) / ratio_h)
                                .max(1)
                                .min(i64::from(self.image_width))
                                as i32;
                            let center = (rect.left + rect.right) / 2;
                            rect.left = (center - target_width / 2)
                                .clamp(0, self.image_width - target_width);
                            rect.right = rect.left + target_width;
                        }
                    }
                    rect
                }
                DragKind::Create => return,
            };
            self.selection = Some(self.snap_rect_to_desktop(next));
        }

        fn snap_rect_to_desktop(&self, mut rect: LocalRect) -> LocalRect {
            const EDGE_SNAP: i32 = 8;
            if rect.left <= EDGE_SNAP {
                rect.left = 0;
            }
            if rect.top <= EDGE_SNAP {
                rect.top = 0;
            }
            if self.image_width - rect.right <= EDGE_SNAP {
                rect.right = self.image_width;
            }
            if self.image_height - rect.bottom <= EDGE_SNAP {
                rect.bottom = self.image_height;
            }
            rect.left = rect.left.clamp(0, self.image_width.saturating_sub(1));
            rect.top = rect.top.clamp(0, self.image_height.saturating_sub(1));
            rect.right = rect.right.clamp(rect.left + 1, self.image_width);
            rect.bottom = rect.bottom.clamp(rect.top + 1, self.image_height);
            rect
        }

        fn update_snap_selection(&mut self, point: POINT, reset_cycle: bool) {
            let _ = self.merge_ready_uia_candidates();
            self.cursor = self.clamp_point(point);
            self.snap_matches.clear();
            for (index, candidate) in self.snap_candidates.iter().enumerate() {
                if point_in_rect(self.cursor, candidate.rect) {
                    self.snap_matches.push(index);
                }
            }
            if reset_cycle || self.snap_match_index >= self.snap_matches.len() {
                self.snap_match_index = 0;
            }
            self.selection = self
                .snap_matches
                .get(self.snap_match_index)
                .map(|index| self.snap_candidates[*index].rect);
        }

        fn cycle_snap_selection(&mut self, delta: isize) {
            if self.merge_ready_uia_candidates() {
                self.update_snap_selection(self.cursor, false);
            }
            if self.snap_matches.is_empty() {
                return;
            }
            let len = self.snap_matches.len() as isize;
            self.snap_match_index =
                (self.snap_match_index as isize + delta).rem_euclid(len) as usize;
            let index = self.snap_matches[self.snap_match_index];
            self.selection = Some(self.snap_candidates[index].rect);
        }

        fn merge_ready_uia_candidates(&mut self) -> bool {
            let Some(receiver) = self.uia_candidates.as_ref() else {
                return false;
            };
            match receiver.try_recv() {
                Ok(mut candidates) => {
                    self.uia_candidates = None;
                    self.snap_candidates.append(&mut candidates);
                    sort_snap_candidates(&mut self.snap_candidates);
                    true
                }
                Err(TryRecvError::Disconnected) => {
                    self.uia_candidates = None;
                    false
                }
                Err(TryRecvError::Empty) => false,
            }
        }

        fn ratio_rect(&mut self, raw: LocalRect) -> LocalRect {
            if !raw.is_valid() {
                return raw;
            }
            let ratio = *self
                .locked_ratio
                .get_or_insert((raw.width() as u32, raw.height() as u32));
            let ratio_width = ratio.0.max(1) as i64;
            let ratio_height = ratio.1.max(1) as i64;
            let mut width = i64::from(raw.width());
            let mut height = i64::from(raw.height());
            if width * ratio_height >= height * ratio_width {
                height = (width * ratio_height / ratio_width).max(1);
            } else {
                width = (height * ratio_width / ratio_height).max(1);
            }

            let max_width = if self.current.x >= self.anchor.x {
                i64::from(self.image_width - self.anchor.x)
            } else {
                i64::from(self.anchor.x)
            };
            let max_height = if self.current.y >= self.anchor.y {
                i64::from(self.image_height - self.anchor.y)
            } else {
                i64::from(self.anchor.y)
            };
            width = width.min(max_width.max(1));
            height = height.min(max_height.max(1));
            if width * ratio_height >= height * ratio_width {
                height = (width * ratio_height / ratio_width)
                    .max(1)
                    .min(max_height.max(1));
            } else {
                width = (height * ratio_width / ratio_height)
                    .max(1)
                    .min(max_width.max(1));
            }
            let width = width as i32;
            let height = height as i32;
            let left = if self.current.x >= self.anchor.x {
                self.anchor.x
            } else {
                self.anchor.x - width
            };
            let top = if self.current.y >= self.anchor.y {
                self.anchor.y
            } else {
                self.anchor.y - height
            };
            LocalRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            }
        }

        fn fixed_rect(&self, point: POINT, width: i32, height: i32) -> LocalRect {
            let width = width.clamp(1, self.image_width);
            let height = height.clamp(1, self.image_height);
            let left = (if point.x >= self.anchor.x {
                self.anchor.x
            } else {
                self.anchor.x - width
            })
            .clamp(0, self.image_width - width);
            let top = (if point.y >= self.anchor.y {
                self.anchor.y
            } else {
                self.anchor.y - height
            })
            .clamp(0, self.image_height - height);
            LocalRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            }
        }

        fn set_fixed_size(&mut self, width: i32, height: i32) {
            let width = width.clamp(1, self.image_width);
            let height = height.clamp(1, self.image_height);
            let left = (self.cursor.x - width / 2).clamp(0, self.image_width - width);
            let top = (self.cursor.y - height / 2).clamp(0, self.image_height - height);
            self.fixed_size = Some((width, height));
            self.selection = Some(LocalRect {
                left,
                top,
                right: left + width,
                bottom: top + height,
            });
            self.anchor = POINT { x: left, y: top };
            self.current = POINT {
                x: left + width,
                y: top + height,
            };
            self.locked_ratio = None;
        }

        fn move_selection(&mut self, dx: i32, dy: i32) {
            let Some(selection) = self.selection else {
                return;
            };
            let left = (selection.left + dx).clamp(0, self.image_width - selection.width());
            let top = (selection.top + dy).clamp(0, self.image_height - selection.height());
            self.selection = Some(LocalRect {
                left,
                top,
                right: left + selection.width(),
                bottom: top + selection.height(),
            });
            self.anchor = POINT { x: left, y: top };
            self.current = POINT {
                x: left + selection.width(),
                y: top + selection.height(),
            };
        }

        fn confirm(&mut self) -> Result<bool, RegionSelectorError> {
            let Some(selection) = self.selection.filter(|value| value.is_valid()) else {
                return Ok(false);
            };
            let global_x = i64::from(self.origin_x) + i64::from(selection.left);
            let global_y = i64::from(self.origin_y) + i64::from(selection.top);
            let x = i32::try_from(global_x)
                .map_err(|_| RegionSelectorError::new("框选区域横坐标超出支持范围。"))?;
            let y = i32::try_from(global_y)
                .map_err(|_| RegionSelectorError::new("框选区域纵坐标超出支持范围。"))?;
            self.result = Some(CaptureRect::new(
                x,
                y,
                selection.width() as u32,
                selection.height() as u32,
            ));
            self.completed = true;
            self.cancelled = false;
            Ok(true)
        }

        fn cancel(&mut self) {
            self.result = None;
            self.completed = true;
            self.cancelled = true;
        }
    }

    impl Drop for SelectorState {
        fn drop(&mut self) {
            unsafe {
                release_paint_resources(self);
            }
        }
    }

    struct DpiContextGuard {
        previous: isize,
    }

    impl DpiContextGuard {
        fn enter() -> Self {
            let previous =
                unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            Self { previous }
        }
    }

    impl Drop for DpiContextGuard {
        fn drop(&mut self) {
            if self.previous != 0 {
                unsafe {
                    let _ = SetThreadDpiAwarenessContext(self.previous);
                }
            }
        }
    }

    pub(super) fn select_region_from_image(
        image: &RgbaImage,
        origin_x: i32,
        origin_y: i32,
        options: RegionSelectorOptions,
    ) -> Result<Option<CaptureRect>, RegionSelectorError> {
        let _dpi_guard = DpiContextGuard::enter();
        let mut state = Box::new(SelectorState::new(image, origin_x, origin_y, options)?);
        let state_ptr: *mut SelectorState = &mut *state;
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance == 0 {
            return Err(last_win32_error("读取当前程序模块句柄失败"));
        }

        let class_name = wide(CLASS_NAME);
        let cursor = unsafe { LoadCursorW(0, IDC_CROSS) };
        let window_class = WNDCLASSW {
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(selector_window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: 0,
            hCursor: cursor,
            hbrBackground: 0,
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
        };
        if unsafe { RegisterClassW(&window_class) } == 0 {
            let code = unsafe { GetLastError() };
            if code != ERROR_CLASS_ALREADY_EXISTS {
                return Err(win32_error("注册截图框选窗口失败", code));
            }
        }

        let title = wide(WINDOW_TITLE);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                origin_x,
                origin_y,
                state.image_width,
                state.image_height,
                0,
                0,
                instance,
                state_ptr.cast::<c_void>(),
            )
        };
        if hwnd == 0 {
            return Err(last_win32_error("创建截图框选窗口失败"));
        }

        let positioned = unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                origin_x,
                origin_y,
                state.image_width,
                state.image_height,
                SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
            )
        };
        if positioned == 0 {
            let error = last_win32_error("定位截图框选窗口失败");
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return Err(error);
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd);
        }

        let mut message: MSG = unsafe { zeroed() };
        while !state.completed {
            let status = unsafe { GetMessageW(&mut message, 0, 0, 0) };
            if status == -1 {
                state.cancel();
                if unsafe { IsWindow(hwnd) } != 0 {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                return Err(last_win32_error("读取截图框选窗口消息失败"));
            }
            if status == 0 {
                state.cancel();
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        if unsafe { IsWindow(hwnd) } != 0 {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        Ok(state.result)
    }

    unsafe extern "system" fn selector_window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }

        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SelectorState;
        let state = state_ptr.as_mut();
        match message {
            WM_LBUTTONDOWN => {
                if let Some(state) = state {
                    if let Some(points) = cursor_points(hwnd, state) {
                        if state.manual_selection_locked {
                            match selector_action_at(hwnd, state, points.client) {
                                Some(SelectorAction::Confirm) => {
                                    if state.confirm().unwrap_or(false) {
                                        let _ = DestroyWindow(hwnd);
                                    }
                                    return 0;
                                }
                                Some(SelectorAction::Cancel) => {
                                    cancel_and_destroy(hwnd, Some(state));
                                    return 0;
                                }
                                None => {}
                            }
                        }
                        state.anchor = state.clamp_point(points.image);
                        state.current = state.anchor;
                        state.cursor = state.anchor;
                        state.locked_ratio = None;
                        let had_manual_selection = state.manual_selection_locked;
                        if had_manual_selection && state.begin_existing_selection_drag(state.anchor)
                        {
                            state.pressed_snap = None;
                            state.dragging = true;
                            let _ = SetCapture(hwnd);
                            let _ = InvalidateRect(hwnd, null(), 0);
                            return 0;
                        }
                        state.manual_selection_locked = false;
                        state.drag_kind = DragKind::Create;
                        state.drag_start_selection = None;
                        state.pressed_snap = if ctrl_down() || had_manual_selection {
                            None
                        } else {
                            state
                                .selection
                                .filter(|selection| point_in_rect(state.anchor, *selection))
                        };
                        state.dragging = true;
                        if state.pressed_snap.is_none() {
                            state.update_selection(state.anchor, shift_down());
                        }
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                if let Some(state) = state {
                    if let Some(points) = cursor_points(hwnd, state) {
                        state.cursor = state.clamp_point(points.image);
                        if state.dragging {
                            if !matches!(state.drag_kind, DragKind::Create) {
                                state.update_existing_selection_drag(points.image);
                                let _ = InvalidateRect(hwnd, null(), 0);
                                return 0;
                            }
                            if state.pressed_snap.is_some()
                                && ((points.image.x - state.anchor.x).abs() > 3
                                    || (points.image.y - state.anchor.y).abs() > 3)
                            {
                                state.pressed_snap = None;
                            }
                            if state.pressed_snap.is_none() {
                                state.update_selection(points.image, shift_down());
                            }
                        } else if !state.manual_selection_locked {
                            if ctrl_down() {
                                state.selection = None;
                                state.snap_matches.clear();
                            } else {
                                state.update_snap_selection(points.image, true);
                            }
                        }
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                if let Some(state) = state.filter(|state| state.dragging) {
                    if let Some(points) = cursor_points(hwnd, state) {
                        state.cursor = state.clamp_point(points.image);
                        if !matches!(state.drag_kind, DragKind::Create) {
                            state.update_existing_selection_drag(points.image);
                        } else if state.pressed_snap.is_none() {
                            state.update_selection(points.image, shift_down());
                        }
                    }
                    state.dragging = false;
                    let _ = ReleaseCapture();
                    if !matches!(state.drag_kind, DragKind::Create) {
                        state.drag_start_selection = None;
                        state.manual_selection_locked = true;
                        let _ = InvalidateRect(hwnd, null(), 0);
                    } else if state.pressed_snap.take().is_some() {
                        if state.confirm().unwrap_or(false) {
                            let _ = DestroyWindow(hwnd);
                        }
                    } else if state.confirm_on_release {
                        if state.confirm().unwrap_or(false) {
                            let _ = DestroyWindow(hwnd);
                        }
                    } else {
                        state.manual_selection_locked = true;
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONDBLCLK => {
                if let Some(state) = state {
                    if state.confirm().unwrap_or(false) {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                0
            }
            WM_MOUSEWHEEL => {
                if let Some(state) = state.filter(|state| !state.dragging) {
                    let delta = ((wparam >> 16) & 0xffff) as u16 as i16;
                    state.cycle_snap_selection(if delta > 0 { -1 } else { 1 });
                    state.manual_selection_locked = false;
                    let _ = InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_KEYDOWN => {
                if let Some(state) = state {
                    match wparam as u32 {
                        value if value == u32::from(VK_RETURN) => {
                            if state.dragging {
                                if let Some(points) = cursor_points(hwnd, state) {
                                    state.cursor = state.clamp_point(points.image);
                                    state.update_selection(points.image, shift_down());
                                }
                                state.dragging = false;
                                let _ = ReleaseCapture();
                            }
                            if state.confirm().unwrap_or(false) {
                                let _ = DestroyWindow(hwnd);
                            } else {
                                let _ = InvalidateRect(hwnd, null(), 0);
                            }
                        }
                        value if value == u32::from(VK_ESCAPE) => {
                            cancel_and_destroy(hwnd, Some(state));
                        }
                        value if value == u32::from(VK_TAB) => {
                            state.cycle_snap_selection(if shift_down() { -1 } else { 1 });
                            state.manual_selection_locked = false;
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        value
                            if value == u32::from(VK_LEFT)
                                || value == u32::from(VK_RIGHT)
                                || value == u32::from(VK_UP)
                                || value == u32::from(VK_DOWN) =>
                        {
                            if !state.dragging {
                                let step = if shift_down() { 10 } else { 1 };
                                let (dx, dy) = match value {
                                    value if value == u32::from(VK_LEFT) => (-step, 0),
                                    value if value == u32::from(VK_RIGHT) => (step, 0),
                                    value if value == u32::from(VK_UP) => (0, -step),
                                    _ => (0, step),
                                };
                                state.move_selection(dx, dy);
                                let _ = InvalidateRect(hwnd, null(), 0);
                            }
                        }
                        value if value == u32::from(VK_F1) => {
                            state.set_fixed_size(320, 240);
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        value if value == u32::from(VK_F2) => {
                            state.set_fixed_size(640, 480);
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        value if value == u32::from(VK_F3) => {
                            state.set_fixed_size(1280, 720);
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        value if value == u32::from(VK_F4) => {
                            state.set_fixed_size(1920, 1080);
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        value if value == u32::from(VK_F5) => {
                            state.fixed_size = None;
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_RBUTTONDOWN | WM_CLOSE | WM_KILLFOCUS | WM_DISPLAYCHANGE => {
                cancel_and_destroy(hwnd, state);
                0
            }
            WM_SETCURSOR => {
                let cursor = LoadCursorW(0, IDC_CROSS);
                if cursor != 0 {
                    let _ = SetCursor(cursor);
                }
                1
            }
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                if let Some(state) = state {
                    paint_selector(hwnd, state);
                } else {
                    let mut paint: PAINTSTRUCT = zeroed();
                    let _ = BeginPaint(hwnd, &mut paint);
                    let _ = EndPaint(hwnd, &paint);
                }
                0
            }
            WM_DPICHANGED => {
                // Coordinates remain physical pixels under Per-Monitor V2. The
                // overlay intentionally keeps the captured virtual-desktop
                // bounds instead of accepting the suggested single-monitor
                // rectangle from lParam.
                let _ = InvalidateRect(hwnd, null(), 0);
                0
            }
            WM_NCDESTROY => {
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    unsafe fn cancel_and_destroy(hwnd: HWND, state: Option<&mut SelectorState>) {
        if let Some(state) = state {
            if state.completed {
                return;
            }
            state.cancel();
        }
        let _ = ReleaseCapture();
        let _ = DestroyWindow(hwnd);
    }

    struct CursorPoints {
        client: POINT,
        image: POINT,
    }

    /// Returns the cursor in both the overlay window's client coordinates and
    /// the captured image's pixel coordinates. The overlay is created at the
    /// image's physical size, but DPI scaling can make the window's client
    /// rectangle differ from the image dimensions. Keeping the two spaces
    /// explicit lets the selection geometry (and therefore the final crop)
    /// stay in image pixels while the toolbar and preview stay in client
    /// pixels.
    unsafe fn cursor_points(hwnd: HWND, state: &SelectorState) -> Option<CursorPoints> {
        let mut client = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut client) == 0 || ScreenToClient(hwnd, &mut client) == 0 {
            return None;
        }
        let mut client_rect: RECT = zeroed();
        if GetClientRect(hwnd, &mut client_rect) == 0 {
            return Some(CursorPoints {
                client,
                image: client,
            });
        }
        let client_width = client_rect.right.saturating_sub(client_rect.left).max(1);
        let client_height = client_rect.bottom.saturating_sub(client_rect.top).max(1);
        let image = POINT {
            x: ((i64::from(client.x) * i64::from(state.image_width)) / i64::from(client_width))
                as i32,
            y: ((i64::from(client.y) * i64::from(state.image_height)) / i64::from(client_height))
                as i32,
        };
        Some(CursorPoints { client, image })
    }

    unsafe fn selector_action_at(
        hwnd: HWND,
        state: &SelectorState,
        point: POINT,
    ) -> Option<SelectorAction> {
        let selection = state.selection.filter(|value| value.is_valid())?;
        let mut client: RECT = zeroed();
        if GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
        let client_width = client.right.saturating_sub(client.left);
        let client_height = client.bottom.saturating_sub(client.top);
        let display_selection = scale_local_rect_to_client(
            selection,
            state.image_width,
            state.image_height,
            client_width,
            client_height,
        );
        let layout = action_toolbar_layout(
            display_selection,
            client_width,
            client_height,
            GetDpiForWindow(hwnd).max(96),
        );
        if point_in_win32_rect(point, layout.confirm) {
            Some(SelectorAction::Confirm)
        } else if point_in_win32_rect(point, layout.cancel) {
            Some(SelectorAction::Cancel)
        } else {
            None
        }
    }

    fn point_in_win32_rect(point: POINT, rect: RECT) -> bool {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn action_toolbar_layout(
        selection: LocalRect,
        client_width: i32,
        client_height: i32,
        dpi: u32,
    ) -> ActionToolbarLayout {
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let margin = scale(8).max(4);
        let available_width = client_width.saturating_sub(margin * 2).max(1);
        let width = scale(430).min(available_width).max(1);
        let height = scale(36).min(client_height.max(1)).max(1);
        let button_width = scale(68).min((width / 3).max(1));
        let max_left = client_width.saturating_sub(width + margin).max(margin);
        let left = selection.left.clamp(margin, max_left);
        let below = selection.bottom.saturating_add(margin);
        let top = if below.saturating_add(height) <= client_height.saturating_sub(margin) {
            below
        } else {
            selection.top.saturating_sub(height + margin).max(margin)
        };
        let cancel = RECT {
            left: left.saturating_add(width - button_width),
            top,
            right: left.saturating_add(width),
            bottom: top.saturating_add(height),
        };
        let confirm = RECT {
            left: cancel.left.saturating_sub(button_width),
            top,
            right: cancel.left,
            bottom: cancel.bottom,
        };
        ActionToolbarLayout {
            info: RECT {
                left,
                top,
                right: confirm.left,
                bottom: confirm.bottom,
            },
            confirm,
            cancel,
        }
    }

    fn magnifier_rect(hwnd: HWND, cursor: POINT, client_width: i32, client_height: i32) -> RECT {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let margin = scale(18);
        let panel_size = scale(160)
            .min(client_width.saturating_sub(scale(16)).max(1))
            .min(client_height.saturating_sub(scale(16)).max(1));
        let left = if cursor.x + margin + panel_size <= client_width {
            cursor.x + margin
        } else {
            cursor.x - margin - panel_size
        }
        .clamp(0, client_width.saturating_sub(panel_size));
        let top = if cursor.y + margin + panel_size <= client_height {
            cursor.y + margin
        } else {
            cursor.y - margin - panel_size
        }
        .clamp(0, client_height.saturating_sub(panel_size));
        RECT {
            left,
            top,
            right: left + panel_size + 1,
            bottom: top + panel_size + 1,
        }
    }

    unsafe fn paint_selector(hwnd: HWND, state: &mut SelectorState) {
        let mut paint: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc == 0 {
            return;
        }

        let mut client: RECT = zeroed();
        if GetClientRect(hwnd, &mut client) != 0 {
            let client_width = client.right.saturating_sub(client.left).max(1);
            let client_height = client.bottom.saturating_sub(client.top).max(1);
            ensure_paint_resources(state, hdc, client_width, client_height);
            if state.back_buffer_dc != 0
                && state.back_buffer != 0
                && state.background_dc != 0
                && state.background_bitmap != 0
            {
                let _ = BitBlt(
                    state.back_buffer_dc,
                    0,
                    0,
                    client_width,
                    client_height,
                    state.background_dc,
                    0,
                    0,
                    SRCCOPY,
                );
                paint_selector_dynamic_contents(
                    hwnd,
                    state.back_buffer_dc,
                    state,
                    client_width,
                    client_height,
                );
                let _ = BitBlt(
                    hdc,
                    0,
                    0,
                    client_width,
                    client_height,
                    state.back_buffer_dc,
                    0,
                    0,
                    SRCCOPY,
                );
            } else {
                // A memory DC allocation failure must not make region capture
                // unusable; draw directly as a last-resort fallback.
                paint_selector_contents(hwnd, hdc, state, client_width, client_height);
            }
        }
        let _ = EndPaint(hwnd, &paint);
    }

    unsafe fn ensure_paint_resources(
        state: &mut SelectorState,
        hdc: isize,
        client_width: i32,
        client_height: i32,
    ) {
        if state.back_buffer_dc != 0 && state.back_buffer != 0 {
            return;
        }

        release_paint_resources(state);
        let back_buffer_dc = CreateCompatibleDC(hdc);
        if back_buffer_dc == 0 {
            return;
        }
        let back_buffer = CreateCompatibleBitmap(hdc, client_width, client_height);
        if back_buffer == 0 {
            let _ = DeleteDC(back_buffer_dc);
            return;
        }
        let old_bitmap = SelectObject(back_buffer_dc, back_buffer as _);
        if old_bitmap == 0 {
            let _ = DeleteObject(back_buffer as _);
            let _ = DeleteDC(back_buffer_dc);
            return;
        }
        state.back_buffer_dc = back_buffer_dc;
        state.back_buffer = back_buffer as _;
        state.back_buffer_old_bitmap = old_bitmap;

        let dim_dc = CreateCompatibleDC(hdc);
        if dim_dc == 0 {
            return;
        }
        let dim_bitmap = CreateCompatibleBitmap(hdc, 1, 1);
        if dim_bitmap == 0 {
            let _ = DeleteDC(dim_dc);
            return;
        }
        let dim_old_bitmap = SelectObject(dim_dc, dim_bitmap as _);
        if dim_old_bitmap == 0 {
            let _ = DeleteObject(dim_bitmap as _);
            let _ = DeleteDC(dim_dc);
            return;
        }
        let brush = CreateSolidBrush(rgb(0, 0, 0));
        if brush != 0 {
            let rect = RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            };
            let _ = FillRect(dim_dc, &rect, brush);
            let _ = DeleteObject(brush);
        }
        state.dim_dc = dim_dc;
        state.dim_bitmap = dim_bitmap as _;
        state.dim_old_bitmap = dim_old_bitmap;

        let background_dc = CreateCompatibleDC(hdc);
        if background_dc == 0 {
            return;
        }
        let background_bitmap = CreateCompatibleBitmap(hdc, client_width, client_height);
        if background_bitmap == 0 {
            let _ = DeleteDC(background_dc);
            return;
        }
        let background_old_bitmap = SelectObject(background_dc, background_bitmap as _);
        if background_old_bitmap == 0 {
            let _ = DeleteObject(background_bitmap as _);
            let _ = DeleteDC(background_dc);
            return;
        }
        state.background_dc = background_dc;
        state.background_bitmap = background_bitmap as _;
        state.background_old_bitmap = background_old_bitmap;
        draw_bitmap(
            background_dc,
            &state.original_bgra,
            &state.bitmap_info,
            state.image_width,
            state.image_height,
            client_width,
            client_height,
        );
        draw_dim_overlay(background_dc, state, client_width, client_height);
    }

    unsafe fn release_paint_resources(state: &mut SelectorState) {
        if state.back_buffer_dc != 0 {
            if state.back_buffer_old_bitmap != 0 {
                let _ = SelectObject(state.back_buffer_dc, state.back_buffer_old_bitmap);
            }
            let _ = DeleteDC(state.back_buffer_dc);
        }
        if state.back_buffer != 0 {
            let _ = DeleteObject(state.back_buffer as _);
        }
        if state.background_dc != 0 {
            if state.background_old_bitmap != 0 {
                let _ = SelectObject(state.background_dc, state.background_old_bitmap);
            }
            let _ = DeleteDC(state.background_dc);
        }
        if state.background_bitmap != 0 {
            let _ = DeleteObject(state.background_bitmap as _);
        }
        if state.dim_dc != 0 {
            if state.dim_old_bitmap != 0 {
                let _ = SelectObject(state.dim_dc, state.dim_old_bitmap);
            }
            let _ = DeleteDC(state.dim_dc);
        }
        if state.dim_bitmap != 0 {
            let _ = DeleteObject(state.dim_bitmap as _);
        }
        state.back_buffer_dc = 0;
        state.back_buffer = 0;
        state.back_buffer_old_bitmap = 0;
        state.background_dc = 0;
        state.background_bitmap = 0;
        state.background_old_bitmap = 0;
        state.dim_dc = 0;
        state.dim_bitmap = 0;
        state.dim_old_bitmap = 0;
    }

    unsafe fn paint_selector_contents(
        hwnd: HWND,
        hdc: isize,
        state: &SelectorState,
        client_width: i32,
        client_height: i32,
    ) {
        draw_bitmap(
            hdc,
            &state.original_bgra,
            &state.bitmap_info,
            state.image_width,
            state.image_height,
            client_width,
            client_height,
        );
        draw_dim_overlay(hdc, state, client_width, client_height);

        paint_selector_dynamic_contents(hwnd, hdc, state, client_width, client_height);
    }

    unsafe fn paint_selector_dynamic_contents(
        hwnd: HWND,
        hdc: isize,
        state: &SelectorState,
        client_width: i32,
        client_height: i32,
    ) {
        if let Some(selection) = state.selection.filter(|value| value.is_valid()) {
            let display_selection = scale_local_rect_to_client(
                selection,
                state.image_width,
                state.image_height,
                client_width,
                client_height,
            );
            // Restore the selected area with exactly the same whole-image
            // transform used for the dimmed background.  Stretching the
            // selected source rectangle independently gives GDI a different
            // sampling grid whenever image and client sizes are not an exact
            // match (notably under DPI virtualization), so the bright preview
            // can show pixels that differ from the eventual image-space crop.
            draw_bitmap_clipped(
                hdc,
                &state.original_bgra,
                &state.bitmap_info,
                state.image_width,
                state.image_height,
                client_width,
                client_height,
                display_selection,
            );
            draw_selection_decoration(
                hwnd,
                hdc,
                state,
                selection,
                display_selection,
                client_width,
                client_height,
            );
        }
        draw_magnifier(hwnd, hdc, state, client_width, client_height);
        if state.show_instructions {
            draw_instruction(hwnd, hdc, client_width);
        }
    }

    fn scale_local_rect_to_client(
        rect: LocalRect,
        image_width: i32,
        image_height: i32,
        client_width: i32,
        client_height: i32,
    ) -> LocalRect {
        let sx = |v: i32| {
            ((i64::from(v) * i64::from(client_width.max(1))) / i64::from(image_width.max(1))) as i32
        };
        let sy = |v: i32| {
            ((i64::from(v) * i64::from(client_height.max(1))) / i64::from(image_height.max(1)))
                as i32
        };
        LocalRect {
            left: sx(rect.left).clamp(0, client_width),
            top: sy(rect.top).clamp(0, client_height),
            right: sx(rect.right).clamp(0, client_width),
            bottom: sy(rect.bottom).clamp(0, client_height),
        }
    }

    fn scale_local_point_to_client(
        point: POINT,
        image_width: i32,
        image_height: i32,
        client_width: i32,
        client_height: i32,
    ) -> POINT {
        POINT {
            x: ((i64::from(point.x) * i64::from(client_width.max(1)))
                / i64::from(image_width.max(1))) as i32,
            y: ((i64::from(point.y) * i64::from(client_height.max(1)))
                / i64::from(image_height.max(1))) as i32,
        }
    }
    unsafe fn draw_dim_overlay(hdc: isize, state: &SelectorState, width: i32, height: i32) {
        if state.dim_dc == 0 {
            return;
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 138,
            AlphaFormat: 0,
        };
        let _ = AlphaBlend(hdc, 0, 0, width, height, state.dim_dc, 0, 0, 1, 1, blend);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_bitmap(
        hdc: isize,
        pixels: &[u8],
        bitmap_info: &BITMAPINFO,
        source_width: i32,
        source_height: i32,
        target_width: i32,
        target_height: i32,
    ) {
        let _ = StretchDIBits(
            hdc,
            0,
            0,
            target_width,
            target_height,
            0,
            0,
            source_width,
            source_height,
            pixels.as_ptr().cast::<c_void>(),
            bitmap_info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_bitmap_clipped(
        hdc: isize,
        pixels: &[u8],
        bitmap_info: &BITMAPINFO,
        source_width: i32,
        source_height: i32,
        target_width: i32,
        target_height: i32,
        clip: LocalRect,
    ) {
        let saved = SaveDC(hdc);
        if saved == 0 {
            return;
        }
        let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
        draw_bitmap(
            hdc,
            pixels,
            bitmap_info,
            source_width,
            source_height,
            target_width,
            target_height,
        );
        let _ = RestoreDC(hdc, saved);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_bitmap_region(
        hdc: isize,
        pixels: &[u8],
        bitmap_info: &BITMAPINFO,
        source_width: i32,
        source_height: i32,
        source_left: i32,
        source_top: i32,
        source_region_width: i32,
        source_region_height: i32,
        target_left: i32,
        target_top: i32,
        target_width: i32,
        target_height: i32,
    ) {
        let _ = StretchDIBits(
            hdc,
            target_left,
            target_top,
            target_width,
            target_height,
            source_left,
            source_top,
            source_region_width.min(source_width),
            source_region_height.min(source_height),
            pixels.as_ptr().cast::<c_void>(),
            bitmap_info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
    }

    unsafe fn draw_magnifier(
        hwnd: HWND,
        hdc: isize,
        state: &SelectorState,
        client_width: i32,
        client_height: i32,
    ) {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let sample = 15.min(state.image_width).min(state.image_height).max(1);
        let source_left = (state.cursor.x - sample / 2).clamp(0, state.image_width - sample);
        let source_top = (state.cursor.y - sample / 2).clamp(0, state.image_height - sample);
        let border = scale(2).max(2);
        let display_cursor = scale_local_point_to_client(
            state.cursor,
            state.image_width,
            state.image_height,
            client_width,
            client_height,
        );
        let panel = magnifier_rect(hwnd, display_cursor, client_width, client_height);
        let panel_left = panel.left;
        let panel_top = panel.top;
        let panel_size = panel.right.saturating_sub(panel.left).saturating_sub(1);

        let background = CreateSolidBrush(rgb(15, 23, 42));
        if background != 0 {
            let rect = RECT {
                left: panel_left,
                top: panel_top,
                right: panel_left + panel_size,
                bottom: panel_top + panel_size,
            };
            let _ = FillRect(hdc, &rect, background);
            let _ = DeleteObject(background);
        }

        let inner_size = panel_size.saturating_sub(border * 2).max(1);
        draw_bitmap_region(
            hdc,
            &state.original_bgra,
            &state.bitmap_info,
            state.image_width,
            state.image_height,
            source_left,
            source_top,
            sample,
            sample,
            panel_left + border,
            panel_top + border,
            inner_size,
            inner_size,
        );

        let pen = CreatePen(PS_SOLID, border, rgb(37, 99, 235));
        let hollow = GetStockObject(NULL_BRUSH);
        let old_pen = if pen != 0 { SelectObject(hdc, pen) } else { 0 };
        let old_brush = if hollow != 0 {
            SelectObject(hdc, hollow)
        } else {
            0
        };
        let _ = Rectangle(
            hdc,
            panel_left,
            panel_top,
            panel_left + panel_size,
            panel_top + panel_size,
        );
        let center = panel_left + panel_size / 2;
        let center_size = scale(4).max(2);
        let _ = Rectangle(
            hdc,
            center - center_size,
            panel_top + panel_size / 2 - center_size,
            center + center_size,
            panel_top + panel_size / 2 + center_size,
        );
        if old_brush != 0 {
            let _ = SelectObject(hdc, old_brush);
        }
        if old_pen != 0 {
            let _ = SelectObject(hdc, old_pen);
        }
        if pen != 0 {
            let _ = DeleteObject(pen);
        }
    }

    unsafe fn draw_selection_decoration(
        hwnd: HWND,
        hdc: isize,
        state: &SelectorState,
        selection: LocalRect,
        display_selection: LocalRect,
        client_width: i32,
        client_height: i32,
    ) {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let blue = rgb(37, 99, 235);
        let border_width = scale(2).max(2);
        let pen = CreatePen(PS_SOLID, border_width, blue);
        let hollow = GetStockObject(NULL_BRUSH);
        let old_pen = if pen != 0 { SelectObject(hdc, pen) } else { 0 };
        let old_brush = if hollow != 0 {
            SelectObject(hdc, hollow)
        } else {
            0
        };
        let _ = Rectangle(
            hdc,
            display_selection.left,
            display_selection.top,
            display_selection.right,
            display_selection.bottom,
        );
        if old_brush != 0 {
            let _ = SelectObject(hdc, old_brush);
        }
        if old_pen != 0 {
            let _ = SelectObject(hdc, old_pen);
        }
        if pen != 0 {
            let _ = DeleteObject(pen);
        }

        if state.manual_selection_locked && !state.dragging {
            let handle_size = scale(7).max(5);
            let half = handle_size / 2;
            let center_x = display_selection.left + display_selection.width() / 2;
            let center_y = display_selection.top + display_selection.height() / 2;
            let handle_brush = CreateSolidBrush(rgb(255, 255, 255));
            let handle_pen = CreatePen(PS_SOLID, scale(1).max(1), blue);
            let old_handle_pen = if handle_pen != 0 {
                SelectObject(hdc, handle_pen)
            } else {
                0
            };
            let old_handle_brush = if handle_brush != 0 {
                SelectObject(hdc, handle_brush as _)
            } else {
                0
            };
            for (x, y) in [
                (display_selection.left, display_selection.top),
                (center_x, display_selection.top),
                (display_selection.right, display_selection.top),
                (display_selection.left, center_y),
                (display_selection.right, center_y),
                (display_selection.left, display_selection.bottom),
                (center_x, display_selection.bottom),
                (display_selection.right, display_selection.bottom),
            ] {
                let _ = Rectangle(hdc, x - half, y - half, x + half + 1, y + half + 1);
            }
            if old_handle_brush != 0 {
                let _ = SelectObject(hdc, old_handle_brush);
            }
            if old_handle_pen != 0 {
                let _ = SelectObject(hdc, old_handle_pen);
            }
            if handle_brush != 0 {
                let _ = DeleteObject(handle_brush as _);
            }
            if handle_pen != 0 {
                let _ = DeleteObject(handle_pen);
            }
        }

        if state.manual_selection_locked && !state.dragging {
            draw_action_toolbar(
                hwnd,
                hdc,
                state,
                selection,
                display_selection,
                client_width,
                client_height,
            );
            return;
        }

        let global_x = i64::from(state.origin_x) + i64::from(selection.left);
        let global_y = i64::from(state.origin_y) + i64::from(selection.top);
        let label = wide(&format!(
            "坐标 {}, {}   尺寸 {} × {}",
            global_x,
            global_y,
            selection.width(),
            selection.height()
        ));
        let font = create_overlay_font(dpi, 13);
        let old_font = if font != 0 {
            SelectObject(hdc, font)
        } else {
            0
        };
        let _ = SetBkMode(hdc, TRANSPARENT as i32);
        let _ = SetTextColor(hdc, rgb(255, 255, 255));
        let padding_x = scale(9);
        let padding_y = scale(5);
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = DrawTextW(
            hdc,
            label.as_ptr(),
            (label.len().saturating_sub(1)) as i32,
            &mut measured,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        let label_width = measured
            .right
            .saturating_sub(measured.left)
            .saturating_add(padding_x * 2)
            .max(scale(72));
        let label_height = measured
            .bottom
            .saturating_sub(measured.top)
            .saturating_add(padding_y * 2)
            .max(scale(28));
        let margin = scale(8);
        let max_left = client_width
            .saturating_sub(label_width + margin)
            .max(margin);
        let label_left = display_selection.left.clamp(margin, max_left);
        let below = display_selection.bottom.saturating_add(margin);
        let label_top = if below.saturating_add(label_height) <= client_height - margin {
            below
        } else {
            display_selection
                .top
                .saturating_sub(label_height + margin)
                .max(margin)
        };
        let mut label_rect = RECT {
            left: label_left,
            top: label_top,
            right: label_left.saturating_add(label_width),
            bottom: label_top.saturating_add(label_height),
        };
        let brush = CreateSolidBrush(blue);
        if brush != 0 {
            let _ = FillRect(hdc, &label_rect, brush);
            let _ = DeleteObject(brush);
        }
        let _ = DrawTextW(
            hdc,
            label.as_ptr(),
            (label.len().saturating_sub(1)) as i32,
            &mut label_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        if old_font != 0 {
            let _ = SelectObject(hdc, old_font);
        }
        if font != 0 {
            let _ = DeleteObject(font);
        }
    }

    unsafe fn draw_action_toolbar(
        hwnd: HWND,
        hdc: isize,
        state: &SelectorState,
        selection: LocalRect,
        display_selection: LocalRect,
        client_width: i32,
        client_height: i32,
    ) {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let layout = action_toolbar_layout(display_selection, client_width, client_height, dpi);
        for (rect, color) in [
            (layout.info, rgb(37, 99, 235)),
            (layout.confirm, rgb(22, 163, 74)),
            (layout.cancel, rgb(51, 65, 85)),
        ] {
            let brush = CreateSolidBrush(color);
            if brush != 0 {
                let _ = FillRect(hdc, &rect, brush);
                let _ = DeleteObject(brush);
            }
        }

        let global_x = i64::from(state.origin_x) + i64::from(selection.left);
        let global_y = i64::from(state.origin_y) + i64::from(selection.top);
        let info = wide(&format!(
            "{}, {}  ·  {} × {}",
            global_x,
            global_y,
            selection.width(),
            selection.height()
        ));
        let confirm = wide("完成");
        let cancel = wide("取消");
        let font = create_overlay_font(dpi, 13);
        let old_font = if font != 0 {
            SelectObject(hdc, font)
        } else {
            0
        };
        let _ = SetBkMode(hdc, TRANSPARENT as i32);
        let _ = SetTextColor(hdc, rgb(255, 255, 255));
        for (text, mut rect) in [
            (&info, layout.info),
            (&confirm, layout.confirm),
            (&cancel, layout.cancel),
        ] {
            let _ = DrawTextW(
                hdc,
                text.as_ptr(),
                (text.len().saturating_sub(1)) as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        if old_font != 0 {
            let _ = SelectObject(hdc, old_font);
        }
        if font != 0 {
            let _ = DeleteObject(font);
        }
    }

    unsafe fn draw_instruction(hwnd: HWND, hdc: isize, client_width: i32) {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let text = wide(
            "移动自动吸附 · 拖动框选 · 松手后点击完成或继续调整 · 双击/Enter 确认 · Esc 取消 · Shift 固定比例 · Ctrl 禁用吸附 · Tab 切换层级",
        );
        let width = scale(720)
            .min(client_width.saturating_sub(scale(16)))
            .max(scale(220));
        let height = scale(54);
        let left = (client_width.saturating_sub(width) / 2).max(0);
        let top = scale(18);
        let mut rect = RECT {
            left,
            top,
            right: left.saturating_add(width),
            bottom: top.saturating_add(height),
        };
        let brush = CreateSolidBrush(rgb(15, 23, 42));
        if brush != 0 {
            let _ = FillRect(hdc, &rect, brush);
            let _ = DeleteObject(brush);
        }
        let font = create_overlay_font(dpi, 12);
        let old_font = if font != 0 {
            SelectObject(hdc, font)
        } else {
            0
        };
        let _ = SetBkMode(hdc, TRANSPARENT as i32);
        let _ = SetTextColor(hdc, rgb(255, 255, 255));
        let _ = DrawTextW(
            hdc,
            text.as_ptr(),
            (text.len().saturating_sub(1)) as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_WORDBREAK | DT_NOPREFIX,
        );
        if old_font != 0 {
            let _ = SelectObject(hdc, old_font);
        }
        if font != 0 {
            let _ = DeleteObject(font);
        }
    }

    unsafe fn create_overlay_font(dpi: u32, points: i32) -> isize {
        let height = -((i64::from(points) * i64::from(dpi) + 36) / 72) as i32;
        let face = wide("Microsoft YaHei UI");
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.into(),
            OUT_DEFAULT_PRECIS.into(),
            CLIP_DEFAULT_PRECIS.into(),
            PROOF_QUALITY.into(),
            (DEFAULT_PITCH | FF_DONTCARE).into(),
            face.as_ptr(),
        )
    }

    fn shift_down() -> bool {
        unsafe { GetKeyState(i32::from(VK_SHIFT)) < 0 }
    }

    fn ctrl_down() -> bool {
        unsafe { GetKeyState(i32::from(VK_CONTROL)) < 0 }
    }

    fn alt_down() -> bool {
        unsafe { GetKeyState(i32::from(VK_MENU)) < 0 }
    }

    const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
        red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_win32_error(context: &str) -> RegionSelectorError {
        win32_error(context, unsafe { GetLastError() })
    }

    fn win32_error(context: &str, code: u32) -> RegionSelectorError {
        RegionSelectorError::new(format!(
            "{context}：{}（Win32={code}）",
            std::io::Error::from_raw_os_error(code as i32)
        ))
    }
}
