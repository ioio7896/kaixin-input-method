#include "candidate_ui.h"

#include "candidate_layout_policy.h"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <new>

#include "candidate_overlay_client.h"
#include "candidate_overlay_placement.h"
#include "ime_model.h"
#include "srf_tip.h"

namespace {

constexpr GUID kCandidateUiGuid = {0x36c3c795,
                                   0x7159,
                                   0x45aa,
                                   {0xab, 0x12, 0x30, 0x22, 0x9a, 0x51, 0xdb, 0xd3}};

std::atomic<std::uint64_t> g_nextExternalOverlayOwnerId{1};

std::uint64_t AllocateExternalOverlayOwnerId() {
  std::uint64_t value =
      g_nextExternalOverlayOwnerId.fetch_add(1, std::memory_order_relaxed);
  if (value == 0) {
    value = g_nextExternalOverlayOwnerId.fetch_add(1, std::memory_order_relaxed);
  }
  return value;
}

struct CandidateAnchorDisplay {
  HMONITOR monitor = nullptr;
  RECT workArea = {};
};

CandidateAnchorDisplay DisplayForCandidateAnchor(const RECT& anchor, bool fullscreenOverlay) {
  CandidateAnchorDisplay display = {};
  display.monitor = MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST);
  MONITORINFO info = {};
  info.cbSize = sizeof(info);
  if (display.monitor && GetMonitorInfoW(display.monitor, &info)) {
    display.workArea = fullscreenOverlay ? info.rcMonitor : info.rcWork;
  }
  return display;
}

}  // namespace

extern void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg);
extern void SrfTsfPerfLog(const wchar_t* tag, const wchar_t* msg);

CSrfCandidateListUIElement::CSrfCandidateListUIElement(CSrfTip* tip)
    : m_tip(tip), m_externalOverlayOwnerId(AllocateExternalOverlayOwnerId()) {
  m_window.SetEvents(this);
}

CSrfCandidateListUIElement::~CSrfCandidateListUIElement() { End(); }

STDMETHODIMP CSrfCandidateListUIElement::QueryInterface(REFIID riid, void** ppvObject) {
  if (!ppvObject) return E_POINTER;
  *ppvObject = nullptr;

  if (riid == IID_IUnknown || riid == IID_ITfUIElement || riid == IID_ITfCandidateListUIElement ||
      riid == IID_ITfCandidateListUIElementBehavior) {
    *ppvObject = static_cast<ITfCandidateListUIElementBehavior*>(this);
  } else {
    return E_NOINTERFACE;
  }

  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) CSrfCandidateListUIElement::AddRef() { return InterlockedIncrement(&m_cRef); }

STDMETHODIMP_(ULONG) CSrfCandidateListUIElement::Release() {
  const ULONG count = InterlockedDecrement(&m_cRef);
  if (count == 0) delete this;
  return count;
}

