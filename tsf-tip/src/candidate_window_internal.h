#pragma once
// Internal, stateless policy helpers shared by candidate window implementation units.
#include "candidate_window.h"
#include <algorithm>
#include <cmath>
extern HMODULE SrfTip_GetDllModule();
extern void SrfTsfDiagnosticLog(const wchar_t*, const wchar_t*);
extern void SrfTsfPerfLog(const wchar_t*, const wchar_t*);
namespace srf_candidate_window_detail {
// Reset rendering caches without exposing GDI resource ownership to message dispatch.
void ResetGdiCache();
/// 返回 DLL 模块句柄；若不可用则回退到宿主 EXE。
inline HINSTANCE DllOrFallbackInstance() {
  HMODULE m = SrfTip_GetDllModule();
  return m ? m : GetModuleHandleW(nullptr);
}

inline constexpr wchar_t kCandidateWndClass[] = L"SRF_TSF_Candidate_Window";
inline constexpr UINT_PTR kCandidatePaintTimerId = 73;
inline constexpr UINT_PTR kCandidateHorizontalShrinkTimerId = 74;
inline constexpr UINT_PTR kCandidateEnvironmentRefreshTimerId = 76;
inline constexpr UINT_PTR kCandidateAnimationTimerId = 77;
inline constexpr UINT_PTR kCandidatePendingIndicatorTimerId = 78;
inline constexpr ULONGLONG kCandidateMinImmediatePaintIntervalMs = 8;
inline constexpr UINT kCandidateHorizontalShrinkDelayMs = 96;
inline constexpr UINT kCandidateEnvironmentRefreshDelayMs = 80;
inline constexpr UINT kCandidateEnvironmentPollMs = 500;
inline constexpr ULONGLONG kCandidateStyleDeferMs = 30;
inline constexpr UINT kCandidateAnimationFrameMs = 15;
inline constexpr UINT kCandidatePendingIndicatorDelayMs = 80;
inline constexpr int kCandidateShowAnimationMs = 0;
inline constexpr int kCandidateSelectionAnimationMs = 50;
inline constexpr int kCandidateHoverAnimationMs = 45;
inline constexpr int kCandidatePressAnimationMs = 24;
inline constexpr int kCandidatePageAnimationMs = 0;
inline constexpr int kHorizontalPageBadgeMinWidth = 44;
inline constexpr int kHorizontalPageBadgeGap = 6;
inline constexpr int kHorizontalPageBadgePaddingY = 4;
inline constexpr wchar_t kDefaultCandidateFontFace[] = L"Microsoft YaHei";
inline constexpr int kPinMenuCommandNone = kSrfCandidateWindowMenuNone;
inline constexpr int kPinMenuCommandPin = kSrfCandidateWindowMenuPin;
inline constexpr int kPinMenuCommandUnpin = kSrfCandidateWindowMenuUnpin;
inline constexpr int kPinMenuCommandRemove = kSrfCandidateWindowMenuRemoveUserPhrase;
inline constexpr int kPinMenuCommandBlock = kSrfCandidateWindowMenuBlockPhrase;
inline constexpr int kPinMenuCommandSource = kSrfCandidateWindowMenuSource;

enum class CandidateWindowUpdateKind {
  Selection,
  Page,
  Content,
  Layout,
  Style,
};

inline std::wstring CandidatePageIndicatorText(UINT pageIndex, UINT totalPages) {
  const UINT safeTotalPages = std::max(1u, totalPages);
  const UINT safePageIndex = std::min(pageIndex, safeTotalPages - 1);
  return std::to_wstring(safePageIndex + 1) + L"/" + std::to_wstring(safeTotalPages);
}

inline float LinearAnimationProgress(ULONGLONG now, ULONGLONG start, int durationMs) {
  if (durationMs <= 0 || now <= start) return durationMs <= 0 ? 1.0f : 0.0f;
  return std::clamp(static_cast<float>(now - start) / static_cast<float>(durationMs), 0.0f, 1.0f);
}

inline float EaseOutCubic(float progress) {
  const float inverse = 1.0f - std::clamp(progress, 0.0f, 1.0f);
  return 1.0f - inverse * inverse * inverse;
}

inline float IndexTransitionWeight(size_t index, int from, int to, float progress) {
  if (from == to) return static_cast<int>(index) == to ? 1.0f : 0.0f;
  if (static_cast<int>(index) == from) return 1.0f - progress;
  if (static_cast<int>(index) == to) return progress;
  return 0.0f;
}

inline RECT InterpolateRect(const RECT& from, const RECT& to, float progress) {
  auto interpolate = [progress](LONG a, LONG b) {
    return static_cast<LONG>(
        std::lround(static_cast<double>(a) + static_cast<double>(b - a) * progress));
  };
  return {interpolate(from.left, to.left), interpolate(from.top, to.top),
          interpolate(from.right, to.right), interpolate(from.bottom, to.bottom)};
}

inline bool ShouldDeferImmediatePaint(CandidateWindowUpdateKind kind, bool windowVisible,
                                      ULONGLONG lastShowTick, ULONGLONG showTick) {
  if (!windowVisible || lastShowTick == 0) return false;
  const ULONGLONG elapsed = showTick - lastShowTick;
  if ((kind == CandidateWindowUpdateKind::Content || kind == CandidateWindowUpdateKind::Layout) &&
      elapsed < kCandidateMinImmediatePaintIntervalMs) {
    return true;
  }
  if (kind == CandidateWindowUpdateKind::Style && elapsed < kCandidateStyleDeferMs) return true;
  return false;
}

inline size_t CandidateItemPaintStateSlot(bool selected, bool hot, bool pressed) {
  if (pressed) return 3;
  if (hot) return 2;
  if (selected) return 1;
  return 0;
}

inline bool RectVectorEquals(const std::vector<RECT>& a, const std::vector<RECT>& b) {
  if (a.size() != b.size()) return false;
  for (size_t i = 0; i < a.size(); ++i) {
    if (!EqualRect(&a[i], &b[i])) return false;
  }
  return true;
}

}  // namespace srf_candidate_window_detail
