/// 延迟候选刷新编辑会话：在定时器触发后以 ASYNC 方式请求编辑会话，
/// 重新查询候选列表并刷新 UI。
class CEditSessionDeferredRefresh final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;

 public:
  explicit CEditSessionDeferredRefresh(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) m_tip->AddRef();
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
    if (m_tip->m_reading.empty()) return S_OK;
    m_tip->RefreshCandidatesAsync();
    m_tip->UpdateCandidateWindow(ec);
    return S_OK;
  }
};

/// Re-samples the TSF text/caret rectangle after a DPI, monitor, or window-mode
/// transition. Candidate ranking is intentionally left untouched.
class CEditSessionCandidateAnchorRefresh final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  SrfFocusSnapshot m_focus = {};

 public:
  CEditSessionCandidateAnchorRefresh(CSrfTip* tip,
                                     SrfFocusSnapshot focus)
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
  STDMETHODIMP_(ULONG) AddRef() override {
    return InterlockedIncrement(&m_cRef);
  }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    m_tip->m_candidateAnchorRefreshEditPending = false;
    m_tip->m_candidateAnchorRefreshRequestTick = 0;
    if (!m_tip->FocusSnapshotMatches(m_focus)) return S_OK;
    if (m_tip->m_candidateUi) {
      m_tip->m_candidateUi->OnCandidateAnchorRefreshed();
    }
    if (m_tip->m_reading.empty() || !m_tip->m_status.composing) return S_OK;
    m_tip->UpdateCandidateWindow(ec);
    return S_OK;
  }
};

/// 后台候选查询完成后，通过异步 EditSession 回到 TSF 线程应用结果。
class CEditSessionApplyAsyncCandidates final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;

 public:
  explicit CEditSessionApplyAsyncCandidates(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) m_tip->AddRef();
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
    m_tip->ApplyAsyncCandidateResult(ec);
    return S_OK;
  }
};

CSrfTip::CSrfTip() {
  m_candidateUi = new (std::nothrow) CSrfCandidateListUIElement(this);
  LoadConfiguration();
  SyncStatusModel();
  InterlockedIncrement(&g_cSrfTipObjects);
}

CSrfTip::~CSrfTip() {
  _UnadviseSinks();
  if (m_deferredTimerHwnd) {
    DestroyWindow(m_deferredTimerHwnd);
    m_deferredTimerHwnd = nullptr;
  }
  if (m_candidateUi) {
    m_candidateUi->End();
    m_candidateUi->Release();
    m_candidateUi = nullptr;
  }
  m_notificationWindow.Destroy();
  InterlockedDecrement(&g_cSrfTipObjects);
}

STDMETHODIMP CSrfTip::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (riid == IID_IUnknown || riid == IID_ITfTextInputProcessor ||
      riid == IID_ITfTextInputProcessorEx) {
    *ppv = static_cast<ITfTextInputProcessorEx*>(this);
  } else if (riid == IID_ITfDisplayAttributeProvider) {
    *ppv = static_cast<ITfDisplayAttributeProvider*>(this);
  } else if (riid == IID_ITfFunctionProvider) {
    *ppv = static_cast<ITfFunctionProvider*>(this);
  } else if (riid == IID_ITfFnConfigure) {
    *ppv = static_cast<ITfFnConfigure*>(this);
  } else {
    return E_NOINTERFACE;
  }
  AddRef();
  return S_OK;
}

STDMETHODIMP_(ULONG) CSrfTip::AddRef() { return InterlockedIncrement(&m_cRef); }

STDMETHODIMP_(ULONG) CSrfTip::Release() {
  const ULONG count = InterlockedDecrement(&m_cRef);
  if (count == 0) delete this;
  return count;
}

STDMETHODIMP CSrfTip::Activate(ITfThreadMgr* ptim, TfClientId tid) { return ActivateEx(ptim, tid, 0); }

STDMETHODIMP CSrfTip::Deactivate() { return _UnadviseSinks(); }

STDMETHODIMP CSrfTip::GetType(GUID* pguid) {
  if (!pguid) return E_POINTER;
  *pguid = CLSID_SrfTsfTip;
  return S_OK;
}

