#include "candidate_window_internal.h"
#include "candidate_layout_policy.h"
#include <windowsx.h>
using namespace srf_candidate_window_detail;

ATOM CCandidateWindow::EnsureWindowClass() {
  WNDCLASSW wc = {};
  wc.lpfnWndProc = &CCandidateWindow::WndProc;
  wc.hInstance = DllOrFallbackInstance();
  wc.lpszClassName = kCandidateWndClass;
  wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  const ATOM atom = RegisterClassW(&wc);
  return !atom && GetLastError() == ERROR_CLASS_ALREADY_EXISTS ? 1 : atom;
}

LRESULT CALLBACK CCandidateWindow::WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  CCandidateWindow* self =
      reinterpret_cast<CCandidateWindow*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));

  if (msg == WM_NCCREATE) {
    CREATESTRUCTW* cs = reinterpret_cast<CREATESTRUCTW*>(lParam);
    self = reinterpret_cast<CCandidateWindow*>(cs->lpCreateParams);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    if (self) self->m_hwnd = hwnd;
    return TRUE;
  }

  if (!self) return DefWindowProcW(hwnd, msg, wParam, lParam);

  switch (msg) {
    case WM_GETOBJECT:
      return self->m_accessibility.GetObject(hwnd, wParam, lParam);
    case WM_SHOWWINDOW:
      self->m_ignoreStationaryMouse = wParam && GetCursorPos(&self->m_cursorAtShow);
      NotifyWinEvent(wParam ? EVENT_OBJECT_IME_SHOW : EVENT_OBJECT_IME_HIDE, hwnd, OBJID_CLIENT,
                     CHILDID_SELF);
      if (wParam)
        self->UpdateAccessibility();
      else
        self->m_accessibility.Hide();
      break;
    case WM_WINDOWPOSCHANGED: {
      const auto* position = reinterpret_cast<const WINDOWPOS*>(lParam);
      if (position && IsWindowVisible(hwnd) &&
          ((position->flags & SWP_NOMOVE) == 0 || (position->flags & SWP_NOSIZE) == 0)) {
        NotifyWinEvent(EVENT_OBJECT_IME_CHANGE, hwnd, OBJID_CLIENT, CHILDID_SELF);
        self->UpdateAccessibility();
      }
      break;
    }
    case SrfCandidateInputPane::kChangedMessage:
      if (self->m_hasAnchorRect && IsWindowVisible(hwnd)) {
        self->ApplyWindowRect(
            self->CalculateWindowRect(self->m_anchorRect, self->m_measuredClientSize));
        self->ScheduleEnvironmentRefresh();
      }
      return 0;
    case SrfCandidateAccessibility::kSelectMessage:
    case SrfCandidateAccessibility::kInvokeMessage: {
      const UINT index = static_cast<UINT>(wParam);
      const bool ownerFocused = self->m_ownerHwnd && IsWindow(self->m_ownerHwnd) &&
                                GetAncestor(GetForegroundWindow(), GA_ROOT) == self->m_ownerHwnd;
      if (ownerFocused && self->m_events && self->m_interactive && !self->m_pendingVisual &&
          self->m_accessibility.CanAct(index, static_cast<ULONG>(lParam))) {
        self->m_events->OnCandidateInteractionStarted();
        if (msg == SrfCandidateAccessibility::kInvokeMessage)
          self->m_events->OnCandidateClicked(index);
        else
          self->m_events->OnCandidateSelected(index);
      }
      return 0;
    }
    case WM_DESTROY:
      if (self->m_shadowHwnd && IsWindow(self->m_shadowHwnd)) DestroyWindow(self->m_shadowHwnd);
      self->m_shadowHwnd = nullptr;
      self->m_inputPane.Detach();
      self->m_accessibility.Destroy();
      self->m_environmentRefreshTimerPending = false;
      self->m_animationTimerPending = false;
      self->m_paintTimerPending = false;
      self->m_horizontalShrinkTimerPending = false;
      self->m_pendingIndicatorTimerPending = false;
      self->m_hasAnchorRect = false;
      self->m_layoutDirty = true;
      self->m_needsMeasure = true;
      break;
    case WM_NCDESTROY:
      SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
      if (self->m_hwnd == hwnd) {
        self->m_hwnd = nullptr;
        self->m_shadowHwnd = nullptr;
      }
      return DefWindowProcW(hwnd, msg, wParam, lParam);
    case WM_NCHITTEST:
      if (self->m_gameOverlay) return HTTRANSPARENT;
      break;
    case WM_MOUSEACTIVATE:
      return MA_NOACTIVATE;
    case WM_ERASEBKGND:
      return 1;
    case WM_TIMER:
      if (wParam == kCandidateAnimationTimerId) {
        self->AdvanceAnimations();
        return 0;
      }
      if (wParam == kCandidatePaintTimerId) {
        self->FlushDeferredPaint();
        return 0;
      }
      if (wParam == kCandidatePendingIndicatorTimerId) {
        self->m_pendingIndicatorTimerPending = false;
        KillTimer(hwnd, kCandidatePendingIndicatorTimerId);
        if (self->m_pendingVisual) {
          self->m_staticPaintDirty = true;
          InvalidateRect(hwnd, nullptr, FALSE);
        }
        return 0;
      }
      if (wParam == kCandidateHorizontalShrinkTimerId) {
        self->FlushPendingHorizontalShrink();
        return 0;
      }
      if (wParam == kCandidateEnvironmentRefreshTimerId) {
        self->FlushEnvironmentRefresh();
        return 0;
      }
      break;
    case WM_DISPLAYCHANGE:
      self->ScheduleEnvironmentRefresh();
      SrfTsfDiagnosticLog(L"candidate-window.environment", L"display-change");
      return 0;
    case WM_SETTINGCHANGE:
      self->RefreshSystemAppearance();
      if (wParam == SPI_SETWORKAREA) {
        self->ScheduleEnvironmentRefresh();
        SrfTsfDiagnosticLog(L"candidate-window.environment", L"work-area-change");
        return 0;
      }
      break;
    case WM_THEMECHANGED:
    case WM_SYSCOLORCHANGE:
      self->RefreshSystemAppearance();
      break;
    case WM_DWMCOMPOSITIONCHANGED:
      self->ScheduleEnvironmentRefresh();
      return 0;
    case WM_DPICHANGED: {
      self->CancelPendingHorizontalShrink();
      self->m_dpi = HIWORD(wParam) != 0 ? HIWORD(wParam) : LOWORD(wParam);
      if (self->m_dpi == 0) self->m_dpi = 96;
      self->m_layoutDirty = true;
      self->m_needsMeasure = true;
      self->m_measuredClientSize = {};
      self->RefreshFonts();
      self->m_itemRects.clear();
      self->m_pinMenuRect = {};
      self->m_pinMenuPinRect = {};
      self->m_pinMenuUnpinRect = {};
      self->m_pinMenuRemoveRect = {};
      self->m_pinMenuBlockRect = {};
      self->m_pinMenuSourceRect = {};
      self->m_pinMenuVisible = false;
      self->m_pinMenuIndex = 0;
      self->m_pinMenuItemPinned = false;
      self->m_pinMenuHotCommand = kPinMenuCommandNone;
      self->m_pinMenuPressedCommand = kPinMenuCommandNone;
      self->ReleasePaintBuffer();
      self->ReleaseStaticPaintBuffer();
      self->ReleaseItemPaintCaches();
      self->ClearMeasuredTextCaches();
      ResetGdiCache();
      RECT* suggested = reinterpret_cast<RECT*>(lParam);
      if (suggested) {
        self->ApplyWindowRect(*suggested);
      }
      InvalidateRect(hwnd, nullptr, FALSE);
      self->ScheduleEnvironmentRefresh();
      return 0;
    }
    case WM_SETCURSOR: {
      if (!self->m_interactive && LOWORD(lParam) == HTCLIENT) {
        SetCursor(LoadCursorW(nullptr, IDC_ARROW));
        return TRUE;
      }
      if (LOWORD(lParam) == HTCLIENT) {
        POINT pt = {};
        if (GetCursorPos(&pt) && ScreenToClient(hwnd, &pt)) {
          const bool overMenuCommand =
              self->m_pinMenuVisible && self->HitTestPinMenu(pt) != kPinMenuCommandNone;
          const bool overCandidate =
              !self->m_pinMenuVisible &&
              (self->m_style.candidateLeftClick || self->m_style.candidateRightClick) &&
              self->HitTest(pt) >= 0;
          SetCursor(LoadCursorW(nullptr, overMenuCommand || overCandidate ? IDC_HAND : IDC_ARROW));
          return TRUE;
        }
      }
      break;
    }
    case WM_MOUSEMOVE: {
      if (!self->m_interactive || !IsWindowVisible(hwnd)) {
        self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
        self->UpdateHotIndex(-1);
        return 0;
      }
      self->BeginTrackMouseLeave();
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (self->m_ignoreStationaryMouse) {
        POINT screenPoint = pt;
        if (ClientToScreen(hwnd, &screenPoint) &&
            !CandidatePointerMoved(self->m_cursorAtShow.x, self->m_cursorAtShow.y, screenPoint.x,
                                   screenPoint.y))
          return 0;
        self->m_ignoreStationaryMouse = false;
      }
      if (self->m_pinMenuVisible) {
        const int rawPinCommand = self->HitTestPinMenuRaw(pt);
        self->UpdatePinMenuHotCommand(
            self->PinMenuCommandEnabled(rawPinCommand) ? rawPinCommand : kPinMenuCommandNone);
        self->UpdateHotIndex(-1);
      } else {
        self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
        self->UpdateHotIndex(self->HitTest(pt));
      }
      return 0;
    }
    case WM_MOUSELEAVE: {
      self->m_trackingMouse = false;
      POINT cursor = {};
      if (self->m_ignoreStationaryMouse && GetCursorPos(&cursor)) {
        self->m_ignoreStationaryMouse = !CandidatePointerMoved(
            self->m_cursorAtShow.x, self->m_cursorAtShow.y, cursor.x, cursor.y);
      }
      self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
      self->UpdateHotIndex(-1);
      return 0;
    }
    case WM_LBUTTONDOWN: {
      if (!self->m_interactive) return 0;
      self->m_rightPressedIndex = -1;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (self->m_pinMenuVisible) {
        const int rawPinCommand = self->HitTestPinMenuRaw(pt);
        const int pinCommand =
            self->PinMenuCommandEnabled(rawPinCommand) ? rawPinCommand : kPinMenuCommandNone;
        if (pinCommand != kPinMenuCommandNone) {
          SetCapture(hwnd);
          self->UpdatePinMenuHotCommand(pinCommand);
          self->m_pinMenuPressedCommand = pinCommand;
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        } else if (rawPinCommand == kPinMenuCommandNone) {
          self->HidePinMenu();
        } else {
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        }
        return 0;
      }
      if (!self->m_style.candidateLeftClick) return 0;
      if (self->HitTest(pt) >= 0 && self->m_events) {
        self->m_events->OnCandidateInteractionStarted();
      }
      SetCapture(hwnd);
      self->UpdatePressedIndex(self->HitTest(pt));
      return 0;
    }
    case WM_LBUTTONUP: {
      if (!self->m_interactive) {
        if (GetCapture() == hwnd) ReleaseCapture();
        self->UpdatePressedIndex(-1);
        return 0;
      }
      const int pressed = self->m_pressedIndex;
      const int pressedCommand = self->m_pinMenuPressedCommand;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (GetCapture() == hwnd) ReleaseCapture();
      if (self->m_pinMenuVisible || self->m_pinMenuPressedCommand != kPinMenuCommandNone) {
        const int pinCommand = self->HitTestPinMenu(pt);
        self->m_pinMenuPressedCommand = kPinMenuCommandNone;
        if (pinCommand != kPinMenuCommandNone && pinCommand == pressedCommand && self->m_events) {
          const UINT indexInPage = self->m_pinMenuIndex;
          self->HidePinMenu();
          if (pinCommand == kPinMenuCommandPin || pinCommand == kPinMenuCommandUnpin) {
            self->m_events->OnCandidatePinRequested(indexInPage, pinCommand == kPinMenuCommandPin);
          } else {
            self->m_events->OnCandidateMenuCommand(indexInPage, pinCommand);
          }
        } else {
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        }
        return 0;
      }
      if (!self->m_style.candidateLeftClick) return 0;
      const int hit = self->HitTest(pt);
      self->UpdatePressedIndex(-1);
      if (hit >= 0 && hit == pressed && self->m_events) {
        self->m_events->OnCandidateClicked(static_cast<UINT>(hit));
      }
      return 0;
    }
    case WM_RBUTTONUP: {
      if (!self->m_interactive) {
        if (GetCapture() == hwnd) ReleaseCapture();
        self->m_rightPressedIndex = -1;
        self->UpdatePressedIndex(-1);
        return 0;
      }
      if (!self->m_style.candidateRightClick) return 0;
      const int pressed = self->m_rightPressedIndex;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (GetCapture() == hwnd) ReleaseCapture();
      const int hit = self->HitTest(pt);
      self->m_rightPressedIndex = -1;
      self->UpdatePressedIndex(-1);
      if (hit >= 0 && hit == pressed && self->m_events) {
        std::wstring line = L"hit=";
        line += std::to_wstring(hit);
        line += L", pt=(";
        line += std::to_wstring(pt.x);
        line += L",";
        line += std::to_wstring(pt.y);
        line += L")";
        SrfTsfDiagnosticLog(L"candidate-pin-menu.right-click", line.c_str());
        POINT screenPt = pt;
        ClientToScreen(hwnd, &screenPt);
        self->m_events->OnCandidateRightClicked(static_cast<UINT>(hit), screenPt);
        self->ShowPinMenu(static_cast<UINT>(hit));
      } else {
        SrfTsfDiagnosticLog(L"candidate-pin-menu.right-click-miss", L"no candidate hit");
      }
      return 0;
    }
    case WM_RBUTTONDOWN: {
      if (!self->m_interactive) return 0;
      if (!self->m_style.candidateRightClick) return 0;
      if (self->m_pinMenuVisible) {
        self->HidePinMenu();
      }
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      const int hit = self->HitTest(pt);
      self->m_rightPressedIndex = hit;
      self->UpdatePressedIndex(hit);
      if (hit >= 0) {
        if (self->m_events) self->m_events->OnCandidateInteractionStarted();
        SetCapture(hwnd);
      }
      return 0;
    }
    case WM_CAPTURECHANGED:
      self->UpdatePressedIndex(-1);
      self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
      self->m_pinMenuPressedCommand = kPinMenuCommandNone;
      self->m_rightPressedIndex = -1;
      return 0;
    case WM_MOUSEWHEEL:
      if (!self->m_interactive) return 0;
      if (self->m_events) self->m_events->OnCandidateWheel(GET_WHEEL_DELTA_WPARAM(wParam));
      return 0;
    case WM_PAINT: {
      PAINTSTRUCT ps = {};
      HDC hdc = BeginPaint(hwnd, &ps);
      if (hdc) {
        self->Paint(hdc, ps.rcPaint);
        EndPaint(hwnd, &ps);
      }
      return 0;
    }
    case WM_PRINTCLIENT: {
      RECT client = {};
      GetClientRect(hwnd, &client);
      if (wParam) self->Paint(reinterpret_cast<HDC>(wParam), client);
      return 0;
    }
    default:
      return DefWindowProcW(hwnd, msg, wParam, lParam);
  }
  return DefWindowProcW(hwnd, msg, wParam, lParam);
}
