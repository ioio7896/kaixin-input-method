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
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject,
        IntersectClipRect, InvalidateRect, Rectangle, RestoreDC, SaveDC, ScreenToClient,
        SelectObject, SetBkMode, SetTextColor, StretchDIBits, UpdateWindow, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
        DIB_RGB_COLORS, DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
        FF_DONTCARE, FW_SEMIBOLD, NULL_BRUSH, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PROOF_QUALITY,
        PS_SOLID, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::{
        GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN,
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
        darkened_bgra: Vec<u8>,
        anchor: POINT,
        current: POINT,
        selection: Option<LocalRect>,
        dragging: bool,
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
            let mut darkened_bgra = Vec::with_capacity(required_bytes);
            for pixel in image.as_raw().chunks_exact(4) {
                let red = pixel[0];
                let green = pixel[1];
                let blue = pixel[2];
                original_bgra.extend_from_slice(&[blue, green, red, 255]);
                darkened_bgra.extend_from_slice(&[darken(blue), darken(green), darken(red), 255]);
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
                darkened_bgra,
                anchor: POINT { x: 0, y: 0 },
                current: POINT { x: 0, y: 0 },
                selection: None,
                dragging: false,
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

        fn update_selection(&mut self, point: POINT) {
            self.current = self.clamp_point(point);
            let selection = LocalRect::from_points(self.anchor, self.current);
            self.selection = selection.is_valid().then_some(selection);
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
                        state.selection = None;
                        state.dragging = true;
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_MOUSEMOVE => {
                if let Some(state) = state.filter(|state| state.dragging) {
                    if let Some(point) = cursor_client_point(hwnd) {
                        state.update_selection(point);
                        let _ = InvalidateRect(hwnd, null(), 0);
                    }
                }
                0
            }
            WM_LBUTTONUP => {
                if let Some(state) = state.filter(|state| state.dragging) {
                    if let Some(point) = cursor_client_point(hwnd) {
                        state.update_selection(point);
                    }
                    state.dragging = false;
                    let _ = ReleaseCapture();
                    match state.confirm() {
                        Ok(true) => {
                            let _ = DestroyWindow(hwnd);
                        }
                        Ok(false) => {
                            let _ = InvalidateRect(hwnd, null(), 0);
                        }
                        Err(_) => {
                            state.cancel();
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                0
            }
            WM_KEYDOWN if wparam as u32 == u32::from(VK_RETURN) => {
                if let Some(state) = state {
                    if state.dragging {
                        if let Some(point) = cursor_client_point(hwnd) {
                            state.update_selection(point);
                        }
                        state.dragging = false;
                        let _ = ReleaseCapture();
                    }
                    if state.confirm().unwrap_or(false) {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                0
            }
            WM_KEYDOWN if wparam as u32 == u32::from(VK_ESCAPE) => {
                cancel_and_destroy(hwnd, state);
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

    unsafe fn paint_selector(hwnd: HWND, state: &SelectorState) {
        let mut paint: PAINTSTRUCT = zeroed();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc == 0 {
            return;
        }

        let mut client: RECT = zeroed();
        if GetClientRect(hwnd, &mut client) != 0 {
            let client_width = client.right.saturating_sub(client.left).max(1);
            let client_height = client.bottom.saturating_sub(client.top).max(1);
            // Compose the complete frame off-screen, then present it in one
            // BitBlt. Repainting the dark preview, selection and labels
            // directly on the overlay exposed intermediate frames whenever
            // Windows requested a repaint, which appeared as a periodic
            // flicker while dragging.
            let back_buffer_dc = CreateCompatibleDC(hdc);
            let back_buffer = if back_buffer_dc != 0 {
                CreateCompatibleBitmap(hdc, client_width, client_height)
            } else {
                0
            };
            if back_buffer_dc != 0 && back_buffer != 0 {
                let old_bitmap = SelectObject(back_buffer_dc, back_buffer as _);
                paint_selector_contents(hwnd, back_buffer_dc, state, client_width, client_height);
                let _ = BitBlt(
                    hdc,
                    0,
                    0,
                    client_width,
                    client_height,
                    back_buffer_dc,
                    0,
                    0,
                    SRCCOPY,
                );
                if old_bitmap != 0 {
                    let _ = SelectObject(back_buffer_dc, old_bitmap);
                }
                let _ = DeleteObject(back_buffer as _);
                let _ = DeleteDC(back_buffer_dc);
            } else {
                if back_buffer != 0 {
                    let _ = DeleteObject(back_buffer as _);
                }
                if back_buffer_dc != 0 {
                    let _ = DeleteDC(back_buffer_dc);
                }
                // A memory DC allocation failure must not make region capture
                // unusable; draw directly as a last-resort fallback.
                paint_selector_contents(hwnd, hdc, state, client_width, client_height);
            }
        }
        let _ = EndPaint(hwnd, &paint);
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
            &state.darkened_bgra,
            &state.bitmap_info,
            state.image_width,
            state.image_height,
            client_width,
            client_height,
        );

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
            draw_selection_decoration(hwnd, hdc, selection, client_width, client_height);
        }
        draw_instruction(hwnd, hdc, client_width);
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

    unsafe fn draw_selection_decoration(
        hwnd: HWND,
        hdc: isize,
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

        let label = wide(&format!("{} × {}", selection.width(), selection.height()));
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
        let text = wide("拖动选择区域  ·  松开或 Enter 确认  ·  Esc / 右键取消");
        let width = scale(440)
            .min(client_width.saturating_sub(scale(16)))
            .max(scale(220));
        let height = scale(38);
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
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
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

    fn darken(channel: u8) -> u8 {
        ((u16::from(channel) * 46) / 100) as u8
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_error_exposes_message_and_display() {
        let error = RegionSelectorError::new("test error");
        assert_eq!(error.message(), "test error");
        assert_eq!(error.to_string(), "test error");
    }
}