STDMETHODIMP CSrfTip::GetDescription(BSTR* pbstrDesc) {
  if (!pbstrDesc) return E_POINTER;
  *pbstrDesc = SysAllocString(L"\u5f00\u5fc3\u8f93\u5165\u6cd5");
  return *pbstrDesc ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfTip::GetFunction(REFGUID rguid, REFIID riid, IUnknown** ppunk) {
  if (!ppunk) return E_POINTER;
  *ppunk = nullptr;
  if (!IsEqualGUID(rguid, GUID_NULL) && !IsEqualGUID(rguid, GUID_FUNCTION_SRF_CONFIGURE) &&
      !IsEqualGUID(rguid, IID_ITfFnConfigure)) {
    return E_INVALIDARG;
  }
  return QueryInterface(riid, reinterpret_cast<void**>(ppunk));
}

STDMETHODIMP CSrfTip::GetDisplayName(BSTR* pbstrName) {
  if (!pbstrName) return E_POINTER;
  *pbstrName = SysAllocString(L"\u8f93\u5165\u6cd5\u8bbe\u7f6e");
  return *pbstrName ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfTip::Show(HWND hwndParent, LANGID /*langid*/, REFGUID /*rguidProfile*/) {
  return LaunchSettingsHelper(hwndParent);
}

STDMETHODIMP CSrfTip::ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD dwFlags) {
  if (!ptim) return E_INVALIDARG;
  const ULONGLONG activateStart = GetTickCount64();
  ULONGLONG stageStart = activateStart;

  // TF_TMAE_UIELEMENTENABLEDONLY (0x4) 表示宿主要求仅通过 UIElement 接口交互，
  // 不应弹出独立候选窗口（常见于游戏、沉浸式全屏应用）。
  m_uiLessMode = (dwFlags & 0x4) != 0;  // TF_TMAE_UIELEMENTENABLEDONLY
  LogActivationEnvironment(dwFlags);

  HRESULT hr = _UnadviseSinks();
  if (FAILED(hr)) return hr;
  DebugLogPerfMs(L"ActivateEx/unadvise", stageStart);
  stageStart = GetTickCount64();

  m_pThreadMgr = ptim;
  m_pThreadMgr->AddRef();
  m_tid = tid;

  hr = m_pThreadMgr->QueryInterface(IID_ITfKeystrokeMgr, reinterpret_cast<void**>(&m_pKeystrokeMgr));
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }

  hr = m_pThreadMgr->QueryInterface(IID_ITfSource, reinterpret_cast<void**>(&m_pSource));
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }

  hr = m_pThreadMgr->QueryInterface(IID_ITfCompartmentMgr, reinterpret_cast<void**>(&m_pCompartmentMgr));
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }

  m_pCompSink = new (std::nothrow) CCompositionSink(this);
  m_pKeySink = new (std::nothrow) CKeyEventSink(this);
  m_pThreadMgrSink = new (std::nothrow) CThreadMgrEventSink(this);
  if (!m_pCompSink || !m_pKeySink || !m_pThreadMgrSink) {
    _UnadviseSinks();
    return E_OUTOFMEMORY;
  }

  hr = m_pSource->AdviseSink(IID_ITfThreadMgrEventSink,
                             static_cast<IUnknown*>(static_cast<ITfThreadMgrEventSink*>(m_pThreadMgrSink)),
                             &m_dwThreadMgrSinkCookie);
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }

  hr = m_pKeystrokeMgr->AdviseKeyEventSink(tid, static_cast<ITfKeyEventSink*>(m_pKeySink), TRUE);
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }
  DebugLogPerfMs(L"ActivateEx/advise-sinks", stageStart);
  stageStart = GetTickCount64();

  EnsureDisplayAttributeAtom();
  LoadConfiguration();
  LoadCompartmentState();
  ApplyAppOptionsForFocusedContext(false);
  SyncCompartmentState();
  RebuildContextModel();
  SyncStatusModel();
  BeginPreservedKeyGuardAfterActivation();
  EnsureTrayHelperRunningAsync();
  DebugLogPerfMs(L"ActivateEx/app-options+state", stageStart);
  stageStart = GetTickCount64();

  hr = RegisterPreservedKeys();
  if (FAILED(hr)) {
    _UnadviseSinks();
    return hr;
  }
  DebugLogPerfMs(L"ActivateEx/register-preserved-keys", stageStart);

  // 在切到本 IME 线程后即异步加载 Rust 引擎，避免首键才触发 warmup 造成首击无候选。
  if (!ShouldForceAsciiForCompatibility()) {
    SrfTip_WarmupEngineAsync();
    SrfTip_PrewarmSingleLetterLookupCacheAsync();
  }
  if (m_candidateUi) {
    const ULONGLONG prepStart = GetTickCount64();
    m_candidateUi->PrepareWindowResources();
    DebugLogPerfMs(L"ActivateEx/prewarm-candidate-resources", prepStart);
  }
  DebugLogPerfMs(L"ActivateEx/total", activateStart);

  return S_OK;
}

