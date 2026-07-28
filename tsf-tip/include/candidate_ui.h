#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <msctf.h>
#include <cstdint>
#include <string>
#include <vector>

#include "candidate_window.h"

class CSrfTip;

class CSrfCandidateListUIElement : public ITfCandidateListUIElementBehavior,
                                   public ICandidateWindowEvents {
 public:
  explicit CSrfCandidateListUIElement(CSrfTip* tip);
  ~CSrfCandidateListUIElement() override;

  STDMETHODIMP QueryInterface(REFIID riid, void** ppvObject) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP GetDescription(BSTR* pbstrDescription) override;
  STDMETHODIMP GetGUID(GUID* pguid) override;
  STDMETHODIMP Show(BOOL bShow) override;
  STDMETHODIMP IsShown(BOOL* pbShow) override;
  STDMETHODIMP GetUpdatedFlags(DWORD* pdwFlags) override;
  STDMETHODIMP GetDocumentMgr(ITfDocumentMgr** ppdim) override;
  STDMETHODIMP GetCount(UINT* puCount) override;
  STDMETHODIMP GetSelection(UINT* puIndex) override;
  STDMETHODIMP GetString(UINT uIndex, BSTR* pstr) override;
  STDMETHODIMP GetPageIndex(UINT* pIndex, UINT uSize, UINT* puPageCnt) override;
  STDMETHODIMP SetPageIndex(UINT* pIndex, UINT uPageCnt) override;
  STDMETHODIMP GetCurrentPage(UINT* puPage) override;
  STDMETHODIMP SetSelection(UINT nIndex) override;
  STDMETHODIMP Finalize() override;
  STDMETHODIMP Abort() override;

  void OnCandidateClicked(UINT indexInPage) override;
  void OnCandidateRightClicked(UINT indexInPage, POINT screenPoint) override;
  void OnCandidatePinRequested(UINT indexInPage, bool pinned) override;
  void OnCandidateMenuCommand(UINT indexInPage, int command) override;
  void OnCandidateWheel(int wheelDelta) override;
  void OnCandidateEnvironmentChanged() override;
  void OnCandidateAnchorRefreshed();
  void OnExternalOverlayStatusChanged();
  void OnExternalOverlayHealthTimer();

  HRESULT BeginOrUpdate();
  void UpdatePresentationState();
  void PrepareWindowResources();
  void End();

 private:
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  DWORD m_uiElementId = TF_INVALID_UIELEMENTID;
  BOOL m_showWindow = TRUE;
  CCandidateWindow m_window;
  DWORD m_updatedFlags = TF_CLUIE_DOCUMENTMGR | TF_CLUIE_COUNT | TF_CLUIE_SELECTION |
                         TF_CLUIE_STRING | TF_CLUIE_PAGEINDEX | TF_CLUIE_CURRENTPAGE;

  bool m_layoutCacheValid = false;
  CandidatePageLayoutMetrics m_cachedLayout = {};
  std::vector<std::wstring> m_cachedLayoutItems;
  unsigned long long m_cachedLayoutItemsVersion = 0;
  RECT m_cachedAnchorRect = {};
  UINT m_cachedAnchorDpi = 0;
  HMONITOR m_cachedAnchorMonitor = nullptr;
  RECT m_cachedAnchorWorkArea = {};
  UINT m_cachedStyleFontSize = 0;
  UINT m_cachedStyleAbbreviateLength = 0;
  bool m_cachedStyleHorizontal = false;
  UINT m_cachedStylePageSize = 0;
  UINT m_cachedStyleHorizontalCount = 0;
  bool m_cachedStyleHorizontalCompact = false;
  std::wstring m_cachedStyleFontFile;
  std::wstring m_cachedStyleSkinFile;
  bool m_cachedStyleSkinLoaded = false;
  SrfCandidateMaterial m_cachedStyleMaterial = SrfCandidateMaterial::Auto;
  SrfCandidateDensity m_cachedStyleDensity = SrfCandidateDensity::Comfortable;
  SrfCandidateLayoutVariant m_cachedStyleLayoutVariant = SrfCandidateLayoutVariant::Classic;
  UINT m_cachedStyleScalePercent = 100;
  SrfOverlayAnchor m_cachedStyleOverlayAnchor = SrfOverlayAnchor::Auto;
  bool m_cachedStyleFullscreenPlacement = false;
  int m_cachedSkinOuterPadX = -1;
  int m_cachedSkinOuterPadY = -1;
  int m_cachedSkinHeaderPadX = -1;
  int m_cachedSkinHeaderPadY = -1;
  int m_cachedSkinHeaderGap = -1;
  int m_cachedSkinItemGap = -1;
  int m_cachedSkinItemPadX = -1;
  int m_cachedSkinItemPadY = -1;
  int m_cachedSkinLabelWidth = -1;
  int m_cachedSkinLabelGap = -1;
  int m_cachedSkinCommentGap = -1;
  int m_cachedSkinMinWidth = -1;
  int m_cachedSkinPreferredWidth = -1;
  int m_cachedSkinMaxWidth = -1;
  int m_cachedSkinMinHorizontalCardWidth = -1;
  int m_cachedSkinMaxHorizontalCardWidth = -1;
  SrfUIStyle m_cachedStyle = {};
  bool m_renderCacheValid = false;
  std::wstring m_cachedRenderTitle;
  std::vector<std::wstring> m_cachedRenderItems;
  std::vector<std::wstring> m_cachedRenderComments;
  std::vector<std::wstring> m_cachedRenderLabels;
  std::vector<bool> m_cachedRenderPinnedItems;
  std::vector<bool> m_cachedRenderClipboardItems;
  std::vector<std::wstring> m_cachedRenderModeTags;
  bool m_cachedRenderInteractive = true;
  bool m_cachedRenderPendingVisual = false;
  // Page text/metadata changes less often than anchor/presentation updates.
  // Keep a separate key so RefreshWindow can reuse the already formatted page
  // vectors without rebuilding labels/comments on every UIElement update.
  bool m_pageDataCacheValid = false;
  unsigned long long m_cachedPageDataVersion = 0;
  UINT m_cachedPageDataStart = 0;
  UINT m_cachedPageDataEnd = 0;
  UINT m_cachedPageDataSelected = 0;
  bool m_cachedPageDataHorizontal = false;
  bool m_cachedPageDataClipboardPanel = false;
  bool m_cachedPageDataHasClipboardPage = false;
  UINT m_cachedPageDataClipboardPage = 1;
  UINT m_cachedPageDataClipboardPages = 1;
  std::vector<std::wstring> m_cachedPageItems;
  std::vector<std::wstring> m_cachedPageComments;
  std::vector<std::wstring> m_cachedPageLabels;
  std::vector<bool> m_cachedPagePinnedItems;
  std::vector<bool> m_cachedPageClipboardItems;
  RECT m_cachedRenderAnchor = {};
  UINT m_cachedRenderPage = 0;
  UINT m_cachedRenderTotalPages = 0;
  UINT m_cachedRenderSelected = 0;
  bool m_externalOverlayVisible = false;
  uint64_t m_externalOverlayOwnerId = 0;
  HWND m_externalOverlaySenderHwnd = nullptr;
  HWND m_externalOverlayTargetHwnd = nullptr;
  DWORD m_externalOverlayTargetProcessId = 0;
  uint64_t m_externalOverlayFocusGeneration = 0;
  uint64_t m_externalOverlayPendingSequence = 0;
  uint64_t m_externalOverlayAppliedSequence = 0;
  ULONGLONG m_externalOverlayPendingSinceTick = 0;
  bool m_externalOverlayHealthTimerPending = false;
  bool m_externalOverlayAnchorRefreshBlocked = false;

  void RefreshWindow();
  void HideExternalOverlay();
  void ResetExternalOverlayState();
  void ScheduleExternalOverlayHealthCheck();
  void CancelExternalOverlayHealthCheck();
};
