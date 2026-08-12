#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <array>
#include <filesystem>
#include <string>
#include <vector>

#include "ime_model.h"

struct CandidatePageLayoutMetrics {
  std::vector<UINT> pageStarts;
  std::vector<int> itemWidths;
};

enum SrfCandidateWindowMenuCommand : int {
  kSrfCandidateWindowMenuNone = 0,
  kSrfCandidateWindowMenuPin = 1,
  kSrfCandidateWindowMenuUnpin = 2,
  kSrfCandidateWindowMenuRemoveUserPhrase = 3,
  kSrfCandidateWindowMenuBlockPhrase = 4,
  kSrfCandidateWindowMenuSource = 5,
};

/// 文本测量结果缓存。HFONT 与 DPI 不变时，相同文本的测量结果可复用。
struct MeasuredTextCache {
  std::wstring text;
  HFONT font = nullptr;
  UINT dpi = 0;
  SIZE size = {};

  bool Matches(HFONT f, UINT d, const std::wstring& t) const {
    return f == font && d == dpi && t == text;
  }
  void Reset() {
    text.clear();
    font = nullptr;
    dpi = 0;
    size = {};
  }
};

/// 单个候选项的位图缓存。用于把 normal/selected/hover/pressed 的渲染结果暂存，
/// 状态切换时直接 BitBlt，减少重复绘制。
struct CandidateItemPaintCache {
  HDC memDc = nullptr;
  HBITMAP bitmap = nullptr;
  HBITMAP oldBitmap = nullptr;
  int w = 0;
  int h = 0;
  size_t itemIndex = SIZE_MAX;
  bool selected = false;
  bool hot = false;
  bool pressed = false;
  bool valid = false;

  void Release();
  bool Ensure(HDC refDc, int width, int height);
};

UINT DpiForScreenRect(const RECT* rect);

CandidatePageLayoutMetrics BuildCandidatePageLayoutMetrics(const SrfUIStyle& style,
                                                           const RECT* anchorRect,
                                                           const std::vector<std::wstring>& items,
                                                           UINT dpi = 0);

/// 完整比较两个 SrfUIStyle 的所有字段。供 SetStyle() 和调用方判断样式是否变化。
bool CandidateWindowStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b);

/// 比较候选窗布局/分页相关字段；纯绘制字段变化不应触发布局重测和窗口重定位。
bool CandidateWindowLayoutStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b);

void ShutdownCandidateWindowRendering();

struct ICandidateWindowEvents {
  virtual ~ICandidateWindowEvents() = default;
  virtual void OnCandidateClicked(UINT indexInPage) = 0;
  virtual void OnCandidateRightClicked(UINT indexInPage, POINT screenPoint) = 0;
  virtual void OnCandidatePinRequested(UINT indexInPage, bool pinned) = 0;
  virtual void OnCandidateMenuCommand(UINT indexInPage, int command) = 0;
  virtual void OnCandidateWheel(int wheelDelta) = 0;
  virtual void OnCandidateEnvironmentChanged() = 0;
};

class CCandidateWindow {
 public:
  CCandidateWindow() = default;
  ~CCandidateWindow();

  void SetEvents(ICandidateWindowEvents* events);
  void SetStyle(const SrfUIStyle& style);
  void SetGameOverlay(bool enabled, bool fullscreen, HWND targetHwnd);
  void PrepareResources();
  void Show(const std::wstring& title, const std::vector<std::wstring>& pageItems,
            const std::vector<std::wstring>& pageComments,
            const std::vector<std::wstring>& pageLabels,
            const std::vector<bool>& pagePinnedItems,
            const std::vector<bool>& pageClipboardItems, UINT pageIndex, UINT totalPages,
            UINT selectedInPage, const RECT& anchorRect,
            const std::vector<std::wstring>& modeTags = {}, bool interactive = true,
            bool pendingVisual = false);
  void Hide();
  void Destroy();
  bool IsVisible() const;
  bool HasPendingPaint() const;
  void FlushPendingPaint();
  void SetPresentationState(bool interactive, bool pendingVisual);