STDMETHODIMP CSrfTip::EnumDisplayAttributeInfo(IEnumTfDisplayAttributeInfo** ppEnum) {
  if (!ppEnum) return E_POINTER;
  *ppEnum = new (std::nothrow) CSrfEnumDisplayAttributeInfo();
  return *ppEnum ? S_OK : E_OUTOFMEMORY;
}

STDMETHODIMP CSrfTip::GetDisplayAttributeInfo(REFGUID guid, ITfDisplayAttributeInfo** ppInfo) {
  if (!ppInfo) return E_POINTER;
  *ppInfo = nullptr;
  if (!IsEqualGUID(guid, GUID_DISPLAY_ATTRIBUTE_SRF_INPUT)) return E_INVALIDARG;
  *ppInfo = new (std::nothrow) CSrfDisplayAttributeInfo();
  return *ppInfo ? S_OK : E_OUTOFMEMORY;
}

void CSrfTip::ClearFocusBoundCandidateState(const wchar_t* reason) {
  m_candidateLookupSerial.fetch_add(1, std::memory_order_acq_rel);
  CancelDeferredCandidateRefresh();
  m_candidates.clear();
  m_candidateMeta.clear();
  m_candidatesReading.clear();
  SetCandidateViewState(SrfCandidateViewState::Empty, reason ? reason : L"focus-bound-clear");
  m_candSel = 0;
  m_candPage = 0;
  m_hasLastCandidateRect = false;
  m_preserveCandidateAnchorReading.clear();
  m_hasStickyCandidateRect = false;
  m_stickyCandidateRect = {};
  m_stickyCandidateAnchorQuality = 0;
  m_lastCandidateAnchorSource.clear();
  m_lastCandidateAnchorQuality = 0;
  m_lastCandidateAnchorSourceSwitchTick = 0;
  m_candidateAnchorSourceSwitchCount = 0;
  InvalidateCandidatePageLayoutCache();
  m_engineHealthNotifiedThisComposition = false;

  {
    std::lock_guard<std::mutex> guard(m_asyncCandidateMutex);
    m_asyncCandidatePending = false;
    m_asyncCandidatePendingSerial = 0;
    m_asyncCandidatePendingReading.clear();
    m_asyncCandidatePendingItems.clear();
    m_asyncCandidatePendingMeta.clear();
    m_asyncCandidatePendingRequestTick = 0;
    m_asyncCandidatePendingFocus = {};
  }
  {
    std::lock_guard<std::mutex> guard(m_candidateWorkerMutex);
    m_candidateWorkerHasRequest = false;
    m_candidateWorkerRequestReading.clear();
    m_candidateWorkerRequestSerial = 0;
    m_candidateWorkerNotifyHwnd = nullptr;
    m_candidateWorkerRequestTick = 0;
    m_candidateWorkerRequestFocus = {};
  }

  RebuildContextModel();
  SyncStatusModel();
  if (m_candidateUi) m_candidateUi->End();
  m_notificationWindow.Hide();

  std::wstring line = L"reason=";
  line += reason ? reason : L"(none)";
  line += L", focus=";
  line += FormatFocusSnapshotForLog(CaptureFocusSnapshot(m_pFocusContext));
  line += L", reading=";
  line += ShortenForLog(m_reading, 24);
  SrfTsfDiagnosticLog(L"focus-bound-state.clear", line.c_str());
}

