#include "candidate_window_internal.h"
#include "candidate_layout_policy.h"
#include <windowsx.h>
using namespace srf_candidate_window_detail;

bool CCandidateWindow::MotionEnabled() const {
  return ShouldAnimateCandidateWindow(
      m_style.candidateReduceMotion, m_systemAnimations,
      m_style.themeMode == SrfThemeMode::HighContrast || m_systemHighContrast, m_gameOverlay,
      !m_style.skinLoaded || m_style.skinAnimationsEnabled);
}

int CCandidateWindow::ResolveAnimationDuration(int skinDuration, int fallbackMs) const {
  return std::clamp(skinDuration >= 0 ? skinDuration : fallbackMs, 0, 240);
}

void CCandidateWindow::StartShowAnimation() {
  const int duration =
      ResolveAnimationDuration(m_style.skinShowAnimationMs, kCandidateShowAnimationMs);
  if (!MotionEnabled() || duration <= 0 || !m_hasTargetWindowRect) {
    m_showAnimationActive = false;
    ApplyAnimatedWindowRect(1.0f);
    ApplyWindowOpacity();
    UpdateShadowWindow();
    return;
  }
  m_showAnimationActive = true;
  m_showAnimationStart = GetTickCount64();
  m_showAnimationDurationMs = duration;
  ApplyAnimatedWindowRect(0.0f);
  ApplyWindowOpacity();
  UpdateShadowWindow();
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartSelectionAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinSelectionAnimationMs, kCandidateSelectionAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex < 0 || nextIndex < 0 ||
      previousIndex == nextIndex || static_cast<size_t>(previousIndex) >= m_itemRects.size() ||
      static_cast<size_t>(nextIndex) >= m_itemRects.size()) {
    m_selectionAnimationActive = false;
    return;
  }
  m_selectionAnimationActive = true;
  m_selectionAnimationFrom = previousIndex;
  m_selectionAnimationTo = nextIndex;
  m_selectionAnimationStart = GetTickCount64();
  m_selectionAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartHoverAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinHoverAnimationMs, kCandidateHoverAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex == nextIndex) {
    m_hoverAnimationActive = false;
    return;
  }
  m_hoverAnimationActive = true;
  m_hoverAnimationFrom = previousIndex;
  m_hoverAnimationTo = nextIndex;
  m_hoverAnimationStart = GetTickCount64();
  m_hoverAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartPressAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinPressAnimationMs, kCandidatePressAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex == nextIndex) {
    m_pressAnimationActive = false;
    return;
  }
  m_pressAnimationActive = true;
  m_pressAnimationFrom = previousIndex;
  m_pressAnimationTo = nextIndex;
  m_pressAnimationStart = GetTickCount64();
  m_pressAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartPageAnimation(int previousPage, int nextPage) {
  const int duration =
      ResolveAnimationDuration(m_style.skinPageAnimationMs, kCandidatePageAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousPage == nextPage) {
    m_pageAnimationActive = false;
    return;
  }
  m_selectionAnimationActive = false;
  m_pageAnimationActive = true;
  m_pageAnimationDirection = nextPage > previousPage ? 1 : -1;
  m_pageAnimationStart = GetTickCount64();
  m_pageAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::CancelAnimations(bool restoreWindowState) {
  if (m_hwnd && m_animationTimerPending) {
    KillTimer(m_hwnd, kCandidateAnimationTimerId);
  }
  m_animationTimerPending = false;
  m_showAnimationActive = false;
  m_selectionAnimationActive = false;
  m_hoverAnimationActive = false;
  m_pressAnimationActive = false;
  m_pageAnimationActive = false;
  if (restoreWindowState && m_hwnd) {
    ApplyAnimatedWindowRect(1.0f);
    ApplyWindowOpacity();
    UpdateShadowWindow();
  }
}

void CCandidateWindow::ScheduleAnimationFrame() {
  if (!m_hwnd || m_animationTimerPending) return;
  if (SetTimer(m_hwnd, kCandidateAnimationTimerId, kCandidateAnimationFrameMs, nullptr)) {
    m_animationTimerPending = true;
  }
}

void CCandidateWindow::AdvanceAnimations() {
  if (!m_hwnd) return;
  if (m_animationTimerPending) {
    KillTimer(m_hwnd, kCandidateAnimationTimerId);
    m_animationTimerPending = false;
  }
  const ULONGLONG now = GetTickCount64();
  bool visualAnimation = false;
  if (m_showAnimationActive) {
    const float progress =
        LinearAnimationProgress(now, m_showAnimationStart, m_showAnimationDurationMs);
    ApplyAnimatedWindowRect(EaseOutCubic(progress));
    ApplyWindowOpacity();
    UpdateShadowWindow();
    if (progress >= 1.0f) {
      m_showAnimationActive = false;
      ApplyAnimatedWindowRect(1.0f);
      ApplyWindowOpacity();
      UpdateShadowWindow();
    }
  }
  auto advanceIndexAnimation = [now, &visualAnimation](bool& active, ULONGLONG start,
                                                       int duration) {
    if (!active) return;
    visualAnimation = true;
    if (LinearAnimationProgress(now, start, duration) >= 1.0f) active = false;
  };
  advanceIndexAnimation(m_selectionAnimationActive, m_selectionAnimationStart,
                        m_selectionAnimationDurationMs);
  advanceIndexAnimation(m_hoverAnimationActive, m_hoverAnimationStart, m_hoverAnimationDurationMs);
  advanceIndexAnimation(m_pressAnimationActive, m_pressAnimationStart, m_pressAnimationDurationMs);
  advanceIndexAnimation(m_pageAnimationActive, m_pageAnimationStart, m_pageAnimationDurationMs);
  if (visualAnimation) {
    InvalidateRect(m_hwnd, nullptr, FALSE);
    UpdateWindow(m_hwnd);
  }
  if (m_showAnimationActive || m_selectionAnimationActive || m_hoverAnimationActive ||
      m_pressAnimationActive || m_pageAnimationActive) {
    ScheduleAnimationFrame();
  }
}