STDMETHODIMP CSrfCandidateListUIElement::GetDescription(BSTR* pbstrDescription) {
  if (!pbstrDescription) return E_POINTER;
  *pbstrDescription = SysAllocString(L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Candidate List");
  return *pbstrDescription ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfCandidateListUIElement::GetGUID(GUID* pguid) {
  if (!pguid) return E_POINTER;
  *pguid = kCandidateUiGuid;
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::Show(BOOL bShow) {
  m_showWindow =
      (m_tip && (m_tip->m_uiLessMode || m_tip->ShouldUseExternalCandidateOverlay()))
          ? TRUE
          : bShow;
  RefreshWindow();
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::IsShown(BOOL* pbShow) {
  if (!pbShow) return E_POINTER;
  *pbShow = m_showWindow;
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::GetUpdatedFlags(DWORD* pdwFlags) {
  if (!pdwFlags) return E_POINTER;
  *pdwFlags = m_updatedFlags;
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::GetDocumentMgr(ITfDocumentMgr** ppdim) {
  if (!ppdim) return E_POINTER;
  *ppdim = nullptr;
  if (!m_tip || !m_tip->m_pThreadMgr) return E_FAIL;
  return m_tip->m_pThreadMgr->GetFocus(ppdim);
}

STDMETHODIMP CSrfCandidateListUIElement::GetCount(UINT* puCount) {
  if (!puCount) return E_POINTER;
  *puCount = static_cast<UINT>(m_tip ? m_tip->m_context.candidates.items.size() : 0);
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::GetSelection(UINT* puIndex) {
  if (!puIndex) return E_POINTER;
  *puIndex = m_tip ? m_tip->m_context.candidates.highlighted : 0;
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::GetString(UINT uIndex, BSTR* pstr) {
  if (!pstr) return E_POINTER;
  *pstr = nullptr;
  if (!m_tip || uIndex >= m_tip->m_context.candidates.items.size()) return E_INVALIDARG;

  const std::wstring& text = m_tip->m_context.candidates.items[uIndex].str;
  *pstr = SysAllocStringLen(text.c_str(), static_cast<UINT>(text.size()));
  return *pstr ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfCandidateListUIElement::GetPageIndex(UINT* pIndex, UINT uSize, UINT* puPageCnt) {
  if (!puPageCnt) return E_POINTER;
  if (!m_tip) {
    *puPageCnt = 0;
    return S_OK;
  }

  const CandidatePageLayoutMetrics layout = m_tip->BuildCandidatePageLayout();
  const UINT pageCount = std::max(1u, static_cast<UINT>(layout.pageStarts.size()));
  *puPageCnt = pageCount;
  if (!pIndex) return S_OK;
  if (uSize < pageCount) return E_INVALIDARG;

  for (UINT i = 0; i < pageCount; ++i) {
    pIndex[i] = layout.pageStarts.empty() ? 0u : layout.pageStarts[i];
  }
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::SetPageIndex(UINT* pIndex, UINT uPageCnt) {
  if (!m_tip || !pIndex || uPageCnt == 0) return E_INVALIDARG;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return S_OK;
  const UINT requestedPage = std::min(uPageCnt - 1, m_tip->CandidatePageForIndex(pIndex[0]));
  m_tip->m_candPage = std::min(requestedPage, m_tip->MaxCandidatePage());
  const UINT pageStart = m_tip->CandidatePageStart(m_tip->m_candPage);
  const UINT pageEndExclusive = m_tip->CandidatePageEndExclusive(m_tip->m_candPage);
  if (m_tip->m_candSel < pageStart || m_tip->m_candSel >= pageEndExclusive) {
    m_tip->m_candSel = pageStart;
  }
  m_tip->ClampCandidateState();
  m_tip->SyncCandidateContextState();
  m_tip->RedrawCandidateUi();
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::GetCurrentPage(UINT* puPage) {
  if (!puPage) return E_POINTER;
  *puPage = m_tip ? m_tip->m_candPage : 0;
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::SetSelection(UINT nIndex) {
  if (!m_tip) return E_FAIL;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return S_OK;
  if (nIndex >= m_tip->m_context.candidates.items.size()) return E_INVALIDARG;
  m_tip->m_candSel = nIndex;
  m_tip->ClampCandidateState();
  m_tip->SyncCandidateContextState();
  m_tip->RedrawCandidateUi();
  return S_OK;
}

STDMETHODIMP CSrfCandidateListUIElement::Finalize() {
  if (!m_tip) return E_FAIL;
  return m_tip->RequestCommitCandidate(m_tip->m_candSel);
}

STDMETHODIMP CSrfCandidateListUIElement::Abort() {
  if (!m_tip) return E_FAIL;
  m_tip->RequestCancelCompositionOnFocusLoss();
  return S_OK;
}

void CSrfCandidateListUIElement::OnCandidateClicked(UINT indexInPage) {
  if (!m_tip) return;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return;
  const size_t idx = static_cast<size_t>(m_tip->CandidatePageStart(m_tip->m_candPage)) + indexInPage;
  if (idx >= m_tip->CandidatePageEndExclusive(m_tip->m_candPage)) return;
  m_tip->m_candSel = static_cast<UINT>(idx);
  (void)m_tip->RequestCommitCandidate(idx);
}

void CSrfCandidateListUIElement::OnCandidateRightClicked(UINT indexInPage, POINT screenPoint) {
  (void)screenPoint;
  if (!m_tip) return;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return;
  const size_t idx = static_cast<size_t>(m_tip->CandidatePageStart(m_tip->m_candPage)) + indexInPage;
  if (idx >= m_tip->CandidatePageEndExclusive(m_tip->m_candPage)) return;
  m_tip->m_candSel = static_cast<UINT>(idx);
  m_tip->ClampCandidateState();
  m_tip->SyncCandidateContextState();
  m_tip->RedrawCandidateUi();
}

void CSrfCandidateListUIElement::OnCandidatePinRequested(UINT indexInPage, bool pinned) {
  if (!m_tip) return;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return;
  const size_t idx = static_cast<size_t>(m_tip->CandidatePageStart(m_tip->m_candPage)) + indexInPage;
  if (idx >= m_tip->CandidatePageEndExclusive(m_tip->m_candPage)) return;
  m_tip->m_candSel = static_cast<UINT>(idx);
  m_tip->ApplyCandidatePinChoice(idx, pinned);
}

void CSrfCandidateListUIElement::OnCandidateMenuCommand(UINT indexInPage, int command) {
  if (!m_tip) return;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return;
  const size_t idx = static_cast<size_t>(m_tip->CandidatePageStart(m_tip->m_candPage)) + indexInPage;
  if (idx >= m_tip->CandidatePageEndExclusive(m_tip->m_candPage)) return;
  m_tip->m_candSel = static_cast<UINT>(idx);
  m_tip->ApplyCandidateMenuCommand(idx, command);
}

void CSrfCandidateListUIElement::OnCandidateWheel(int wheelDelta) {
  if (!m_tip || m_tip->m_context.candidates.items.empty()) return;
  if (m_tip->m_candidatesReading != m_tip->m_reading) return;

  if (m_tip->m_uiStyle.pagingOnScroll) {
    const UINT offset = m_tip->CandidateIndexInPage(m_tip->m_candSel);
    if (wheelDelta > 0) {
      if (m_tip->m_candPage > 0) --m_tip->m_candPage;
    } else if (wheelDelta < 0) {
      if (m_tip->m_candPage < m_tip->MaxCandidatePage()) ++m_tip->m_candPage;
    }
    const UINT pageStart = m_tip->CandidatePageStart(m_tip->m_candPage);
    const UINT pageEndExclusive = m_tip->CandidatePageEndExclusive(m_tip->m_candPage);
    m_tip->m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
  } else {
    if (wheelDelta > 0) {
      if (m_tip->m_candSel > 0) --m_tip->m_candSel;
    } else if (wheelDelta < 0 && m_tip->m_candSel + 1 < m_tip->m_context.candidates.items.size()) {
      ++m_tip->m_candSel;
    }
  }

  m_tip->ClampCandidateState();
  m_tip->SyncCandidateContextState();
  m_tip->RedrawCandidateUi();
}

void CSrfCandidateListUIElement::OnCandidateEnvironmentChanged() {
  if (!m_tip) return;
  m_layoutCacheValid = false;
  m_renderCacheValid = false;
  m_pageDataCacheValid = false;
  m_tip->RefreshCandidateWindowEnvironment();
}

void CSrfCandidateListUIElement::OnCandidateAnchorRefreshed() {
  if (!m_externalOverlayAnchorRefreshBlocked) return;
  m_externalOverlayAnchorRefreshBlocked = false;
  m_layoutCacheValid = false;
  m_renderCacheValid = false;
  m_pageDataCacheValid = false;
}

void CSrfCandidateListUIElement::ScheduleExternalOverlayHealthCheck() {
  if (m_externalOverlayHealthTimerPending || !m_tip ||
      !m_tip->EnsureDeferredTimerWindow()) {
    return;
  }
  if (SetTimer(m_tip->m_deferredTimerHwnd,
               CSrfTip::kExternalOverlayHealthTimerId,
               CSrfTip::kExternalOverlayHealthIntervalMs, nullptr)) {
    m_externalOverlayHealthTimerPending = true;
  }
}

void CSrfCandidateListUIElement::CancelExternalOverlayHealthCheck() {
  if (m_externalOverlayHealthTimerPending && m_tip &&
      m_tip->m_deferredTimerHwnd) {
    KillTimer(m_tip->m_deferredTimerHwnd,
              CSrfTip::kExternalOverlayHealthTimerId);
  }
  m_externalOverlayHealthTimerPending = false;
}

void CSrfCandidateListUIElement::ResetExternalOverlayState() {
  CancelExternalOverlayHealthCheck();
  m_externalOverlayVisible = false;
  m_externalOverlaySenderHwnd = nullptr;
  m_externalOverlayTargetHwnd = nullptr;
  m_externalOverlayTargetProcessId = 0;
  m_externalOverlayFocusGeneration = 0;
  m_externalOverlayPendingSequence = 0;
  m_externalOverlayAppliedSequence = 0;
  m_externalOverlayPendingSinceTick = 0;
}

void CSrfCandidateListUIElement::OnExternalOverlayStatusChanged() {
  if (!m_externalOverlayVisible || !m_externalOverlaySenderHwnd ||
      m_externalOverlayPendingSequence == 0) {
    CancelExternalOverlayHealthCheck();
    return;
  }
  const SrfCandidateOverlayStatus status =
      GetExternalCandidateOverlayClient().QueryStatus(
          m_externalOverlaySenderHwnd, m_externalOverlayOwnerId,
          m_externalOverlayPendingSequence);
  if (status == SrfCandidateOverlayStatus::SequenceApplied) {
    m_externalOverlayAppliedSequence = m_externalOverlayPendingSequence;
    m_externalOverlayPendingSinceTick = 0;
    m_window.Hide();
    ScheduleExternalOverlayHealthCheck();
    return;
  }
  if (status == SrfCandidateOverlayStatus::OwnerVisible &&
      m_externalOverlayAppliedSequence != 0) {
    m_window.Hide();
    ScheduleExternalOverlayHealthCheck();
    return;
  }
  if (status == SrfCandidateOverlayStatus::OwnerVisible) {
    // The helper has accepted the first frame but has not applied it yet.
    // Keep the local candidate visible and wait for SequenceApplied.
    ScheduleExternalOverlayHealthCheck();
    return;
  }
  if (status == SrfCandidateOverlayStatus::Superseded) {
    // Another TIP owner in this process won the external-overlay lease. Do not
    // health-refresh this stale owner, otherwise two UI threads can repeatedly
    // steal the single helper from one another.
    ResetExternalOverlayState();
    m_renderCacheValid = false;
    m_pageDataCacheValid = false;
    m_window.Hide();
    return;
  }

  // The helper either discarded the queued frame, exited, or lost its
  // candidate window. Keep/restore the local candidate immediately and let
  // RefreshWindow restart the dedicated helper in the background.
  ResetExternalOverlayState();
  m_renderCacheValid = false;
  m_pageDataCacheValid = false;
  m_externalOverlayAnchorRefreshBlocked = true;
  RefreshWindow();
  (void)m_tip->RequestCandidateWindowAnchorRefresh();
}

void CSrfCandidateListUIElement::OnExternalOverlayHealthTimer() {
  OnExternalOverlayStatusChanged();
}

HRESULT CSrfCandidateListUIElement::BeginOrUpdate() {
  if (!m_tip || !m_tip->m_pThreadMgr) return E_FAIL;
  if (m_tip->ShouldHideUiForCompatibility()) {
    SrfTsfDiagnosticLog(L"candidate-ui-element.skip", L"compatibility policy requested hide");
    End();
    return S_OK;
  }
  if (!m_tip->m_status.composing || m_tip->m_context.candidates.Empty()) {
    std::wstring line = L"composing=";
    line += m_tip->m_status.composing ? L"1" : L"0";
    line += L", contextEmpty=";
    line += m_tip->m_context.candidates.Empty() ? L"1" : L"0";
    SrfTsfDiagnosticLog(L"candidate-ui-element.skip", line.c_str());
    End();
    return S_OK;
  }

  ITfUIElementMgr* manager = nullptr;
  HRESULT hr =
      m_tip->m_pThreadMgr->QueryInterface(IID_ITfUIElementMgr, reinterpret_cast<void**>(&manager));
  if (FAILED(hr) || !manager) {
    const HRESULT failHr = FAILED(hr) ? hr : E_FAIL;
    if (m_tip->ShouldUseExternalCandidateOverlay()) {
      RefreshWindow();
      return S_OK;
    }
    if (m_tip->m_fullscreenCompatActive &&
        m_tip->EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::ShowUi) {
      m_tip->RecordCompatibilityUiFallback(L"ITfUIElementMgr", failHr);
    }
    return failHr;
  }

  bool refreshedWindow = false;
  if (m_uiElementId == TF_INVALID_UIELEMENTID) {
    BOOL show = TRUE;
    hr = manager->BeginUIElement(this, &show, &m_uiElementId);
    m_showWindow =
        (m_tip->m_uiLessMode || show || m_tip->ShouldUseExternalCandidateOverlay())
            ? TRUE
            : FALSE;
    std::wstring line = L"begin show=";
    line += show ? L"1" : L"0";
    line += L", uiElementId=";
    line += std::to_wstring(m_uiElementId);
    SrfTsfPerfLog(L"candidate-ui-element.begin", line.c_str());
  } else {
    RefreshWindow();
    refreshedWindow = true;
    hr = manager->UpdateUIElement(m_uiElementId);
    std::wstring line = L"update hr=0x";
    wchar_t hrBuf[16] = {};
    swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(hr));
    line += hrBuf;
    line += L", uiElementId=";
    line += std::to_wstring(m_uiElementId);
    SrfTsfPerfLog(L"candidate-ui-element.update", line.c_str());
  }

  manager->Release();
  if (FAILED(hr)) {
    if (m_tip->ShouldUseExternalCandidateOverlay()) {
      RefreshWindow();
      return S_OK;
    }
    if (m_tip->m_fullscreenCompatActive &&
        m_tip->EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::ShowUi) {
      m_tip->RecordCompatibilityUiFallback(L"CandidateUIElement", hr);
    }
    return hr;
  }

  if (!refreshedWindow) RefreshWindow();
  return S_OK;
}

void CSrfCandidateListUIElement::UpdatePresentationState() {
  if (!m_tip || !m_renderCacheValid) return;
  const bool interactive = m_tip->CandidateViewInteractive();
  const bool pendingVisual = m_tip->CandidateViewPendingVisual();
  if (m_cachedRenderInteractive == interactive &&
      m_cachedRenderPendingVisual == pendingVisual) {
    return;
  }

  m_updatedFlags = TF_CLUIE_DOCUMENTMGR | TF_CLUIE_STRING;
  if (m_tip->ShouldUseExternalCandidateOverlay()) {
    // The external helper needs a new snapshot for the presentation flags;
    // RefreshWindow reuses the existing layout cache.
    RefreshWindow();
  } else {
    m_cachedRenderInteractive = interactive;
    m_cachedRenderPendingVisual = pendingVisual;
    m_window.SetPresentationState(interactive, pendingVisual);
  }

  if (!m_tip->m_pThreadMgr || m_uiElementId == TF_INVALID_UIELEMENTID) return;
  ITfUIElementMgr* manager = nullptr;
  if (SUCCEEDED(m_tip->m_pThreadMgr->QueryInterface(
          IID_ITfUIElementMgr, reinterpret_cast<void**>(&manager))) && manager) {
    manager->UpdateUIElement(m_uiElementId);
    manager->Release();
  }
}

void CSrfCandidateListUIElement::PrepareWindowResources() {
  if (!m_tip) return;
  if (m_tip->ShouldHideUiForCompatibility()) return;
  const bool gameOverlay = m_tip->CandidateGameOverlayActive();
  const bool fullscreenOverlay = m_tip->FullscreenCandidateOverlayActive();
  m_window.SetGameOverlay(
      gameOverlay, fullscreenOverlay, gameOverlay ? m_tip->CandidateOverlayTargetWindow() : nullptr);
  m_window.SetStyle(m_tip->EffectiveCandidateUiStyle());
  m_window.PrepareResources();
  if (m_tip->ShouldUseExternalCandidateOverlay()) {
    GetExternalCandidateOverlayClient().Prewarm();
  }
}

void CSrfCandidateListUIElement::End() {
  HideExternalOverlay();
  m_externalOverlayAnchorRefreshBlocked = false;
  m_window.Hide();
  m_layoutCacheValid = false;
  m_cachedLayoutItems.clear();
  m_cachedLayoutItemsVersion = 0;
  m_cachedLayout = {};
  m_cachedAnchorDpi = 0;
  m_cachedAnchorMonitor = nullptr;
  m_cachedAnchorWorkArea = {};
  m_cachedStyle = {};
  m_renderCacheValid = false;
  m_pageDataCacheValid = false;
  m_cachedRenderTitle.clear();
  m_cachedRenderItems.clear();
  m_cachedRenderComments.clear();
  m_cachedRenderLabels.clear();
  m_cachedRenderPinnedItems.clear();
  m_cachedRenderClipboardItems.clear();
  m_cachedRenderModeTags.clear();
  m_cachedPageItems.clear();
  m_cachedPageComments.clear();
  m_cachedPageLabels.clear();
  m_cachedPagePinnedItems.clear();
  m_cachedPageClipboardItems.clear();
  m_cachedRenderInteractive = true;
  m_cachedRenderPendingVisual = false;
  if (!m_tip || !m_tip->m_pThreadMgr || m_uiElementId == TF_INVALID_UIELEMENTID) return;

  ITfUIElementMgr* manager = nullptr;
  if (SUCCEEDED(
          m_tip->m_pThreadMgr->QueryInterface(IID_ITfUIElementMgr, reinterpret_cast<void**>(&manager))) &&
      manager) {
    manager->EndUIElement(m_uiElementId);
    manager->Release();
  }

  m_uiElementId = TF_INVALID_UIELEMENTID;
  m_showWindow = TRUE;
}

void CSrfCandidateListUIElement::HideExternalOverlay() {
  if (m_externalOverlayVisible && m_externalOverlaySenderHwnd) {
    GetExternalCandidateOverlayClient().Hide(
        m_externalOverlaySenderHwnd, m_externalOverlayOwnerId,
        m_externalOverlayTargetHwnd,
        m_externalOverlayTargetProcessId, m_externalOverlayFocusGeneration);
  }
  ResetExternalOverlayState();
}

void CSrfCandidateListUIElement::RefreshWindow() {
  if (!m_tip || m_tip->ShouldHideUiForCompatibility() || !m_showWindow ||
      !m_tip->m_hasLastCandidateRect || !m_tip->m_status.composing ||
      m_tip->m_context.candidates.Empty()) {
    const bool fullscreenShowUi =
        m_tip && m_tip->m_fullscreenCompatActive &&
        m_tip->EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::ShowUi;
    if (fullscreenShowUi && m_tip->m_status.composing &&
        !m_tip->m_context.candidates.Empty() &&
        (!m_showWindow || !m_tip->m_hasLastCandidateRect)) {
      m_tip->RecordCompatibilityUiFallback(
          !m_showWindow ? L"CandidateUIHostHidden" : L"CandidateAnchorMissing", E_FAIL);
    }
    std::wstring line = L"tip=";
    line += m_tip ? L"1" : L"0";
    line += L", uiLess=";
    line += (m_tip && m_tip->m_uiLessMode) ? L"1" : L"0";
    line += L", compatHide=";
    line += (m_tip && m_tip->ShouldHideUiForCompatibility()) ? L"1" : L"0";
    line += L", showWindow=";
    line += m_showWindow ? L"1" : L"0";
    line += L", hasAnchor=";
    line += (m_tip && m_tip->m_hasLastCandidateRect) ? L"1" : L"0";
    line += L", composing=";
    line += (m_tip && m_tip->m_status.composing) ? L"1" : L"0";
    line += L", candidateEmpty=";
    line += (!m_tip || m_tip->m_context.candidates.Empty()) ? L"1" : L"0";
    SrfTsfDiagnosticLog(L"candidate-window.hide", line.c_str());
    HideExternalOverlay();
    m_window.Hide();
    m_layoutCacheValid = false;
    m_cachedLayoutItems.clear();
    m_cachedLayoutItemsVersion = 0;
    m_cachedLayout = {};
    m_cachedAnchorDpi = 0;
    m_cachedAnchorMonitor = nullptr;
    m_cachedAnchorWorkArea = {};
    m_cachedStyle = {};
    m_renderCacheValid = false;
    m_pageDataCacheValid = false;
    m_cachedRenderTitle.clear();
    m_cachedRenderItems.clear();
    m_cachedRenderComments.clear();
    m_cachedRenderLabels.clear();
    m_cachedRenderPinnedItems.clear();
    m_cachedRenderClipboardItems.clear();
    m_cachedRenderModeTags.clear();
    m_cachedPageItems.clear();
    m_cachedPageComments.clear();
    m_cachedPageLabels.clear();
    m_cachedPagePinnedItems.clear();
    m_cachedPageClipboardItems.clear();
    m_cachedRenderInteractive = true;
    m_cachedRenderPendingVisual = false;
    return;
  }

  const auto& displayItems = m_tip->BuildCandidateDisplayItems();
  const RECT& anchor = m_tip->m_lastCandidateRect;
  const SrfUIStyle st = m_tip->EffectiveCandidateUiStyle();
  const bool gameOverlay = m_tip->CandidateGameOverlayActive();
  const bool fullscreenOverlay = m_tip->FullscreenCandidateOverlayActive();
  const bool wantsExternalOverlay =
      m_tip->ShouldUseExternalCandidateOverlay() &&
      !m_externalOverlayAnchorRefreshBlocked;
  HWND overlayTarget =
      (gameOverlay || wantsExternalOverlay) ? m_tip->CandidateOverlayTargetWindow()
                                           : nullptr;
  m_window.SetGameOverlay(gameOverlay, fullscreenOverlay, overlayTarget);
  RECT layoutAnchor = anchor;
  const bool anchorPhysical =
      wantsExternalOverlay && overlayTarget &&
      ConvertCandidateOverlayAnchorToPhysical(overlayTarget, anchor, &layoutAnchor);
  UINT anchorDpi = overlayTarget ? GetDpiForWindow(overlayTarget) : 0;
  if (anchorDpi == 0) anchorDpi = DpiForScreenRect(&layoutAnchor);
  const CandidateAnchorDisplay anchorDisplay =
      DisplayForCandidateAnchor(layoutAnchor, fullscreenOverlay);

  const bool sameItems =
      m_layoutCacheValid && m_cachedLayoutItemsVersion == m_tip->CandidateDisplayVersion() &&
      m_cachedLayoutItems.size() == displayItems.size() &&
      std::equal(m_cachedLayoutItems.begin(), m_cachedLayoutItems.end(), displayItems.begin());
  const bool sameAnchor =
      m_layoutCacheValid && m_cachedAnchorRect.left == layoutAnchor.left &&
      m_cachedAnchorRect.top == layoutAnchor.top &&
      m_cachedAnchorRect.right == layoutAnchor.right &&
      m_cachedAnchorRect.bottom == layoutAnchor.bottom;
  const bool sameDpi = m_layoutCacheValid && m_cachedAnchorDpi == anchorDpi;
  const bool sameDisplay =
      m_layoutCacheValid && m_cachedAnchorMonitor == anchorDisplay.monitor &&
      m_cachedAnchorWorkArea.left == anchorDisplay.workArea.left &&
      m_cachedAnchorWorkArea.top == anchorDisplay.workArea.top &&
      m_cachedAnchorWorkArea.right == anchorDisplay.workArea.right &&
      m_cachedAnchorWorkArea.bottom == anchorDisplay.workArea.bottom;
  const bool sameStyle = m_layoutCacheValid && m_cachedStyleFontSize == st.candidateFontSize &&
                         m_cachedStyleAbbreviateLength == st.candidateAbbreviateLength &&
                         m_cachedStyleHorizontal == st.candidateHorizontal &&
                         m_cachedStylePageSize == st.candidatePageSize &&
                         m_cachedStyleHorizontalCount == st.candidateHorizontalCount &&
                         m_cachedStyleHorizontalCompact == st.candidateHorizontalCompact &&
                         m_cachedStyleFontFile == st.candidateFontFile &&
                         m_cachedStyleSkinFile == st.candidateSkinFile &&
                         m_cachedStyleMaterial == st.candidateMaterial &&
                         m_cachedStyleDensity == st.candidateDensity &&
                         m_cachedStyleLayoutVariant == st.candidateLayoutVariant &&
                         m_cachedStyleScalePercent == st.candidateScalePercent &&
                         m_cachedStyleOverlayAnchor == st.candidateOverlayAnchor &&
                         m_cachedStyleFullscreenPlacement == st.candidateFullscreenPlacement &&
                         m_cachedStyleSkinLoaded == st.skinLoaded &&
                         m_cachedSkinOuterPadX == st.skinOuterPadX &&
                         m_cachedSkinOuterPadY == st.skinOuterPadY &&
                         m_cachedSkinHeaderPadX == st.skinHeaderPadX &&
                         m_cachedSkinHeaderPadY == st.skinHeaderPadY &&
                         m_cachedSkinHeaderGap == st.skinHeaderGap &&
                         m_cachedSkinItemGap == st.skinItemGap &&
                         m_cachedSkinItemPadX == st.skinItemPadX &&
                         m_cachedSkinItemPadY == st.skinItemPadY &&
                         m_cachedSkinLabelWidth == st.skinLabelWidth &&
                         m_cachedSkinLabelGap == st.skinLabelGap &&
                         m_cachedSkinCommentGap == st.skinCommentGap &&
                         m_cachedSkinMinWidth == st.skinMinWidth &&
                         m_cachedSkinPreferredWidth == st.skinPreferredWidth &&
                         m_cachedSkinMaxWidth == st.skinMaxWidth &&
                         m_cachedSkinMinHorizontalCardWidth == st.skinMinHorizontalCardWidth &&
                         m_cachedSkinMaxHorizontalCardWidth == st.skinMaxHorizontalCardWidth;

  if (!sameItems || !sameAnchor || !sameStyle || !sameDpi || !sameDisplay) {
    m_cachedLayout =
        BuildCandidatePageLayoutMetrics(st, &layoutAnchor, displayItems, anchorDpi);
    m_cachedLayoutItems = displayItems;
    m_cachedLayoutItemsVersion = m_tip->CandidateDisplayVersion();
    m_cachedAnchorRect = layoutAnchor;
    m_cachedAnchorDpi = anchorDpi;
    m_cachedAnchorMonitor = anchorDisplay.monitor;
    m_cachedAnchorWorkArea = anchorDisplay.workArea;
    m_cachedStyleFontSize = st.candidateFontSize;
    m_cachedStyleAbbreviateLength = st.candidateAbbreviateLength;
    m_cachedStyleHorizontal = st.candidateHorizontal;
    m_cachedStylePageSize = st.candidatePageSize;
    m_cachedStyleHorizontalCount = st.candidateHorizontalCount;
    m_cachedStyleHorizontalCompact = st.candidateHorizontalCompact;
    m_cachedStyleFontFile = st.candidateFontFile;
    m_cachedStyleSkinFile = st.candidateSkinFile;
    m_cachedStyleSkinLoaded = st.skinLoaded;
    m_cachedStyleMaterial = st.candidateMaterial;
    m_cachedStyleDensity = st.candidateDensity;
    m_cachedStyleLayoutVariant = st.candidateLayoutVariant;
    m_cachedStyleScalePercent = st.candidateScalePercent;
    m_cachedStyleOverlayAnchor = st.candidateOverlayAnchor;
    m_cachedStyleFullscreenPlacement = st.candidateFullscreenPlacement;
    m_cachedSkinOuterPadX = st.skinOuterPadX;
    m_cachedSkinOuterPadY = st.skinOuterPadY;
    m_cachedSkinHeaderPadX = st.skinHeaderPadX;
    m_cachedSkinHeaderPadY = st.skinHeaderPadY;
    m_cachedSkinHeaderGap = st.skinHeaderGap;
    m_cachedSkinItemGap = st.skinItemGap;
    m_cachedSkinItemPadX = st.skinItemPadX;
    m_cachedSkinItemPadY = st.skinItemPadY;
    m_cachedSkinLabelWidth = st.skinLabelWidth;
    m_cachedSkinLabelGap = st.skinLabelGap;
    m_cachedSkinCommentGap = st.skinCommentGap;
    m_cachedSkinMinWidth = st.skinMinWidth;
    m_cachedSkinPreferredWidth = st.skinPreferredWidth;
    m_cachedSkinMaxWidth = st.skinMaxWidth;
    m_cachedSkinMinHorizontalCardWidth = st.skinMinHorizontalCardWidth;
    m_cachedSkinMaxHorizontalCardWidth = st.skinMaxHorizontalCardWidth;
    m_layoutCacheValid = true;
  }

  const CandidatePageLayoutMetrics& layout = m_cachedLayout;
  const UINT clampedPage = layout.pageStarts.empty()
                               ? 0u
                               : std::min(m_tip->m_candPage, static_cast<UINT>(layout.pageStarts.size() - 1));
  const UINT start = layout.pageStarts.empty() ? 0u : layout.pageStarts[clampedPage];
  const UINT end = clampedPage + 1 < layout.pageStarts.size()
                       ? layout.pageStarts[clampedPage + 1]
                       : static_cast<UINT>(displayItems.size());
  const bool clipboardPanel = m_tip->CurrentCandidatesClipboardQuickMode();
  const bool samePageData =
      m_pageDataCacheValid &&
      m_cachedPageDataVersion == m_tip->CandidateDisplayVersion() &&
      m_cachedPageDataStart == start && m_cachedPageDataEnd == end &&
      m_cachedPageDataSelected == m_tip->m_candSel &&
      m_cachedPageDataHorizontal == st.candidateHorizontal &&
      m_cachedPageDataClipboardPanel == clipboardPanel;
  std::vector<std::wstring> pageItems;
  std::vector<std::wstring> pageComments;
  std::vector<std::wstring> pageLabels;
  std::vector<bool> pagePinnedItems;
  std::vector<bool> pageClipboardItems;
  UINT clipboardPage = 1;
  UINT clipboardPages = 1;
  bool hasClipboardPage = false;
  if (samePageData) {
    // Candidate text/metadata are unchanged. Reuse the already formatted
    // vectors and avoid rebuilding labels/comments (and the selected display
    // string) on every anchor or overlay refresh.
    pageItems = m_cachedPageItems;
    pageComments = m_cachedPageComments;
    pageLabels = m_cachedPageLabels;
    pagePinnedItems = m_cachedPagePinnedItems;
    pageClipboardItems = m_cachedPageClipboardItems;
    hasClipboardPage = m_cachedPageDataHasClipboardPage;
    clipboardPage = m_cachedPageDataClipboardPage;
    clipboardPages = m_cachedPageDataClipboardPages;
  } else {
    pageItems.reserve(end - start);
    pageComments.reserve(end - start);
    pageLabels.reserve(end - start);
    pagePinnedItems.reserve(end - start);
    pageClipboardItems.reserve(end - start);
    for (UINT i = start; i < end; ++i) {
      const bool clipboardCandidate = m_tip->IsClipboardQuickCandidate(i);
      const bool clipboardItem = clipboardPanel || clipboardCandidate;
      if (clipboardCandidate && !hasClipboardPage) {
        hasClipboardPage =
            m_tip->ClipboardCandidatePageInfo(i, &clipboardPage, &clipboardPages);
      }
      const bool useDisplayItem = i < displayItems.size();
      pageItems.push_back(useDisplayItem
                              ? displayItems[i]
                              : (i < m_tip->m_context.candidates.items.size()
                                     ? m_tip->m_context.candidates.items[i].str
                                     : std::wstring()));
      const bool selectedItem = i == m_tip->m_candSel;
      if (selectedItem && !clipboardItem) {
        pageItems.back() = m_tip->FormatCandidateDisplayText(i);
      }
      // 横排必须保持单行；纠错候选已有“~”前缀，不再用第二行注释重复表达。
      // 纵排中，拼音、注解与调试分数只跟随当前候选，减少整栏视觉噪音。
      const bool showComment = ShouldShowCandidateComment(
          st.candidateHorizontal, clipboardItem, selectedItem);
      pageComments.push_back(
          showComment && i < m_tip->m_context.candidates.comments.size()
              ? m_tip->m_context.candidates.comments[i].str
              : std::wstring());
      pageLabels.push_back(i < m_tip->m_context.candidates.labels.size()
                               ? m_tip->m_context.candidates.labels[i].str
                               : std::to_wstring(i - start + 1));
      pagePinnedItems.push_back(m_tip->IsClipboardCandidatePinned(i) ||
                                m_tip->IsCandidatePinned(i));
      pageClipboardItems.push_back(clipboardItem);
    }
    m_cachedPageItems = pageItems;
    m_cachedPageComments = pageComments;
    m_cachedPageLabels = pageLabels;
    m_cachedPagePinnedItems = pagePinnedItems;
    m_cachedPageClipboardItems = pageClipboardItems;
    m_cachedPageDataVersion = m_tip->CandidateDisplayVersion();
    m_cachedPageDataStart = start;
    m_cachedPageDataEnd = end;
    m_cachedPageDataSelected = m_tip->m_candSel;
    m_cachedPageDataHorizontal = st.candidateHorizontal;
    m_cachedPageDataClipboardPanel = clipboardPanel;
    m_cachedPageDataHasClipboardPage = hasClipboardPage;
    m_cachedPageDataClipboardPage = clipboardPage;
    m_cachedPageDataClipboardPages = clipboardPages;
    m_pageDataCacheValid = true;
  }

  const UINT selectedInPage =
      (m_tip->m_candSel >= start && m_tip->m_candSel < end) ? (m_tip->m_candSel - start) : 0u;
  const UINT totalPages = std::max(1u, static_cast<UINT>(layout.pageStarts.size()));
  const UINT renderPage = hasClipboardPage ? clipboardPage : clampedPage + 1;
  const UINT renderTotalPages = hasClipboardPage ? clipboardPages : totalPages;
  const std::wstring title = m_tip->CandidateBarMainTitle();
  std::vector<std::wstring> modeTags = m_tip->CandidateBarModeTags();
  const bool interactive = m_tip->CandidateViewInteractive();
  const bool pendingVisual = m_tip->CandidateViewPendingVisual();
  m_window.SetStyle(st);
  if (!m_renderCacheValid || !CandidateWindowStyleEquals(m_cachedStyle, st)) {
    m_updatedFlags = TF_CLUIE_DOCUMENTMGR | TF_CLUIE_COUNT | TF_CLUIE_SELECTION |
                     TF_CLUIE_STRING | TF_CLUIE_PAGEINDEX | TF_CLUIE_CURRENTPAGE;
  } else {
    DWORD flags = TF_CLUIE_DOCUMENTMGR;
    if (m_cachedRenderItems.size() != pageItems.size()) flags |= TF_CLUIE_COUNT;
    if (m_cachedRenderItems != pageItems || m_cachedRenderComments != pageComments ||
        m_cachedRenderLabels != pageLabels ||
        m_cachedRenderClipboardItems != pageClipboardItems) {
      flags |= TF_CLUIE_STRING;
    }
    if (m_cachedRenderSelected != selectedInPage) flags |= TF_CLUIE_SELECTION;
    if (m_cachedRenderPage != renderPage || m_cachedRenderTotalPages != renderTotalPages) {
      flags |= TF_CLUIE_PAGEINDEX | TF_CLUIE_CURRENTPAGE;
    }
    if (flags == TF_CLUIE_DOCUMENTMGR &&
        (m_cachedRenderTitle != title || m_cachedRenderModeTags != modeTags ||
         m_cachedRenderPinnedItems != pagePinnedItems ||
         m_cachedRenderInteractive != interactive ||
         m_cachedRenderPendingVisual != pendingVisual)) {
      flags |= TF_CLUIE_STRING;
    }
    m_updatedFlags = flags;
  }
  const bool sameRender =
      m_renderCacheValid && CandidateWindowStyleEquals(m_cachedStyle, st) &&
      sameAnchor && sameDpi && sameDisplay &&
      m_cachedRenderTitle == title && m_cachedRenderItems == pageItems &&
      m_cachedRenderComments == pageComments && m_cachedRenderLabels == pageLabels &&
      m_cachedRenderPinnedItems == pagePinnedItems &&
      m_cachedRenderClipboardItems == pageClipboardItems && m_cachedRenderModeTags == modeTags &&
      m_cachedRenderInteractive == interactive &&
      m_cachedRenderPendingVisual == pendingVisual &&
      m_cachedRenderPage == renderPage &&
      m_cachedRenderTotalPages == renderTotalPages && m_cachedRenderSelected == selectedInPage &&
      m_cachedRenderAnchor.left == m_tip->m_lastCandidateRect.left &&
      m_cachedRenderAnchor.top == m_tip->m_lastCandidateRect.top &&
      m_cachedRenderAnchor.right == m_tip->m_lastCandidateRect.right &&
      m_cachedRenderAnchor.bottom == m_tip->m_lastCandidateRect.bottom;
  if (sameRender && !wantsExternalOverlay && m_window.IsVisible()) {
    SrfTsfPerfLog(L"candidate-window.skip", L"render state unchanged");
    return;
  }

  std::wstring line = L"page=";
  line += std::to_wstring(clampedPage + 1);
  line += L"/";
  line += std::to_wstring(totalPages);
  line += L", items=";
  line += std::to_wstring(pageItems.size());
  line += L", selectedInPage=";
  line += std::to_wstring(selectedInPage);
  line += L", viewState=";
  line += m_tip->CandidateViewStateName();
  line += L", interactive=";
  line += interactive ? L"1" : L"0";
  line += L", pendingVisual=";
  line += pendingVisual ? L"1" : L"0";
  line += L", backend=";
  line += wantsExternalOverlay ? L"external" : L"in-process";
  SrfTsfPerfLog(L"candidate-window.show", line.c_str());
  bool externalAccepted = false;
  if (wantsExternalOverlay && m_tip->EnsureDeferredTimerWindow()) {
    HWND targetHwnd = m_tip->CandidateOverlayTargetWindow();
    DWORD targetProcessId = 0;
    if (targetHwnd) GetWindowThreadProcessId(targetHwnd, &targetProcessId);
    ITfContext* focusContext =
        m_tip->m_pCompositionContext ? m_tip->m_pCompositionContext : m_tip->m_pFocusContext;
    const SrfFocusSnapshot focus = m_tip->CaptureFocusSnapshot(focusContext);
    if (targetHwnd && targetProcessId != 0 && m_tip->FocusSnapshotMatches(focus)) {
      SrfCandidateOverlaySnapshot snapshot = {};
      snapshot.visible = true;
      snapshot.pendingVisual = pendingVisual;
      snapshot.gameCompact = gameOverlay;
      snapshot.fullscreenPlacement = fullscreenOverlay;
      snapshot.layoutResolved = true;
      snapshot.horizontalLayout = st.candidateHorizontal;
      snapshot.horizontalCompact = st.candidateHorizontalCompact;
      snapshot.anchorPhysical = anchorPhysical;
      snapshot.caretAnchor =
          !gameOverlay || st.candidateOverlayAnchor == SrfOverlayAnchor::Caret;
      snapshot.targetProcessId = targetProcessId;
      snapshot.targetHwnd = targetHwnd;
      snapshot.focusGeneration = focus.generation;
      snapshot.anchor = layoutAnchor;
      snapshot.pageIndex = renderPage;
      snapshot.totalPages = renderTotalPages;
      snapshot.selectedInPage = selectedInPage;
      snapshot.appPath = m_tip->m_activeAppName;
      snapshot.title = title;
      snapshot.items = pageItems;
      snapshot.comments = pageComments;
      snapshot.labels = pageLabels;
      snapshot.pinnedItems = pagePinnedItems;
      snapshot.clipboardItems = pageClipboardItems;
      snapshot.modeTags = modeTags;
      const bool sameExternalSession =
          m_externalOverlayVisible &&
          m_externalOverlaySenderHwnd == m_tip->m_deferredTimerHwnd &&
          m_externalOverlayTargetHwnd == targetHwnd &&
          m_externalOverlayTargetProcessId == targetProcessId &&
          m_externalOverlayFocusGeneration == focus.generation;
      if (sameRender && sameExternalSession && m_externalOverlayPendingSequence != 0) {
        // The helper already accepted this exact render snapshot. Its health
        // timer owns pending/applied confirmation; a normal redraw must not
        // synchronously resend the same WM_COPYDATA packet.
        externalAccepted = true;
        ScheduleExternalOverlayHealthCheck();
      } else {
        std::uint64_t acceptedSequence = 0;
        externalAccepted = GetExternalCandidateOverlayClient().Show(
            m_tip->m_deferredTimerHwnd, m_externalOverlayOwnerId, snapshot,
            &acceptedSequence);
        if (externalAccepted) {
          if (!sameExternalSession) m_externalOverlayAppliedSequence = 0;
          m_externalOverlayVisible = true;
          m_externalOverlaySenderHwnd = m_tip->m_deferredTimerHwnd;
          m_externalOverlayTargetHwnd = targetHwnd;
          m_externalOverlayTargetProcessId = targetProcessId;
          m_externalOverlayFocusGeneration = focus.generation;
          m_externalOverlayPendingSequence = acceptedSequence;
          m_externalOverlayPendingSinceTick = GetTickCount64();
          ScheduleExternalOverlayHealthCheck();
        }
      }
    }
  }
  const bool externalReady =
      externalAccepted && m_externalOverlayAppliedSequence != 0;
  if (externalReady) {
    m_window.Hide();
  } else {
    if (!externalAccepted) HideExternalOverlay();
    m_window.Show(title, pageItems, pageComments, pageLabels, pagePinnedItems,
                  pageClipboardItems, renderPage, renderTotalPages, selectedInPage,
                  m_tip->m_lastCandidateRect, modeTags, interactive, pendingVisual);
  }
  m_cachedStyle = st;
  m_renderCacheValid = true;
  m_cachedRenderTitle = title;
  m_cachedRenderItems = std::move(pageItems);
  m_cachedRenderComments = std::move(pageComments);
  m_cachedRenderLabels = std::move(pageLabels);
  m_cachedRenderPinnedItems = std::move(pagePinnedItems);
  m_cachedRenderClipboardItems = std::move(pageClipboardItems);
  m_cachedRenderModeTags = std::move(modeTags);
  m_cachedRenderInteractive = interactive;
  m_cachedRenderPendingVisual = pendingVisual;
  m_cachedRenderAnchor = m_tip->m_lastCandidateRect;
  m_cachedRenderPage = renderPage;
  m_cachedRenderTotalPages = renderTotalPages;
  m_cachedRenderSelected = selectedInPage;
}
