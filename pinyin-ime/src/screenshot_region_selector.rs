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
    select_region_from_image(&frame.image, frame.origin_x, frame.origin_y)
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
    platform::select_region_from_image(image, origin_x, origin_y)
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn select_region_from_image(
        _image: &RgbaImage,
        _origin_x: i32,
        _origin_y: i32,
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
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
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
        GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_DOWN, VK_ESCAPE, VK_F1, VK_F2, VK_F3,
        VK_F4, VK_F5, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetCursorPos, GetMessageW, GetWindowLongPtrW, IsWindow, LoadCursorW, RegisterClassW,
        SetCursor, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow,
        TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST,
        IDC_CROSS, MSG, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SW_SHOW, WM_CLOSE, WM_DISPLAYCHANGE,
        WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    const CLASS_NAME: &str = "KaixinScreenshotRegionSelector_v1";
    const WINDOW_TITLE: &str = "开心输入法截图区域选择";

    #[derive(Clone, Copy, Debug, Default)]
    struct LocalRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
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
        dragging: bool,
        fixed_size: Option<(i32, i32)>,
        locked_ratio: Option<(u32, u32)>,
        back_buffer_dc: isize,
        back_buffer: isize,
        back_buffer_old_bitmap: isize,
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
                dragging: false,
                fixed_size: None,
                locked_ratio: None,
                back_buffer_dc: 0,
                back_buffer: 0,
                back_buffer_old_bitmap: 0,
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
            let raw = LocalRect::from_points(self.anchor, self.current);
            let selection = if lock_ratio {
                self.ratio_rect(raw)
            } else {
                self.locked_ratio = None;
                raw
            };
            self.selection = selection.is_valid().then_some(selection);
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
    ) -> Result<Option<CaptureRect>, RegionSelectorError> {
        let mut state = Box::new(SelectorState::new(image, origin_x, origin_y)?);
        let state_ptr: *mut SelectorState = &mut *state;
        let _dpi_guard = DpiContextGuard::enter();
        let instance = unsafe { GetModuleHandleW(null()) };
        if instance == 0 {
            return Err(last_win32_error("读取当前程序模块句柄失败"));
        }

        let class_name = wide(CLASS_NAME);
        let cursor = unsafe { LoadCursorW(0, IDC_CROSS) };
        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
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
                    if let Some(point) = cursor_client_point(hwnd) {
                        state.anchor = state.clamp_point(point);
                        state.current = state.anchor;
                        state.cursor = state.anchor;
                        state.locked_ratio = None;
                        state.dragging = true;
                        state.update_selection(state.anchor, shift_down());
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                if let Some(state) = state {
                    if let Some(point) = cursor_client_point(hwnd) {
                        state.cursor = state.clamp_point(point);
                        if state.dragging {
                            state.update_selection(point, shift_down());
                        }
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                if let Some(state) = state.filter(|state| state.dragging) {
                    if let Some(point) = cursor_client_point(hwnd) {
                        state.cursor = state.clamp_point(point);
                        state.update_selection(point, shift_down());
                    }
                    state.dragging = false;
                    let _ = ReleaseCapture();
                    let _ = InvalidateRect(hwnd, null(), 0);
                }
                0
            }
            WM_KEYDOWN => {
                if let Some(state) = state {
                    match wparam as u32 {
                        value if value == u32::from(VK_RETURN) => {
                            if state.dragging {
                                if let Some(point) = cursor_client_point(hwnd) {
                                    state.cursor = state.clamp_point(point);
                                    state.update_selection(point, shift_down());
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

    unsafe fn cursor_client_point(hwnd: HWND) -> Option<POINT> {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 || ScreenToClient(hwnd, &mut point) == 0 {
            return None;
        }
        Some(point)
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
            if state.back_buffer_dc != 0 && state.back_buffer != 0 {
                paint_selector_contents(
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

        if let Some(selection) = state.selection.filter(|value| value.is_valid()) {
            let saved_dc = SaveDC(hdc);
            if saved_dc != 0 {
                let _ = IntersectClipRect(
                    hdc,
                    selection.left,
                    selection.top,
                    selection.right,
                    selection.bottom,
                );
                draw_bitmap(
                    hdc,
                    &state.original_bgra,
                    &state.bitmap_info,
                    state.image_width,
                    state.image_height,
                    client_width,
                    client_height,
                );
                let _ = RestoreDC(hdc, saved_dc);
            }
            draw_selection_decoration(hwnd, hdc, state, selection, client_width, client_height);
        }
        draw_magnifier(hwnd, hdc, state, client_width, client_height);
        draw_instruction(hwnd, hdc, client_width);
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
        let margin = scale(18);
        let border = scale(2).max(2);
        let panel_size = scale(160)
            .min(client_width.saturating_sub(scale(16)).max(1))
            .min(client_height.saturating_sub(scale(16)).max(1));
        let panel_left = if state.cursor.x + margin + panel_size <= client_width {
            state.cursor.x + margin
        } else {
            state.cursor.x - margin - panel_size
        }
        .clamp(0, client_width.saturating_sub(panel_size));
        let panel_top = if state.cursor.y + margin + panel_size <= client_height {
            state.cursor.y + margin
        } else {
            state.cursor.y - margin - panel_size
        }
        .clamp(0, client_height.saturating_sub(panel_size));

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
            selection.left,
            selection.top,
            selection.right,
            selection.bottom,
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
        let label_left = selection.left.clamp(margin, max_left);
        let below = selection.bottom.saturating_add(margin);
        let label_top = if below.saturating_add(label_height) <= client_height - margin {
            below
        } else {
            selection
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

    unsafe fn draw_instruction(hwnd: HWND, hdc: isize, client_width: i32) {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
        let text = wide(
            "拖动选择  ·  松开后方向键微调  ·  Enter 确认  ·  Esc / 右键取消  ·  Shift 锁比例  ·  F1-F4 固定尺寸",
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
