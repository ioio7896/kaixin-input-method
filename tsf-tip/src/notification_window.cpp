#include "notification_window.h"

#include <algorithm>

/// 由 dllmain.cpp 提供，返回 DLL 自身的 HMODULE（非宿主 EXE）。
extern HMODULE SrfTip_GetDllModule();

namespace {

/// 返回 DLL 模块句柄；若不可用则回退到宿主 EXE。
HINSTANCE DllOrFallbackInstance() {
  HMODULE m = SrfTip_GetDllModule();
  return m ? m : GetModuleHandleW(nullptr);
}

constexpr wchar_t kNotificationClass[] = L"SRF_TSF_Notification_Window";
constexpr UINT_PTR kHideTimerId = 17;

UINT CurrentDpi(HWND hwnd) {
  if (hwnd) {
    const UINT dpi = GetDpiForWindow(hwnd);
    if (dpi != 0) return dpi;
  }
  const UINT dpi = GetDpiForSystem();
  return dpi == 0 ? 96u : dpi;
}

LOGFONTW BaseFont() {
  NONCLIENTMETRICSW ncm = {};
  ncm.cbSize = sizeof(ncm);
  if (SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, sizeof(ncm), &ncm, 0)) return ncm.lfMessageFont;

  LOGFONTW font = {};
  font.lfHeight = -14;
  font.lfWeight = FW_MEDIUM;
  lstrcpyW(font.lfFaceName, L"Segoe UI");
  return font;
}

int PointToPixels(int points, UINT dpi) { return -MulDiv(points, static_cast<int>(dpi), 72); }

struct NotificationColors {
  COLORREF bg;
  COLORREF border;
  COLORREF text;
};

NotificationColors ColorsForTone(SrfNotificationTone tone) {
  switch (tone) {
    case SrfNotificationTone::English:
      return {RGB(25, 92, 178), RGB(88, 157, 232), RGB(245, 250, 255)};
    case SrfNotificationTone::Chinese:
      return {RGB(180, 48, 58), RGB(238, 116, 124), RGB(255, 248, 248)};
    default:
      return {RGB(43, 49, 60), RGB(78, 88, 106), RGB(241, 245, 255)};
  }
}

}  // namespace

CNotificationWindow::~CNotificationWindow() { Destroy(); }

void CNotificationWindow::Show(const std::wstring& text, const RECT* anchorRect, UINT timeoutMs,
                               SrfNotificationTone tone) {
  if (text.empty()) return;
  if (!EnsureWindow()) return;

  m_text = text;
  m_tone = tone;
  const RECT rect = CalculateWindowRect(anchorRect);
  SetWindowPos(m_hwnd, HWND_TOPMOST, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top,
               SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW);
  InvalidateRect(m_hwnd, nullptr, TRUE);

  KillTimer(m_hwnd, kHideTimerId);
  if (timeoutMs > 0) SetTimer(m_hwnd, kHideTimerId, timeoutMs, nullptr);
}

void CNotificationWindow::Hide() {
  if (!m_hwnd) return;
  KillTimer(m_hwnd, kHideTimerId);
  ShowWindow(m_hwnd, SW_HIDE);
}

void CNotificationWindow::Destroy() {
  DestroyFont();
  if (m_hwnd) {
    DestroyWindow(m_hwnd);
    m_hwnd = nullptr;
  }
}

bool CNotificationWindow::EnsureWindow() {
  if (m_hwnd) return true;
  if (!EnsureWindowClass()) return false;

  m_hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, kNotificationClass,
                           L"\u5f00\u5fc3\u8f93\u5165\u6cd5\u901a\u77e5", WS_POPUP, CW_USEDEFAULT, CW_USEDEFAULT, 240, 48,
                           nullptr, nullptr, DllOrFallbackInstance(), this);
  if (!m_hwnd) return false;

  m_dpi = CurrentDpi(m_hwnd);
  RefreshFont();
  return true;
}

void CNotificationWindow::RefreshFont() {
  DestroyFont();
  LOGFONTW font = BaseFont();
  font.lfHeight = PointToPixels(12, m_dpi);
  font.lfWeight = FW_SEMIBOLD;
  m_font = CreateFontIndirectW(&font);
}

void CNotificationWindow::DestroyFont() {
  if (m_font) DeleteObject(m_font);
  m_font = nullptr;
}

RECT CNotificationWindow::CalculateWindowRect(const RECT* anchorRect) const {
  SIZE size = MeasureWindow();

  RECT work = {0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)};
  if (anchorRect) {
    MONITORINFO mi = {};
    mi.cbSize = sizeof(mi);
    HMONITOR monitor = MonitorFromRect(anchorRect, MONITOR_DEFAULTTONEAREST);
    if (monitor && GetMonitorInfoW(monitor, &mi)) work = mi.rcWork;
  }

  const int margin = Scale(10);
  int x = work.right - size.cx - margin;
  int y = work.bottom - size.cy - margin;
  if (anchorRect) {
    x = anchorRect->left;
    y = anchorRect->bottom + Scale(10);
    if (x + size.cx > work.right - margin) x = work.right - size.cx - margin;
    if (y + size.cy > work.bottom - margin) y = anchorRect->top - size.cy - Scale(10);
  }

  x = std::clamp<int>(x, work.left + margin, work.right - size.cx - margin);
  y = std::clamp<int>(y, work.top + margin, work.bottom - size.cy - margin);
  return RECT{x, y, x + size.cx, y + size.cy};
}

