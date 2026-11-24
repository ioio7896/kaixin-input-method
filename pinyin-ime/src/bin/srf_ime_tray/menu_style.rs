//! Owner-drawn native tray menu. Windows retains focus, keyboard navigation,
//! scrolling and command dispatch; this module only supplies layout and paint.
use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::path::Path;
use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_SELECTED};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const MENU_WIDTH: i32 = 276;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(super) static POPUP_ROUNDED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_MEASUREITEM => {
                let measure =
                    &mut *(lparam as *mut windows_sys::Win32::UI::Controls::MEASUREITEMSTRUCT);
                let item = &*(measure.itemData as *const Item);
                let dc = GetDC(hwnd);
                measure.itemHeight = item.height();
                measure.itemWidth = item.width(dc);
                ReleaseDC(hwnd, dc);
                1
            }
            WM_DRAWITEM => {
                let paint = &*(lparam as *const DRAWITEMSTRUCT);
                draw(paint, &*(paint.itemData as *const Item));
                1
            }
            WM_TIMER => {
                EndMenu();
                0
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    #[test]
    fn native_popup_rounds_and_releases_its_hook_when_cancelled() {
        unsafe {
            let name = wide("KaixinTrayMenuLifecycleTest");
            let instance =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(owner_proc),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..zeroed()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let hwnd = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                0,
                0,
                instance,
                std::ptr::null(),
            );
            assert_ne!(hwnd, 0);
            let mut menu = Menu::new(POINT { x: 0, y: 0 }).unwrap();
            menu.palette = Palette::light();
            menu.push(Item::header("中文输入", false));
            menu.push(Item::action(100, "输入法设置…", Icon::Settings));
            SetTimer(hwnd, 1, 100, None);
            assert_eq!(menu.track(hwnd, POINT { x: 0, y: 0 }), 0);
            KillTimer(hwnd, 1);
            DestroyWindow(hwnd);
            UnregisterClassW(name.as_ptr(), instance);
            assert!(POPUP_ROUNDED.load(Ordering::Relaxed));
            assert!(POPUP_STYLE.with(Cell::get).is_none());
        }
    }

    #[test]
    fn native_menu_retains_labels_checks_and_stable_item_data() {
        unsafe {
            let mut menu = Menu::new(POINT { x: 0, y: 0 }).unwrap();
            menu.push(Item::section("常用工具"));
            let mut item = Item::action(100, "剪贴板管理器", Icon::Clipboard);
            item.checked = true;
            menu.push(item);
            for id in 101..140 {
                menu.push(Item::action(id, "截图", Icon::Screenshot));
            }
            menu.push(Item::submenu(
                "最近截图",
                Icon::Recent,
                vec![Item::action(140, "识别最近截图", Icon::Ocr)],
            ));
            assert_eq!(GetMenuItemCount(menu.handle), 42);
            let mut name = [0u16; 32];
            let mut info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_DATA | MIIM_STRING,
                dwTypeData: name.as_mut_ptr(),
                cch: name.len() as u32,
                ..zeroed()
            };
            assert_ne!(GetMenuItemInfoW(menu.handle, 1, 1, &mut info), 0);
            assert_eq!(info.wID, 100);
            assert_ne!(info.fType & MFT_OWNERDRAW, 0);
            assert_ne!(info.fState & MFS_CHECKED, 0);
            assert_eq!(info.fState & MFS_DISABLED, 0);
            assert_eq!(
                String::from_utf16_lossy(&name[..info.cch as usize]),
                "剪贴板管理器"
            );
            assert_eq!(
                (info.dwItemData as *const Item).as_ref().unwrap().command,
                100
            );
            info.fMask = MIIM_STATE;
            assert_ne!(GetMenuItemInfoW(menu.handle, 0, 1, &mut info), 0);
            assert_ne!(info.fState & MFS_DISABLED, 0);
            let child = GetSubMenu(menu.handle, 41);
            assert_ne!(child, 0);
            assert_ne!(GetMenuItemInfoW(menu.handle, 41, 1, &mut info), 0);
            assert_eq!(info.fState & MFS_DISABLED, 0);
            assert_eq!(GetMenuItemID(child, 0), 140);
            drop(menu);
            assert_eq!(IsMenu(child), 0);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Status,
    Keyboard,
    Gamepad,
    Settings,
    Clipboard,
    Handwrite,
    Symbols,
    Translate,
    Calculator,
    Screenshot,
    Recent,
    Ocr,
    Exit,
}

pub(super) enum Kind {
    Header(Vec<u16>),
    Section,
    Action,
    Submenu(Vec<Item>),
    Footer,
}

