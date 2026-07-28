#include "candidate_overlay_placement.h"

#include <algorithm>
#include <cwchar>
#include <cwctype>
#include <string>
#include <utility>
#include <vector>

#include <dwmapi.h>

#include "candidate_window.h"

namespace {

struct OverlayMonitorInfo {
  HMONITOR monitor = nullptr;
  RECT monitorArea = {};
  RECT workArea = {};
  std::wstring device;
  bool primary = false;
};

BOOL CALLBACK CollectOverlayMonitor(HMONITOR monitor, HDC, LPRECT, LPARAM context) {
  auto* monitors = reinterpret_cast<std::vector<OverlayMonitorInfo>*>(context);
  if (!monitors) return FALSE;
  MONITORINFOEXW info = {};
  info.cbSize = sizeof(info);
  if (!GetMonitorInfoW(monitor, &info)) return TRUE;
  OverlayMonitorInfo item = {};
  item.monitor = monitor;
  item.monitorArea = info.rcMonitor;
  item.workArea = info.rcWork;
  item.device = info.szDevice;
  item.primary = (info.dwFlags & MONITORINFOF_PRIMARY) != 0;
  monitors->push_back(std::move(item));
  return TRUE;
}

int DisplayDeviceNumber(const std::wstring& device) {
  std::size_t firstDigit = device.size();
  while (firstDigit > 0 && std::iswdigit(device[firstDigit - 1])) --firstDigit;
  if (firstDigit == device.size()) return 1000;
  wchar_t* end = nullptr;
  const long parsed = wcstol(device.c_str() + firstDigit, &end, 10);
  return end && *end == L'\0' && parsed >= 0 && parsed <= 999 ? static_cast<int>(parsed)
                                                               : 1000;
}

std::vector<OverlayMonitorInfo> EnumerateOverlayMonitors() {
  std::vector<OverlayMonitorInfo> monitors;
  EnumDisplayMonitors(nullptr, nullptr, CollectOverlayMonitor,
                      reinterpret_cast<LPARAM>(&monitors));
  std::sort(monitors.begin(), monitors.end(), [](const auto& left, const auto& right) {
    const int leftNumber = DisplayDeviceNumber(left.device);
    const int rightNumber = DisplayDeviceNumber(right.device);
    if (leftNumber != rightNumber) return leftNumber < rightNumber;
    if (left.monitorArea.left != right.monitorArea.left) {
      return left.monitorArea.left < right.monitorArea.left;
    }
    return left.monitorArea.top < right.monitorArea.top;
  });
  return monitors;
}

const OverlayMonitorInfo* FindOverlayMonitor(
    const std::vector<OverlayMonitorInfo>& monitors, HWND targetHwnd,
    const SrfAppOptions* options) {
  if (monitors.empty()) return nullptr;
  const std::wstring selection =
      options && options->hasOverlayMonitor ? options->overlayMonitor : L"auto";
  if (selection == L"primary") {
    const auto it = std::find_if(monitors.begin(), monitors.end(),
                                 [](const auto& item) { return item.primary; });
    return it != monitors.end() ? &*it : &monitors.front();
  }
  if (selection != L"auto") {
    wchar_t* end = nullptr;
    const unsigned long index = wcstoul(selection.c_str(), &end, 10);
    if (end && *end == L'\0' && index < monitors.size()) return &monitors[index];
  }

  HMONITOR targetMonitor = nullptr;
  if (targetHwnd && IsWindow(targetHwnd)) {
    targetMonitor = MonitorFromWindow(targetHwnd, MONITOR_DEFAULTTONEAREST);
  }
  if (!targetMonitor) {
    const HWND foreground = GetForegroundWindow();
    if (foreground) {
      targetMonitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
    }
  }
  const auto it = std::find_if(monitors.begin(), monitors.end(),
                               [targetMonitor](const auto& item) {
                                 return item.monitor == targetMonitor;
                               });
  if (it != monitors.end()) return &*it;
  const auto primary = std::find_if(monitors.begin(), monitors.end(),
                                    [](const auto& item) { return item.primary; });
  return primary != monitors.end() ? &*primary : &monitors.front();
}

RECT TargetWindowRect(HWND targetHwnd) {
  RECT rect = {};
  if (!targetHwnd || !IsWindow(targetHwnd)) return rect;
  if (FAILED(DwmGetWindowAttribute(targetHwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &rect,
                                   sizeof(rect))) ||
      rect.right <= rect.left || rect.bottom <= rect.top) {
    rect = {};
    (void)GetWindowRect(targetHwnd, &rect);
  }
  return rect;
}

void OffsetAnchorLogical(const SrfAppOptions* options, RECT* rect) {
  if (!options || !rect) return;
  const int logicalX = options->hasOverlayOffsetX ? options->overlayOffsetX : 0;
  const int logicalY = options->hasOverlayOffsetY ? options->overlayOffsetY : 0;
  if (logicalX == 0 && logicalY == 0) return;
  UINT dpi = DpiForScreenRect(rect);
  if (dpi == 0) dpi = 96;
  const int dx = MulDiv(logicalX, static_cast<int>(dpi), 96);
  const int dy = MulDiv(logicalY, static_cast<int>(dpi), 96);
  OffsetRect(rect, dx, dy);
}

}  // namespace

