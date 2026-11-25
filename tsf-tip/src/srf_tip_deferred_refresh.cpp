#include "srf_tip_detail.h"

using namespace srf_tip_detail;

class CEditSessionDeferredRefresh final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  SrfFocusSnapshot m_focus = {};

 public:
  explicit CEditSessionDeferredRefresh(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) {
      m_tip->AddRef();
      m_focus = m_tip->CaptureFocusSnapshot(nullptr);
    }
  }
  ~CEditSessionDeferredRefresh() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    if (!m_tip->FocusSnapshotMatches(m_focus)) return S_OK;
    if (m_tip->m_compositionState.reading.empty()) return S_OK;
    m_tip->RefreshCandidatesAsync();
    m_tip->UpdateCandidateWindow(ec);
    return S_OK;
  }
};

class CEditSessionCandidateAnchorRefresh final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  SrfFocusSnapshot m_focus = {};

 public:
  CEditSessionCandidateAnchorRefresh(CSrfTip* tip, SrfFocusSnapshot focus)
      : m_tip(tip), m_focus(focus) {
    if (m_tip) m_tip->AddRef();
  }
  ~CEditSessionCandidateAnchorRefresh() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    if (!m_tip->FocusSnapshotMatches(m_focus)) return S_OK;
    m_tip->m_candidateAnchorRefreshEditPending = false;
    m_tip->m_candidateAnchorRefreshRequestTick = 0;
    if (m_tip->m_candidateUi) {
      m_tip->m_candidateUi->OnCandidateAnchorRefreshed();
    }
    if (m_tip->m_compositionState.reading.empty() || !m_tip->m_status.composing) return S_OK;
    m_tip->UpdateCandidateWindow(ec);
    return S_OK;
  }
};

class CEditSessionApplyAsyncCandidates final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  SrfFocusSnapshot m_focus = {};

 public:
  explicit CEditSessionApplyAsyncCandidates(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) {
      m_tip->AddRef();
      m_focus = m_tip->CaptureFocusSnapshot(nullptr);
    }
  }
  ~CEditSessionApplyAsyncCandidates() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    if (!m_tip->FocusSnapshotMatches(m_focus)) return S_OK;
    m_tip->ApplyAsyncCandidateResult(ec);
    return S_OK;
  }
};

// ---------------------------------------------------------------------------
// 延迟候选刷新定时器
// ---------------------------------------------------------------------------

LRESULT CALLBACK CSrfTip::DeferredTimerWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  CSrfTip* self = reinterpret_cast<CSrfTip*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
  if (msg == WM_NCCREATE) {
    CREATESTRUCTW* cs = reinterpret_cast<CREATESTRUCTW*>(lParam);
    self = reinterpret_cast<CSrfTip*>(cs->lpCreateParams);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    return TRUE;
  }
  if (msg == WM_TIMER && wParam == kEngineInputHealthTimerId && self) {
    self->RefreshGameInputHealth();
    if (self->m_pKeySink) self->m_pKeySink->RefreshToggleLatches();
    if (!self->m_compositionState.reading.empty()) self->EnsureEngineInputReady();
    return 0;
  }
  if (msg == WM_TIMER && wParam == kGameChatProbeTimerId && self) {
    if (self->m_gameChat.chatPhase == SrfGameChatPhase::Awaiting &&
        !self->RefreshPureGameModeFromRegistry()) {
      self->RefreshGameChatFocus();
    } else {
      KillTimer(hwnd, kGameChatProbeTimerId);
    }
    return 0;
  }
  if (msg == WM_TIMER && wParam == kDeferredCandidateTimerId && self) {
    self->OnDeferredCandidateRefreshTimer();
    return 0;
  }
  if (msg == WM_TIMER && wParam == kCandidateUiRedrawTimerId && self) {
    self->OnCandidateUiRedrawTimer();
    return 0;
  }
  if (msg == WM_TIMER && wParam == kDeferredFocusClearTimerId && self) {
    self->OnDeferredFocusContextClearTimer();
    return 0;
  }
  if (msg == WM_TIMER && wParam == kExternalOverlayHealthTimerId && self) {
    if (self->m_candidateUi) self->m_candidateUi->OnExternalOverlayHealthTimer();
    return 0;
  }
  if (msg == WM_TIMER && wParam == kCandidateAnchorRefreshTimerId && self) {
    self->OnCandidateWindowAnchorRefreshTimer();
    return 0;
  }
  const UINT overlayStatusMessage = SrfCandidateOverlayStatusChangedMessage();
  if (overlayStatusMessage != 0 && msg == overlayStatusMessage && self) {
    if (self->m_candidateUi) self->m_candidateUi->OnExternalOverlayStatusChanged();
    return 0;
  }
  if (msg == kAsyncCandidateResultMessage && self) {
    self->OnAsyncCandidateResultMessage();
    return 0;
  }
  if (msg == kLearnCommitCompletedMessage && self) {
    self->OnLearnCommitCompleted(static_cast<unsigned long long>(wParam), lParam != 0);
    return 0;
  }
  if (msg == WM_NCDESTROY) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    return 0;
  }
  return DefWindowProcW(hwnd, msg, wParam, lParam);
}

