#pragma once

#include <string_view>

inline bool ShouldDelayHorizontalCandidateShrink(bool horizontal, bool wasVisible,
                                                 bool hadAnchor, long previousWidth,
                                                 long naturalWidth,
                                                 bool contentUpdate) {
  return horizontal && wasVisible && hadAnchor && previousWidth > 0 &&
         previousWidth > naturalWidth && contentUpdate;
}

inline bool ShouldShowCandidateComment(bool horizontal, bool clipboardItem,
                                       bool selectedItem) {
  // Horizontal candidates are a single-line strip. Corrections already carry
  // the visible "~" prefix, so their metadata must not create a second row.
  return !horizontal && (clipboardItem || selectedItem);
}

inline std::wstring_view DefaultCandidateSelectedIndicator(bool horizontal) {
  return horizontal ? L"bottom_bar" : L"left_bar";
}

inline bool ShouldAnimateCandidateWindow(bool reduceMotion, bool systemAnimationsEnabled,
                                         bool highContrast, bool gameOverlay,
                                         bool skinAnimationsEnabled) {
  return !reduceMotion && systemAnimationsEnabled && !highContrast && !gameOverlay &&
         skinAnimationsEnabled;
}