SIZE CNotificationWindow::MeasureWindow() const {
  SIZE size = {Scale(220), Scale(44)};
  HWND measureWindow = m_hwnd ? m_hwnd : GetDesktopWindow();
  HDC hdc = GetDC(measureWindow);
  if (!hdc) return size;

  HFONT oldFont = static_cast<HFONT>(SelectObject(hdc, m_font ? m_font : GetStockObject(DEFAULT_GUI_FONT)));
  RECT textRect = {0, 0, Scale(320), 0};
  DrawTextW(hdc, m_text.c_str(), static_cast<int>(m_text.size()), &textRect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
  if (oldFont) SelectObject(hdc, oldFont);
  ReleaseDC(measureWindow, hdc);

  size.cx = std::max(size.cx, textRect.right - textRect.left + Scale(28));
  size.cy = std::max(size.cy, textRect.bottom - textRect.top + Scale(20));
  return size;
}

int CNotificationWindow::Scale(int value) const {
  return MulDiv(value, static_cast<int>(m_dpi), 96);
}

int CNotificationWindow::StrokeWidth() const {
  return m_dpi >= 144 ? 2 : 1;
}

void CNotificationWindow::Paint(HDC hdc) {
  RECT client = {};
  GetClientRect(m_hwnd, &client);
  const NotificationColors colors = ColorsForTone(m_tone);

  HBRUSH bg = CreateSolidBrush(colors.bg);
  FillRect(hdc, &client, bg);
  DeleteObject(bg);

  HPEN borderPen = CreatePen(PS_SOLID, StrokeWidth(), colors.border);
  HGDIOBJ oldPen = SelectObject(hdc, borderPen);
  HGDIOBJ oldBrush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
  RoundRect(hdc, client.left, client.top, client.right, client.bottom, Scale(18), Scale(18));
  SelectObject(hdc, oldBrush);
  SelectObject(hdc, oldPen);
  DeleteObject(borderPen);

  HFONT oldFont = static_cast<HFONT>(SelectObject(hdc, m_font ? m_font : GetStockObject(DEFAULT_GUI_FONT)));
  SetBkMode(hdc, TRANSPARENT);
  SetTextColor(hdc, colors.text);
  RECT textRect = client;
  textRect.left += Scale(14);
  textRect.right -= Scale(14);
  DrawTextW(hdc, m_text.c_str(), static_cast<int>(m_text.size()), &textRect,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
  if (oldFont) SelectObject(hdc, oldFont);
}

ATOM CNotificationWindow::EnsureWindowClass() {
  static ATOM atom = 0;
  if (atom) return atom;

  WNDCLASSEXW wc = {};
  wc.cbSize = sizeof(wc);
  wc.style = CS_HREDRAW | CS_VREDRAW;
  wc.lpfnWndProc = &CNotificationWindow::WndProc;
  wc.hInstance = DllOrFallbackInstance();
  wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  wc.lpszClassName = kNotificationClass;
  atom = RegisterClassExW(&wc);
  if (!atom && GetLastError() == ERROR_CLASS_ALREADY_EXISTS) atom = 1;
  return atom;
}

LRESULT CALLBACK CNotificationWindow::WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  CNotificationWindow* self =
      reinterpret_cast<CNotificationWindow*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));

  if (msg == WM_NCCREATE) {
    CREATESTRUCTW* cs = reinterpret_cast<CREATESTRUCTW*>(lParam);
    self = reinterpret_cast<CNotificationWindow*>(cs->lpCreateParams);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    return TRUE;
  }

  switch (msg) {
    case WM_PAINT: {
      if (!self) break;
      PAINTSTRUCT ps = {};
      HDC hdc = BeginPaint(hwnd, &ps);
      self->Paint(hdc);
      EndPaint(hwnd, &ps);
      return 0;
    }
    case WM_TIMER:
      if (self && wParam == kHideTimerId) {
        self->Hide();
        return 0;
      }
      break;
    case WM_DPICHANGED:
      if (self) {
        self->m_dpi = HIWORD(wParam);
        self->RefreshFont();
        const SIZE measured = self->MeasureWindow();
        RECT* suggested = reinterpret_cast<RECT*>(lParam);
        if (suggested) {
          SetWindowPos(hwnd, nullptr, suggested->left, suggested->top,
                       measured.cx, measured.cy,
                       SWP_NOACTIVATE | SWP_NOZORDER);
        } else {
          RECT current = {};
          GetWindowRect(hwnd, &current);
          SetWindowPos(hwnd, nullptr, current.left, current.top, measured.cx, measured.cy,
                       SWP_NOACTIVATE | SWP_NOZORDER);
        }
        InvalidateRect(hwnd, nullptr, TRUE);
      }
      return 0;
    case WM_ERASEBKGND:
      return 1;
    case WM_NCDESTROY:
      if (self) {
        self->DestroyFont();
        if (self->m_hwnd == hwnd) self->m_hwnd = nullptr;
      }
      SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
      return 0;
    default:
      break;
  }

  return DefWindowProcW(hwnd, msg, wParam, lParam);
}