void CSrfTip::OnLearnCommitCompleted(unsigned long long requestId, bool succeeded) {
  const auto it =
      std::find_if(m_pendingLearnNotifications.begin(), m_pendingLearnNotifications.end(),
                   [requestId](const auto& pending) { return pending.requestId == requestId; });
  if (it == m_pendingLearnNotifications.end()) return;
  std::wstring reading = std::move(it->reading);
  std::wstring phrase = std::move(it->phrase);
  m_pendingLearnNotifications.erase(it);
  if (!succeeded) {
    SrfTsfDiagnosticLog(L"composed-phrase.learn", L"engine did not confirm save");
    return;
  }
  // The user can start typing the same word again before the asynchronous
  // composed-phrase write finishes.  The bridge cache is invalidated before
  // this completion is posted; refresh an active matching composition so the
  // learned phrase appears immediately instead of waiting for a third input.
  if (m_status.composing && !m_compositionState.reading.empty() &&
      m_compositionState.reading == reading) {
    ScheduleDeferredCandidateRefresh(1);
  }
  if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
    ShowNotification(SrfNotificationKind::AppOptions,
                     L"\u5df2\u8bb0\u5f55\u751f\u8bcd\uff1a" + ShortenForLog(phrase, 12));
  }
}

bool CSrfTip::EnsureDeferredTimerWindow() {
  if (m_deferredTimerHwnd) return true;
  // 注册隐藏窗口类（仅用于承载定时器和后台候选结果消息）。
  static ATOM atom = 0;
  if (!atom) {
    WNDCLASSEXW wc = {};
    wc.cbSize = sizeof(wc);
    wc.lpfnWndProc = &CSrfTip::DeferredTimerWndProc;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = SrfCandidateOverlayClientWindowClass();
    atom = RegisterClassExW(&wc);
    if (!atom && GetLastError() == ERROR_CLASS_ALREADY_EXISTS) atom = 1;
  }
  if (!atom) return false;

  m_deferredTimerHwnd = CreateWindowExW(0, SrfCandidateOverlayClientWindowClass(), L"", 0, 0, 0, 0,
                                        0, HWND_MESSAGE, nullptr, GetModuleHandleW(nullptr), this);
  return m_deferredTimerHwnd != nullptr;
}

void CSrfTip::ScheduleDeferredCandidateRefresh() {
  ScheduleDeferredCandidateRefresh(kDeferredCandidateRetryMs);
}

void CSrfTip::ScheduleDeferredCandidateRefresh(DWORD delayMs) {
  delayMs = std::max<DWORD>(1, delayMs);
  const ULONGLONG dueTick = GetTickCount64() + delayMs;
  if (m_deferredRefreshPending) {
    if (m_deferredRefreshReading == m_compositionState.reading && m_deferredRefreshDueTick != 0 &&
        m_deferredRefreshDueTick <= dueTick) {
      SrfTsfPerfLog(L"candidate-refresh.defer", L"timer already pending");
      return;
    }
    if (m_deferredTimerHwnd) KillTimer(m_deferredTimerHwnd, kDeferredCandidateTimerId);
    m_deferredRefreshPending = false;
    m_deferredRefreshDueTick = 0;
  }

  if (!EnsureDeferredTimerWindow()) return;

  if (SetTimer(m_deferredTimerHwnd, kDeferredCandidateTimerId, delayMs, nullptr)) {
    m_deferredRefreshPending = true;
    m_deferredRefreshReading = m_compositionState.reading;
    m_deferredRefreshDueTick = dueTick;
    std::wstring line = L"reading=" + ShortenForLog(m_compositionState.reading, 24);
    line += L", delayMs=";
    line += std::to_wstring(delayMs);
    SrfTsfPerfLog(L"candidate-refresh.defer", line.c_str());
  }
}

