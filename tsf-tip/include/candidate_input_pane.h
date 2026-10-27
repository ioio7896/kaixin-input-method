#pragma once
#include <windows.h>
#include <shobjidl.h>
#include <algorithm>
#include <climits>

// Preserve candidate size. Prefer the nearest free side of a docked or floating
// touch keyboard, including keyboards on monitors with negative coordinates.
inline RECT SrfAvoidInputPane(RECT rect, const RECT& pane, const RECT& work, int gap) {
  RECT overlap = {};
  if (IsRectEmpty(&pane) || !IntersectRect(&overlap, &rect, &pane)) return rect;
  const LONG w = rect.right - rect.left, h = rect.bottom - rect.top;
  RECT options[] = {{rect.left, pane.top - gap - h, rect.right, pane.top - gap},
                    {rect.left, pane.bottom + gap, rect.right, pane.bottom + gap + h},
                    {pane.left - gap - w, rect.top, pane.left - gap, rect.bottom},
                    {pane.right + gap, rect.top, pane.right + gap + w, rect.bottom}};
  long long bestDistance = LLONG_MAX;
  RECT best = rect;
  for (RECT candidate : options) {
    const LONG x = std::clamp(candidate.left, work.left, (std::max)(work.left, work.right - w));
    const LONG y = std::clamp(candidate.top, work.top, (std::max)(work.top, work.bottom - h));
    candidate = {x, y, x + w, y + h};
    if (IntersectRect(&overlap, &candidate, &pane)) continue;
    const long long dx = static_cast<long long>(x) - rect.left;
    const long long dy = static_cast<long long>(y) - rect.top;
    const long long distance = dx * dx + dy * dy;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = candidate;
    }
  }
  return best;
}

class SrfCandidateInputPane {
 public:
  static constexpr UINT kChangedMessage = WM_APP + 0x31a;
  ~SrfCandidateInputPane() { Detach(); }
  void Attach(HWND candidate, HWND owner);
  void Detach();
  RECT Location(HWND candidate) const;

 private:
  IFrameworkInputPane* m_pane = nullptr;
  DWORD m_cookie = 0;
  bool m_advised = false;
  HWND m_owner = nullptr;
};