void CSrfTip::SetFocusContext(ITfContext* pic) {
  CancelDeferredFocusContextClear();
  if (m_pFocusContext == pic) return;
  const SrfFocusSnapshot oldFocus = CaptureFocusSnapshot(m_pFocusContext);
  ++m_focusGeneration;
  if (m_pFocusContext) m_pFocusContext->Release();
  m_pFocusContext = pic;
  if (m_pFocusContext) m_pFocusContext->AddRef();
  m_cachedFocusedHwnd = nullptr;
  m_cachedFocusedProcessId = 0;
  m_cachedFocusedProcessName.clear();
  ClearFocusBoundCandidateState(L"focus-context-change");
  InvalidateHotPathStateCache();
  const SrfFocusSnapshot newFocus = CaptureFocusSnapshot(m_pFocusContext);
  std::wstring line = L"old=";
  line += FormatFocusSnapshotForLog(oldFocus);
  line += L", new=";
  line += FormatFocusSnapshotForLog(newFocus);
  SrfTsfDiagnosticLog(L"focus-context.changed", line.c_str());
}

void CSrfTip::BeginPreservedKeyGuardAfterActivation() {
  m_lastActivationTick = GetTickCount64();
  m_ignoreImeToggleUntilModifiersReleased = HasCtrlOrShiftDown();
}

void CSrfTip::LogActivationEnvironment(DWORD dwFlags) {
  HWND hwnd = GetForegroundWindow();
  HWND root = hwnd ? GetAncestor(hwnd, GA_ROOT) : nullptr;
  if (root && IsWindowVisible(root)) hwnd = root;

  DWORD processId = 0;
  if (hwnd) (void)GetWindowThreadProcessId(hwnd, &processId);

  bool appContainerKnown = false;
  const bool appContainer = CurrentProcessAppContainerState(&appContainerKnown);

  wchar_t flags[32] = {};
  swprintf_s(flags, L"0x%08lX", static_cast<unsigned long>(dwFlags));

  std::wstring processName = m_activeAppName;
  if (processName.empty()) processName = FocusedProcessName();

  std::wstring line = L"flags=";
  line += flags;
  line += L", uiless=";
  line += m_uiLessMode ? L"1" : L"0";
  line += L", appcontainer=";
  line += appContainerKnown ? (appContainer ? L"1" : L"0") : L"unknown";
  line += L", integrity=";
  line += CurrentProcessIntegrityLabel();
  line += L", pid=";
  line += processId == 0 ? L"0" : std::to_wstring(processId);
  line += L", process=";
  line += SanitizeDiagnosticValue(processName);
  line += L", class=";
  line += SanitizeDiagnosticValue(WindowClassName(hwnd));
  line += L", sensitive=";
  line += m_sensitiveInputActive ? L"1" : L"0";
  line += L", uielement=1";
  line += L", immersive=1";
  line += L", secure=1";
  SrfTsfDiagnosticLog(L"activate.environment", line.c_str());
}

bool CSrfTip::ShouldSuppressImeTogglePreservedKey() {
  const bool modifiersDown = HasCtrlOrShiftDown();

  if (m_ignoreImeToggleUntilModifiersReleased) {
    if (modifiersDown) return true;
    m_ignoreImeToggleUntilModifiersReleased = false;
  }

  if (!modifiersDown || m_lastActivationTick == 0) return false;
  return GetTickCount64() - m_lastActivationTick <= kImeTogglePreservedKeyGuardMs;
}