void CSrfTip::OnAsyncCandidateResultMessage() {
  ITfContext* context = m_compositionState.pCompositionContext
                            ? m_compositionState.pCompositionContext
                            : m_pFocusContext;
  if (!context || m_tid == 0) return;

  CEditSessionApplyAsyncCandidates* edit =
      new (std::nothrow) CEditSessionApplyAsyncCandidates(this);
  if (!edit) return;

  HRESULT hrSession = E_FAIL;
  HRESULT hr = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  edit->Release();
  wchar_t buf[176] = {};
  swprintf_s(buf, L"RequestEditSession hr=0x%08lX session=0x%08lX", static_cast<unsigned long>(hr),
             static_cast<unsigned long>(hrSession));
  SrfTsfPerfLog(L"candidate-refresh.async-edit", buf);

  if (FAILED(hr) || FAILED(hrSession)) {
    ScheduleDeferredCandidateRefresh();
  }
}

void CSrfTip::CancelDeferredCandidateRefresh() {
  if (!m_deferredRefreshPending) return;
  if (m_deferredTimerHwnd) KillTimer(m_deferredTimerHwnd, kDeferredCandidateTimerId);
  m_deferredRefreshPending = false;
  m_deferredRefreshReading.clear();
  m_deferredRefreshDueTick = 0;
}

void CSrfTip::ScheduleCandidateUiRedraw(DWORD delayMs) {
  if (!m_candidateUi) return;
  if (!EnsureDeferredTimerWindow()) {
    if (delayMs <= kCandidateUiRedrawCoalesceMs) RedrawCandidateUiNow();
    return;
  }

  const DWORD resolvedDelay = std::max<DWORD>(1, delayMs);
  const ULONGLONG dueTick = GetTickCount64() + resolvedDelay;
  if (m_candidateUiRedrawPending) {
    if (m_candidateUiRedrawDueTick != 0 && m_candidateUiRedrawDueTick <= dueTick) {
      return;
    }
    KillTimer(m_deferredTimerHwnd, kCandidateUiRedrawTimerId);
    m_candidateUiRedrawPending = false;
    m_candidateUiRedrawDueTick = 0;
  }

  if (SetTimer(m_deferredTimerHwnd, kCandidateUiRedrawTimerId, resolvedDelay, nullptr)) {
    m_candidateUiRedrawPending = true;
    m_candidateUiRedrawDueTick = dueTick;
  } else {
    if (delayMs <= kCandidateUiRedrawCoalesceMs) RedrawCandidateUiNow();
  }
}

void CSrfTip::CancelScheduledCandidateUiRedraw() {
  if (!m_candidateUiRedrawPending) return;
  if (m_deferredTimerHwnd) KillTimer(m_deferredTimerHwnd, kCandidateUiRedrawTimerId);
  m_candidateUiRedrawPending = false;
  m_candidateUiRedrawDueTick = 0;
}

void CSrfTip::OnCandidateUiRedrawTimer() {
  CancelScheduledCandidateUiRedraw();
  RedrawCandidateUiNow();
}

bool CSrfTip::ShouldDeferFocusContextClear() const {
  if (!m_pFocusContext) return false;
  return m_compositionState.pComposition || m_status.composing ||
         !m_compositionState.reading.empty() || !m_candidateView.candidates.empty() ||
         !m_context.candidates.Empty();
}

bool CSrfTip::ScheduleDeferredFocusContextClear() {
  if (!ShouldDeferFocusContextClear()) return false;
  if (!EnsureDeferredTimerWindow()) return false;

  const SrfFocusSnapshot scheduledFocus = CaptureFocusSnapshot(m_pFocusContext);
  const ULONGLONG dueTick = GetTickCount64() + kTransientFocusLossGraceMs;
  if (m_deferredFocusClearPending) {
    if (m_deferredFocusClearDueTick != 0 && m_deferredFocusClearDueTick <= dueTick) {
      SrfTsfPerfLog(L"focus-context.defer-clear", L"timer already pending");
      return true;
    }
    KillTimer(m_deferredTimerHwnd, kDeferredFocusClearTimerId);
    m_deferredFocusClearPending = false;
    m_deferredFocusClearDueTick = 0;
    m_deferredFocusClearSnapshot = {};
  }

  if (SetTimer(m_deferredTimerHwnd, kDeferredFocusClearTimerId, kTransientFocusLossGraceMs,
               nullptr)) {
    m_deferredFocusClearPending = true;
    m_deferredFocusClearDueTick = dueTick;
    m_deferredFocusClearSnapshot = scheduledFocus;
    std::wstring line = L"delayMs=";
    line += std::to_wstring(kTransientFocusLossGraceMs);
    line += L", reading=";
    line += ShortenForLog(m_compositionState.reading, 24);
    SrfTsfPerfLog(L"focus-context.defer-clear", line.c_str());
    return true;
  }
  m_deferredFocusClearSnapshot = {};
  return false;
}