pub(super) struct Item {
    pub command: usize,
    pub text: Vec<u16>,
    pub subtitle: Vec<u16>,
    pub icon: Icon,
    pub checked: bool,
    pub attention: bool,
    pub kind: Kind,
    dpi: i32,
    palette: Palette,
}

impl Item {
    pub fn action(command: usize, text: &str, icon: Icon) -> Self {
        Self {
            command,
            text: wide(text),
            subtitle: vec![0],
            icon,
            checked: false,
            attention: false,
            kind: Kind::Action,
            dpi: 96,
            palette: Palette::light(),
        }
    }

    pub fn header(status: &str, attention: bool) -> Self {
        let mut item = Self::action(0, "开心输入法", Icon::Status);
        item.kind = Kind::Header(wide(status));
        item.attention = attention;
        item
    }

    pub fn submenu(text: &str, icon: Icon, children: Vec<Self>) -> Self {
        let mut item = Self::action(0, text, icon);
        item.kind = Kind::Submenu(children);
        item
    }

    pub fn section(text: &str) -> Self {
        let mut item = Self::action(0, text, Icon::Status);
        item.kind = Kind::Section;
        item
    }

    pub fn height(&self) -> u32 {
        let height = match self.kind {
            Kind::Header(_) => 66,
            Kind::Section => 24,
            Kind::Footer => 36,
            Kind::Action if self.subtitle.len() > 1 => 42,
            Kind::Action | Kind::Submenu(_) => 32,
        };
        px(height, self.dpi) as u32
    }

    pub unsafe fn width(&self, dc: HDC) -> u32 {
        // Account for long/localized labels instead of clipping text to a fixed width.
        let label = text_width(dc, &self.text, 14, self.dpi);
        let subtitle = match &self.kind {
            Kind::Header(status) => text_width(dc, status, 12, self.dpi),
            _ => text_width(dc, &self.subtitle, 11, self.dpi),
        };
        let text = label.max(subtitle);
        (px(MENU_WIDTH, self.dpi).max(text + px(110, self.dpi))) as u32
    }
}

/// Boxes keep itemData stable while Windows tracks the popup. The menu and its
/// background brush outlive TrackPopupMenu and are released together on all exits.
pub(super) struct Menu {
    pub handle: isize,
    #[allow(
        clippy::vec_box,
        reason = "Win32 itemData stores stable pointers while the vector grows."
    )]
    items: Vec<Box<Item>>,
    brush: HBRUSH,
    dpi: i32,
    palette: Palette,
}

impl Menu {
    pub unsafe fn new(anchor: POINT) -> Option<Self> {
        let palette = Palette::system();
        let brush = CreateSolidBrush(palette.background);
        let handle = create_popup(brush);
        if handle == 0 {
            DeleteObject(brush);
            return None;
        }
        let monitor = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let (mut x, mut y) = (96, 96);
        let dpi = if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) >= 0 {
            x as i32
        } else {
            96
        };
        Some(Self {
            handle,
            items: Vec::new(),
            brush,
            dpi,
            palette,
        })
    }

    pub unsafe fn push(&mut self, item: Item) {
        self.insert(self.handle, item);
    }

    unsafe fn insert(&mut self, menu: HMENU, mut item: Item) {
        let (submenu, children) = if let Kind::Submenu(children) = &mut item.kind {
            let submenu = create_popup(self.brush);
            if submenu == 0 {
                return;
            }
            (submenu, std::mem::take(children))
        } else {
            (0, Vec::new())
        };
        item.dpi = self.dpi;
        item.palette = self.palette;
        let item = Box::new(item);
        let mut native_text = match &item.kind {
            Kind::Header(status) => wide(&format!(
                "开心输入法，{}",
                String::from_utf16_lossy(&status[..status.len() - 1])
            )),
            _ if item.subtitle.len() > 1 => wide(&format!(
                "{}，{}",
                String::from_utf16_lossy(&item.text[..item.text.len() - 1]),
                String::from_utf16_lossy(&item.subtitle[..item.subtitle.len() - 1])
            )),
            _ => item.text.clone(),
        };
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE
                | MIIM_ID
                | MIIM_STATE
                | MIIM_DATA
                | MIIM_STRING
                | if submenu != 0 { MIIM_SUBMENU } else { 0 },
            fType: MFT_OWNERDRAW,
            fState: if item.command == 0 && submenu == 0 {
                MFS_DISABLED
            } else {
                MFS_ENABLED
            } | if item.checked { MFS_CHECKED } else { 0 },
            wID: item.command as u32,
            hSubMenu: submenu,
            dwItemData: (&*item as *const Item) as usize,
            // Keep a native text label for accessibility and mnemonic handling.
            dwTypeData: native_text.as_mut_ptr(),
            cch: native_text.len().saturating_sub(1) as u32,
            ..zeroed()
        };
        if InsertMenuItemW(menu, GetMenuItemCount(menu) as u32, 1, &info) == 0 {
            if submenu != 0 {
                DestroyMenu(submenu);
            }
            return;
        }
        self.items.push(item);
        // The parent menu now owns the submenu handle. Keep all item data in
        // the same arena until DestroyMenu has released the whole menu tree.
        for child in children {
            self.insert(submenu, child);
        }
    }

    pub unsafe fn track(&self, owner: HWND, point: POINT) -> u32 {
        let _popup_style = PopupStyle::new(self.dpi, self.palette);
        TrackPopupMenu(
            self.handle,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_VERTICAL | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            owner,
            std::ptr::null(),
        ) as u32
    }
}