void CSrfTip::ApplyAppOptionsForFocusedContext(bool showNotification) {
  const ULONGLONG start = GetTickCount64();
  ULONGLONG stageStart = start;
  InvalidateHotPathStateCache();
  const bool prevImeOpen = m_imeOpen;
  const bool prevInlinePreedit = m_uiStyle.inlinePreedit;
  const bool prevEnhancedPosition = m_uiStyle.enhancedPosition;
  const std::wstring prevAppName = m_activeAppName;

  RefreshRuntimeConfig();
  m_activeAppName = FocusedProcessName();
  m_uiStyle = ResolveUiStyleForApp(m_activeAppName);
  InvalidateCandidatePageLayoutCache();
  RefreshCompatibilityState();
  m_lastCompatibilityRefreshTick = GetTickCount64();
  if (ShouldUseExternalCandidateOverlay()) {
    GetExternalCandidateOverlayClient().Prewarm();
  }
  DebugLogPerfMs(L"AppOptions/load+resolve-app", stageStart);
  stageStart = GetTickCount64();

  bool usedAppOptions = false;
  if (const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName)) {
    if (options->hasAsciiMode) {
      m_imeOpen = !options->asciiMode;
      usedAppOptions = true;
    }
  } else if (m_config.globalAscii) {
    m_imeOpen = !LoadGlobalAsciiState();
  }
  if (prevImeOpen != m_imeOpen) {
    ApplyDefaultPunctuationForImeMode();
  }
  DebugLogPerfMs(L"AppOptions/apply-ime-mode", stageStart);
  stageStart = GetTickCount64();

  SyncCompartmentState();
  SyncStatusModel();
  DebugLogPerfMs(L"AppOptions/sync-status", stageStart);
  stageStart = GetTickCount64();

  if (showNotification && !ShouldHideUiForCompatibility() &&
      m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
    if (prevAppName != m_activeAppName || prevImeOpen != m_imeOpen ||
        prevInlinePreedit != m_uiStyle.inlinePreedit ||
        prevEnhancedPosition != m_uiStyle.enhancedPosition) {
      std::wstring text = m_activeAppName.empty() ? L"App defaults updated" : (m_activeAppName + L": ");
      text += m_imeOpen ? L"Chinese" : L"English";
      if (usedAppOptions) text += m_uiStyle.inlinePreedit ? L", inline" : L", popup";
      ShowNotification(SrfNotificationKind::AppOptions, text);
    }
  }

  if (!m_reading.empty()) {
    RebuildContextModel();
    RedrawCandidateUi();
  }
  DebugLogPerfMs(L"AppOptions/rebuild+redraw", stageStart);
  DebugLogPerfMs(L"AppOptions/total", start);
}

void CSrfTip::RequestCancelCompositionOnFocusLoss() {
  if (!m_pComposition || !m_pCompositionContext || m_tid == 0) return;
  ++m_focusCancelSequence;
  const uint64_t seq = m_focusCancelSequence;
  const uint64_t gen = m_compositionGeneration;
  CEditSessionCancelFocus* edit = new (std::nothrow) CEditSessionCancelFocus(this, gen, seq);
  if (!edit) return;

  HRESULT hrSession = E_FAIL;
  const ULONGLONG requestStart = GetTickCount64();
  const HRESULT hr =
      m_pCompositionContext->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  const ULONGLONG requestElapsed = GetTickCount64() - requestStart;
  edit->Release();
  if (SrfTsfDebugTraceEnabled()) {
    wchar_t buf[176] = {};
    swprintf_s(buf,
               L"RequestCancelCompositionOnFocusLoss async wait=%llums hr=0x%08lX session=0x%08lX seq=%llu gen=%llu",
               static_cast<unsigned long long>(requestElapsed),
               static_cast<unsigned long>(hr), static_cast<unsigned long>(hrSession),
               static_cast<unsigned long long>(seq), static_cast<unsigned long long>(gen));
    SrfTsfDebugLog(buf);
  }
}

void CSrfTip::HandleFocusLossCancelEditSession(TfEditCookie ec, uint64_t generation, uint64_t cancelSequence) {
  if (cancelSequence != m_focusCancelSequence) {
    if (SrfTsfDebugTraceEnabled()) {
      wchar_t buf[120] = {};
      swprintf_s(buf, L"HandleFocusLossCancel stale seq (got %llu cur %llu)",
                 static_cast<unsigned long long>(cancelSequence),
                 static_cast<unsigned long long>(m_focusCancelSequence));
      SrfTsfDebugLog(buf);
    }
    return;
  }
  if (generation != m_compositionGeneration) {
    if (SrfTsfDebugTraceEnabled()) {
      SrfTsfDebugLog(L"HandleFocusLossCancel stale generation");
    }
    return;
  }
  if (!m_pComposition) return;
  if (SrfTsfDebugTraceEnabled()) {
    SrfTsfDebugLog(L"HandleFocusLossCancel -> CancelCompositionEdit");
  }
  CancelCompositionEdit(ec);
}