void CSrfTip::CancelDeferredFocusContextClear() {
  if (!m_deferredFocusClearPending) return;
  if (m_deferredTimerHwnd) KillTimer(m_deferredTimerHwnd, kDeferredFocusClearTimerId);
  m_deferredFocusClearPending = false;
  m_deferredFocusClearDueTick = 0;
  m_deferredFocusClearSnapshot = {};
  SrfTsfPerfLog(L"focus-context.defer-clear.cancel", L"timer canceled");
}

void CSrfTip::CancelCandidateWindowAnchorRefreshRetry() {
  if (m_candidateAnchorRefreshRetryPending && m_deferredTimerHwnd) {
    KillTimer(m_deferredTimerHwnd, kCandidateAnchorRefreshTimerId);
  }
  m_candidateAnchorRefreshRetryPending = false;
}

void CSrfTip::ScheduleCandidateWindowAnchorRefreshRetry() {
  if (m_candidateAnchorRefreshRetryPending || m_compositionState.reading.empty() ||
      !m_status.composing || !EnsureDeferredTimerWindow()) {
    return;
  }
  if (SetTimer(m_deferredTimerHwnd, kCandidateAnchorRefreshTimerId, kCandidateAnchorRefreshRetryMs,
               nullptr)) {
    m_candidateAnchorRefreshRetryPending = true;
  }
}

bool CSrfTip::RequestCandidateWindowAnchorRefresh() {
  if (m_compositionState.reading.empty() || !m_status.composing || m_tid == 0) return false;
  const ULONGLONG now = GetTickCount64();
  if (m_candidateAnchorRefreshEditPending) {
    if (m_candidateAnchorRefreshRequestTick != 0 &&
        now - m_candidateAnchorRefreshRequestTick < kCandidateAnchorRefreshStaleMs) {
      return true;
    }
    m_candidateAnchorRefreshEditPending = false;
    m_candidateAnchorRefreshRequestTick = 0;
  }

  ITfContext* context = m_compositionState.pCompositionContext
                            ? m_compositionState.pCompositionContext
                            : m_pFocusContext;
  if (!context) {
    ScheduleCandidateWindowAnchorRefreshRetry();
    return false;
  }
  const SrfFocusSnapshot focus = CaptureFocusSnapshot(context);
  CEditSessionCandidateAnchorRefresh* edit =
      new (std::nothrow) CEditSessionCandidateAnchorRefresh(this, focus);
  if (!edit) {
    ScheduleCandidateWindowAnchorRefreshRetry();
    return false;
  }

  m_candidateAnchorRefreshEditPending = true;
  m_candidateAnchorRefreshRequestTick = now;
  HRESULT hrSession = E_FAIL;
  const HRESULT hr = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READ, &hrSession);
  edit->Release();
  if (FAILED(hr) || FAILED(hrSession)) {
    m_candidateAnchorRefreshEditPending = false;
    m_candidateAnchorRefreshRequestTick = 0;
    ScheduleCandidateWindowAnchorRefreshRetry();
    std::wstring line = L"hr=";
    line += std::to_wstring(static_cast<unsigned long>(hr));
    line += L", session=";
    line += std::to_wstring(static_cast<unsigned long>(hrSession));
    SrfTsfDiagnosticLog(L"candidate-anchor.refresh-request-failed", line.c_str());
    return false;
  }
  CancelCandidateWindowAnchorRefreshRetry();
  return true;
}

void CSrfTip::OnCandidateWindowAnchorRefreshTimer() {
  CancelCandidateWindowAnchorRefreshRetry();
  if (m_compositionState.reading.empty() || !m_status.composing) return;
  (void)RequestCandidateWindowAnchorRefresh();
}