unsafe fn create_popup(brush: HBRUSH) -> HMENU {
    let handle = CreatePopupMenu();
    if handle != 0 {
        SetMenuInfo(
            handle,
            &MENUINFO {
                cbSize: size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND | MIM_STYLE,
                dwStyle: MNS_NOCHECK,
                hbrBack: brush,
                ..zeroed()
            },
        );
    }
    handle
}

thread_local! {
    static POPUP_STYLE: Cell<Option<(i32, Palette)>> = const { Cell::new(None) };
}

// A thread-local hook exists only while our native popup is being tracked. It
// rounds the menu's own window without subclassing it or intercepting input.
struct PopupStyle {
    hook: HHOOK,
}
impl PopupStyle {
    unsafe fn new(dpi: i32, palette: Palette) -> Self {
        POPUP_STYLE.with(|style| style.set(Some((dpi, palette))));
        Self {
            hook: SetWindowsHookExW(WH_CALLWNDPROC, Some(style_popup), 0, GetCurrentThreadId()),
        }
    }
}
impl Drop for PopupStyle {
    fn drop(&mut self) {
        unsafe {
            if self.hook != 0 {
                UnhookWindowsHookEx(self.hook);
            }
            POPUP_STYLE.with(|style| style.set(None));
        }
    }
}