void CSrfTip::CancelCompositionEdit(TfEditCookie ec) {
  if (!m_pComposition) {
    ReleaseCompositionState();
    return;
  }

  if (m_candidateUi) m_candidateUi->End();

  ITfRange* range = nullptr;
  if (SUCCEEDED(m_pComposition->GetRange(&range)) && range) {
    (void)range->SetText(ec, 0, L"", 0);
    range->Release();
  }
  const HRESULT endHr = m_pComposition->EndComposition(ec);
  if (SUCCEEDED(endHr)) {
    ClearCompositionBufferState();
    if (SrfTsfDebugTraceEnabled()) {
      SrfTsfDebugLog(L"CancelCompositionEdit EndComposition OK + ClearCompositionBufferState");
    }
  } else {
    // 仍清本地缓冲，减轻「幽灵拼音」；ITfComposition 由 OnCompositionTerminated 或后续路径释放
    ClearCompositionBufferState();
    if (SrfTsfDebugTraceEnabled()) {
      wchar_t buf[96] = {};
      swprintf_s(buf, L"CancelCompositionEdit EndComposition FAILED hr=0x%08lX, cleared buffer anyway",
                 static_cast<unsigned long>(endHr));
      SrfTsfDebugLog(buf);
    }
  }
}

HRESULT CSrfTip::RegisterPreservedKeys() {
  if (!m_pKeystrokeMgr || m_tid == 0) return E_FAIL;

  struct PreservedKeyEntry {
    const GUID* guid;
    TF_PRESERVEDKEY key;
    const wchar_t* description;
  };

  std::vector<PreservedKeyEntry> entries;
  if (m_config.input.cnEnHotkey == 0 || m_config.input.cnEnHotkey == 1) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_IME, {VK_SHIFT, TF_MOD_CONTROL},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Chinese/English"});
  }
  if (m_config.input.cnEnHotkey == 0 || m_config.input.cnEnHotkey == 2) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_IME_CTRL_SPACE, {VK_SPACE, TF_MOD_CONTROL},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Chinese/English (Ctrl+Space)"});
  }
  if (m_config.input.fullShapeHotkeyEnabled) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_FULLSHAPE, {VK_SPACE, TF_MOD_SHIFT},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Full Shape"});
  }
  if (m_config.input.punctHotkeyEnabled) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_PUNCT, {VK_OEM_PERIOD, TF_MOD_CONTROL},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Chinese Punctuation"});
  }
  if (m_config.input.fuzzyHotkeyEnabled) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_FUZZY, {'F', TF_MOD_CONTROL | TF_MOD_SHIFT},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Fuzzy Pinyin"});
  }
  if (m_config.input.doubleHotkeyEnabled) {
    entries.push_back({&GUID_PRESERVEDKEY_SRF_TOGGLE_DOUBLE, {'D', TF_MOD_CONTROL | TF_MOD_SHIFT},
                       L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Toggle Double Pinyin"});
  }

  for (const auto& entry : entries) {
    const HRESULT hr =
        m_pKeystrokeMgr->PreserveKey(m_tid, *entry.guid, &entry.key, entry.description,
                                     static_cast<ULONG>(wcslen(entry.description)));
    if (FAILED(hr) && hr != TF_E_ALREADY_EXISTS) return hr;
  }

  m_hasRegisteredScreenshotKey = false;
  if (m_config.screenshot.hotkey.enabled && m_config.screenshot.hotkey.vk != 0) {
    const TF_PRESERVEDKEY key = {m_config.screenshot.hotkey.vk,
                                 m_config.screenshot.hotkey.modifiers};
    const wchar_t description[] = L"\u5f00\u5fc3\u8f93\u5165\u6cd5 Screenshot";
    const HRESULT hr = m_pKeystrokeMgr->PreserveKey(
        m_tid, GUID_PRESERVEDKEY_SRF_SCREENSHOT, &key, description,
        static_cast<ULONG>(wcslen(description)));
    if (FAILED(hr) && hr != TF_E_ALREADY_EXISTS) return hr;
    m_registeredScreenshotKey = key;
    m_hasRegisteredScreenshotKey = true;
  }

  return S_OK;
}

void CSrfTip::UnregisterPreservedKeys() {
  if (!m_pKeystrokeMgr) return;

  const std::pair<const GUID*, TF_PRESERVEDKEY> entries[] = {
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_IME, {VK_SHIFT, TF_MOD_CONTROL}},
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_IME_CTRL_SPACE, {VK_SPACE, TF_MOD_CONTROL}},
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_FULLSHAPE, {VK_SPACE, TF_MOD_SHIFT}},
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_PUNCT, {VK_OEM_PERIOD, TF_MOD_CONTROL}},
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_FUZZY, {'F', TF_MOD_CONTROL | TF_MOD_SHIFT}},
      {&GUID_PRESERVEDKEY_SRF_TOGGLE_DOUBLE, {'D', TF_MOD_CONTROL | TF_MOD_SHIFT}},
  };

  for (const auto& entry : entries) {
    (void)m_pKeystrokeMgr->UnpreserveKey(*entry.first, &entry.second);
  }
  if (m_hasRegisteredScreenshotKey) {
    (void)m_pKeystrokeMgr->UnpreserveKey(GUID_PRESERVEDKEY_SRF_SCREENSHOT,
                                         &m_registeredScreenshotKey);
    m_hasRegisteredScreenshotKey = false;
    m_registeredScreenshotKey = {};
  }
}

