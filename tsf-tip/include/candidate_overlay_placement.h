#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include "ime_model.h"

inline bool CandidateOverlayRectCoversMonitor(const RECT& windowRect,
                                              const RECT& monitorRect,
                                              LONG tolerance = 2) {
  if (tolerance < 0 || windowRect.right <= windowRect.left ||
      windowRect.bottom <= windowRect.top ||
      monitorRect.right <= monitorRect.left ||
      monitorRect.bottom <= monitorRect.top) {
    return false;
  }
  return windowRect.left <= monitorRect.left + tolerance &&
         windowRect.top <= monitorRect.top + tolerance &&
         windowRect.right >= monitorRect.right - tolerance &&
         windowRect.bottom >= monitorRect.bottom - tolerance;
}

SrfOverlayAnchor EffectiveOverlayAnchor(const SrfAppOptions* options);

// Resolves auto and fixed screen/window anchors. Caret is intentionally
// returned as false because the caller owns the TSF text/caret rectangle.
bool ResolveCandidateGameOverlayAnchor(HWND targetHwnd, bool fullscreenPlacement,
                                       const SrfAppOptions* options, RECT* output);

// Applies the per-game logical-pixel offset to a TSF/caret-provided anchor.
void ApplyCandidateGameOverlayOffset(const SrfAppOptions* options, RECT* rect);

// Converts a source-process logical/virtualized screen rectangle into physical
// screen pixels before it crosses into the PMv2 overlay process.
bool ConvertCandidateOverlayAnchorToPhysical(HWND targetHwnd, const RECT& input,
                                             RECT* output);

// Re-evaluates the target window after Alt+Enter instead of trusting the
// fullscreen bit captured by an older TIP snapshot.
bool IsCandidateOverlayTargetFullscreen(HWND targetHwnd);