SrfOverlayAnchor EffectiveOverlayAnchor(const SrfAppOptions* options) {
  return options && options->hasOverlayAnchor ? options->overlayAnchor
                                               : SrfOverlayAnchor::Auto;
}

bool ResolveCandidateGameOverlayAnchor(HWND targetHwnd, bool fullscreenPlacement,
                                       const SrfAppOptions* options, RECT* output) {
  if (!output) return false;
  const SrfOverlayAnchor anchor = EffectiveOverlayAnchor(options);
  if (anchor == SrfOverlayAnchor::Caret) return false;

  const std::vector<OverlayMonitorInfo> monitors = EnumerateOverlayMonitors();
  const OverlayMonitorInfo* monitor = FindOverlayMonitor(monitors, targetHwnd, options);
  if (!monitor) return false;
  const RECT area = fullscreenPlacement ? monitor->monitorArea : monitor->workArea;
  UINT dpi = DpiForScreenRect(&area);
  if (dpi == 0) dpi = 96;
  const int margin = MulDiv(fullscreenPlacement ? 48 : 24, static_cast<int>(dpi), 96);
  const int height = std::max(1, MulDiv(20, static_cast<int>(dpi), 96));

  LONG x = area.left + margin;
  LONG y = area.bottom - margin;
  SrfOverlayAnchor resolvedAnchor = anchor;
  if (anchor == SrfOverlayAnchor::Auto) {
    const bool explicitMonitor =
        options && options->hasOverlayMonitor && options->overlayMonitor != L"auto";
    if (!fullscreenPlacement && !explicitMonitor) {
      const RECT target = TargetWindowRect(targetHwnd);
      if (target.right > target.left && target.bottom > target.top) {
        x = std::clamp<LONG>(target.left + margin, area.left + margin,
                             std::max<LONG>(area.left + margin, area.right - margin));
        y = std::clamp<LONG>(target.bottom - margin, area.top + margin,
                             std::max<LONG>(area.top + margin, area.bottom - margin));
      }
    }
    resolvedAnchor = SrfOverlayAnchor::BottomLeft;
  }

  switch (resolvedAnchor) {
    case SrfOverlayAnchor::TopLeft:
      x = area.left + margin;
      y = area.top + margin;
      break;
    case SrfOverlayAnchor::TopCenter:
      x = area.left + (area.right - area.left) / 2;
      y = area.top + margin;
      break;
    case SrfOverlayAnchor::TopRight:
      x = area.right - margin;
      y = area.top + margin;
      break;
    case SrfOverlayAnchor::BottomCenter:
      x = area.left + (area.right - area.left) / 2;
      y = area.bottom - margin;
      break;
    case SrfOverlayAnchor::BottomRight:
      x = area.right - margin;
      y = area.bottom - margin;
      break;
    case SrfOverlayAnchor::BottomLeft:
    case SrfOverlayAnchor::Auto:
    default:
      break;
  }

  output->left = x;
  output->right = x + 1;
  if (resolvedAnchor == SrfOverlayAnchor::TopLeft ||
      resolvedAnchor == SrfOverlayAnchor::TopCenter ||
      resolvedAnchor == SrfOverlayAnchor::TopRight) {
    output->top = y;
    output->bottom = y + height;
  } else {
    output->bottom = y;
    output->top = y - height;
  }
  OffsetAnchorLogical(options, output);
  return output->right > output->left && output->bottom > output->top;
}

void ApplyCandidateGameOverlayOffset(const SrfAppOptions* options, RECT* rect) {
  OffsetAnchorLogical(options, rect);
}

bool ConvertCandidateOverlayAnchorToPhysical(HWND targetHwnd, const RECT& input,
                                             RECT* output) {
  if (!output || !targetHwnd || !IsWindow(targetHwnd) || input.right <= input.left ||
      input.bottom <= input.top) {
    return false;
  }
  POINT topLeft = {input.left, input.top};
  POINT bottomRight = {input.right, input.bottom};
  if (!LogicalToPhysicalPointForPerMonitorDPI(targetHwnd, &topLeft) ||
      !LogicalToPhysicalPointForPerMonitorDPI(targetHwnd, &bottomRight)) {
    return false;
  }
  output->left = topLeft.x;
  output->top = topLeft.y;
  output->right = bottomRight.x;
  output->bottom = bottomRight.y;
  return output->right > output->left && output->bottom > output->top;
}

bool IsCandidateOverlayTargetFullscreen(HWND targetHwnd) {
  if (!targetHwnd || !IsWindowVisible(targetHwnd)) return false;
  HWND root = GetAncestor(targetHwnd, GA_ROOT);
  if (root && IsWindowVisible(root)) targetHwnd = root;

  RECT windowRect = TargetWindowRect(targetHwnd);
  if (windowRect.right <= windowRect.left || windowRect.bottom <= windowRect.top) {
    return false;
  }
  const HMONITOR monitor =
      MonitorFromWindow(targetHwnd, MONITOR_DEFAULTTONEAREST);
  MONITORINFO info = {};
  info.cbSize = sizeof(info);
  if (!monitor || !GetMonitorInfoW(monitor, &info)) return false;

  return CandidateOverlayRectCoversMonitor(windowRect, info.rcMonitor);
}