 private:
  HWND m_hwnd = nullptr;
  HWND m_shadowHwnd = nullptr;
  ICandidateWindowEvents* m_events = nullptr;
  std::wstring m_title;
  std::vector<std::wstring> m_items;
  std::vector<std::wstring> m_comments;
  std::vector<std::wstring> m_labels;
  std::vector<bool> m_pinnedItems;
  std::vector<bool> m_clipboardItems;
  std::vector<std::wstring> m_modeTags;
  std::vector<std::wstring> m_displayItems;
  std::vector<RECT> m_itemRects;
  std::vector<std::array<CandidateItemPaintCache, 4>> m_itemPaintCaches;
  UINT m_pageIndex = 0;
  UINT m_totalPages = 0;
  UINT m_selectedInPage = 0;
  int m_hotIndex = -1;
  int m_pressedIndex = -1;
  bool m_pinMenuVisible = false;
  UINT m_pinMenuIndex = 0;
  bool m_pinMenuItemPinned = false;
  int m_pinMenuHotCommand = 0;
  int m_pinMenuPressedCommand = 0;
  int m_rightPressedIndex = -1;
  mutable RECT m_pinMenuRect = {};
  mutable RECT m_pinMenuPinRect = {};
  mutable RECT m_pinMenuUnpinRect = {};
  mutable RECT m_pinMenuRemoveRect = {};
  mutable RECT m_pinMenuBlockRect = {};
  mutable RECT m_pinMenuSourceRect = {};
  RECT m_anchorRect = {};
  bool m_hasAnchorRect = false;
  UINT m_dpi = 96;
  SrfUIStyle m_style = {};
  std::filesystem::path m_customFontPath;
  std::wstring m_customFontFace;
  HFONT m_titleFont = nullptr;
  HFONT m_bodyFont = nullptr;
  HFONT m_bodyStrongFont = nullptr;
  HFONT m_metaFont = nullptr;
  HFONT m_labelFont = nullptr;
  HFONT m_chipFont = nullptr;
  HDC m_paintMemDc = nullptr;
  HBITMAP m_paintBitmap = nullptr;
  HBITMAP m_paintOldBitmap = nullptr;
  int m_paintBufferW = 0;
  int m_paintBufferH = 0;
  HDC m_staticMemDc = nullptr;
  HBITMAP m_staticBitmap = nullptr;
  HBITMAP m_staticOldBitmap = nullptr;
  int m_staticBufferW = 0;
  int m_staticBufferH = 0;
  bool m_trackingMouse = false;
  bool m_interactive = true;
  bool m_pendingVisual = false;
  ULONGLONG m_pendingVisualSince = 0;
  bool m_pendingIndicatorTimerPending = false;
  bool m_gameOverlay = false;
  bool m_fullscreenOverlayPlacement = false;
  HWND m_overlayTargetHwnd = nullptr;
  bool m_layoutDirty = true;
  bool m_fullPaintDirty = true;
  bool m_staticPaintDirty = true;
  bool m_fontsDirty = true;
  bool m_needsMeasure = true;
  bool m_resourcesPrepared = false;
  bool m_renderResourcesPrimed = false;
  bool m_pendingStyleUpdate = false;
  bool m_pendingLayoutStyleUpdate = false;
  bool m_lastLayoutHorizontal = false;
  SrfCandidateLayoutVariant m_lastLayoutVariant = SrfCandidateLayoutVariant::Classic;
  SIZE m_measuredClientSize = {};
  ULONGLONG m_lastShowTick = 0;
  RECT m_targetWindowRect = {};
  bool m_hasTargetWindowRect = false;
  bool m_paintTimerPending = false;
  bool m_animationTimerPending = false;
  bool m_horizontalShrinkTimerPending = false;
  bool m_environmentRefreshTimerPending = false;
  bool m_environmentRefreshForced = false;
  bool m_overlayEnvironmentValid = false;
  HMONITOR m_overlayObservedMonitor = nullptr;
  RECT m_overlayObservedTargetRect = {};
  UINT m_overlayObservedDpi = 0;
  SIZE m_pendingHorizontalShrinkSize = {};
  RECT m_pendingHorizontalShrinkAnchor = {};
  std::vector<RECT> m_pendingHorizontalShrinkItemRects;
  MeasuredTextCache m_titleSizeCache;
  MeasuredTextCache m_pageTextSizeCache;
  std::vector<MeasuredTextCache> m_labelSizeCache;
  UINT m_lastDisplayAbbreviateLength = 0;
  HRGN m_pendingDirtyRgn = nullptr;
  bool m_hasPendingDirtyRgn = false;
  unsigned long long m_paintCount = 0;
  unsigned long long m_fullPaintCount = 0;