void CSrfTip::ClearCompositionBufferState() {
  ++m_compositionGeneration;
  m_candidateLookupSerial.fetch_add(1, std::memory_order_acq_rel);
  CancelCandidateWindowAnchorRefreshRetry();
  m_reading.clear();
  m_readingCursor = 0;
  m_candidates.clear();
  m_candidateMeta.clear();
  m_candidatesReading.clear();
  SetCandidateViewState(SrfCandidateViewState::Empty, L"composition-clear");
  m_candSel = 0;
  m_candPage = 0;
  m_hasLastCandidateRect = false;
  m_preserveCandidateAnchorReading.clear();
  m_hasStickyCandidateRect = false;
  m_stickyCandidateRect = {};
  m_stickyCandidateAnchorQuality = 0;
  m_lastCandidateAnchorSource.clear();
  m_lastCandidateAnchorQuality = 0;
  m_lastCandidateAnchorSourceSwitchTick = 0;
  m_candidateAnchorSourceSwitchCount = 0;
  InvalidateCandidatePageLayoutCache();
  m_engineHealthNotifiedThisComposition = false;
  m_userPhraseComposeActive = false;
  m_userPhraseComposeValid = false;
  m_userPhraseComposeOriginalReading.clear();
  m_userPhraseComposeCommitted.clear();
  ApplyRustModeFlags();
  m_context.Clear();
  {
    std::lock_guard<std::mutex> guard(m_asyncCandidateMutex);
    m_asyncCandidatePending = false;
    m_asyncCandidatePendingReading.clear();
    m_asyncCandidatePendingItems.clear();
    m_asyncCandidatePendingMeta.clear();
    m_asyncCandidatePendingRequestTick = 0;
    m_asyncCandidatePendingFocus = {};
  }
  {
    std::lock_guard<std::mutex> guard(m_candidateWorkerMutex);
    m_candidateWorkerHasRequest = false;
    m_candidateWorkerRequestReading.clear();
    m_candidateWorkerRequestSerial = 0;
    m_candidateWorkerNotifyHwnd = nullptr;
    m_candidateWorkerRequestTick = 0;
    m_candidateWorkerRequestFocus = {};
  }
  SyncStatusModel();
  if (m_candidateUi) m_candidateUi->End();
  if (SrfTsfDebugTraceEnabled()) {
    SrfTsfDebugLog(L"ClearCompositionBufferState");
  }
  ApplyPendingRuntimeConfigIfSafe();
}

void CSrfTip::ReleaseCompositionObjects() {
  if (m_pComposition) {
    m_pComposition->Release();
    m_pComposition = nullptr;
  }
  if (m_pCompositionContext) {
    m_pCompositionContext->Release();
    m_pCompositionContext = nullptr;
  }
}

void CSrfTip::ReleaseCompositionState() {
  ClearCompositionBufferState();
  ReleaseCompositionObjects();
}