unsafe extern "system" fn style_popup(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && lparam != 0 {
        let message = &*(lparam as *const CWPSTRUCT);
        if message.message == WM_WINDOWPOSCHANGED && message.lParam != 0 {
            let pos = &*(message.lParam as *const WINDOWPOS);
            if pos.flags & SWP_NOSIZE == 0 && pos.cx > 0 && pos.cy > 0 {
                let mut class = [0u16; 16];
                let length = GetClassNameW(message.hwnd, class.as_mut_ptr(), class.len() as i32);
                if length == 6 && class[..6] == [35, 51, 50, 55, 54, 56] {
                    // native popup #32768
                    if let Some((dpi, palette)) = POPUP_STYLE.with(Cell::get) {
                        if !palette.high_contrast {
                            let region = CreateRoundRectRgn(
                                0,
                                0,
                                pos.cx + 1,
                                pos.cy + 1,
                                px(20, dpi),
                                px(20, dpi),
                            );
                            if region != 0 {
                                if SetWindowRgn(message.hwnd, region, 0) == 0 {
                                    DeleteObject(region);
                                } else {
                                    #[cfg(test)]
                                    tests::POPUP_ROUNDED
                                        .store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                            // Windows 11 adds compositor corners and a matching subtle border;
                            // Windows 10 retains the native drop shadow and uses the region above.
                            let corner = 2u32;
                            DwmSetWindowAttribute(
                                message.hwnd,
                                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                                (&corner as *const u32).cast(),
                                size_of::<u32>() as u32,
                            );
                            DwmSetWindowAttribute(
                                message.hwnd,
                                DWMWA_BORDER_COLOR as u32,
                                (&palette.line as *const COLORREF).cast(),
                                size_of::<COLORREF>() as u32,
                            );
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            DestroyMenu(self.handle);
            if self.brush != 0 {
                DeleteObject(self.brush);
            }
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}
fn px(value: i32, dpi: i32) -> i32 {
    (value * dpi + 48) / 96
}

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    line: COLORREF,
    hover: COLORREF,
    accent: COLORREF,
    header_start: COLORREF,
    header_end: COLORREF,
    warning_bg: COLORREF,
    warning: COLORREF,
    danger_bg: COLORREF,
    danger: COLORREF,
    high_contrast: bool,
}

impl Palette {
    fn light() -> Self {
        Self {
            background: rgb(250, 251, 255),
            text: rgb(31, 42, 68),
            muted: rgb(82, 96, 121),
            line: rgb(226, 231, 241),
            hover: rgb(232, 238, 255),
            accent: rgb(48, 98, 220),
            header_start: rgb(234, 247, 252),
            header_end: rgb(243, 238, 252),
            warning_bg: rgb(255, 245, 225),
            warning: rgb(165, 94, 14),
            danger_bg: rgb(255, 235, 236),
            danger: rgb(198, 58, 65),
            high_contrast: false,
        }
    }

    fn dark() -> Self {
        Self {
            background: rgb(28, 33, 46),
            text: rgb(232, 238, 252),
            muted: rgb(155, 169, 192),
            line: rgb(55, 64, 83),
            hover: rgb(47, 62, 94),
            accent: rgb(149, 187, 255),
            header_start: rgb(33, 48, 64),
            header_end: rgb(43, 39, 64),
            warning_bg: rgb(66, 49, 31),
            warning: rgb(255, 201, 112),
            danger_bg: rgb(75, 37, 45),
            danger: rgb(255, 156, 162),
            high_contrast: false,
        }
    }

    unsafe fn contrast() -> Self {
        Self {
            background: GetSysColor(COLOR_MENU),
            text: GetSysColor(COLOR_MENUTEXT),
            muted: GetSysColor(COLOR_GRAYTEXT),
            line: GetSysColor(COLOR_MENUTEXT),
            hover: GetSysColor(COLOR_HIGHLIGHT),
            accent: GetSysColor(COLOR_HIGHLIGHTTEXT),
            header_start: GetSysColor(COLOR_MENU),
            header_end: GetSysColor(COLOR_MENU),
            warning_bg: GetSysColor(COLOR_MENU),
            warning: GetSysColor(COLOR_MENUTEXT),
            danger_bg: GetSysColor(COLOR_HIGHLIGHT),
            danger: GetSysColor(COLOR_HIGHLIGHTTEXT),
            high_contrast: true,
        }
    }

    unsafe fn system() -> Self {
        let mut contrast: HIGHCONTRASTW = zeroed();
        contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            (&mut contrast as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
        {
            return Self::contrast();
        }
        let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("AppsUseLightTheme");
        let (mut light, mut size) = (1u32, 4u32);
        let status = RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut size,
        );
        if status == 0 && light == 0 {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

fn blend(a: COLORREF, b: COLORREF, weight: u32) -> COLORREF {
    let channel = |shift: u32| {
        (((a >> shift) & 255u32) * (255 - weight) + ((b >> shift) & 255u32) * weight) / 255
    };
    rgb(channel(0) as u8, channel(8) as u8, channel(16) as u8)
}

unsafe fn fill(dc: HDC, rect: RECT, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    if brush != 0 {
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}

unsafe fn rounded(dc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    if brush == 0 {
        return;
    }
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius * 2,
        radius * 2,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(brush);
}

unsafe fn gradient(dc: HDC, rect: RECT, dpi: i32, start: COLORREF, end: COLORREF) {
    let corner = if rect.right - rect.left <= px(26, dpi) {
        14
    } else {
        20
    };
    let region = CreateRoundRectRgn(
        rect.left,
        rect.top,
        rect.right + 1,
        rect.bottom + 1,
        px(corner, dpi),
        px(corner, dpi),
    );
    let saved = SaveDC(dc);
    if saved != 0 && region != 0 {
        SelectClipRgn(dc, region);
    }
    let width = (rect.right - rect.left).max(1);
    for x in rect.left..rect.right {
        fill(
            dc,
            RECT {
                left: x,
                right: x + 1,
                ..rect
            },
            blend(start, end, ((x - rect.left) * 255 / width) as u32),
        );
    }
    if saved != 0 {
        RestoreDC(dc, saved);
    }
    if region != 0 {
        DeleteObject(region);
    }
}

unsafe fn font(size: i32, dpi: i32, weight: i32) -> HFONT {
    let family = wide("Microsoft YaHei UI");
    CreateFontW(
        -px(size, dpi),
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        CLEARTYPE_QUALITY as u32,
        (DEFAULT_PITCH | FF_SWISS) as u32,
        family.as_ptr(),
    )
}

unsafe fn text_width(dc: HDC, text: &[u16], size: i32, dpi: i32) -> i32 {
    let font = font(size, dpi, 400);
    let old = SelectObject(dc, font);
    let mut extent: SIZE = zeroed();
    GetTextExtentPoint32W(
        dc,
        text.as_ptr(),
        text.len().saturating_sub(1) as i32,
        &mut extent,
    );
    SelectObject(dc, old);
    if font != 0 {
        DeleteObject(font);
    }
    extent.cx
}

unsafe fn label(
    dc: HDC,
    text: &[u16],
    mut rect: RECT,
    size: i32,
    dpi: i32,
    weight: i32,
    color: COLORREF,
) {
    let font = font(size, dpi, weight);
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(
        dc,
        text.as_ptr(),
        text.len().saturating_sub(1) as i32,
        &mut rect,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(dc, old);
    if font != 0 {
        DeleteObject(font);
    }
}

fn rect_at(rect: RECT, dpi: i32, x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT {
        left: rect.left + px(x, dpi),
        top: rect.top + px(y, dpi),
        right: rect.left + px(x + w, dpi),
        bottom: rect.top + px(y + h, dpi),
    }
}

unsafe fn smile(dc: HDC, rect: RECT, color: COLORREF, dpi: i32) {
    let pen = CreatePen(PS_SOLID, px(2, dpi).max(1), color);
    if pen == 0 {
        return;
    }
    let old_pen = SelectObject(dc, pen);
    let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
    for curve in [
        &[(5, 9), (5, 5), (9, 5), (9, 9)],
        &[(15, 9), (15, 5), (19, 5), (19, 9)],
        &[(5, 14), (8, 21), (16, 21), (19, 14)],
    ] {
        let points: Vec<_> = curve.iter().map(|&(x, y)| menu_point(rect, x, y)).collect();
        PolyBezier(dc, points.as_ptr(), points.len() as u32);
    }
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
}

pub(super) unsafe fn draw(draw: &DRAWITEMSTRUCT, item: &Item) {
    // Restore all caller DC state, including colors, clipping and selected objects.
    let saved = SaveDC(draw.hDC);
    paint(draw, item);
    if saved != 0 {
        RestoreDC(draw.hDC, saved);
    }
}

unsafe fn paint(draw: &DRAWITEMSTRUCT, item: &Item) {
    let (dc, r, d, p) = (draw.hDC, draw.rcItem, item.dpi, item.palette);
    fill(dc, r, p.background);
    match &item.kind {
        Kind::Header(ref status) => {
            let panel = RECT {
                left: r.left + px(6, d),
                top: r.top + px(5, d),
                right: r.right - px(6, d),
                bottom: r.bottom - px(3, d),
            };
            gradient(dc, panel, d, p.header_start, p.header_end);
            let logo = rect_at(r, d, 17, 16, 34, 34);
            if p.high_contrast {
                rounded(dc, logo, px(8, d), p.text);
                smile(dc, rect_at(r, d, 21, 20, 26, 26), p.background, d);
            } else {
                gradient(dc, logo, d, rgb(42, 122, 184), rgb(114, 78, 199));
                smile(dc, rect_at(r, d, 21, 20, 26, 26), rgb(255, 255, 255), d);
            }
            label(
                dc,
                &item.text,
                RECT {
                    right: r.right - px(16, d),
                    ..rect_at(r, d, 62, 12, 194, 24)
                },
                18,
                d,
                700,
                p.text,
            );
            label(
                dc,
                status,
                RECT {
                    right: r.right - px(16, d),
                    ..rect_at(r, d, 62, 37, 194, 17)
                },
                12,
                d,
                400,
                if item.attention {
                    p.warning
                } else if p.high_contrast {
                    p.text
                } else {
                    p.muted
                },
            );
            return;
        }
        Kind::Section => {
            let text_r = rect_at(r, d, 17, 0, 130, 24);
            let section_ink = if p.high_contrast { p.text } else { p.muted };
            label(dc, &item.text, text_r, 11, d, 500, section_ink);
            let start = r.left + px(29, d) + text_width(dc, &item.text, 11, d);
            let y = r.top + px(12, d);
            fill(
                dc,
                RECT {
                    left: start,
                    top: y,
                    right: r.right - px(18, d),
                    bottom: y + px(1, d),
                },
                p.line,
            );
            return;
        }
        _ => {}
    }
    let selected = draw.itemState & ODS_SELECTED != 0;
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let footer = matches!(item.kind, Kind::Footer);
    let retry = matches!(item.icon, Icon::Status);
    let settings = matches!(item.icon, Icon::Settings);
    let active = RECT {
        left: r.left + px(7, d),
        right: r.right - px(7, d),
        top: r.top + px(if footer { 5 } else { 1 }, d),
        bottom: r.bottom - px(1, d),
    };
    if footer {
        fill(
            dc,
            RECT {
                bottom: r.top + px(1, d),
                ..r
            },
            p.line,
        );
    }
    if selected || (retry && item.attention) {
        let color = if footer && selected {
            p.danger_bg
        } else if selected {
            p.hover
        } else {
            p.warning_bg
        };
        rounded(dc, active, px(7, d), color);
    }
    let ink = if disabled {
        p.muted
    } else if selected && footer {
        p.danger
    } else if selected {
        p.accent
    } else if retry && item.attention {
        p.warning
    } else if footer && !p.high_contrast {
        p.muted
    } else {
        p.text
    };
    let icon_top = r.top + (r.bottom - r.top - px(18, d)) / 2 + if footer { px(2, d) } else { 0 };
    let icon_r = RECT {
        left: r.left + px(19, d),
        top: icon_top,
        right: r.left + px(37, d),
        bottom: icon_top + px(18, d),
    };
    if retry {
        draw_retry(dc, icon_r, ink, d);
    } else {
        draw_vector_menu_icon(
            dc,
            item.icon,
            icon_r,
            if selected || p.high_contrast {
                ink
            } else if settings {
                p.accent
            } else {
                p.muted
            },
            d,
        );
    }
    let label_r = RECT {
        left: r.left + px(47, d),
        right: r.right - px(53, d),
        top: r.top + if footer { px(4, d) } else { 0 },
        bottom: r.bottom,
    };
    if item.subtitle.len() > 1 {
        label(
            dc,
            &item.text,
            RECT {
                top: r.top + px(2, d),
                bottom: r.top + px(23, d),
                ..label_r
            },
            14,
            d,
            500,
            ink,
        );
        label(
            dc,
            &item.subtitle,
            RECT {
                top: r.top + px(21, d),
                bottom: r.bottom - px(2, d),
                ..label_r
            },
            11,
            d,
            400,
            if selected || p.high_contrast {
                ink
            } else {
                p.muted
            },
        );
    } else {
        label(
            dc,
            &item.text,
            label_r,
            14,
            d,
            if retry || settings { 600 } else { 400 },
            ink,
        );
    }
    if item.subtitle.len() > 1 {
        let switch = RECT {
            left: r.right - px(52, d),
            right: r.right - px(18, d),
            top: r.top + px(12, d),
            bottom: r.top + px(30, d),
        };
        rounded(
            dc,
            switch,
            px(9, d),
            if item.checked {
                if p.high_contrast {
                    p.hover
                } else {
                    p.accent
                }
            } else {
                p.line
            },
        );
        let x = if item.checked {
            switch.right - px(16, d)
        } else {
            switch.left + px(2, d)
        };
        rounded(
            dc,
            RECT {
                left: x,
                right: x + px(14, d),
                top: switch.top + px(2, d),
                bottom: switch.bottom - px(2, d),
            },
            px(7, d),
            if p.high_contrast {
                if item.checked {
                    p.accent
                } else {
                    p.background
                }
            } else {
                rgb(255, 255, 255)
            },
        );
    } else if item.checked {
        draw_menu_checkmark(dc, r.right - px(30, d), (r.top + r.bottom) / 2, p.accent, d);
    }
}

unsafe fn draw_retry(dc: HDC, rect: RECT, color: COLORREF, dpi: i32) {
    let pen = CreatePen(PS_SOLID, px(2, dpi).max(1), color);
    if pen == 0 {
        return;
    }
    let old = SelectObject(dc, pen);
    menu_path(
        dc,
        rect,
        &[
            (19, 8),
            (17, 5),
            (12, 3),
            (7, 4),
            (3, 8),
            (3, 14),
            (7, 19),
            (13, 21),
            (18, 18),
        ],
    );
    menu_path(dc, rect, &[(19, 2), (19, 8), (13, 8)]);
    SelectObject(dc, old);
    DeleteObject(pen);
}

/// Render the exact production paint routines without starting helpers or touching
/// user configuration. Used for visual QA at fractional DPI and theme variants.
pub(super) unsafe fn preview(
    path: &Path,
    items: Vec<Item>,
    dpi: i32,
    theme: &str,
    selected_command: usize,
) -> Result<(), String> {
    let palette = match theme {
        "dark" => Palette::dark(),
        "contrast" => Palette::contrast(),
        _ => Palette::light(),
    };
    let mut items = items;
    for item in &mut items {
        item.dpi = dpi.clamp(96, 384);
        item.palette = palette;
    }
    let dc = CreateCompatibleDC(0);
    if dc == 0 {
        return Err("CreateCompatibleDC failed".into());
    }
    let width = items.iter().map(|item| item.width(dc)).max().unwrap_or(320);
    let height: u32 = items.iter().map(Item::height).sum();
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        },
        ..zeroed()
    };
    let mut bits = std::ptr::null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
    if bitmap == 0 || bits.is_null() {
        DeleteDC(dc);
        return Err("CreateDIBSection failed".into());
    }
    let old = SelectObject(dc, bitmap);
    let mut y = 0;
    for item in &items {
        let bottom = y + item.height() as i32;
        let draw_item = DRAWITEMSTRUCT {
            hDC: dc,
            rcItem: RECT {
                left: 0,
                top: y,
                right: width as i32,
                bottom,
            },
            itemState: if item.command != 0 && item.command == selected_command {
                ODS_SELECTED
            } else {
                0
            },
            ..zeroed()
        };
        draw(&draw_item, item);
        y = bottom;
    }
    GdiFlush();
    let buffer =
        std::slice::from_raw_parts(bits.cast::<u8>(), width as usize * height as usize * 4);
    let pixels: Vec<u8> = buffer
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[2], p[1], p[0], 255])
        .collect();
    SelectObject(dc, old);
    DeleteObject(bitmap);
    DeleteDC(dc);
    image::save_buffer(path, &pixels, width, height, image::ColorType::Rgba8)
        .map_err(|e| e.to_string())
}

fn menu_point(rect: RECT, x: i32, y: i32) -> POINT {
    POINT {
        x: rect.left + x * (rect.right - rect.left) / 24,
        y: rect.top + y * (rect.bottom - rect.top) / 24,
    }
}

unsafe fn menu_line(hdc: HDC, rect: RECT, x1: i32, y1: i32, x2: i32, y2: i32) {
    let a = menu_point(rect, x1, y1);
    let b = menu_point(rect, x2, y2);
    let _ = MoveToEx(hdc, a.x, a.y, std::ptr::null_mut());
    let _ = LineTo(hdc, b.x, b.y);
}

unsafe fn menu_path(hdc: HDC, rect: RECT, points: &[(i32, i32)]) {
    for pair in points.windows(2) {
        menu_line(hdc, rect, pair[0].0, pair[0].1, pair[1].0, pair[1].1);
    }
}

unsafe fn menu_circle(hdc: HDC, rect: RECT, cx: i32, cy: i32, radius: i32) {
    let top_left = menu_point(rect, cx - radius, cy - radius);
    let bottom_right = menu_point(rect, cx + radius, cy + radius);
    let _ = Ellipse(hdc, top_left.x, top_left.y, bottom_right.x, bottom_right.y);
}

unsafe fn menu_rect(hdc: HDC, rect: RECT, left: i32, top: i32, right: i32, bottom: i32) {
    let a = menu_point(rect, left, top);
    let b = menu_point(rect, right, bottom);
    let _ = Rectangle(hdc, a.x, a.y, b.x, b.y);
}

unsafe fn draw_menu_checkmark(hdc: HDC, x: i32, y: i32, color: COLORREF, scale: i32) {
    let pen = CreatePen(PS_SOLID, px(1, scale).max(1), color);
    if pen == 0 {
        return;
    }
    let old_pen = SelectObject(hdc, pen as _);
    let _ = MoveToEx(hdc, x, y, std::ptr::null_mut());
    let _ = LineTo(hdc, x + px(2, scale), y + px(2, scale));
    let _ = LineTo(hdc, x + px(6, scale), y - px(3, scale));
    if old_pen != 0 {
        let _ = SelectObject(hdc, old_pen);
    }
    let _ = DeleteObject(pen as _);
}

unsafe fn fill_menu_circle(hdc: HDC, rect: RECT, cx: i32, cy: i32, radius: i32, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    if brush == 0 {
        return;
    }
    let old_brush = SelectObject(hdc, brush as _);
    menu_circle(hdc, rect, cx, cy, radius);
    if old_brush != 0 {
        let _ = SelectObject(hdc, old_brush);
    }
    let _ = DeleteObject(brush as _);
}

unsafe fn draw_vector_menu_icon(hdc: HDC, icon: Icon, rect: RECT, color: COLORREF, scale: i32) {
    let pen = CreatePen(PS_SOLID, px(1, scale).max(1), color);
    if pen == 0 {
        return;
    }
    let old_pen = SelectObject(hdc, pen as _);
    let old_brush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));
    let _ = SetBkMode(hdc, TRANSPARENT as i32);

    match icon {
        Icon::Status => fill_menu_circle(hdc, rect, 12, 12, 4, color),
        Icon::Keyboard => {
            menu_rect(hdc, rect, 3, 5, 21, 19);
            menu_line(hdc, rect, 6, 9, 8, 9);
            menu_line(hdc, rect, 10, 9, 12, 9);
            menu_line(hdc, rect, 14, 9, 16, 9);
            menu_line(hdc, rect, 18, 9, 19, 9);
            menu_line(hdc, rect, 7, 14, 17, 14);
        }
        Icon::Gamepad => {
            menu_path(
                hdc,
                rect,
                &[
                    (3, 10),
                    (5, 6),
                    (19, 6),
                    (21, 10),
                    (22, 19),
                    (19, 20),
                    (15, 16),
                    (9, 16),
                    (5, 20),
                    (2, 19),
                    (3, 10),
                ],
            );
            menu_line(hdc, rect, 7, 9, 7, 14);
            menu_line(hdc, rect, 4, 12, 10, 12);
            fill_menu_circle(hdc, rect, 16, 10, 1, color);
            fill_menu_circle(hdc, rect, 19, 13, 1, color);
        }
        Icon::Settings => {
            menu_circle(hdc, rect, 12, 12, 6);
            menu_circle(hdc, rect, 12, 12, 2);
            menu_line(hdc, rect, 12, 2, 12, 5);
            menu_line(hdc, rect, 12, 19, 12, 22);
            menu_line(hdc, rect, 2, 12, 5, 12);
            menu_line(hdc, rect, 19, 12, 22, 12);
            menu_line(hdc, rect, 5, 5, 7, 7);
            menu_line(hdc, rect, 17, 17, 19, 19);
            menu_line(hdc, rect, 19, 5, 17, 7);
            menu_line(hdc, rect, 7, 17, 5, 19);
        }
        Icon::Clipboard => {
            menu_rect(hdc, rect, 6, 5, 19, 21);
            menu_rect(hdc, rect, 9, 3, 16, 7);
            menu_line(hdc, rect, 9, 11, 16, 11);
            menu_line(hdc, rect, 9, 15, 16, 15);
            menu_line(hdc, rect, 9, 19, 13, 19);
        }
        Icon::Handwrite => {
            menu_path(hdc, rect, &[(5, 18), (7, 19), (18, 8), (14, 4), (5, 18)]);
            menu_line(hdc, rect, 14, 4, 17, 7);
            menu_line(hdc, rect, 5, 18, 4, 21);
            menu_line(hdc, rect, 4, 21, 7, 19);
        }
        Icon::Symbols => {
            menu_rect(hdc, rect, 3, 3, 21, 21);
            for offset in [9, 15] {
                menu_line(hdc, rect, offset, 3, offset, 21);
                menu_line(hdc, rect, 3, offset, 21, offset);
            }
        }
        Icon::Translate => {
            menu_path(hdc, rect, &[(4, 18), (8, 6), (12, 18)]);
            menu_line(hdc, rect, 6, 14, 10, 14);
            menu_line(hdc, rect, 15, 5, 15, 18);
            menu_line(hdc, rect, 13, 8, 19, 8);
            menu_line(hdc, rect, 16, 12, 20, 18);
            menu_line(hdc, rect, 20, 12, 16, 18);
        }
        Icon::Calculator => {
            menu_rect(hdc, rect, 5, 3, 19, 21);
            menu_line(hdc, rect, 7, 7, 17, 7);
            menu_line(hdc, rect, 8, 12, 11, 12);
            menu_line(hdc, rect, 13, 12, 16, 12);
            menu_line(hdc, rect, 8, 16, 11, 16);
            menu_line(hdc, rect, 13, 16, 16, 16);
        }
        Icon::Screenshot => {
            menu_line(hdc, rect, 3, 8, 3, 3);
            menu_line(hdc, rect, 3, 3, 8, 3);
            menu_line(hdc, rect, 16, 3, 21, 3);
            menu_line(hdc, rect, 21, 3, 21, 8);
            menu_line(hdc, rect, 3, 16, 3, 21);
            menu_line(hdc, rect, 3, 21, 8, 21);
            menu_line(hdc, rect, 16, 21, 21, 21);
            menu_line(hdc, rect, 21, 21, 21, 16);
        }
        Icon::Recent => {
            menu_circle(hdc, rect, 12, 12, 8);
            menu_path(hdc, rect, &[(12, 7), (12, 12), (16, 14)]);
        }
        Icon::Ocr => {
            menu_rect(hdc, rect, 5, 3, 19, 21);
            menu_line(hdc, rect, 8, 8, 16, 8);
            menu_line(hdc, rect, 8, 12, 16, 12);
            menu_line(hdc, rect, 8, 16, 14, 16);
        }
        Icon::Exit => {
            menu_circle(hdc, rect, 12, 13, 7);
            menu_line(hdc, rect, 12, 3, 12, 12);
        }
    }

    if old_brush != 0 {
        let _ = SelectObject(hdc, old_brush);
    }
    if old_pen != 0 {
        let _ = SelectObject(hdc, old_pen);
    }
    let _ = DeleteObject(pen as _);
}