  bool m_showAnimationActive = false;
  ULONGLONG m_showAnimationStart = 0;
  int m_showAnimationDurationMs = 0;
  bool m_selectionAnimationActive = false;
  int m_selectionAnimationFrom = -1;
  int m_selectionAnimationTo = -1;
  ULONGLONG m_selectionAnimationStart = 0;
  int m_selectionAnimationDurationMs = 0;
  bool m_hoverAnimationActive = false;
  int m_hoverAnimationFrom = -1;
  int m_hoverAnimationTo = -1;
  ULONGLONG m_hoverAnimationStart = 0;
  int m_hoverAnimationDurationMs = 0;
  bool m_pressAnimationActive = false;
  int m_pressAnimationFrom = -1;
  int m_pressAnimationTo = -1;
  ULONGLONG m_pressAnimationStart = 0;
  int m_pressAnimationDurationMs = 0;
  bool m_pageAnimationActive = false;
  int m_pageAnimationDirection = 0;
  ULONGLONG m_pageAnimationStart = 0;
  int m_pageAnimationDurationMs = 0;

  // 上次生成字体时的 key，用于避免重复创建相同字体。
  UINT m_lastFontDpi = 0;
  std::wstring m_lastFontFace;
  UINT m_lastFontSize = 0;
  int m_lastFontWeight = 0;
  int m_lastSelectedWeight = 0;
  int m_lastLabelWeight = 0;
  int m_lastChipWeight = 0;
  std::wstring m_lastFontFile;

  bool EnsureWindow();
  bool EnsureShadowWindow();
  void UpdateShadowWindow();
  void ShowShadowWindow();
  void HideShadowWindow();
  void ApplyMouseTransparency();
  void ApplyWindowOpacity();
  void RefreshFonts();
  bool PrimeRenderResources();
  void DestroyFonts();
  void RebuildDisplayItems();
  int Scale(int value) const;
  SIZE MeasureClientSize(int maxWidth, std::vector<RECT>* outRects);
  RECT CalculateWindowRect(const RECT& anchorRect, SIZE content);
  void ApplyWindowRegion(int width, int height, bool redraw);
  void ApplyWindowRect(const RECT& rect);
  void ApplyAnimatedWindowRect(float showProgress);
  void Paint(HDC hdc, const RECT& paintRect);
  void PaintFull(HDC memDc, const RECT& client, const RECT* dirtyRect = nullptr);
  void ReleasePaintBuffer();
  void ReleaseStaticPaintBuffer();
  void ReleaseItemPaintCaches();
  void ReleaseItemPaintCacheAt(size_t index);
  void FlushPendingInvalidates();
  void ScheduleDeferredPaint(ULONGLONG now);
  void FlushDeferredPaint();
  void UpdatePendingIndicatorTimer(bool pendingVisual);
  void ScheduleEnvironmentRefresh();
  void ScheduleOverlayEnvironmentPoll();
  void CancelEnvironmentRefresh();
  void FlushEnvironmentRefresh();
  bool OverlayEnvironmentChanged();
  void CaptureOverlayEnvironment();
  void CancelPendingHorizontalShrink();
  void SchedulePendingHorizontalShrink(const RECT& anchorRect, SIZE clientSize,
                                       const std::vector<RECT>& itemRects);
  void FlushPendingHorizontalShrink();
  bool EnsurePaintBuffer(HDC hdc, int w, int h);
  bool EnsureStaticPaintBuffer(HDC hdc, int w, int h);
  SIZE MeasureSingleLineCached(HDC hdc, HFONT font, const std::wstring& text);
  SIZE MeasureLabelCached(HDC hdc, const std::wstring& text, size_t index);
  void ClearMeasuredTextCaches();
  int HitTest(POINT pt) const;
  int HitTestPinMenuRaw(POINT pt) const;
  int HitTestPinMenu(POINT pt) const;
  bool PinMenuCommandEnabled(int command) const;
  void ShowPinMenu(UINT indexInPage);
  void HidePinMenu();
  void LayoutPinMenuRect(int clientWidth, int contentBottom) const;
  void InvalidateCandidateIndex(int index);
  void UpdateHotIndex(int hotIndex);
  void UpdatePressedIndex(int pressedIndex);
  void UpdatePinMenuHotCommand(int command);
  void BeginTrackMouseLeave();
  bool MotionEnabled() const;
  int ResolveAnimationDuration(int skinDuration, int fallbackMs) const;
  void StartShowAnimation();
  void StartSelectionAnimation(int previousIndex, int nextIndex);
  void StartHoverAnimation(int previousIndex, int nextIndex);
  void StartPressAnimation(int previousIndex, int nextIndex);
  void StartPageAnimation(int previousPage, int nextPage);
  void CancelAnimations(bool restoreWindowState);
  void ScheduleAnimationFrame();
  void AdvanceAnimations();

  static ATOM EnsureWindowClass();
  static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
};