void CSrfTip::OnDeferredFocusContextClearTimer() {
  if (!m_deferredFocusClearPending) {
    SrfTsfPerfLog(L"focus-context.defer-clear.skip", L"stale queued timer");
    return;
  }
  const SrfFocusSnapshot scheduledFocus = m_deferredFocusClearSnapshot;
  const bool shouldClear = ShouldDeferFocusContextClear();
  if (m_deferredTimerHwnd) KillTimer(m_deferredTimerHwnd, kDeferredFocusClearTimerId);
  m_deferredFocusClearPending = false;
  m_deferredFocusClearDueTick = 0;
  m_deferredFocusClearSnapshot = {};

  if (!shouldClear) {
    SrfTsfPerfLog(L"focus-context.defer-clear.skip", L"state already stable");
    return;
  }
  if (scheduledFocus.generation != m_focusGeneration || !FocusSnapshotMatches(scheduledFocus)) {
    std::wstring line = L"scheduled=";
    line += FormatFocusSnapshotForLog(scheduledFocus);
    line += L", current=";
    line += FormatFocusSnapshotForLog(CaptureFocusSnapshot(m_pFocusContext));
    SrfTsfPerfLog(L"focus-context.defer-clear.skip-stale", line.c_str());
    return;
  }

  ITfDocumentMgr* focusedDocument = nullptr;
  if (m_pThreadMgr && SUCCEEDED(m_pThreadMgr->GetFocus(&focusedDocument)) && focusedDocument) {
    ITfContext* focusedContext = nullptr;
    const HRESULT topHr = focusedDocument->GetTop(&focusedContext);
    focusedDocument->Release();
    if (SUCCEEDED(topHr) && focusedContext) {
      const uintptr_t focusedCookie = reinterpret_cast<uintptr_t>(focusedContext);
      if (scheduledFocus.contextCookie == 0 || focusedCookie == scheduledFocus.contextCookie) {
        SrfTsfPerfLog(L"focus-context.defer-clear.skip", L"TSF focus restored");
      } else {
        SrfTsfDiagnosticLog(L"focus-context.defer-clear.reconcile",
                            L"TSF focus moved before timer callback");
        RequestCancelCompositionOnFocusLoss();
        SetFocusContext(focusedContext);
        ApplyAppOptionsForFocusedContext(false);
      }
      focusedContext->Release();
      return;
    }
  }

  SrfTsfDiagnosticLog(L"focus-context.defer-clear.fire", L"pid=0 grace elapsed");
  RequestCancelCompositionOnFocusLoss();
  SetFocusContext(nullptr);
}

void CSrfTip::OnDeferredCandidateRefreshTimer() {
  const std::wstring scheduledReading = m_deferredRefreshReading;
  CancelDeferredCandidateRefresh();
  SrfTsfPerfLog(L"candidate-refresh.defer-fire", L"timer fired");

  // The reading has been cleared; no refresh is needed.
  if (m_compositionState.reading.empty()) return;
  if (!scheduledReading.empty() && scheduledReading != m_compositionState.reading) {
    std::wstring line = L"scheduled=" + ShortenForLog(scheduledReading, 24);
    line += L", current=";
    line += ShortenForLog(m_compositionState.reading, 24);
    SrfTsfPerfLog(L"candidate-refresh.defer-stale", line.c_str());
    if (m_candidateView.candidates.empty()) ScheduleDeferredCandidateRefresh();
    return;
  }
  // Candidates already arrived for the active reading; partial first batches and stale
  // retained rows still get a fill-in refresh.
  if (!m_candidateView.candidates.empty() &&
      m_candidateView.candidatesReading == m_compositionState.reading &&
      !CurrentCandidatesPartial()) {
    return;
  }

  // Request an async edit session to refresh candidates.
  ITfContext* context = m_compositionState.pCompositionContext
                            ? m_compositionState.pCompositionContext
                            : m_pFocusContext;
  if (!context || m_tid == 0) return;

  CEditSessionDeferredRefresh* edit = new (std::nothrow) CEditSessionDeferredRefresh(this);
  if (!edit) return;

  HRESULT hrSession = E_FAIL;
  HRESULT hr = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  edit->Release();
  wchar_t buf[160] = {};
  swprintf_s(buf, L"RequestEditSession hr=0x%08lX session=0x%08lX", static_cast<unsigned long>(hr),
             static_cast<unsigned long>(hrSession));
  SrfTsfPerfLog(L"candidate-refresh.defer-edit", buf);

  // If the async edit session also fails while recovery is active, try again later.
  if (FAILED(hr) || FAILED(hrSession)) {
    const SrfEngineState state = SrfTip_GetEngineState();
    if (state == SrfEngineState::Loading ||
        (state == SrfEngineState::Failed && m_config.engine.retryOnFailure)) {
      ScheduleDeferredCandidateRefresh();
    }
  }
}
